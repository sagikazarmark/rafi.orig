//! The evening end to end: join, freeze, draw, verify, purge, over HTTP.

use super::*;

#[tokio::test]
async fn full_raffle_flow() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(
        &app,
        &[("KCD Budapest ticket", 2), ("CNCF exam voucher", 1)],
    )
    .await;
    let code = raffle.join_code.clone();
    assert_eq!(code.len(), service::JOIN_CODE_LEN);
    assert!(code.bytes().all(|b| service::codes::ALPHABET.contains(&b)));

    // The join page renders the form.
    let (status, _, page) = send(
        &router,
        Request::get(format!("/j/{code}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("name=\"email\""));
    // The page ends with the loader of the participant build this server
    // was compiled against, which links the build's hashed glue and its
    // fallback under /participant/, where build_router serves them.
    assert!(page.contains(crate::pages::LOADER), "{page}");
    assert!(
        crate::pages::LOADER.contains("import(\"/participant/assets/rafi-participant-dxh")
            && crate::pages::LOADER.contains("\"/participant/fallback.js\""),
        "{}",
        crate::pages::LOADER
    );
    // Typed as the projector shows it, in groups and in any case: the
    // canonical code is redirected to.
    let grouped = format!("{}-{}", &code[..3], code[3..].to_lowercase());
    let (status, headers, _) = send(
        &router,
        Request::get(format!("/j/{grouped}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], format!("/j/{code}"));

    // Participants join; the cookie brings them back to their entry.
    let mut cookies = Vec::new();
    for i in 0..5 {
        let (status, cookie, _) = join(
            &router,
            &code,
            &format!("Person {i}"),
            &format!("p{i}@example.com"),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "join {i}");
        cookies.push(cookie.expect("entrant cookie"));
    }
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{code}"))
            .header(header::COOKIE, &cookies[0])
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("Person 0"));
    let me = entrants::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.display_name == "Person 0")
        .unwrap();
    assert!(
        page.contains(me.claim_code.as_deref().unwrap()),
        "claim code is shown to the entrant"
    );

    // Same email (normalized) on another device: already in, no new entry, no cookie.
    let (status, cookie, page) = join(&router, &code, "Someone", "P0+other@Example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_none());
    assert!(page.contains("already entered"));

    // A person without a phone.
    add_one(&app, ACTOR, &raffle.id, "No Phone", None)
        .await
        .unwrap();
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 6);

    // Freeze; joining is closed afterwards.
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    assert_eq!(r.raffle.state, RaffleState::Frozen);
    let (status, cookie, _) = join(&router, &code, "Late", "late@example.com").await;
    assert_ne!(status, StatusCode::SEE_OTHER);
    assert!(cookie.is_none());
    assert!(
        add_one(&app, ACTOR, &raffle.id, "Late", None)
            .await
            .is_err()
    );
    assert!(
        service::upsert_prize(
            &app,
            ACTOR,
            &raffle.id,
            PrizeInput {
                name: "x".into(),
                qty: 1,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );

    // Draw every item; the first winner is not here and the item is redrawn.
    let state = r.drawn.as_ref().unwrap();
    let items: Vec<_> = state.items().iter().map(|u| u.id.clone()).collect();
    let mut winners = Vec::new();
    for (i, item) in items.iter().enumerate() {
        service::command(
            &app,
            ACTOR,
            &raffle.id,
            DrawRequest { item: item.clone() },
            &format!("c{i}"),
        )
        .await
        .unwrap();
        let r = wait_for(&app, &raffle.id, |r| {
            r.drawn
                .as_ref()
                .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
        })
        .await;
        if i == 0 {
            let first = r
                .drawn
                .as_ref()
                .unwrap()
                .item(item)
                .unwrap()
                .winner()
                .cloned()
                .unwrap();
            service::command(
                &app,
                ACTOR,
                &raffle.id,
                Decision::Resolve {
                    k: None,
                    item: item.clone(),
                    resolution: Resolution::NotHere,
                },
                "nh",
            )
            .await
            .unwrap();
            service::command(
                &app,
                ACTOR,
                &raffle.id,
                DrawRequest { item: item.clone() },
                "redraw",
            )
            .await
            .unwrap();
            let r = wait_for(&app, &raffle.id, |r| {
                r.drawn
                    .as_ref()
                    .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
            })
            .await;
            assert_ne!(
                r.drawn
                    .as_ref()
                    .unwrap()
                    .item(item)
                    .unwrap()
                    .winner()
                    .cloned()
                    .unwrap(),
                first
            );
        }
        let r = full(&app, &raffle.id).await;
        winners.push(
            r.drawn
                .as_ref()
                .unwrap()
                .item(item)
                .unwrap()
                .winner()
                .cloned()
                .unwrap(),
        );
        service::command(
            &app,
            ACTOR,
            &raffle.id,
            Decision::Resolve {
                k: None,
                item: item.clone(),
                resolution: Resolution::Claimed,
            },
            &format!("r{i}"),
        )
        .await
        .unwrap();
    }
    let mut unique = winners.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), winners.len(), "one win per person");

    // Drawing a claimed item again is rejected.
    assert!(
        service::command(
            &app,
            ACTOR,
            &raffle.id,
            DrawRequest {
                item: items[0].clone(),
            },
            "x"
        )
        .await
        .is_err()
    );

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Finished);
    assert!(r.finished_at.is_some());

    // The audit log names each command by its verb, with what it was about.
    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&raffle.id)
        .all()
        .await
        .unwrap();
    let actions: Vec<&str> = audit.iter().map(|(a, _)| a.as_str()).collect();
    assert!(actions.contains(&"freeze"), "{audit:?}");
    assert!(!actions.contains(&"command"), "{audit:?}");
    assert!(
        audit.contains(&("draw".into(), items[0].to_string())),
        "{audit:?}"
    );
    assert!(
        audit.contains(&("resolve".into(), format!("{} not_here", items[0]))),
        "{audit:?}"
    );
    assert!(
        audit.contains(&("resolve".into(), format!("{} claimed", items[0]))),
        "{audit:?}"
    );
    assert_eq!(audit.last(), Some(&("finish".into(), String::new())));

    // The public log verifies independently.
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_raffle::drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), items.len() + 1);

    // CSV export lists the winners (and the not-here entrant).
    let csv = service::export_csv(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(csv.lines().count(), 1 + items.len() + 1);

    // Purging: personal data is erased but the draw stays verifiable.
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(entrants.len(), 6);
    assert!(
        entrants
            .iter()
            .all(|e| e.erased && e.email_raw.is_none() && e.token_hash.is_none())
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_raffle::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_raffle::drand::resolve_chain
        )
        .ok()
    );
    assert!(data.names.is_empty(), "no names are left: {:?}", data.names);
    assert!(entrants.iter().all(|e| e.claim_code.is_none()));
    // The join code no longer answers (nobody can be recognized on it, and
    // guessing codes finds only raffles inside their hold); the slug does.
    let (status, _, _) = send(
        &router,
        Request::get(format!("/j/{code}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a purged raffle's code");
    let (status, _, _) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the draw stays verifiable");
}

/// The evening over HTTP: the projector's view says whose list it is,
/// where the notice is and whether people were added from a list; each
/// draw, and the phones' latest winner, whether the winner has a phone to
/// tell them; the admin, per entrant; and `verify.json` counts the frozen
/// list by source.
#[tokio::test]
async fn the_evening_over_http() {
    let (app, router, raffle) = dev_raffle_with(Config::for_tests(), &[("Book", 2)]).await;
    let id = raffle.id.as_str();
    let dev = service::Actor::solo("dev");
    let view_url = format!("/api/r/{}/view", raffle.slug);

    let (_, _, view) = get_json(&router, &view_url).await;
    assert_eq!(view["organizer"], "Cloud Native Budapest", "{view}");
    assert_eq!(
        view["privacy_url"],
        format!("{}/privacy/{}", app.policy.public_url, raffle.slug)
    );
    assert_eq!(view["list_added"], false);

    let add = add_args(
        id,
        "b-evening",
        "list",
        serde_json::json!([{ "name": "Anna Kovács", "email": "anna@example.com" }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &add)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let anna: rafi_api::AddResult = serde_json::from_str(&body).unwrap();
    let anna = anna.added[0].id.clone();
    let (status, _, _) = join(&router, &raffle.join_code, "Bea", "bea@example.com").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, _, view) = get_json(&router, &view_url).await;
    assert_eq!(view["list_added"], true, "{view}");

    let (status, _, body) = send(
        &router,
        admin_call("get_raffle", &serde_json::json!({ "id": id }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let admin: rafi_api::RaffleAdmin = serde_json::from_str(&body).unwrap();
    let no_phone: Vec<(&str, bool)> = admin
        .entrants
        .iter()
        .map(|e| (e.display_name.as_str(), e.no_phone))
        .collect();
    assert_eq!(no_phone, [("Anna Kovács", true), ("Bea", false)]);
    assert!(admin.actions.missing.is_empty(), "opened: nothing missing");

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
    for item in &items {
        let winner = draw_and_claim_as(&app, dev, id, item).await;
        let (_, _, view) = get_json(&router, &view_url).await;
        let last = view["draws"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["entrant"], winner.as_str());
        assert_eq!(last["no_phone"], winner == anna, "{last}");
        let (_, _, phone) = get_json(&router, &format!("/api/r/{}/participant", raffle.slug)).await;
        assert_eq!(phone["last"]["no_phone"], winner == anna, "{phone}");
    }

    let (_, _, verify) = get_json(&router, &format!("/api/r/{}/verify.json", raffle.slug)).await;
    assert_eq!(
        verify["sources"],
        serde_json::json!({ "joined": 1, "added": 1, "fake": 0 }),
        "{verify}"
    );
}
