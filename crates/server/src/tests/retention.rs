//! Retention: the housekeeping run's erasures, expiries and purges, restores, a
//! stopped run.

use super::*;

/// The clocks on frozen raffles: non-winners go on the erasure date,
/// delivered winners with them, other winners after the hold; a finished
/// raffle is then purged, a drawing one is erased where it stands and
/// finished by the engine. (Never-frozen raffles are
/// [`never_frozen_raffles_expire`].)
#[tokio::test]
async fn housekeeping_erases_on_the_dates() {
    let (recording, live) = recording();
    let app = app_with_live(Config::for_tests(), live).await;
    let hold = app.policy.retention.winner_hold_secs();

    // A finished raffle: two prizes, one winner delivered, one not, one
    // drawn but not here. Today's, kept seven days.
    let finished = setup_open(&app, &[("Book", 1), ("Mug", 1)]).await;
    let router = routes(app.clone());
    for (name, email) in [
        ("Anna", "anna@x.org"),
        ("Bence", "bence@x.org"),
        ("Csilla", "csilla@x.org"),
        ("Dani", "dani@x.org"),
    ] {
        let (status, _, _) = join(&router, &finished.join_code, name, email).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
    }
    service::start_draw(&app, ACTOR, &finished.id)
        .await
        .unwrap();
    let r = wait_for(&app, &finished.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let delivered = draw_and_claim(&app, &finished.id, &items[0]).await;
    let held = draw_and_claim(&app, &finished.id, &items[1]).await;
    service::patch_entrant(
        &app,
        ACTOR,
        &finished.id,
        &delivered,
        rafi_api::EntrantPatch {
            delivered: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::command(&app, ACTOR, &finished.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &finished.id).await.unwrap();
    assert_eq!(admin.retention.held_winners, 1);
    assert!(admin.actions.deliver_all);
    let held_entrant = admin.entrants.iter().find(|e| e.id == held).unwrap();
    assert_eq!(held_entrant.held_until, Some(finished.erase_at + hold));

    // A drawing raffle, never finished: one claimed winner.
    let drawing = setup_open(&app, &[("Pen", 1)]).await;
    for name in ["Erik", "Fanni"] {
        add_one(&app, ACTOR, &drawing.id, name, None).await.unwrap();
    }
    service::start_draw(&app, ACTOR, &drawing.id).await.unwrap();
    let r = wait_for(&app, &drawing.id, |r| r.raffle.opened()).await;
    let pen = r.drawn.as_ref().unwrap().items()[0].id.clone();
    let drawing_winner = draw_and_claim(&app, &drawing.id, &pen).await;

    // Ten days on: due, not at the ceiling.
    app.clock.advance(10 * DAY);
    let now = app.clock.now();
    assert!(
        finished.erase_at < now && finished.erase_at + hold > now,
        "due, not at the ceiling"
    );
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(
        (report.expired, report.erased, report.purged),
        (0, 4, 0),
        "2 non-winners + the delivered winner, and the drawing raffle's non-winner, erased"
    );

    // Finished: everyone but the held winner is erased, the raffle stays finished.
    let entrants = entrants::list(&app.db, &finished.id).await.unwrap();
    for e in &entrants {
        assert_eq!(e.erased, e.id != held, "{}: {}", e.display_name, e.id);
        if e.erased {
            assert!(e.email_raw.is_none() && e.claim_code.is_none() && e.token_hash.is_none());
        }
    }
    let r = raffles::get(&app.db, &finished.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Finished);
    assert!(r.purged_at.is_none());
    let audit = audit_of(&app, &finished.id).await;
    assert_eq!(
        audit.last().unwrap(),
        &("auto_erase".to_owned(), "3".to_owned())
    );
    let data = service::verify_data(&app, &finished.slug).await.unwrap();
    let report_v = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_raffle::drand::resolve_chain,
    );
    assert!(report_v.ok(), "the draw still verifies");
    assert_eq!(data.names.len(), 1, "only the held winner is named");
    assert!(
        recording.notified().contains(&RaffleId::new(&finished.id)),
        "screens were nudged"
    );

    // Drawing: erased where it stands.
    let entrants = entrants::list(&app.db, &drawing.id).await.unwrap();
    for e in &entrants {
        assert_eq!(e.erased, e.id != drawing_winner, "{}", e.display_name);
    }
    assert_eq!(
        raffles::get(&app.db, &drawing.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Drawing
    );

    // A second run changes nothing.
    let again = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((again.erased, again.purged), (0, 0));
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&finished.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 3, "each erasure logged once");
    assert!(
        !service::housekeeping::stale(&app).await.unwrap(),
        "a run just succeeded"
    );

    // The ceiling: the held winner goes, the finished raffle is purged, the
    // drawing one is finished by the engine and purged on the run after.
    app.clock.set(finished.erase_at + hold + 1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(
        (report.erased, report.purged),
        (2, 1),
        "both held winners, the finished raffle purged: {report:?}"
    );
    assert!(
        entrants::list(&app.db, &finished.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );
    let r = raffles::get(&app.db, &finished.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Purged);
    assert!(r.purged_at.is_some());
    assert_eq!(
        audit_of(&app, &finished.id).await.last().unwrap(),
        &("auto_purge".to_owned(), String::new())
    );
    assert!(
        recording.ended().contains(&RaffleId::new(&finished.id)),
        "its channel ended"
    );
    assert!(
        entrants::list(&app.db, &drawing.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );
    let r = wait_for(&app, &drawing.id, |r| {
        r.raffle.state == RaffleState::Finished
    })
    .await;
    assert!(
        r.raffle.finished_at.is_some(),
        "the engine finished it, best effort"
    );
    app.clock.advance(1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.purged, 1, "purged on the run after it finished");
    assert_eq!(
        raffles::get(&app.db, &drawing.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    let data = service::verify_data(&app, &finished.slug).await.unwrap();
    assert!(data.names.is_empty());
    assert!(
        rafi_raffle::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_raffle::drand::resolve_chain
        )
        .ok()
    );
}

/// After a restore, the erasure log, the purges and the organizers who left
/// are re-applied, and whatever is due is erased again, removals at their
/// request past their use forgotten again.
#[tokio::test]
async fn erasures_are_reapplied_after_a_restore() {
    let app = app().await;
    let raffle = finished_raffle(&app).await;
    let ids: Vec<String> = entrants::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    service::remove_entrant(&app, ACTOR, &raffle.id, &ids[0], false)
        .await
        .unwrap();
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let left = finished_raffle(&app).await;
    service::remove_owner_data(&app, ACTOR).await.unwrap();

    // "Restore": the personal data is back, the first raffle finished again,
    // the organizer named again on the second.
    app.db
        .query("UPDATE entrants SET erased = 0, display_name = 'Back', email_raw = 'back@x.org', claim_code = 'K9' WHERE raffle_id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    app.db
        .query("UPDATE raffles SET state = 'finished' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    app.db
        .query(
            "UPDATE raffles SET organizer_name = 'Back', contact_email = 'back@x.org' WHERE id = ?",
        )
        .bind(&left.id)
        .run()
        .await
        .unwrap();
    // A removal at their request from before the freeze, back too.
    app.db
        .query(
            "INSERT INTO add_removals (raffle_id, email_norm, removed_at) VALUES (?, 'back@x.org', 1)",
        )
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();

    let done = service::reapply_erasures(&app).await.unwrap();
    assert!(
        entrants::removals(&app.db, &raffle.id)
            .await
            .unwrap()
            .is_empty(),
        "a frozen raffle's removals serve nothing: forgotten again"
    );
    assert_eq!((done.entrants, done.raffles, done.organizers), (3, 1, 1));
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.erased && e.email_raw.is_none() && e.claim_code.is_none())
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    let left = raffles::get(&app.db, &left.id).await.unwrap().unwrap();
    assert!(left.organizer_name.is_empty() && left.contact_email.is_empty());
    let again = service::reapply_erasures(&app).await.unwrap();
    assert_eq!(
        (again.entrants, again.raffles, again.organizers),
        (0, 0, 0),
        "idempotent"
    );
}

/// `whoami` warns when housekeeping hasn't succeeded for two hours, counted
/// from the first raffle: a fresh installation isn't stale.
#[tokio::test]
async fn a_stopped_run_is_reported() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let me = async |router: &Router| -> rafi_api::Me {
        let (status, _, body) = send(router, admin_call("whoami", "")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    };
    assert!(!me(&router).await.retention_stale, "nothing to erase yet");
    create(&app, "dev", "First").await;
    app.clock.advance(service::housekeeping::STALE_AFTER_SECS);
    assert!(
        !me(&router).await.retention_stale,
        "a raffle no older than two hours"
    );
    app.clock.advance(1);
    assert!(
        me(&router).await.retention_stale,
        "two hours with something to erase and no run"
    );
    service::housekeeping::run(&app).await.unwrap();
    assert!(!me(&router).await.retention_stale);
    app.clock
        .advance(service::housekeeping::STALE_AFTER_SECS + 1);
    assert!(me(&router).await.retention_stale);
}

/// A frozen raffle nobody ever drew from: erased on the date where it stands,
/// finished by the engine at the ceiling, purged on the run after.
#[tokio::test]
async fn a_frozen_raffle_is_erased_in_place_and_finished_at_the_ceiling() {
    let (recording, live) = recording();
    let app = app_with_live(Config::for_tests(), live).await;
    let hold = app.policy.retention.winner_hold_secs();
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    for name in ["Anna", "Bence"] {
        add_one(&app, ACTOR, &raffle.id, name, Some(format!("{name}@x.org")))
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Frozen
    );

    app.clock.advance(10 * DAY);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((report.erased, report.purged), (2, 0), "{report:?}");
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Frozen, "erased where it stands");
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );
    assert!(recording.notified().contains(&RaffleId::new(&raffle.id)));

    app.clock.set(raffle.erase_at + hold + 1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(
        (report.erased, report.purged),
        (0, 0),
        "nothing left to erase; the engine is asked to finish"
    );
    wait_for(&app, &raffle.id, |r| {
        r.raffle.state == RaffleState::Finished
    })
    .await;
    app.clock.advance(1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.purged, 1);
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    assert!(recording.ended().contains(&RaffleId::new(&raffle.id)));
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_raffle::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_raffle::drand::resolve_chain
        )
        .ok(),
        "a raffle with no draws still verifies"
    );
}

/// A raffle nobody froze expires on its erasure date, whatever its state:
/// its entrants are deleted outright, it is purged, audited with the count,
/// and its channel ended. One whose date hasn't come is untouched. Expiry
/// logs no erasures: the next run would expire a restored raffle again.
#[tokio::test]
async fn never_frozen_raffles_expire() {
    let (recording, live) = recording();
    let app = app_with_live(Config::for_tests(), live).await;
    let open = setup_open(&app, &[("Cap", 1)]).await;
    add_one(&app, ACTOR, &open.id, "Gabi", Some("gabi@x.org".into()))
        .await
        .unwrap();
    let draft = create(&app, OWNER, "Draft").await;
    // Ten days on, a fresh one.
    app.clock.advance(10 * DAY);
    let fresh = setup_open(&app, &[("Hat", 1)]).await;
    add_one(&app, ACTOR, &fresh.id, "Hanna", None)
        .await
        .unwrap();

    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((report.expired, report.erased), (2, 0), "{report:?}");
    assert!(
        recording.ended().contains(&RaffleId::new(&open.id)),
        "an expired raffle's channel is destroyed"
    );
    for id in [&open.id, &draft] {
        assert!(
            entrants::list(&app.db, id).await.unwrap().is_empty(),
            "deleted outright"
        );
        let r = raffles::get(&app.db, id).await.unwrap().unwrap();
        assert_eq!(r.state, RaffleState::Purged);
        assert!(r.purged_at.is_some() && r.live_ended_at.is_some());
    }
    assert_eq!(
        audit_of(&app, &open.id).await.last().unwrap(),
        &("expire".to_owned(), "1".to_owned())
    );
    assert_eq!(
        audit_of(&app, &draft).await.last().unwrap(),
        &("expire".to_owned(), "0".to_owned())
    );
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&open.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 0, "expiry logs no erasures");
    assert!(
        entrants::list(&app.db, &fresh.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.erased),
        "not due: untouched"
    );
    let again = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(again.expired, 0, "idempotent");

    // The admin of an expired raffle says so, and verify.json has nothing:
    // there was never a manifest.
    let admin = service::admin(&app, OWNER, &open.id).await.unwrap();
    assert_eq!(admin.view.state, RaffleState::Purged);
    assert!(admin.view.manifest_hash.is_none());
    assert!(matches!(
        service::verify_data(&app, &open.slug).await,
        Err(service::Error::NotFound)
    ));
}

/// A frozen raffle the engine doesn't hold (its freeze stopped before the
/// engine opened it, and the organizer walked away) is purged at the
/// ceiling instead of being nudged every hour for good; one frozen without
/// a manifest expires on the date like a never-frozen raffle.
#[tokio::test]
async fn abandoned_freezes_leave_limbo() {
    let app = app().await;
    let hold = app.cfg.retention.winner_hold_days * DAY;

    // A manifest written, never handed to the engine (as `Start::Resend`
    // would find it): everyone is a non-winner, so all are erased on the
    // date, and at the ceiling the engine refuses to finish what it never
    // opened, which purges the raffle here.
    let orphan = setup_open(&app, &[("Book", 1)]).await;
    let orphan = retain(&app, &orphan.id, 0).await;
    add_one(&app, ACTOR, &orphan.id, "Anna", None)
        .await
        .unwrap();
    assert!(
        raffles::freeze(
            &app.db,
            &orphan.id,
            RaffleState::Open,
            true,
            app.clock.now()
        )
        .await
        .unwrap()
    );
    let manifest = rafi_raffle::Manifest::new(
        orphan.id.as_str(),
        rafi_raffle::BeaconSource::Drand(app.chain),
        entrants::active_ids(&app.db, &orphan.id)
            .await
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect(),
        vec![rafi_raffle::ManifestItem {
            id: "book-1".into(),
            name: "Book".into(),
        }],
    );
    raffles::set_manifest(
        &app.db,
        &orphan.id,
        &manifest.canonical_json(),
        &manifest.hash().to_string(),
        app.clock.now(),
    )
    .await
    .unwrap();
    let r = raffles::get(&app.db, &orphan.id).await.unwrap().unwrap();
    assert!(r.state == RaffleState::Frozen && r.manifest.is_some() && !r.opened());

    // Frozen without a manifest: the freeze stopped after closing registration.
    let stuck = setup_open(&app, &[("Book", 1)]).await;
    let stuck = retain(&app, &stuck.id, 0).await;
    add_one(&app, ACTOR, &stuck.id, "Bence", None)
        .await
        .unwrap();
    assert!(
        raffles::freeze(&app.db, &stuck.id, RaffleState::Open, true, app.clock.now())
            .await
            .unwrap()
    );

    app.clock.advance(10 * DAY);
    assert!(
        service::admin(&app, OWNER, &stuck.id)
            .await
            .unwrap()
            .retention
            .expires_unfrozen,
        "the admin says it expires"
    );
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.expired, 1, "the manifest-less freeze expired");
    assert_eq!(
        raffles::get(&app.db, &stuck.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    assert!(entrants::list(&app.db, &stuck.id).await.unwrap().is_empty());
    assert_eq!(
        raffles::get(&app.db, &orphan.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Frozen,
        "the one with a manifest is erased, not moved, before the ceiling"
    );
    assert!(
        entrants::list(&app.db, &orphan.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );

    app.clock.set(orphan.erase_at + hold + 1);
    service::housekeeping::run(&app).await.unwrap();
    let r = raffles::get(&app.db, &orphan.id).await.unwrap().unwrap();
    assert_eq!(
        r.state,
        RaffleState::Purged,
        "purged, the engine not holding it"
    );
    assert!(r.purged_at.is_some() && r.live_ended_at.is_some());
    assert_eq!(
        audit_of(&app, &orphan.id).await.last().unwrap(),
        &("auto_purge".to_owned(), "unfinished".to_owned())
    );
    app.clock.advance(1);
    let again = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((again.expired, again.purged), (0, 0), "nothing left to do");
}
