//! The participant page and the join: proof-of-work, consent, the cap, names,
//! the erasure date.

use super::*;

#[tokio::test]
async fn join_requires_proof_of_work_and_consent() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let post = |body: String| {
        Request::post(format!("/j/{}", raffle.join_code))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    };
    let (status, _, page) = send(
        &router,
        post(form(&[
            ("name", "Bot"),
            ("email", "bot@example.com"),
            ("rules", "on"),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("bot check"));
    let proof = solved_pow(&router).await;
    let (status, _, page) = send(
        &router,
        post(form(&[
            ("name", "Ann"),
            ("email", "ann@example.com"),
            ("proof", &proof),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("accept the rules"));
    let proof = solved_pow(&router).await;
    let (status, _, _) = send(
        &router,
        post(form(&[
            ("name", "Porn King"),
            ("email", "x@example.com"),
            ("rules", "on"),
            ("proof", &proof),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The hard limit: the join's `INSERT` refuses entrants beyond the cap, with
/// a clear message; someone already in is still told so.
#[tokio::test]
async fn joins_stop_at_the_entrant_cap() {
    let mut cfg = Config::for_tests();
    cfg.max_entrants = 2;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    for i in 0..2 {
        let (status, cookie, _) = join(
            &router,
            &raffle.join_code,
            &format!("P{i}"),
            &format!("p{i}@example.com"),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "join {i}");
        assert!(cookie.is_some());
    }
    let (status, cookie, page) = join(&router, &raffle.join_code, "Late", "late@example.com").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(cookie.is_none());
    assert!(page.contains("this raffle is full"), "{page}");
    assert!(page.contains("id=\"join\""), "the form stays");
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 2);

    let (status, _, page) = join(&router, &raffle.join_code, "Again", "p0@example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("already entered"), "{page}");
}

/// Typing the email of someone the organizer added says the email is in,
/// in the words used for everyone (no device has their claim code), and
/// never attaches this device to the entry: no cookie, no token stored, the
/// entry as it was.
#[tokio::test]
async fn an_added_entrants_email_doesnt_attach_a_device() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let added = add_one(
        &app,
        ACTOR,
        &raffle.id,
        "Anna Kovács",
        Some("anna@example.com".into()),
    )
    .await
    .unwrap();
    let before = entrants::get(&app.db, &raffle.id, &added)
        .await
        .unwrap()
        .unwrap();

    let (status, cookie, page) =
        join(&router, &raffle.join_code, "Anna", "Anna+phone@Example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_none(), "no entry cookie");
    assert!(
        page.contains(
            "This email is already entered. Your entry is safe. If you win, go to the organizer."
        ),
        "{page}"
    );
    assert!(!page.contains("Your claim code"), "no entry shown: {page}");
    assert!(!page.contains("the device you entered on"), "{page}");
    assert!(!page.contains("id=\"join\""), "no form to try again");

    let all = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(all.len(), 1, "no second entry");
    let after = &all[0];
    assert_eq!(after.id, added);
    assert_eq!(after.token_hash, None, "no device attached");
    assert_eq!(after.source, rafi_lifecycle::entrant::Source::Manual);
    assert_eq!(after.display_name, before.display_name);
    assert_eq!(after.email_raw, before.email_raw);
    assert_eq!(after.claim_code, before.claim_code);
    assert_eq!(
        (&after.consent_version, after.consent_rules_at),
        (&None, None),
        "added by the organizer: no consent recorded"
    );
}

/// A proof-of-work is redeemed once: the second use fails the `pow_used`
/// primary key. Expired rows leave with the housekeeping run.
#[tokio::test]
async fn proof_of_work_replays_are_refused() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let proof = solved_pow(&router).await;
    let post = |name: &str, email: &str| {
        Request::post(format!("/j/{}", raffle.join_code))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form(&[
                ("name", name),
                ("email", email),
                ("rules", "on"),
                ("proof", &proof),
            ])))
            .unwrap()
    };
    let (status, _, _) = send(&router, post("Anna", "anna@example.com")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, _, page) = send(&router, post("Bot", "bot@example.com")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("bot check"), "{page}");
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 1);

    let payload = service::pow::decode(&proof).unwrap();
    let rows: Vec<(String, i64)> = app
        .db
        .query("SELECT challenge, expires_at FROM pow_used")
        .all()
        .await
        .unwrap();
    assert_eq!(
        rows,
        [(
            payload.challenge.parameters.nonce.clone(),
            payload.challenge.parameters.expires_at.unwrap()
        )]
    );
    assert!(!service::pow::redeem(&app, &proof).await.unwrap());

    // A forged solution never reaches the table.
    let mut forged = payload.clone();
    forged.challenge.signature = Some("00".repeat(32));
    assert!(
        !service::pow::redeem(&app, &service::pow::encode(&forged))
            .await
            .unwrap()
    );

    app.db
        .query("INSERT INTO pow_used (challenge, expires_at) VALUES ('old', 1)")
        .run()
        .await
        .unwrap();
    service::housekeeping::run(&app).await.unwrap();
    let (left,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM pow_used")
        .one()
        .await
        .unwrap();
    assert_eq!(left, 1, "only the unexpired challenge stays");
}

/// Only the server's own, unexpired, correctly solved challenges pass, each
/// once.
#[tokio::test]
async fn proof_of_work_forgeries_are_refused() {
    let app = app().await;
    let issue = |key: &str, expires_at: Option<i64>| {
        altcha::create_challenge(altcha::CreateChallengeOptions {
            algorithm: service::pow::ALGORITHM.into(),
            cost: 1,
            key_prefix: service::pow::KEY_PREFIX.into(),
            expires_at,
            hmac_signature_secret: Some(key.into()),
            ..Default::default()
        })
        .unwrap()
    };
    // Challenges expire by real time (the library's clock), not the app's.
    let now = service::Clock::system().now();
    let soon = Some(now + 600);
    let refused = async |payload: &altcha::Payload| {
        !service::pow::redeem(&app, &service::pow::encode(payload))
            .await
            .unwrap()
    };

    let expired = solve_pow_from(issue(crate::config::TEST_POW_KEY, Some(now - 1)), 0);
    assert!(refused(&expired).await, "expired");
    let foreign = solve_pow_from(issue("another-key", soon), 0);
    assert!(refused(&foreign).await, "signed with another key");
    let eternal = solve_pow_from(issue(crate::config::TEST_POW_KEY, None), 0);
    assert!(refused(&eternal).await, "no expiresAt");

    let good = solve_pow_from(issue(crate::config::TEST_POW_KEY, soon), 0);
    let mut cheaper = good.clone();
    cheaper.challenge.parameters.cost = 0;
    assert!(refused(&cheaper).await, "a changed parameter");
    let mut counter = good.clone();
    counter.solution.counter += 1;
    assert!(refused(&counter).await, "a tampered counter");
    let mut key = good.clone();
    key.solution.derived_key = format!("00{}", "1".repeat(62));
    assert!(refused(&key).await, "the wrong derived key");

    // Accepted once; another valid counter for the same challenge is a replay.
    assert!(!refused(&good).await, "the genuine solution");
    let other = solve_pow_from(good.challenge.clone(), good.solution.counter + 1);
    assert_ne!(other.solution.counter, good.solution.counter);
    assert!(
        refused(&other).await,
        "a second solution of the same challenge"
    );

    assert!(
        !service::pow::redeem(&app, "not base64!").await.unwrap(),
        "junk"
    );
    let long = "A".repeat(service::pow::MAX_PAYLOAD_LEN + 4);
    assert!(
        !service::pow::redeem(&app, &long).await.unwrap(),
        "too long"
    );
}

/// Leaving the name blank gets a generated one, which the page then shows;
/// the form offers generated names to pick from.
#[tokio::test]
async fn a_blank_name_gets_a_generated_one() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("id=\"name-dice\""), "the 🎲 button");
    let name_input = page
        .split("id=\"name\"")
        .nth(1)
        .and_then(|rest| rest.split('>').next())
        .unwrap();
    assert!(!name_input.contains("required"), "optional: {name_input}");
    let config: rafi_live_client::PageConfig = serde_json::from_str(
        page.split("id=\"rafi-config\">")
            .nth(1)
            .and_then(|rest| rest.split("</script>").next())
            .unwrap(),
    )
    .unwrap();
    assert!(!config.name_ideas.is_empty());
    for idea in &config.name_ideas {
        assert_eq!(
            rafi_lifecycle::entrant::display_name(idea).as_ref(),
            Ok(idea)
        );
    }

    let (status, cookie, _) = join(&router, &raffle.join_code, "   ", "blank@example.com").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let entrant = entrants::list(&app.db, &raffle.id).await.unwrap().remove(0);
    let generated: std::collections::HashSet<String> = (0
        ..rafi_lifecycle::entrant::GENERATED_NAMES)
        .map(rafi_lifecycle::entrant::generated_name)
        .collect();
    assert!(
        generated.contains(&entrant.display_name),
        "{}",
        entrant.display_name
    );
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .header(header::COOKIE, cookie.unwrap())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains(&entrant.display_name), "shown on the ticket");
    assert!(
        page.contains(&format!("/v/{}#me={}", raffle.slug, entrant.id)),
        "the verification link finds them"
    );

    // A name that is too long is still refused.
    let (status, _, page) = join(
        &router,
        &raffle.join_code,
        &"x".repeat(rafi_lifecycle::entrant::MAX_NAME_CHARS + 1),
        "long@example.com",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("leave it blank"), "{page}");
}

/// Once a raffle's erasure date has passed, nobody joins it, even while it is
/// still open (it expires within the hour): the form would promise erasure
/// on a day gone by. The admin is warned a day ahead.
#[tokio::test]
async fn nobody_joins_past_the_erasure_date() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    // A period of 0: erased at midnight, and a day passes.
    let raffle = retain(&app, &raffle.id, 0).await;
    app.clock.advance(DAY);
    assert!(raffle.past_erase_date(app.clock.now()));
    assert_eq!(
        raffle.state,
        RaffleState::Open,
        "still open until housekeeping runs"
    );

    let (status, cookie, body) = join(&router, &raffle.join_code, "Anna", "anna@example.com").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(cookie.is_none());
    assert!(body.contains("Joining has closed."), "{body}");
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .is_empty()
    );
    // The join page says so too, with no form.
    let (status, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!page.contains("name=\"email\""), "no form: {page}");

    // A day ahead, the admin is warned; past the date, told it has passed.
    let soon = setup_open(&app, &[("Book", 1)]).await;
    retain(&app, &soon.id, 0).await;
    let r = service::admin(&app, OWNER, &soon.id)
        .await
        .unwrap()
        .retention;
    assert!(r.erasure_imminent && !r.past_erase_date, "{r:?}");
    let r = service::admin(&app, OWNER, &raffle.id)
        .await
        .unwrap()
        .retention;
    assert!(!r.erasure_imminent && r.past_erase_date, "{r:?}");
}
