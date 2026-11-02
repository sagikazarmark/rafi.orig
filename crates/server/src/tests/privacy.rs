//! Privacy: the notices, erasure on request, exports, an organizer removing
//! their data.

use super::*;

/// Purging logs every erasure once, by ID, without personal data.
#[tokio::test]
async fn erasures_are_logged_without_personal_data() {
    let app = app().await;
    let raffle = finished_raffle(&app).await;
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.erased && e.display_name.starts_with('#'))
    );
    let log: Vec<serde_json::Value> = app.db.query("SELECT * FROM erasures").all().await.unwrap();
    assert_eq!(log.len(), entrants.len());
    for row in &log {
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            ["at", "entrant_id", "id", "raffle_id"],
            "no personal data"
        );
        assert_eq!(row["raffle_id"], raffle.id.as_str());
    }
    let mut logged: Vec<&str> = log
        .iter()
        .filter_map(|r| r["entrant_id"].as_str())
        .collect();
    logged.sort_unstable();
    let mut ids: Vec<&str> = entrants.iter().map(|e| e.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(logged, ids, "every entrant, once");

    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let again: Vec<serde_json::Value> = app.db.query("SELECT * FROM erasures").all().await.unwrap();
    assert_eq!(again.len(), log.len(), "purging again has nothing to do");
}

/// From the freeze until the raffle finishes, nobody leaves the list; once it
/// has, one entrant's personal data can be erased, and the published names
/// drop them.
#[tokio::test]
async fn entrants_are_erased_only_once_the_raffle_is_over() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let mut ids = Vec::new();
    for name in ["Anna", "Bence", "Csilla"] {
        ids.push(add_one(&app, ACTOR, &raffle.id, name, None).await.unwrap());
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let refused = service::remove_entrant(&app, ACTOR, &raffle.id, &ids[0], false).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::NotRemovable))
        ),
        "{refused:?}"
    );
    // Hiding still works.
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &ids[1],
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.erased)
    );

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::remove_entrant(&app, ACTOR, &raffle.id, &ids[0], false)
        .await
        .unwrap();
    let anna = entrants::get(&app.db, &raffle.id, &ids[0])
        .await
        .unwrap()
        .unwrap();
    assert!(anna.erased && anna.claim_code.is_none());

    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let names: std::collections::HashMap<&str, &str> = data
        .names
        .iter()
        .map(|(id, n)| (id.as_str(), n.as_str()))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(!names.contains_key(ids[0].as_str()), "erased: no name");
    assert_eq!(names[ids[1].as_str()], rafi_live_client::HIDDEN_NAME);
    assert_eq!(names[ids[2].as_str()], "Csilla");
    let manifest: rafi_raffle::Manifest = serde_json::from_str(&data.manifest).unwrap();
    assert_eq!(
        manifest.entrants.len(),
        3,
        "the erased ID stays in the list"
    );
    assert!(!data.test_mode);
}

/// The join page, the rules and both privacy notices tell entrants what
/// happens to their data, who is responsible, and how old they must be.
#[tokio::test]
async fn the_join_page_and_the_notices_say_when_data_is_erased() {
    use rafi_lifecycle::dates::Date;
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let mut settings = service::settings_of(&raffle);
    settings.event_date = "2026-10-28".into();
    settings.retention_days = 7;
    settings.privacy_email = "dpo@example.com".into();
    settings.privacy_text = "We never sell anything.\nAsk us anything.".into();
    settings.privacy_url = "https://example.com/privacy".into();
    service::update_settings(&app, ACTOR, &raffle.id, settings)
        .await
        .unwrap();

    let (status, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("I accept the "), "{page}");
    assert!(
        page.contains(">raffle rules</a>"),
        "the checkbox is about the rules"
    );
    assert!(
        !page.contains("accept the raffle rules and the privacy notice"),
        "the notice is read, not accepted"
    );
    assert!(
        page.contains("Your name and email are erased on 4 November 2026; a winner&#39;s once their prize is delivered, at the latest on 2 February 2027."),
        "{page}"
    );
    assert!(page.contains("you confirm you are 16 or older"), "{page}");
    assert!(!page.contains("marketing") && !page.contains("future meetups"));
    assert!(page.contains(&format!("/privacy/{}", raffle.slug)));

    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("Cloud Native Budapest is the data controller"),
        "{notice}"
    );
    assert!(
        notice.contains("Privacy requests: dpo@example.com"),
        "the privacy contact wins"
    );
    assert!(notice.contains("The event is on 28 October 2026."));
    assert!(notice.contains("erased on 4 November 2026"));
    assert!(notice.contains("erased on 2 February 2027 at the latest"));
    assert!(
        notice.contains("Rafi Test Operator (privacy@rafi.test) runs this service"),
        "{notice}"
    );
    assert!(notice.contains("version test-1"));
    assert!(
        notice.contains("href=\"/privacy\""),
        "links the installation's notice"
    );
    assert!(!notice.contains("Clerk"), "no hard-coded subprocessors");
    assert!(
        !notice.contains("Art. 6(1)(a)"),
        "no consent-based processing"
    );
    let own = notice
        .find("From the organizer")
        .expect("the organizer's own section");
    let generated = notice.find("Who processes it").unwrap();
    assert!(own > generated, "after the generated sections");
    assert!(notice.contains("We never sell anything."));
    assert!(notice.contains("href=\"https://example.com/privacy\""));

    // Whoever the organizer adds is told what of theirs is held, where it
    // came from, why and on what basis, and, on its own, how to object.
    let added = notice
        .find("If the organizer added you")
        .expect("the section for people the organizer adds");
    assert!(added < notice.find("What is public").unwrap());
    let section = &notice[added..notice.find("What is public").unwrap()];
    for said in [
        "your name and, if the organizer gives it, your email address: nothing else",
        "from you, at the event, or from the organizer&#39;s records about the event, such as its registration or guest list",
        "for this prize draw only",
        "the organizer&#39;s legitimate interest in rewarding the attendees of “",
        "with a prize draw (GDPR Art. 6(1)(f))",
        "on the event screen and on the verification page, as everyone&#39;s is",
    ] {
        assert!(section.contains(said), "{said}: {section}");
    }
    let object = section
        .split("<p")
        .find(|p| p.contains("You can object"))
        .expect("the right to object, in a paragraph of its own");
    assert!(
        !object.contains("Art. 6(1)(f)"),
        "apart from the basis: {object}"
    );
    for said in [
        "ask Cloud Native Budapest (dpo@example.com) to take you off",
        "Before the list of entrants is frozen for the draw, you are removed, and stay out if the organizer adds the list again.",
        "After that, your name is hidden and your data is erased when the raffle is over",
        "only your anonymous ID stays in the draw, so it can still be verified",
    ] {
        assert!(object.contains(said), "{said}: {object}");
    }
    // What is about joining says so.
    for said in [
        "If you join with the QR code: your display name and email address",
        "To join with the QR code, your email address is required",
        "When you join with the QR code, we set one strictly necessary cookie",
        "If you join with the QR code, it doesn&#39;t have to be your real name",
    ] {
        assert!(notice.contains(said), "{said}: {notice}");
    }
    assert!(!notice.contains("Your email address is required to enter"));

    let (status, _, rules) = send(
        &router,
        Request::get(format!("/rules/{}", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(rules.contains("Entrants must be 16 or older:"), "{rules}");
    // Both ways in, for every raffle, so switching registration changes
    // nothing anyone accepted; and who answers for the age of whom.
    for said in [
        "The raffle is for the people attending the event in person.",
        "Entrants are the people who join with the QR code while registration is open, and the people the organizer adds (for example, from the event&#39;s guest list). If the organizer added you, you can ask them to take you off.",
        "Organizers and volunteers cannot win.",
        "by joining, you confirm you are; the organizer is responsible for the people it adds meeting the age limit.",
        "Winners must be present",
    ] {
        assert!(rules.contains(said), "{said}: {rules}");
    }

    // The installation's own documents, with their versions.
    for (path, text) in [
        ("/privacy", "The test installation"),
        ("/terms", "<strong>processing agreement</strong>"),
    ] {
        let (status, _, body) =
            send(&router, Request::get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains(text), "{path}: {body}");
        assert!(body.contains("version test-1"), "{path}");
    }

    // The admin sees the same dates.
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.retention.erase_date, "4 November 2026");
    assert_eq!(admin.retention.winners_erase_date, "2 February 2027");
    assert_eq!(
        admin.retention.erase_at,
        Date::parse("2026-10-28").unwrap().ends_utc() + 7 * DAY
    );
    assert_eq!(admin.retention.max_days, app.policy.retention.max_days);
    assert_eq!(
        admin.retention.winner_hold_days,
        app.policy.retention.winner_hold_days
    );
}

/// Erasing one entrant on request: refused while the list is frozen before
/// the erasure date, allowed once the date has passed; the admin says which.
#[tokio::test]
async fn erasure_on_request_follows_the_date() {
    use rafi_lifecycle::entrant::Removal;
    let app = app().await;
    // Anna and Bence in a raffle frozen today; returns Anna's ID.
    let frozen = async |r: &RaffleRow| {
        let id = add_one(&app, ACTOR, &r.id, "Anna", Some("anna@x.org".into()))
            .await
            .unwrap();
        add_one(&app, ACTOR, &r.id, "Bence", None).await.unwrap();
        service::start_draw(&app, ACTOR, &r.id).await.unwrap();
        wait_for(&app, &r.id, |r| r.raffle.opened()).await;
        id
    };

    // Ten days ago's raffle, and today's.
    let past = setup_open(&app, &[("Book", 1)]).await;
    let past_anna = frozen(&past).await;
    app.clock.advance(10 * DAY);
    let before = setup_open(&app, &[("Book", 1)]).await;
    let before_anna = frozen(&before).await;
    let now = app.clock.now();
    assert!(before.erase_at > now && past.erase_at <= now);
    let ids = [before_anna, past_anna];

    let refused = service::remove_entrant(&app, ACTOR, &before.id, &ids[0], false).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::NotRemovable))
        ),
        "{refused:?}"
    );
    assert_eq!(
        service::admin(&app, OWNER, &before.id)
            .await
            .unwrap()
            .actions
            .removal,
        None
    );
    assert!(
        !entrants::get(&app.db, &before.id, &ids[0])
            .await
            .unwrap()
            .unwrap()
            .erased
    );

    assert_eq!(
        service::admin(&app, OWNER, &past.id)
            .await
            .unwrap()
            .actions
            .removal,
        Some(Removal::Erase)
    );
    service::remove_entrant(&app, ACTOR, &past.id, &ids[1], false)
        .await
        .unwrap();
    let anna = entrants::get(&app.db, &past.id, &ids[1])
        .await
        .unwrap()
        .unwrap();
    assert!(anna.erased && anna.email_raw.is_none());
    assert_eq!(
        audit_of(&app, &past.id).await.last().unwrap(),
        &("erase_entrant".to_owned(), ids[1].clone())
    );
    // The raffle is still being drawn; the other entrant is untouched until
    // housekeeping or a request.
    assert_eq!(
        raffles::get(&app.db, &past.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Frozen
    );
    assert_eq!(
        entrants::list(&app.db, &past.id)
            .await
            .unwrap()
            .iter()
            .filter(|e| !e.erased)
            .count(),
        1
    );
}

/// The export has the winners with their details, the delivered flag and
/// formula-safe cells, and every export is audited: who, which raffle, how
/// many rows.
#[tokio::test]
async fn exports_have_the_winners_and_are_audited() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1), ("Mug", 1)]).await;
    // A name that a spreadsheet would run as a formula.
    let formula = add_one(
        &app,
        ACTOR,
        &raffle.id,
        "=SUM(A1)",
        Some("sum@x.org".into()),
    )
    .await
    .unwrap();
    add_one(&app, ACTOR, &raffle.id, "Bence", Some("bence@x.org".into()))
        .await
        .unwrap();
    add_one(&app, ACTOR, &raffle.id, "Csilla", None)
        .await
        .unwrap();
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
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &w1,
        rafi_api::EntrantPatch {
            delivered: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();

    let csv = service::export_csv(&app, ACTOR, &raffle.id).await.unwrap();
    assert!(csv.starts_with('\u{FEFF}'), "a byte order mark, for Excel");
    let lines: Vec<&str> = csv.trim_start_matches('\u{FEFF}').lines().collect();
    assert_eq!(
        lines[0],
        "entrant_id,display_name,email,source,claim_code,outcome,prize,delivered"
    );
    assert_eq!(lines.len(), 3, "the two winners");
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    for (winner, delivered) in [(&w1, "yes"), (&w2, "no")] {
        let e = entrants.iter().find(|e| &e.id == winner).unwrap();
        let line = lines
            .iter()
            .find(|l| l.starts_with(&format!("\"{winner}\"")))
            .unwrap();
        let email = e.email_raw.clone().unwrap_or_default();
        let claim = e.claim_code.clone().unwrap();
        let name = if e.id == formula {
            "'=SUM(A1)"
        } else {
            &e.display_name
        };
        assert!(
            line.contains(&format!(
                "\"{name}\",\"{email}\",\"manual\",\"{claim}\",\"claimed\""
            )),
            "{line}"
        );
        assert!(line.ends_with(&format!("\"{delivered}\"")), "{line}");
    }
    assert_eq!(
        audit_of(&app, &raffle.id).await.last().unwrap(),
        &("export_csv".to_owned(), "2".to_owned())
    );
    assert!(
        service::export_csv(&app, service::Actor::solo("someone-else"), &raffle.id)
            .await
            .is_err(),
        "another organizer's raffle doesn't exist for them"
    );
}

/// An organizer removes their data: drafts go, finished raffles lose their
/// personal data and the organizer, defaults go, the terms' record and the
/// verifiable draw stay. Not while a raffle is being drawn.
#[tokio::test]
async fn an_organizer_can_remove_their_data() {
    let app = app().await;
    let router = routes(app.clone());
    service::save_defaults(
        &app,
        ACTOR,
        rafi_api::RaffleDefaults {
            organizer_name: "Org".into(),
            contact_email: "a@b.c".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let finished = finished_raffle(&app).await;
    let draft = create(&app, OWNER, "Draft").await;
    let drawing = setup_open(&app, &[("Book", 1)]).await;
    add_one(&app, ACTOR, &drawing.id, "Anna", None)
        .await
        .unwrap();
    service::start_draw(&app, ACTOR, &drawing.id).await.unwrap();
    wait_for(&app, &drawing.id, |r| r.raffle.opened()).await;

    let refused = service::remove_owner_data(&app, ACTOR).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::EventLive))
        ),
        "{refused:?}"
    );
    assert!(
        raffles::get(&app.db, &draft).await.unwrap().is_some(),
        "nothing happened"
    );

    service::command(&app, ACTOR, &drawing.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::remove_owner_data(&app, ACTOR).await.unwrap();

    assert!(
        raffles::get(&app.db, &draft).await.unwrap().is_none(),
        "the draft is gone"
    );
    for r in [&finished, &drawing] {
        let row = raffles::get(&app.db, &r.id).await.unwrap().unwrap();
        assert_eq!(row.state, RaffleState::Purged);
        assert!(
            row.organizer_name.is_empty()
                && row.contact_email.is_empty()
                && row.privacy_email.is_empty()
        );
        assert!(
            entrants::list(&app.db, &r.id)
                .await
                .unwrap()
                .iter()
                .all(|e| e.erased)
        );
        let data = service::verify_data(&app, &r.slug).await.unwrap();
        assert!(
            rafi_raffle::verify(
                &data.manifest,
                &data.manifest_hash,
                &data.events,
                rafi_raffle::drand::resolve_chain
            )
            .ok()
        );
        assert_eq!(
            audit_of(&app, &r.id).await.last().unwrap(),
            &("purge".to_owned(), "owner left".to_owned())
        );
        assert!(row.organizer_left_at.is_some() && row.purged_at.is_some());
    }
    assert_eq!(
        audit_of(&app, &draft).await.last().unwrap(),
        &("delete".to_owned(), "owner left".to_owned())
    );
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&finished.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 3, "each erasure logged");
    assert_eq!(
        service::get_defaults(&app, OWNER).await.unwrap(),
        Default::default()
    );
    assert!(
        service::list(&app, OWNER)
            .await
            .unwrap()
            .iter()
            .all(|r| r.state == RaffleState::Purged)
    );

    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", finished.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("who has since left this service"),
        "{notice}"
    );
    assert!(!notice.contains("hello@example.com"));
    assert!(
        notice.contains("ask the organizer to take you off"),
        "nobody named: {notice}"
    );
}

/// The notice without the optional fields: the contact email is the privacy
/// contact, there is no section from the organizer; and a public raffle
/// without an organizer names "the raffle organizer".
#[tokio::test]
async fn the_notice_without_the_optional_fields() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("Privacy requests: hello@example.com"),
        "{notice}"
    );
    assert!(
        notice.contains("ask Cloud Native Budapest (hello@example.com) to take you off"),
        "the contact email, without a privacy contact: {notice}"
    );
    assert!(!notice.contains("From the organizer"), "{notice}");
    assert!(!notice.contains("has since left"));

    // Published but nameless: it can't open, but its notice is public.
    let id = service::create(&app, ACTOR, "Nameless", &service::EventDate::today(&app))
        .await
        .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    let nameless = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", nameless.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("The raffle organizer is the data controller for this raffle."),
        "{notice}"
    );
    assert!(!notice.contains("Privacy requests:"));
    assert!(
        notice.contains("You can object to this at any time: ask the organizer to take you off."),
        "nobody to name yet: {notice}"
    );
}

/// Leaving with nothing but defaults: they go, nothing else is touched.
#[tokio::test]
async fn leaving_with_no_raffles() {
    let app = app().await;
    service::save_defaults(
        &app,
        ACTOR,
        rafi_api::RaffleDefaults {
            organizer_name: "Org".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let other = setup_open(&app, &[("Book", 1)]).await;
    app.db
        .query("UPDATE raffles SET owner_id = 'someone-else' WHERE id = ?")
        .bind(&other.id)
        .run()
        .await
        .unwrap();
    service::remove_owner_data(&app, ACTOR).await.unwrap();
    assert_eq!(
        service::get_defaults(&app, OWNER).await.unwrap(),
        Default::default()
    );
    assert_eq!(
        raffles::get(&app.db, &other.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Open,
        "another owner's raffle is untouched"
    );
}
