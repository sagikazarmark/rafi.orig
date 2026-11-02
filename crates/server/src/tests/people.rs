//! The organizer's entrants: fakes, adds, added names, removals and objections.

use super::*;

#[tokio::test]
async fn fake_entrants_are_added_in_one_batch() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let mut settings = service::settings_of(&raffle);
    settings.test_mode = true;
    service::update_settings(&app, ACTOR, &raffle.id, settings)
        .await
        .unwrap();
    service::add_fakes(
        &app,
        ACTOR,
        &raffle.id,
        rafi_lifecycle::entrant::MAX_FAKES + 1,
    )
    .await
    .unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(entrants.len(), rafi_lifecycle::entrant::MAX_FAKES as usize);
    assert!(
        entrants
            .iter()
            .all(|e| e.source == rafi_lifecycle::entrant::Source::Fake && !e.hidden)
    );
    let ids: std::collections::HashSet<_> = entrants.iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), entrants.len());
}

/// Switching test mode off removes the fake entrants; a raffle out of test
/// mode doesn't freeze with fakes; fakes get generated names; the
/// verification data says a rehearsal is one.
#[tokio::test]
async fn fakes_stay_in_test_mode() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let set_test_mode = |on: bool| {
        let mut settings = service::settings_of(&raffle);
        settings.test_mode = on;
        service::update_settings(&app, ACTOR, &raffle.id, settings)
    };
    set_test_mode(true).await.unwrap();
    service::add_fakes(&app, ACTOR, &raffle.id, 5)
        .await
        .unwrap();
    add_one(&app, ACTOR, &raffle.id, "Real Person", None)
        .await
        .unwrap();
    let listed = entrants::list(&app.db, &raffle.id).await.unwrap();
    let fakes: Vec<_> = listed
        .iter()
        .filter(|e| e.source == rafi_lifecycle::entrant::Source::Fake)
        .collect();
    assert_eq!(fakes.len(), 5);
    let generated: std::collections::HashSet<String> = (0
        ..rafi_lifecycle::entrant::GENERATED_NAMES)
        .map(rafi_lifecycle::entrant::generated_name)
        .collect();
    assert!(
        fakes
            .iter()
            .all(|e| generated.contains(&e.display_name) && e.consent_version.is_none()),
        "{fakes:?}"
    );

    set_test_mode(false).await.unwrap();
    let left = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(left.len(), 1, "the fakes went with test mode");
    assert_eq!(left[0].display_name, "Real Person");
    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&raffle.id)
        .all()
        .await
        .unwrap();
    assert_eq!(
        &audit[audit.len() - 2..],
        [
            ("remove_fakes".into(), "5".into()),
            ("settings".into(), "test_mode".into())
        ],
        "the removal, and the setting that changed, by name: {audit:?}"
    );
    assert!(
        service::add_fakes(&app, ACTOR, &raffle.id, 1)
            .await
            .is_err(),
        "no fakes outside test mode"
    );

    // Fakes left behind all the same (say, by a restored backup): no freeze.
    app.db
        .query(
            "INSERT INTO entrants (id, raffle_id, display_name, source, claim_code, created_at)
             VALUES ('ffffffffffffffffffffffffffffffff', ?, 'Stray', 'fake', 'S1', 0)",
        )
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    let refused = service::start_draw(&app, ACTOR, &raffle.id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::Start(
                rafi_lifecycle::start::Refusal::FakeEntrants
            )))
        ),
        "{refused:?}"
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Open
    );

    // A rehearsal freezes with its fakes, and says so.
    set_test_mode(true).await.unwrap();
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(data.test_mode);
    assert!(
        matches!(
            set_test_mode(false).await,
            Err(service::Error::Refused(service::Refusal::TestModeLocked))
        ),
        "test mode is fixed once frozen"
    );
}

/// Adding people over the admin API: one person as a batch of one (no
/// consent, a claim code, retried safely), a list in its order, each
/// refusal with its status, the rows' problems by index, and undo.
#[tokio::test]
async fn adding_people_over_http() {
    let mut cfg = Config::for_tests();
    cfg.max_entrants = 4;
    let (app, router, raffle) = dev_raffle(cfg).await;
    let id = raffle.id.as_str();

    let one = add_args(
        id,
        "b-one",
        "one",
        serde_json::json!([{ "name": "Anna", "email": "anna@example.com" }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &one)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: rafi_api::AddResult = serde_json::from_str(&body).unwrap();
    assert_eq!((result.added.len(), result.skipped.len()), (1, 0));
    let anna = entrants::get(&app.db, id, &result.added[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(anna.source, rafi_lifecycle::entrant::Source::Manual);
    assert!(anna.consent_version.is_none() && anna.claim_code.is_some());
    // The reply was lost; the browser sends the same add again.
    let (status, _, again) = send(&router, admin_call("add_entrants", &one)).await;
    assert_eq!((status, again), (StatusCode::OK, body));
    assert_eq!(entrants::list(&app.db, id).await.unwrap().len(), 1);

    // An invalid row refuses the add, with every row's problem.
    let bad = add_args(
        id,
        "b-bad",
        "list",
        serde_json::json!([
            { "name": "Bence" },
            { "name": "Csilla", "email": "not an email" },
            { "name": "Dora", "email": "ANNA@example.com", "list_ref": "7" },
            { "name": "Emil", "list_ref": "7" },
        ]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, "rows");
    let rows: Vec<(u32, rafi_api::RowError)> =
        err.rows.iter().map(|r| (r.row, r.problem)).collect();
    assert_eq!(
        rows,
        [
            (1, rafi_api::RowError::Email),
            (3, rafi_api::RowError::DuplicateListRef)
        ],
        "an email already in is no problem: it is skipped"
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["rows"][0],
        serde_json::json!({ "row": 1, "problem": "email" })
    );

    // A list: in its order, who is in skipped; past the cap, refused whole.
    let list = add_args(
        id,
        "b-list",
        "list",
        serde_json::json!([
            { "name": "Zoe" },
            { "name": "Anna K.", "email": "Anna+list@example.com" },
            { "name": "Bence", "list_ref": "T-1" },
            { "name": "Csilla" },
            { "name": "Dora" },
        ]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &list)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, "room");
    assert!(
        err.message.contains("Only 3 more people fit"),
        "{}",
        err.message
    );
    assert_eq!(entrants::list(&app.db, id).await.unwrap().len(), 1);
    app.clock.advance(1);
    let list = add_args(
        id,
        "b-list",
        "list",
        serde_json::json!([
            { "name": "Zoe" },
            { "name": "Anna K.", "email": "Anna+list@example.com" },
            { "name": "Bence", "list_ref": "T-1" },
            { "name": "Csilla" },
        ]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &list)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: rafi_api::AddResult = serde_json::from_str(&body).unwrap();
    assert_eq!(
        result.added.iter().map(|a| a.row).collect::<Vec<_>>(),
        [0, 2, 3]
    );
    assert_eq!(result.skipped[0].row, 1);
    assert!(body.contains(r#""reason":"already_in""#), "{body}");
    let names: Vec<String> = entrants::list(&app.db, id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.display_name)
        .collect();
    assert_eq!(names, ["Zoe", "Bence", "Csilla", "Anna"]);

    // The admin's view offers to undo the list; undoing it takes it back.
    let (_, _, body) = send(
        &router,
        admin_call("get_raffle", &format!(r#"{{"id":"{id}"}}"#)),
    )
    .await;
    let admin: rafi_api::RaffleAdmin = serde_json::from_str(&body).unwrap();
    assert_eq!(
        admin.actions.undo,
        Some(rafi_api::UndoBatch {
            batch_id: "b-list".into(),
            count: 3
        })
    );
    let bence = admin
        .entrants
        .iter()
        .find(|e| e.display_name == "Bence")
        .unwrap();
    assert_eq!(bence.list_ref.as_deref(), Some("T-1"));
    let (status, _, body) = send(
        &router,
        admin_call(
            "undo_add",
            &format!(r#"{{"id":"{id}","batch_id":"b-list"}}"#),
        ),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "3"));
    assert_eq!(entrants::list(&app.db, id).await.unwrap().len(), 1);

    // After the freeze: nothing is added, nothing undone.
    service::start_draw(&app, service::Actor::solo("dev"), id)
        .await
        .unwrap();
    let late = add_args(id, "b-late", "one", serde_json::json!([{ "name": "Late" }]));
    let (status, _, body) = send(&router, admin_call("add_entrants", &late)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("entrants_frozen"), "{body}");
    let (status, _, body) = send(
        &router,
        admin_call(
            "undo_add",
            &format!(r#"{{"id":"{id}","batch_id":"b-one"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? AND action IN ('add', 'undo_add') ORDER BY id")
        .bind(id)
        .all()
        .await
        .unwrap();
    assert_eq!(
        audit,
        [
            (
                "add".into(),
                "batch=b-one origin=one added=1 skipped=0 approved=0 lifted=0".into()
            ),
            (
                "add".into(),
                "batch=b-list origin=list added=3 skipped=1 approved=0 lifted=0".into()
            ),
            ("undo_add".into(), "batch=b-list removed=3".into()),
        ]
    );
}

/// Adds and undos are limited per organizer (429), and the admin API reads
/// at most 512 KB of arguments (413), before anything else.
#[tokio::test]
async fn adding_people_is_bounded_over_http() {
    let (app, router, raffle) = dev_raffle(Config::for_tests()).await;
    let id = raffle.id.as_str();

    let huge = add_args(
        id,
        "b-huge",
        "list",
        serde_json::json!([{ "name": "x".repeat(crate::admin::MAX_BODY_BYTES) }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &huge)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, rafi_api::admin::codes::TOO_LARGE);

    let (budget, _) = service::Limit::AddPerOwner.budget();
    for i in 0..budget {
        let add = add_args(
            id,
            &format!("b-{i}"),
            "one",
            serde_json::json!([{ "name": format!("Person {i}") }]),
        );
        let (status, _, body) = send(&router, admin_call("add_entrants", &add)).await;
        assert_eq!(status, StatusCode::OK, "{i}: {body}");
    }
    let add = add_args(
        id,
        "b-more",
        "one",
        serde_json::json!([{ "name": "One more" }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &add)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, "limited");
    let (status, _, _) = send(
        &router,
        admin_call("undo_add", &format!(r#"{{"id":"{id}","batch_id":"b-0"}}"#)),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        entrants::list(&app.db, id).await.unwrap().len(),
        budget as usize
    );
}

/// Over HTTP: a name the organizer adds that someone is already shown with
/// (who joined, who was added before, an earlier row; case, spacing and
/// composition aside) refuses the add with a 400 and the rows, naming
/// whom; one person too. A blocked name goes in only approved, the
/// approval lifting the blocklist alone, and is counted in the audit with
/// the name format.
#[tokio::test]
async fn added_names_are_unique_and_approved_over_http() {
    let (app, router, raffle) = dev_raffle(Config::for_tests()).await;
    let id = raffle.id.as_str();
    let (status, _, _) = join(&router, &raffle.join_code, "Anna K.", "anna@example.com").await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let add = add_args(
        id,
        "b-clash",
        "list",
        serde_json::json!([
            { "name": "ANNA  K." },
            { "name": "Bence" },
            { "name": "be\u{301}la" },
            { "name": "bence" },
            { "name": "Béla" },
        ]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &add)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, "rows");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["rows"],
        serde_json::json!([
            { "row": 0, "problem": "name_taken", "shown_as": "Anna K." },
            { "row": 3, "problem": "duplicate_name", "shown_as": "Bence" },
            { "row": 4, "problem": "duplicate_name", "shown_as": "béla" },
        ])
    );
    assert!(
        err.message
            .contains("row 1: someone is already shown as “Anna K.”"),
        "{}",
        err.message
    );
    assert_eq!(entrants::list(&app.db, id).await.unwrap().len(), 1);

    // The one-person form is held to it too.
    let one = add_args(
        id,
        "b-one",
        "one",
        serde_json::json!([{ "name": "anna k." }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &one)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.message, "Someone is already shown as “Anna K.”.");
    assert_eq!(err.rows[0].problem, rafi_api::RowError::NameTaken);

    // A blocked name, refused unless approved; an approval lifts nothing
    // else.
    let blocked = |batch: &str, approved: bool, name: &str| {
        add_args(
            id,
            batch,
            "list",
            serde_json::json!([
                { "name": "Nazir Ahmed", "approved": approved },
                { "name": name, "approved": approved },
            ]),
        )
    };
    let (status, _, body) = send(
        &router,
        admin_call("add_entrants", &blocked("b-blocked", false, "Bence")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["rows"],
        serde_json::json!([{ "row": 0, "problem": "blocked_name" }])
    );
    let (status, _, body) = send(
        &router,
        admin_call("add_entrants", &blocked("b-sneaky", true, "Bence\u{200B}")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["rows"],
        serde_json::json!([{ "row": 1, "problem": "name" }]),
        "nothing invisible, approved or not"
    );
    let mut approved: serde_json::Value =
        serde_json::from_str(&blocked("b-approved", true, "Bence")).unwrap();
    approved["add"]["name_format"] = "full".into();
    let (status, _, body) = send(&router, admin_call("add_entrants", &approved.to_string())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<String> = entrants::list(&app.db, id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.display_name)
        .collect();
    assert_eq!(names, ["Nazir Ahmed", "Bence", "Anna K."]);
    let (detail,): (String,) = app
        .db
        .query("SELECT detail FROM audit WHERE raffle_id = ? AND action = 'add'")
        .bind(id)
        .one()
        .await
        .unwrap();
    assert_eq!(
        detail,
        "batch=b-approved origin=list added=2 skipped=0 approved=1 lifted=0 format=full"
    );

    // An unknown format is a bad request, not a guess.
    let mut unknown: serde_json::Value = serde_json::from_str(&add_args(
        id,
        "b-x",
        "list",
        serde_json::json!([{ "name": "Csilla" }]),
    ))
    .unwrap();
    unknown["add"]["name_format"] = "initials".into();
    let (status, _, body) = send(&router, admin_call("add_entrants", &unknown.to_string())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("invalid_arguments"), "{body}");
}

/// Names the organizer gives are stored whole up to 64 characters, joiners'
/// up to 24: an add of 64 goes in and one of 65 is refused with its row, a
/// rename of anyone to 64 goes in, a join of 25 is still refused. The long
/// names reach the projector, the phones, the verification page and the
/// winners' export whole.
#[tokio::test]
async fn names_the_organizer_gives_are_up_to_64_characters() {
    use rafi_lifecycle::entrant::{MAX_ADDED_NAME_CHARS, MAX_NAME_CHARS};
    let (app, router, raffle) = dev_raffle_with(Config::for_tests(), &[("Book", 2)]).await;
    let id = raffle.id.as_str();
    let dev = service::Actor::solo("dev");

    // 64 characters once composed: the "Á" is sent decomposed, as some
    // exports write it, 65 code points.
    let maria = "Maria Fernanda de los Ángeles García-López y Hernández Rodríguez";
    assert_eq!(maria.chars().count(), MAX_ADDED_NAME_CHARS);
    let decomposed = maria.replace('Á', "A\u{301}");
    let add = add_args(
        id,
        "b-long",
        "list",
        serde_json::json!([{ "name": decomposed }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &add)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: rafi_api::AddResult = serde_json::from_str(&body).unwrap();
    let maria_id = result.added[0].id.clone();
    let stored = entrants::get(&app.db, id, &maria_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.display_name, maria, "stored whole, composed");

    let too_long = add_args(
        id,
        "b-too-long",
        "one",
        serde_json::json!([{ "name": format!("{maria}s") }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &too_long)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    let rows: Vec<(u32, rafi_api::RowError)> =
        err.rows.iter().map(|r| (r.row, r.problem)).collect();
    assert_eq!(rows, [(0, rafi_api::RowError::Name)]);
    assert!(err.message.contains("1–64 characters"), "{}", err.message);

    // A joiner keeps 24.
    let (status, _, page) = join(
        &router,
        &raffle.join_code,
        &"x".repeat(MAX_NAME_CHARS + 1),
        "long@example.com",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("at most 24 characters"), "{page}");
    assert!(
        page.contains(&format!("maxlength=\"{MAX_NAME_CHARS}\"")),
        "the form says so too"
    );
    let (status, _, _) = join(&router, &raffle.join_code, "Bea", "bea@example.com").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let bea_id = entrants::list(&app.db, id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.display_name == "Bea")
        .unwrap()
        .id;

    // The organizer renames anyone up to 64, a joiner too.
    let beatrix = "Beatrix Kovácsné Szabó-Nagy, Budapesti Műszaki Egyetem, Budapest";
    assert_eq!(beatrix.chars().count(), MAX_ADDED_NAME_CHARS);
    let rename = |name: &str| {
        admin_call(
            "patch_entrant",
            &serde_json::json!({ "id": id, "entrant_id": bea_id, "patch": { "display_name": name } })
                .to_string(),
        )
    };
    let (status, _, body) = send(&router, rename(&format!("{beatrix}!"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("1–64 characters"), "{body}");
    let (status, _, body) = send(&router, rename(beatrix)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // On the projector, before the freeze: whole.
    let (_, _, view) = get_json(&router, &format!("/api/r/{}/view", raffle.slug)).await;
    assert_eq!(view["recent_names"][0], maria, "{view}");

    // Both win; every public and exported place has them whole.
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
    let mut won = Vec::new();
    for item in &items {
        won.push(draw_and_claim_as(&app, dev, id, item).await);
    }
    let name_of = |entrant: &str| if entrant == maria_id { maria } else { beatrix };
    let latest = won.last().unwrap().clone();
    won.sort();
    let mut both = vec![maria_id.clone(), bea_id.clone()];
    both.sort();
    assert_eq!(won, both);

    let (_, _, phone) = get_json(&router, &format!("/api/r/{}/participant", raffle.slug)).await;
    assert_eq!(
        phone["last"]["display_name"],
        name_of(&latest),
        "the phones: {phone}"
    );

    let (_, _, verify) = get_json(&router, &format!("/api/r/{}/verify.json", raffle.slug)).await;
    let names: Vec<(String, String)> = serde_json::from_value(verify["names"].clone()).unwrap();
    for entrant in [&maria_id, &bea_id] {
        assert!(
            names.contains(&(entrant.clone(), name_of(entrant).to_owned())),
            "verify.json: {names:?}"
        );
    }

    let csv = service::export_csv(&app, dev, id).await.unwrap();
    for name in [maria, beatrix] {
        assert!(csv.contains(&format!("\"{name}\"")), "{csv}");
    }
}

/// Removing someone at their request and someone taking themselves off,
/// over the admin API: `at_request` is optional, a later add says who it
/// left out and why, a row can lift it; after the freeze, "Took themselves
/// off" and its undo, with what the admin page is told and the refusals'
/// statuses.
#[tokio::test]
async fn removals_and_objections_over_http() {
    let (app, router, raffle) = dev_raffle(Config::for_tests()).await;
    let id = raffle.id.as_str();
    let list = add_args(
        id,
        "b-1",
        "list",
        serde_json::json!([
            { "name": "Anna", "email": "anna@example.com", "list_ref": "A1" },
            { "name": "Bence", "email": "bence@example.com" },
            { "name": "Csilla" },
        ]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &list)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: rafi_api::AddResult = serde_json::from_str(&body).unwrap();
    let (anna, bence, csilla) = (
        result.added[0].id.clone(),
        result.added[1].id.clone(),
        result.added[2].id.clone(),
    );
    let remove = |entrant: &str, extra: &str| {
        admin_call(
            "remove_entrant",
            &format!(r#"{{"id":"{id}","entrant_id":"{entrant}"{extra}}}"#),
        )
    };
    let (status, _, body) = send(&router, remove(&anna, r#","at_request":true"#)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = send(&router, remove(&bence, "")).await;
    assert_eq!(status, StatusCode::OK, "without at_request: {body}");
    let get = || admin_call("get_raffle", &format!(r#"{{"id":"{id}"}}"#));
    let (_, _, body) = send(&router, get()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["removed"].as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(json["removed"][0]["email_key"], "anna@example.com");
    assert_eq!(json["removed"][0]["list_ref"], "A1");

    // The list again: Anna is left out, and says why; lifted, she's in.
    let again = add_args(
        id,
        "b-2",
        "list",
        serde_json::json!([
            { "name": "Anna", "email": "anna@example.com" },
            { "name": "Bence", "email": "bence@example.com" },
        ]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &again)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["skipped"],
        serde_json::json!([{ "row": 0, "reason": "removed_at_request" }]),
        "{body}"
    );
    let lifted = add_args(
        id,
        "b-3",
        "list",
        serde_json::json!([{ "name": "Anna", "list_ref": "A1", "lift_removal": true }]),
    );
    let (status, _, body) = send(&router, admin_call("add_entrants", &lifted)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: rafi_api::AddResult = serde_json::from_str(&body).unwrap();
    let removed_anna = anna;
    let anna = result.added[0].id.clone();
    let (_, _, body) = send(&router, get()).await;
    let admin: rafi_api::RaffleAdmin = serde_json::from_str(&body).unwrap();
    assert!(admin.removed.is_empty(), "lifted");

    // After the freeze: Csilla takes herself off.
    service::start_draw(&app, service::Actor::solo("dev"), id)
        .await
        .unwrap();
    let object = |name: &str, entrant: &str| {
        admin_call(
            name,
            &format!(r#"{{"id":"{id}","entrant_id":"{entrant}"}}"#),
        )
    };
    let (status, _, body) = send(&router, object("object_entrant", &csilla)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, _, body) = send(&router, get()).await;
    let admin: rafi_api::RaffleAdmin = serde_json::from_str(&body).unwrap();
    let c = admin.entrants.iter().find(|e| e.id == csilla).unwrap();
    assert!(c.objected && c.hidden && c.actions.unobject && !c.actions.object);
    let a = admin.entrants.iter().find(|e| e.id == anna).unwrap();
    assert!(!a.objected && a.actions.object && !a.actions.unobject);
    let show = format!(
        r#"{{"id":"{id}","entrant_id":"{csilla}","patch":{{"display_name":null,"hidden":false,"delivered":null}}}}"#
    );
    let (status, _, body) = send(&router, admin_call("patch_entrant", &show)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("objected_hidden"), "{body}");
    let (status, _, body) = send(&router, remove(&anna, r#","at_request":true"#)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("not_removable"), "{body}");
    let (status, _, body) = send(&router, object("object_entrant", "nobody")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _, body) = send(&router, object("unobject_entrant", &csilla)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let audit: Vec<(String, String)> = app
        .db
        .query(
            "SELECT action, detail FROM audit WHERE raffle_id = ?
             AND action IN ('remove_entrant', 'object', 'unobject') ORDER BY id",
        )
        .bind(id)
        .all()
        .await
        .unwrap();
    assert_eq!(
        audit,
        [
            (
                "remove_entrant".into(),
                format!("{removed_anna} at_request")
            ),
            ("remove_entrant".into(), bence.clone()),
            ("object".into(), csilla.clone()),
            ("unobject".into(), csilla.clone()),
        ]
    );
}
