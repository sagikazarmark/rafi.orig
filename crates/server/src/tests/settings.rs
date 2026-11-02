//! Raffles' lifecycle and settings: guards, defaults, registration, the
//! organizer, deleting.

use super::*;

#[tokio::test]
async fn lifecycle_guards() {
    let app = app().await;
    let id = create(&app, OWNER, "Guards").await;
    assert!(
        service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open)
            .await
            .is_err(),
        "cannot open a draft"
    );
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    assert!(
        service::start_draw(&app, ACTOR, &id).await.is_err(),
        "no prizes"
    );
    service::upsert_prize(
        &app,
        ACTOR,
        &id,
        PrizeInput {
            name: "P".into(),
            qty: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        service::start_draw(&app, ACTOR, &id).await.is_err(),
        "no entrants"
    );
    // The failed freeze must not leave the raffle frozen.
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Open
    );
    assert!(
        service::add_fakes(&app, ACTOR, &id, 3).await.is_err(),
        "fakes need test mode"
    );
    let dup = service::duplicate(&app, ACTOR, &id, &service::EventDate::today(&app))
        .await
        .unwrap();
    assert_eq!(prizes::list(&app.db, &dup).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_freeze_interrupted_before_its_commit_can_still_start() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 2)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        add_one(&app, ACTOR, &raffle.id, name, None).await.unwrap();
    }
    // The freeze's first write landed, the manifest's did not.
    app.db
        .query("UPDATE raffles SET state = 'frozen' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();

    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let Full {
        raffle: r, drawn, ..
    } = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let doc: rafi_raffle::Manifest =
        serde_json::from_str(r.manifest.as_deref().expect("a manifest")).unwrap();
    assert_eq!(doc.entrants.len(), 3);
    assert_eq!(doc.items.len(), 2);
    assert_eq!(
        r.manifest_hash.as_deref(),
        Some(rafi_raffle::ManifestHash::of(r.manifest.as_deref().unwrap()).to_string()).as_deref()
    );
    assert!(r.frozen_at.is_some());
    assert_eq!(
        drawn.unwrap().manifest_hash().to_string(),
        r.manifest_hash.unwrap()
    );

    // Starting again re-sends the same commitment.
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let again = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(again.manifest, r.manifest);
}

/// The additional rules are fixed once an entrant locks them: fakes don't
/// count, a person added by hand does, and the rest of the settings still
/// save.
#[tokio::test]
async fn rules_are_fixed_once_an_entrant_locks_them() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let save = |rules: &str, test_mode: bool| {
        let mut settings = service::settings_of(&raffle);
        settings.rules = rules.into();
        settings.test_mode = test_mode;
        service::update_settings(&app, ACTOR, &raffle.id, settings)
    };
    save("Rehearsal rules.", true).await.unwrap();
    service::add_fakes(&app, ACTOR, &raffle.id, 3)
        .await
        .unwrap();
    save("Fakes don't accept anything.", true).await.unwrap();
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(admin.actions.edit_rules);

    add_one(&app, ACTOR, &raffle.id, "No Phone", None)
        .await
        .unwrap();
    let refused = save("Changed after an entrant locked them.", true).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::SettingsFixed(fields)))
                if fields == &["additional rules"]
        ),
        "{refused:?}"
    );
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(!admin.actions.edit_rules);
    assert_eq!(admin.settings.rules, "Fakes don't accept anything.");
    // The event date, the period, the privacy text and link lock with them,
    // and the refusal names what changed.
    let mut settings = admin.settings.clone();
    settings.event_date = "2026-12-31".into();
    settings.retention_days = 1;
    settings.privacy_url = "https://example.com/other".into();
    let refused = service::update_settings(&app, ACTOR, &raffle.id, settings).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::SettingsFixed(fields)))
                if fields == &["event date", "retention period", "privacy notice link"]
        ),
        "{refused:?}"
    );
    let before = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    // The same rules, other settings changed: saved. The privacy contact
    // stays editable: a contact must be current.
    let mut settings = admin.settings;
    settings.title = "Renamed".into();
    settings.privacy_email = "dpo@example.com".into();
    service::update_settings(&app, ACTOR, &raffle.id, settings)
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.title, "Renamed");
    assert_eq!(r.privacy_email, "dpo@example.com");
    assert_eq!(
        (r.event_date, r.erase_at, r.retention_days),
        (before.event_date, before.erase_at, before.retention_days),
        "the dates didn't move"
    );
}

/// A new raffle starts with a copy of its owner's defaults: changing them
/// later leaves it as it is.
#[tokio::test]
async fn new_raffles_start_with_the_owners_defaults() {
    let app = app().await;
    let defaults = rafi_api::RaffleDefaults {
        organizer_name: " Cloud Native Budapest ".into(),
        contact_email: "hello@example.com".into(),
        privacy_email: "dpo@example.com".into(),
        privacy_text: "We keep nothing else.".into(),
        privacy_url: "https://example.com/privacy".into(),
        rules: "Entrants must be 18 or older.".into(),
        retention_days: Some(3),
        registration: false,
    };
    service::save_defaults(&app, ACTOR, defaults).await.unwrap();
    let saved = service::get_defaults(&app, OWNER).await.unwrap();
    assert_eq!(saved.organizer_name, "Cloud Native Budapest", "trimmed");
    assert!(!saved.registration);

    let event = service::EventDate {
        date: "2026-10-28".into(),
        ends_at: None,
    };
    let id = service::create(&app, ACTOR, "Meetup", &event)
        .await
        .unwrap();
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(
        (
            r.organizer_name.as_str(),
            r.contact_email.as_str(),
            r.privacy_email.as_str(),
            r.privacy_text.as_str(),
            r.privacy_url.as_str(),
            r.rules.as_str(),
            r.retention_days,
            r.registration,
        ),
        (
            "Cloud Native Budapest",
            "hello@example.com",
            "dpo@example.com",
            "We keep nothing else.",
            "https://example.com/privacy",
            "Entrants must be 18 or older.",
            3,
            false,
        )
    );
    assert_eq!(r.event_date, "2026-10-28");
    assert_eq!(
        r.erase_at,
        r.event_ends_at + 3 * 86_400,
        "the erasure date follows the owner's period"
    );

    service::save_defaults(&app, ACTOR, Default::default())
        .await
        .unwrap();
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(r.rules, "Entrants must be 18 or older.", "a copy");
    let other = service::create(&app, service::Actor::solo("someone-else"), "Theirs", &event)
        .await
        .unwrap();
    let r = raffles::get(&app.db, &other).await.unwrap().unwrap();
    assert!(r.organizer_name.is_empty() && r.rules.is_empty());
    assert!(r.registration, "registration on without defaults");
    assert!(
        service::get_defaults(&app, "someone-else")
            .await
            .unwrap()
            .registration,
        "none saved: on"
    );
    assert_eq!(
        r.retention_days, app.policy.retention.days,
        "the installation's period without defaults"
    );

    // Defaults are checked like settings.
    let over = rafi_api::RaffleDefaults {
        retention_days: Some(app.policy.retention.max_days + 1),
        ..Default::default()
    };
    assert!(matches!(
        service::save_defaults(&app, ACTOR, over).await,
        Err(service::Error::Refused(service::Refusal::Retention { .. }))
    ));
}

/// Registration opens only once the notice can name the organizer and a
/// way to reach them.
#[tokio::test]
async fn registration_needs_an_organizer() {
    let app = app().await;
    let id = service::create(&app, ACTOR, "Nameless", &service::EventDate::today(&app))
        .await
        .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &id).await.unwrap();
    assert_eq!(
        admin.actions.missing,
        ["the organizer's name", "a contact or privacy email"]
    );
    let refused = service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::OrganizerMissing { fields, registration: true }))
                if fields == &["the organizer's name", "a contact or privacy email"]
        ),
        "{refused:?}"
    );
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Ready
    );

    // A privacy contact alone is a way to reach them.
    let mut settings = admin.settings;
    settings.organizer_name = "Org".into();
    settings.privacy_email = "dpo@example.com".into();
    service::update_settings(&app, ACTOR, &id, settings)
        .await
        .unwrap();
    assert!(
        service::admin(&app, OWNER, &id)
            .await
            .unwrap()
            .actions
            .missing
            .is_empty()
    );
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Open
    );
}

/// Switches raffle `id`'s registration (joining with the QR code) as
/// `actor`.
async fn set_registration(app: &AppState, id: &str, registration: bool) -> service::Result<()> {
    let mut settings = service::settings_of(&raffles::get(&app.db, id).await.unwrap().unwrap());
    settings.registration = registration;
    service::update_settings(app, ACTOR, id, settings).await
}

/// Without registration, the projector's view says so, and `/j/` has no
/// form and takes no join: the organizer adds the people taking part, and
/// the page links the rules, the notice and, once frozen, the verification
/// page.
#[tokio::test]
async fn without_registration_nobody_joins() {
    let app = app().await;
    let router = routes(app.clone());
    let id = create(&app, OWNER, "Gala").await;
    set_registration(&app, &id, false).await.unwrap();
    service::upsert_prize(
        &app,
        ACTOR,
        &id,
        PrizeInput {
            name: "Book".into(),
            qty: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let get = |url: String| {
        let router = router.clone();
        async move { send(&router, Request::get(url).body(Body::empty()).unwrap()).await }
    };

    let (status, _, body) = get(format!("/api/r/{}/view", raffle.slug)).await;
    assert_eq!(status, StatusCode::OK);
    let view: rafi_api::RaffleView = serde_json::from_str(&body).unwrap();
    assert!(!view.registration);
    assert_eq!(view.state, RaffleState::Ready);

    let no_registration =
        "This raffle has no registration: the organizer adds the people taking part.";
    let (status, _, page) = get(format!("/j/{}", raffle.join_code)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(no_registration), "{page}");
    assert!(!page.contains("name=\"email\""), "no form: {page}");
    assert!(!page.contains("not open yet"), "{page}");
    for link in [
        format!("href=\"/rules/{}\"", raffle.slug),
        format!("href=\"/privacy/{}\"", raffle.slug),
    ] {
        assert!(page.contains(&link), "{link}: {page}");
    }
    assert!(
        !page.contains(">Verification page<"),
        "nothing frozen yet: {page}"
    );

    let (status, cookie, body) = join(&router, &raffle.join_code, "Anna", "anna@example.com").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(cookie.is_none());
    assert!(body.contains(no_registration), "{body}");
    assert!(!body.contains("name=\"email\""), "{body}");
    assert!(entrants::list(&app.db, &id).await.unwrap().is_empty());

    // The organizer's list is who takes part.
    add_one(&app, ACTOR, &id, "Anna", None).await.unwrap();
    service::start_draw(&app, ACTOR, &id).await.unwrap();
    let (_, _, page) = get(format!("/j/{}", raffle.join_code)).await;
    assert!(page.contains(no_registration), "{page}");
    assert!(
        page.contains(&format!("href=\"/v/{}\"", raffle.slug))
            && page.contains(">Verification page<"),
        "{page}"
    );
    assert_eq!(
        set_registration(&app, &id, true)
            .await
            .unwrap_err()
            .to_string(),
        "Joining with the QR code can't be switched on or off while registration is open (close it first), or once the list is frozen."
    );
}

/// A raffle without registration freezes from Published only after the
/// checks opening would have made: a named controller and contact, and an
/// erasure date not passed.
#[tokio::test]
async fn a_freeze_from_published_checks_what_opening_would() {
    let app = app().await;
    let id = service::create(&app, ACTOR, "Nameless", &service::EventDate::today(&app))
        .await
        .unwrap();
    set_registration(&app, &id, false).await.unwrap();
    // Erased at the end of today.
    retain(&app, &id, 0).await;
    service::upsert_prize(
        &app,
        ACTOR,
        &id,
        PrizeInput {
            name: "Book".into(),
            qty: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    add_one(&app, ACTOR, &id, "Anna", None).await.unwrap();

    let refused = service::start_draw(&app, ACTOR, &id).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::OrganizerMissing { fields, registration: false }))
                if fields == &["the organizer's name", "a contact or privacy email"]
        ),
        "{refused:?}"
    );
    name_organizer(&app, ACTOR, &id).await;
    app.clock.advance(DAY);
    let refused = service::start_draw(&app, ACTOR, &id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::ErasureDatePassed))
        ),
        "{refused:?}"
    );
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Ready,
        "nothing frozen"
    );
}

/// Registration can't be switched off while it is open: phones in the
/// middle of the form would be cut off. Switched on (the default), the
/// raffle opens as ever.
#[tokio::test]
async fn registration_stays_while_open() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    assert!(raffle.registration);
    let refused = set_registration(&app, &raffle.id, false).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(
                service::Refusal::RegistrationLocked
            ))
        ),
        "{refused:?}"
    );
    assert!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .registration
    );
}

/// A raffle dated in the past, before anyone joined, is warned about: it
/// expires on its erasure date, or within the hour once that has passed.
#[tokio::test]
async fn a_past_event_date_is_warned_about() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    app.clock.advance(2 * DAY);
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(!admin.retention.past_erase_date);
    assert!(admin.retention.expires_unfrozen, "{:?}", admin.retention);
    assert!(!admin.retention.unfinished_past_event);
    assert!(raffle.erase_at > app.clock.now());

    app.clock.advance(8 * DAY);
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(admin.retention.past_erase_date && admin.retention.expires_unfrozen);
    // The date can still move: nobody has joined.
    assert!(admin.actions.edit_rules);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.expired, 1);
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.view.state, RaffleState::Purged);
    assert!(!admin.retention.expires_unfrozen, "nothing left to expire");
}

/// Over HTTP: a locked setting is a 409 that names the field; an event date
/// over a year ahead is a 400; registration can't open once the erasure date
/// has passed.
#[tokio::test]
async fn settings_refusals_over_http() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    // `setup_open` works as OWNER; dev auth is `dev`, so hand it over.
    app.db
        .query("UPDATE raffles SET owner_id = 'dev' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    add_one(&app, service::Actor::solo("dev"), &raffle.id, "Anna", None)
        .await
        .unwrap();

    let settings = |event_date: &str, retention_days: i64| {
        let mut s = service::settings_of(&raffle);
        s.event_date = event_date.into();
        s.event_ends_at = None;
        s.retention_days = retention_days;
        format!(
            r#"{{"id":"{}","settings":{}}}"#,
            raffle.id,
            serde_json::to_string(&s).unwrap()
        )
    };
    let (status, _, body) = send(
        &router,
        admin_call("save_settings", &settings("2026-12-24", 7)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, "settings_fixed");
    assert_eq!(
        err.message,
        "The event date cannot change: entrants are already in the raffle under what was set."
    );

    let fresh = create(&app, "dev", "Far").await;
    let far = raffles::get(&app.db, &fresh).await.unwrap().unwrap();
    let mut s = service::settings_of(&far);
    s.event_date = "2099-12-31".into();
    s.event_ends_at = None;
    let (status, _, body) = send(
        &router,
        admin_call(
            "save_settings",
            &format!(
                r#"{{"id":"{fresh}","settings":{}}}"#,
                serde_json::to_string(&s).unwrap()
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("at most 366 days ahead"), "{body}");

    // Past its erasure date, a never-frozen raffle can't open.
    let stale = create(&app, "dev", "Stale").await;
    app.clock.advance(10 * DAY);
    service::lifecycle(
        &app,
        service::Actor::solo("dev"),
        &stale,
        LifecycleCommand::Publish,
    )
    .await
    .unwrap();
    let (status, _, body) = send(
        &router,
        admin_call(
            "set_lifecycle",
            &format!(r#"{{"id":"{stale}","action":"open"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("expires within the hour"), "{body}");
}

/// A raffle can be deleted until its first draw is announced: a freeze by
/// accident has a way out, and a raffle that drew keeps its record. The
/// admin is told which, and a frozen raffle's engine state goes with it.
#[tokio::test]
async fn deleting_stops_at_the_first_draw() {
    let app = app().await;
    let router = routes(app.clone());

    // Frozen by accident: deletable, and the engine forgets it.
    let frozen = setup_open(&app, &[("Book", 1)]).await;
    join(&router, &frozen.join_code, "Anna", "anna@example.com").await;
    service::start_draw(&app, ACTOR, &frozen.id).await.unwrap();
    wait_for(&app, &frozen.id, |r| r.raffle.opened()).await;
    assert!(
        service::admin(&app, OWNER, &frozen.id)
            .await
            .unwrap()
            .actions
            .delete
    );
    service::delete(&app, ACTOR, &frozen.id).await.unwrap();
    assert!(raffles::get(&app.db, &frozen.id).await.unwrap().is_none());
    let engine = app.engine.as_ref().unwrap();
    let forgotten = engine
        .decide(
            &RaffleId::new(&frozen.id),
            &Decision::Finish,
            "after-delete",
        )
        .await;
    assert!(
        forgotten.is_err(),
        "the engine was asked to finish and forgot the raffle: {forgotten:?}"
    );

    // Drawn: the record stays; the admin offers no delete, and a call refuses.
    let drawn = setup_open(&app, &[("Mug", 1)]).await;
    join(&router, &drawn.join_code, "Bence", "bence@example.com").await;
    service::start_draw(&app, ACTOR, &drawn.id).await.unwrap();
    let state = wait_for(&app, &drawn.id, |r| r.raffle.opened())
        .await
        .drawn
        .unwrap();
    let item = state.items()[0].id.clone();
    service::command(
        &app,
        ACTOR,
        &drawn.id,
        DrawRequest { item: item.clone() },
        "d1",
    )
    .await
    .unwrap();
    wait_for(&app, &drawn.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.items()[0].status() == ItemStatus::Drawn)
    })
    .await;
    assert!(
        !service::admin(&app, OWNER, &drawn.id)
            .await
            .unwrap()
            .actions
            .delete
    );
    let refused = service::delete(&app, ACTOR, &drawn.id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::Drawn))
        ),
        "{refused:?}"
    );
    assert_eq!(service::Refusal::Drawn.code(), "drawn");
    assert!(
        service::Refusal::Drawn
            .to_string()
            .contains("stays verifiable")
    );
    // Finished with the one draw, still not deletable: purge is the way.
    service::command(
        &app,
        ACTOR,
        &drawn.id,
        Decision::Resolve {
            k: None,
            item,
            resolution: Resolution::Claimed,
        },
        "r1",
    )
    .await
    .unwrap();
    service::command(&app, ACTOR, &drawn.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &drawn.id).await.unwrap();
    assert!(!admin.actions.delete && admin.actions.purge);
}
