//! Draws: the engine's chain, stalls, claims while a draw waits, stale
//! consoles, delivery, more prizes than people.

use super::*;

/// [`wait_for`] for slow conditions (stalls take a round period plus the timeout).
async fn wait_long<F: Fn(&Full) -> bool>(app: &AppState, id: &str, f: F) -> Full {
    wait_within(app, id, Duration::from_secs(20), f).await
}

/// An app whose drand relay is unreachable: every draw stalls on its round
/// (quicknet, 3 s lead, gives up 1 s after the round is due).
async fn unreachable_drand_app() -> AppState {
    let mut cfg = Config::for_tests();
    cfg.drand_chain = rafi_raffle::drand::chains::QUICKNET_HASH.to_string();
    let drand = rafi_engine::drand::DrandConfig {
        relays: vec!["http://127.0.0.1:9".into()],
        mock: false,
        lead_secs: 3,
        timeout_secs: 1,
    };
    app_with_engine(cfg, drand, LiveUpdates::polling()).await
}

/// The app decides the chain raffles commit to: an engine on another chain
/// is refused before anything is frozen.
#[tokio::test]
async fn an_engine_on_another_chain_is_refused_before_freezing() {
    let mut cfg = Config::for_tests();
    cfg.drand_chain = rafi_raffle::drand::chains::QUICKNET_HASH.to_string();
    let app = app_with_engine(cfg, mock_drand(), LiveUpdates::polling()).await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    add_one(&app, ACTOR, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let err = service::start_draw(&app, ACTOR, &raffle.id)
        .await
        .unwrap_err();
    assert!(matches!(err, service::Error::Refused(_)), "{err}");
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Open, "nothing was frozen");
    assert!(r.manifest.is_none(), "nothing was committed");
}

/// Without a raffle engine configured, nothing can start.
#[tokio::test]
async fn draws_need_restate() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = AppState::new(cfg).await.unwrap();
    assert!(app.engine.is_none());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    add_one(&app, ACTOR, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let err = service::start_draw(&app, ACTOR, &raffle.id)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no raffle engine"), "{err}");
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Open,
        "nothing was frozen"
    );
}

/// A draw whose round never arrives in time stalls on that round: retrying
/// keeps the round, and the only exit (abandon) is logged for the verifier.
#[tokio::test]
async fn stalled_draw_keeps_its_round() {
    use rafi_raffle::Event;

    let app = unreachable_drand_app().await;
    let raffle = setup_open(&app, &[("Prize", 3)]).await;
    for i in 0..4 {
        add_one(&app, ACTOR, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    async fn draw_cmd(
        app: &AppState,
        raffle_id: &str,
        item: &str,
        id: &str,
    ) -> crate::service::Result<()> {
        service::command(app, ACTOR, raffle_id, DrawRequest { item: item.into() }, id).await
    }
    let draw = |item: &str, id: &str| {
        let (item, id) = (item.to_owned(), id.to_owned());
        let (app, raffle_id) = (app.clone(), raffle.id.clone());
        async move { draw_cmd(&app, &raffle_id, &item, &id).await }
    };
    let stalls = |r: &Full| {
        r.drawn
            .as_ref()
            .map(|s| {
                s.events()
                    .iter()
                    .filter(|e| matches!(e, Event::Stalled { .. }))
                    .count()
            })
            .unwrap_or_default()
    };

    draw(items[0].as_str(), "d1").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 1).await;
    let pending = r
        .drawn
        .as_ref()
        .unwrap()
        .pending()
        .cloned()
        .expect("still pending");

    // Another item cannot start; this one retries the same round.
    assert!(draw(items[1].as_str(), "d2").await.is_err());
    draw(items[0].as_str(), "d3").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 2).await;
    let s = r.drawn.as_ref().unwrap();
    assert_eq!(s.pending(), Some(&pending), "same draw, same round");
    let announces = s
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Announced { .. }))
        .count();
    assert_eq!(announces, 1);

    // The organizer abandons it: logged, and the item can be drawn again.
    service::command(
        &app,
        ACTOR,
        &raffle.id,
        Decision::Abandon {
            item: items[0].clone(),
        },
        "a1",
    )
    .await
    .unwrap();
    // A new draw of the item announces the same draw number on a current round.
    draw(items[0].as_str(), "d4").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 3).await;
    let next = r.drawn.as_ref().unwrap().pending().cloned().unwrap();
    assert_eq!(next.k, pending.k, "the abandoned draw's number is reused");
    assert!(
        next.round >= pending.round,
        "a new draw commits to a current round"
    );

    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_raffle::drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    let abandoned: Vec<_> = report.abandoned.iter().map(|c| c.round).collect();
    assert_eq!(abandoned, [pending.round]);
    assert!(rafi_raffle::verify_public(&data.manifest, &data.manifest_hash, &data.events).ok());
}

/// While a draw waits for its round, "not here" is refused; "claimed" is
/// accepted, and the engine applies it once that draw settles.
#[tokio::test]
async fn only_claiming_passes_a_waiting_draw() {
    let drand = rafi_engine::drand::DrandConfig {
        lead_secs: 3,
        ..mock_drand()
    };
    let app = app_with_engine(Config::for_tests(), drand, LiveUpdates::polling()).await;
    let raffle = setup_open(&app, &[("Prize", 2)]).await;
    for i in 0..3 {
        add_one(&app, ACTOR, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let draw = |item: &str, id: &str| {
        let (item, id) = (item.to_owned(), id.to_owned());
        let (app, raffle_id) = (app.clone(), raffle.id.clone());
        async move {
            let request = DrawRequest { item: item.into() };
            service::command(&app, ACTOR, &raffle_id, request, &id).await
        }
    };
    let resolve = |resolution, id: &str| {
        let id = id.to_owned();
        let (app, raffle_id, item) = (app.clone(), raffle.id.clone(), items[0].clone());
        async move {
            let decision = Decision::Resolve {
                k: None,
                item,
                resolution,
            };
            service::command(&app, ACTOR, &raffle_id, decision, &id).await
        }
    };

    draw(items[0].as_str(), "d1").await.unwrap();
    wait_long(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&items[0]).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    draw(items[1].as_str(), "d2").await.unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.pending().as_ref().is_some_and(|p| p.item == items[1]))
    })
    .await;
    assert!(resolve(Resolution::NotHere, "nh").await.is_err());
    resolve(Resolution::Claimed, "cl").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&items[0]).unwrap().status() == ItemStatus::Claimed)
    })
    .await;
    assert_eq!(
        r.drawn.as_ref().unwrap().item(&items[1]).unwrap().status(),
        ItemStatus::Drawn,
        "applied after the draw settled"
    );
}

/// "All prizes delivered" puts every held winner on the non-winner clock.
#[tokio::test]
async fn all_prizes_delivered() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1), ("Mug", 1)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        add_one(&app, ACTOR, &raffle.id, name, None).await.unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let w1 = draw_and_claim(&app, &raffle.id, &items[0]).await;
    let w2 = draw_and_claim(&app, &raffle.id, &items[1]).await;

    let refused = service::deliver_all(&app, ACTOR, &raffle.id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::NotOver(
                service::Unfinished::DeliverAll
            )))
        ),
        "not before the finish: {refused:?}"
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "All prizes can be marked delivered together once the raffle has finished. One winner's prize can be marked delivered any time after they claimed it."
    );
    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.retention.held_winners, 2);
    assert!(admin.actions.deliver_all);

    assert_eq!(
        service::deliver_all(&app, ACTOR, &raffle.id).await.unwrap(),
        2
    );
    for w in [&w1, &w2] {
        assert!(
            entrants::get(&app.db, &raffle.id, w)
                .await
                .unwrap()
                .unwrap()
                .delivered_at
                .is_some()
        );
    }
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.retention.held_winners, 0);
    assert!(!admin.actions.deliver_all);
    assert!(admin.entrants.iter().all(|e| e.held_until.is_none()));
    assert_eq!(
        audit_of(&app, &raffle.id).await.last().unwrap(),
        &("deliver_all".to_owned(), "2".to_owned())
    );
    assert_eq!(
        service::deliver_all(&app, ACTOR, &raffle.id).await.unwrap(),
        0,
        "idempotent"
    );
}

/// A mixed evening: a book handed over on the spot, a voucher sent later.
/// Claiming the book delivers it; the voucher's winner is held until it is
/// marked delivered, and their phone says it will be sent.
#[tokio::test]
async fn a_prize_handed_over_on_the_spot_is_delivered_by_its_claim() {
    use rafi_lifecycle::prize::Delivery;

    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Book", 1), ("Voucher", 1)]).await;
    let prize_ids: Vec<String> = prizes::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    service::set_prize_delivery(&app, ACTOR, &raffle.id, &prize_ids[0], Delivery::OnTheSpot)
        .await
        .unwrap();
    for name in ["Anna", "Bence", "Csilla"] {
        add_one(&app, ACTOR, &raffle.id, name, None).await.unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let book = draw_and_claim(&app, &raffle.id, &items[0]).await;
    let voucher = draw_and_claim(&app, &raffle.id, &items[1]).await;

    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    let delivered = |id: &str| {
        admin
            .entrants
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .delivered
    };
    assert!(delivered(&book), "the book left with its winner");
    assert!(!delivered(&voucher), "the voucher is still to be sent");
    assert_eq!(admin.retention.held_winners, 1);
    assert!(
        audit_of(&app, &raffle.id)
            .await
            .contains(&("deliver".to_owned(), book.clone())),
        "the log says the claim delivered it"
    );

    let (_, _, view) = get_json(&router, &format!("/api/r/{}/participant", raffle.slug)).await;
    let sent_later: Vec<(String, bool)> = view["winners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["entrant_id"].as_str().unwrap().to_owned(),
                w["sent_later"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(sent_later, [(book, false), (voucher.clone(), true)]);

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    assert_eq!(
        service::deliver_all(&app, ACTOR, &raffle.id).await.unwrap(),
        1,
        "only the voucher was left"
    );
    let refused =
        service::set_prize_delivery(&app, ACTOR, &raffle.id, &prize_ids[1], Delivery::OnTheSpot)
            .await
            .unwrap_err();
    assert!(
        matches!(
            refused,
            service::Error::Refused(service::Refusal::PrizeDeliveryFixed)
        ),
        "{refused:?}"
    );
}

/// Two consoles on one raffle. Console A shows Alice drawn for the book;
/// console B says she isn't here and draws again: Bob. A, still showing
/// Alice, presses "Claimed": refused, as it names the draw it shows, and
/// Bob's draw stays undecided until B, which shows it, claims it.
#[tokio::test]
async fn a_stale_console_cannot_resolve_the_redraw() {
    let (app, router, raffle) = dev_raffle(Config::for_tests()).await;
    let id = raffle.id.as_str();
    let dev = service::Actor::solo("dev");
    for name in ["Alice", "Bob"] {
        add_one(&app, dev, id, name, None).await.unwrap();
    }
    let call = |name: &'static str, args: serde_json::Value| {
        let router = router.clone();
        async move {
            let (status, _, body) = send(&router, admin_call(name, &args.to_string())).await;
            (status, body)
        }
    };
    let (status, body) = call("start_draw", serde_json::json!({ "id": id })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let r = wait_for(&app, id, |r| r.raffle.opened()).await;
    let item = r.drawn.as_ref().unwrap().items()[0].id.clone();
    let drawn_by = |k: u64| {
        let item = item.clone();
        move |r: &Full| {
            r.drawn
                .as_ref()
                .and_then(|s| s.item(&item))
                .is_some_and(|i| i.status() == ItemStatus::Drawn && i.k() == Some(k))
        }
    };
    let draw = |command_id: &'static str| {
        call(
            "draw_item",
            serde_json::json!({ "id": id, "item": item, "command_id": command_id }),
        )
    };
    let resolve = |k: u64, resolution: &'static str, command_id: &'static str| {
        call(
            "resolve_item",
            serde_json::json!({
                "id": id, "item": item, "k": k, "resolution": resolution, "command_id": command_id,
            }),
        )
    };

    let (status, body) = draw("a-draw").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_for(&app, id, drawn_by(1)).await;
    // Console B: not here, and drawn again.
    let (status, body) = resolve(1, "not_here", "b-not-here").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = draw("b-draw").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_for(&app, id, drawn_by(2)).await;

    // Console A, showing draw 1.
    let (status, body) = resolve(1, "claimed", "a-claimed").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let error: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(error.code, "draw_changed", "{body}");
    assert!(error.message.contains("drawn again"), "{body}");
    let outcome = |k: u64| {
        app.db
            .query("SELECT outcome FROM draws WHERE raffle_id = ? AND k = ?")
            .bind(id)
            .bind(k)
            .one::<(String,)>()
    };
    assert_eq!(
        outcome(2).await.unwrap().0,
        "drawn",
        "the redraw's winner is undecided"
    );

    // Console B, showing draw 2.
    let (status, body) = resolve(2, "claimed", "b-claimed").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_for(&app, id, |r| {
        r.drawn
            .as_ref()
            .and_then(|s| s.item(&item))
            .is_some_and(|i| i.status() == ItemStatus::Claimed && i.k() == Some(2))
    })
    .await;
    assert_eq!(outcome(1).await.unwrap().0, "not_here");
    assert_eq!(outcome(2).await.unwrap().0, "claimed");
}

/// More prizes than people (T4-7): two entrants, four items. Pass 1 gives
/// the book and the mug (handed over on the spot) one each; pass 2 gives
/// the voucher (sent later) to one of them again. Their phone lists both
/// prizes, the CSV has a row per prize, and the voucher's claim takes back
/// the delivery the first prize's claim recorded, as the voucher is still
/// to be sent. The sticker goes to the other one in pass 2, who declines
/// it; then nobody is left to draw it from.
#[tokio::test]
async fn more_prizes_than_people() {
    use rafi_lifecycle::prize::Delivery;

    let prizes = [("Book", 1), ("Mug", 1), ("Voucher", 1), ("Sticker", 1)];
    let (app, router, raffle) = dev_raffle_with(Config::for_tests(), &prizes).await;
    let id = raffle.id.as_str();
    let dev = service::Actor::solo("dev");
    let prize_ids: Vec<String> = prizes::list(&app.db, id)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    for prize in &prize_ids[..2] {
        service::set_prize_delivery(&app, dev, id, prize, Delivery::OnTheSpot)
            .await
            .unwrap();
    }
    for name in ["Anna", "Bence"] {
        add_one(&app, dev, id, name, None).await.unwrap();
    }
    service::start_draw(&app, dev, id).await.unwrap();
    let r = wait_for(&app, id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    assert_eq!(items.len(), 4);

    let first = draw_and_claim_as(&app, dev, id, &items[0]).await;
    // The admin's entrants: what each holds, in draw order, and where they
    // stand; nothing for whoever the draws haven't picked.
    let held = |who: String| {
        let app = app.clone();
        async move {
            let admin = service::admin(&app, "dev", id).await.unwrap();
            let e = admin.entrants.into_iter().find(|e| e.id == who).unwrap();
            let prizes: Vec<(String, rafi_raffle::Outcome)> =
                e.prizes.into_iter().map(|p| (p.name, p.outcome)).collect();
            (e.outcome, prizes)
        }
    };
    let not_yet = service::admin(&app, "dev", id)
        .await
        .unwrap()
        .entrants
        .into_iter()
        .find(|e| e.id != first)
        .unwrap();
    assert_eq!((not_yet.outcome, not_yet.prizes), (None, vec![]));
    let second = draw_and_claim_as(&app, dev, id, &items[1]).await;
    assert_ne!(first, second, "pass 1 draws everyone once");
    let twice = draw_and_claim_as(&app, dev, id, &items[2]).await;
    assert!([&first, &second].contains(&&twice), "pass 2: {twice}");
    let once = if twice == first { &second } else { &first };
    let r = full(&app, id).await;
    let drawn = r.drawn.as_ref().unwrap();
    let passes: Vec<u8> = drawn.draws().iter().map(|d| d.pass).collect();
    assert_eq!(passes, [1, 1, 2]);

    // Their phone has both prizes, in the order drawn, each as it stands.
    let (status, _, view) = get_json(&router, &format!("/api/r/{}/participant", raffle.slug)).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let view: rafi_live_client::ParticipantView = serde_json::from_value(view).unwrap();
    let wins = |who: &str| -> Vec<(String, bool, bool)> {
        view.wins_of(who)
            .iter()
            .map(|w| (w.prize_name.clone(), w.claimed, w.sent_later))
            .collect()
    };
    let earlier = if twice == first { "Book" } else { "Mug" };
    let later = if twice == first { "Mug" } else { "Book" };
    assert_eq!(
        wins(&twice),
        [
            (earlier.to_owned(), true, false),
            ("Voucher".to_owned(), true, true)
        ]
    );
    assert_eq!(wins(once), [(later.to_owned(), true, false)]);
    let me = rafi_live_client::screen(&view, Some(&twice), view.state, 0);
    assert!(
        matches!(&me, rafi_live_client::Screen::Won { prizes, .. } if prizes.len() == 2),
        "{me:?}"
    );

    // The People tab lists both of theirs, as the phone does.
    use rafi_raffle::Outcome::Claimed;
    let claimed = |prize: &str| (prize.to_owned(), Claimed);
    assert_eq!(
        held(twice.clone()).await,
        (Some(Claimed), vec![claimed(earlier), claimed("Voucher")])
    );
    assert_eq!(
        held(once.clone()).await,
        (Some(Claimed), vec![claimed(later)])
    );

    // A row per prize: three.
    let csv = service::export_csv(&app, dev, id).await.unwrap();
    let rows: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(rows.len(), 3, "{csv}");
    assert_eq!(
        rows.iter().filter(|l| l.contains(&twice)).count(),
        2,
        "{csv}"
    );
    assert!(
        rows.iter()
            .any(|l| l.contains(&twice) && l.contains("\"claimed\",\"Voucher\"")),
        "{csv}"
    );

    // The on-the-spot prize's claim delivered; the voucher's takes it back,
    // as it is still to be sent. The other winner stays delivered.
    let audit = audit_of(&app, id).await;
    let deliveries: Vec<(&str, &str)> = audit
        .iter()
        .filter(|(action, _)| action == "deliver" || action == "undeliver")
        .map(|(a, d)| (a.as_str(), d.as_str()))
        .collect();
    let mut expected = vec![("deliver", first.as_str()), ("deliver", second.as_str())];
    expected.push(("undeliver", twice.as_str()));
    assert_eq!(deliveries, expected);
    let delivered = |who: String| {
        let app = app.clone();
        async move {
            entrants::get(&app.db, id, &who)
                .await
                .unwrap()
                .unwrap()
                .delivered_at
                .is_some()
        }
    };
    assert!(!delivered(twice.clone()).await, "the voucher is owed");
    assert!(delivered(once.clone()).await);

    // The sticker: the other winner, the last one pass 2 hasn't drawn, who
    // declines it; drawn again, there is nobody left.
    let call = |name: &'static str, args: serde_json::Value| {
        let router = router.clone();
        async move {
            let (status, _, body) = send(&router, admin_call(name, &args.to_string())).await;
            (status, body)
        }
    };
    let sticker = items[3].clone();
    let (status, body) = call(
        "draw_item",
        serde_json::json!({ "id": id, "item": sticker, "command_id": "sticker-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let r = wait_for(&app, id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&sticker).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    let s = r.drawn.as_ref().unwrap();
    assert_eq!(s.item(&sticker).unwrap().winner().unwrap().as_str(), once);
    let (status, body) = call(
        "resolve_item",
        serde_json::json!({
            "id": id, "item": sticker, "resolution": "declined", "command_id": "sticker-declined",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_for(&app, id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&sticker).unwrap().status() == ItemStatus::Available)
    })
    .await;
    let (status, body) = call(
        "draw_item",
        serde_json::json!({ "id": id, "item": sticker, "command_id": "sticker-2" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let error: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(
        error.message, "There are no eligible entrants left.",
        "{body}"
    );

    // Declining a second prize leaves the first: still three rows, and the
    // phone still lists what each holds.
    let csv = service::export_csv(&app, dev, id).await.unwrap();
    assert_eq!(csv.lines().count(), 1 + 3, "{csv}");
    let (_, _, view) = get_json(&router, &format!("/api/r/{}/participant", raffle.slug)).await;
    let view: rafi_live_client::ParticipantView = serde_json::from_value(view).unwrap();
    assert_eq!(view.wins_of(&twice).len(), 2);
    assert_eq!(view.wins_of(once).len(), 1);
    // The admin too: they still stand as claimed, with the prize they hold.
    assert_eq!(
        held(once.clone()).await,
        (Some(Claimed), vec![claimed(later)])
    );
    assert_eq!(held(twice.clone()).await.1.len(), 2);
}
