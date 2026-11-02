//! Retention and erasure: the erasure log, what is due, racing erasures, an
//! organizer leaving.

use super::*;

pub(in crate::conformance) async fn erases_and_logs_erasures(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    // Erasing everyone first: the statement's first run binds NULL.
    check(
        !entrants::erase(db, "r1", None, 0).await.map_err(fail)?,
        "nobody to erase yet",
    )?;
    for id in ["a", "b", "c", "d"] {
        join(db, "r1", id, &format!("{id}@x"), 10).await?;
    }
    // Before the freeze, removing deletes.
    entrants::delete(db, "r1", "d", None).await.map_err(fail)?;
    check(
        entrants::list(db, "r1").await.map_err(fail)?.len() == 3,
        "removed",
    )?;
    patch(
        db,
        "r1",
        "a",
        entrants::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await?;
    patch(
        db,
        "r1",
        "b",
        entrants::Patch {
            display_name: Some("Bea"),
            delivered_at: Some(Some(9)),
            ..Default::default()
        },
    )
    .await?;
    let b = entrant(db, "r1", "b").await?;
    check(
        b.display_name == "Bea"
            && b.delivered_at == Some(9)
            && entrant(db, "r1", "a").await?.hidden,
        "patched",
    )?;
    patch(
        db,
        "r1",
        "b",
        entrants::Patch {
            delivered_at: Some(None),
            ..Default::default()
        },
    )
    .await?;
    check(
        entrant(db, "r1", "b").await?.delivered_at.is_none(),
        "not delivered after all",
    )?;

    // Until the raffle is over, nobody is erased.
    for state in [RaffleState::Open, RaffleState::Frozen, RaffleState::Drawing] {
        force_state(db, "r1", state).await?;
        check(
            !entrants::erase(db, "r1", Some("a"), 0)
                .await
                .map_err(fail)?
                && !entrants::erase(db, "r1", None, 0).await.map_err(fail)?,
            &format!("nobody is erased while {}", state.as_str()),
        )?;
    }
    check(!entrant(db, "r1", "a").await?.erased, "still there")?;
    force_state(db, "r1", RaffleState::Finished).await?;

    check(
        entrants::erase(db, "r1", Some("a"), 0)
            .await
            .map_err(fail)?,
        "erasing one",
    )?;
    check(
        !entrants::erase(db, "r1", Some("a"), 0)
            .await
            .map_err(fail)?,
        "erasing them again",
    )?;
    let a = entrant(db, "r1", "a").await?;
    check(
        a.erased
            && a.display_name == "#a"
            && a.email_raw.is_none()
            && a.email_norm.is_none()
            && a.token_hash.is_none()
            && a.claim_code.is_none(),
        "their personal data is gone",
    )?;
    check(
        !entrant(db, "r1", "b").await?.erased,
        "nobody else was erased",
    )?;
    check(
        entrants::claim_codes(db, "r1").await.map_err(fail)?.len() == 2,
        "the erased claim code is no longer in use",
    )?;
    check(
        entrants::erase(db, "r1", None, 0).await.map_err(fail)?,
        "erasing the rest",
    )?;
    check(
        entrant(db, "r1", "b").await?.display_name == "#b",
        "everyone is erased",
    )?;
    let log: Vec<(String, String)> = db
        .query("SELECT raffle_id, entrant_id FROM erasures ORDER BY entrant_id")
        .all()
        .await
        .map_err(fail)?;
    check(
        log == [
            ("r1".to_owned(), "a".to_owned()),
            ("r1".to_owned(), "b".to_owned()),
            ("r1".to_owned(), "c".to_owned()),
        ],
        &format!("each erasure is logged once: {log:?}"),
    )
}

/// Housekeeping's clocks, on every backend: a never-frozen raffle expires,
/// non-winners and delivered winners go on the erasure date, held winners
/// after the hold, a finished raffle is then purged and a drawing one isn't
/// moved; every write is logged and idempotent.
pub(in crate::conformance) async fn retention_erases_what_is_due(db: &Db) -> Outcome {
    // Finished: n (non-winner), w (winner), d (delivered winner).
    raffle_in(db, "fin", RaffleState::Open).await?;
    for (id, email) in [("n", "n@x"), ("w", "w@x"), ("d", "d@x")] {
        check(join(db, "fin", id, email, 10).await?, "joining fin")?;
    }
    claimed(db, "fin", 1, "w").await?;
    claimed(db, "fin", 2, "d").await?;
    patch(
        db,
        "fin",
        "d",
        entrants::Patch {
            delivered_at: Some(Some(10)),
            ..Default::default()
        },
    )
    .await?;
    force_state(db, "fin", RaffleState::Finished).await?;
    erase_at(db, "fin", 100).await?;
    // Drawing, never finished: x (non-winner), y (winner).
    raffle_in(db, "dr", RaffleState::Open).await?;
    for (id, email) in [("x", "x@x"), ("y", "y@x")] {
        check(join(db, "dr", id, email, 10).await?, "joining dr")?;
    }
    claimed(db, "dr", 1, "y").await?;
    force_state(db, "dr", RaffleState::Drawing).await?;
    erase_at(db, "dr", 100).await?;
    // Open, never frozen: z. And a draft, a published and a closed one, all
    // never frozen, which expire too.
    raffle_in(db, "op", RaffleState::Open).await?;
    check(join(db, "op", "z", "z@x", 10).await?, "joining op")?;
    erase_at(db, "op", 100).await?;
    for (id, state) in [
        ("draft", RaffleState::Draft),
        ("ready", RaffleState::Ready),
        ("closed", RaffleState::Closed),
    ] {
        raffle_in(db, id, RaffleState::Open).await?;
        check(
            join(db, id, &format!("z-{id}"), &format!("{id}@x"), 10).await?,
            "joining",
        )?;
        force_state(db, id, state).await?;
        erase_at(db, id, 100).await?;
    }
    // Frozen without a manifest (a freeze that stopped after closing
    // registration): nothing to keep verifiable, so it expires too.
    raffle_in(db, "stuck", RaffleState::Open).await?;
    check(join(db, "stuck", "s", "s@x", 10).await?, "joining stuck")?;
    check(
        raffles::freeze(db, "stuck", RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "closing stuck's registration",
    )?;
    check(
        raffle(db, "stuck").await?.state == RaffleState::Frozen
            && raffle(db, "stuck").await?.manifest.is_none(),
        "stuck is frozen without a manifest",
    )?;
    erase_at(db, "stuck", 100).await?;
    // Finished, due later: q.
    raffle_in(db, "later", RaffleState::Open).await?;
    check(join(db, "later", "q", "q@x", 10).await?, "joining later")?;
    force_state(db, "later", RaffleState::Finished).await?;
    erase_at(db, "later", 1000).await?;

    // Before the date: nothing.
    check(
        retention::expire_due(db, 99)
            .await
            .map_err(fail)?
            .is_empty(),
        "nothing expires at 99",
    )?;
    check(
        retention::erase_due(db, 99, 900).await.map_err(fail)? == retention::Erased::default(),
        "nothing is due at 99",
    )?;

    // The date: every never-frozen raffle expires; n, d and x are erased; w
    // and y are held.
    let mut expired = retention::expire_due(db, 100).await.map_err(fail)?;
    expired.sort();
    check(
        expired == ["closed", "draft", "op", "ready", "stuck"],
        &format!(
            "every never-frozen raffle expires, and the one frozen without a manifest: {expired:?}"
        ),
    )?;
    for id in ["op", "draft", "ready", "closed", "stuck"] {
        check(
            entrants::list(db, id).await.map_err(fail)?.is_empty()
                && raffle(db, id).await?.state == RaffleState::Purged,
            &format!("{id}'s entrants are deleted and it is purged"),
        )?;
    }
    let op = raffle(db, "op").await?;
    check(
        op.state == RaffleState::Purged
            && op.purged_at == Some(100)
            && op.live_ended_at == Some(100),
        "op is purged",
    )?;
    check(
        audit_of(db, "op").await? == [("expire".to_owned(), "1".to_owned())],
        "op's expiry is audited with the count",
    )?;
    let who: (String, String) = db
        .query("SELECT actor, user_id FROM audit WHERE raffle_id = 'op'")
        .one()
        .await
        .map_err(fail)?;
    check(
        who == ("housekeeping".to_owned(), "housekeeping".to_owned()),
        &format!("housekeeping is the actor in both columns: {who:?}"),
    )?;
    let erased = retention::erase_due(db, 100, 900).await.map_err(fail)?;
    check(
        erased
            == retention::Erased {
                entrants: 3,
                raffles: vec!["dr".into(), "fin".into()],
                purged: vec![],
            },
        &format!("at 100: {erased:?}"),
    )?;
    erased_are(db, "fin", &["n", "d"]).await?;
    erased_are(db, "dr", &["x"]).await?;
    erased_are(db, "later", &[]).await?;
    check(
        raffle(db, "fin").await?.state == RaffleState::Finished
            && raffle(db, "dr").await?.state == RaffleState::Drawing,
        "nothing moved",
    )?;
    check(
        audit_of(db, "fin").await? == [("auto_erase".to_owned(), "2".to_owned())]
            && audit_of(db, "dr").await? == [("auto_erase".to_owned(), "1".to_owned())],
        "one audit row per raffle, with the count",
    )?;
    let logged: Vec<(String, String, i64)> = db
        .query("SELECT raffle_id, entrant_id, at FROM erasures ORDER BY raffle_id, entrant_id")
        .all()
        .await
        .map_err(fail)?;
    check(
        logged
            == [
                ("dr".to_owned(), "x".to_owned(), 100),
                ("fin".to_owned(), "d".to_owned(), 100),
                ("fin".to_owned(), "n".to_owned(), 100),
            ],
        &format!("the erasure log: {logged:?}"),
    )?;
    check(
        retention::erase_due(db, 100, 900).await.map_err(fail)? == retention::Erased::default(),
        "a second run at 100 changes nothing",
    )?;

    // The ceiling: w and y go, fin is purged, dr is left for the engine; q
    // is due on its own date.
    check(
        retention::unfinished_at_ceiling(db, 999, 900, None)
            .await
            .map_err(fail)?
            .is_empty(),
        "dr isn't at the ceiling at 999",
    )?;
    let erased = retention::erase_due(db, 1000, 900).await.map_err(fail)?;
    check(
        erased
            == retention::Erased {
                entrants: 3,
                raffles: vec!["dr".into(), "fin".into(), "later".into()],
                purged: vec!["fin".into()],
            },
        &format!("at 1000: {erased:?}"),
    )?;
    erased_are(db, "fin", &["n", "d", "w"]).await?;
    erased_are(db, "dr", &["x", "y"]).await?;
    erased_are(db, "later", &["q"]).await?;
    let fin = raffle(db, "fin").await?;
    check(
        fin.state == RaffleState::Purged && fin.purged_at == Some(1000),
        "fin is purged at the ceiling",
    )?;
    check(
        audit_of(db, "fin").await?.last() == Some(&("auto_purge".to_owned(), String::new())),
        "the purge is audited",
    )?;
    check(
        raffle(db, "dr").await?.state == RaffleState::Drawing
            && raffle(db, "later").await?.state == RaffleState::Finished,
        "dr and later aren't moved",
    )?;
    check(
        retention::unfinished_at_ceiling(db, 1000, 900, None)
            .await
            .map_err(fail)?
            == ["dr"],
        "dr is for the engine to finish",
    )?;

    // A restore: fin is finished again, its entrants back; the tool re-applies.
    force_state(db, "fin", RaffleState::Finished).await?;
    db.query("UPDATE entrants SET erased = 0, email_raw = 'back' WHERE raffle_id = 'fin'")
        .run()
        .await
        .map_err(fail)?;
    check(
        entrants::reapply_erasures(db).await.map_err(fail)? == 3,
        "three erasures re-applied",
    )?;
    erased_are(db, "fin", &["n", "d", "w"]).await?;
    check(
        retention::reapply_purges(db, 2000).await.map_err(fail)? == (0, 1)
            && raffle(db, "fin").await?.state == RaffleState::Purged,
        "the purge re-applied; nothing left to erase after the log was",
    )?;
    // A restore from a backup without the erasure log: a purged raffle has
    // nobody un-erased, so its entrants are erased on that alone, and logged.
    db.query("UPDATE entrants SET erased = 0, email_raw = 'back' WHERE raffle_id = 'fin'")
        .run()
        .await
        .map_err(fail)?;
    db.query("DELETE FROM erasures WHERE raffle_id = 'fin'")
        .run()
        .await
        .map_err(fail)?;
    check(
        entrants::reapply_erasures(db).await.map_err(fail)? == 0,
        "no log to re-apply",
    )?;
    check(
        retention::reapply_purges(db, 2001).await.map_err(fail)? == (3, 0),
        "the purged raffle's three entrants are erased on the purge alone",
    )?;
    erased_are(db, "fin", &["n", "d", "w"]).await?;
    let relogged: (i64,) = db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = 'fin' AND at = 2001")
        .one()
        .await
        .map_err(fail)?;
    check(relogged.0 == 3, "and logged again")?;
    check(
        entrants::reapply_erasures(db).await.map_err(fail)? == 0
            && retention::reapply_purges(db, 2002).await.map_err(fail)? == (0, 0),
        "re-applying is idempotent",
    )?;

    // When the run last succeeded.
    check(
        housekeeping::last_ok(db).await.map_err(fail)?.is_none(),
        "never ran",
    )?;
    housekeeping::record_ok(db, 1000).await.map_err(fail)?;
    housekeeping::record_ok(db, 1001).await.map_err(fail)?;
    check(
        housekeeping::last_ok(db).await.map_err(fail)? == Some(1001),
        "the latest run",
    )
}

/// Erasures that race (an organizer's purge against housekeeping, each on
/// its own connection) log each entrant once: every erasure is one batch
/// that reads `erased = 0` and sets it, and the backend's transactions must
/// keep that exact (Postgres: serializable, retried on conflict). Relaxing
/// the isolation level would double the log and the audit counts.
pub(in crate::conformance) async fn racing_erasures_log_each_entrant_once(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let ids: Vec<String> = (0..12).map(|n| format!("{n:032}")).collect();
    for id in &ids {
        check(join(db, "r1", id, &format!("{id}@x"), 20).await?, "joining")?;
    }
    force_state(db, "r1", RaffleState::Finished).await?;
    erase_at(db, "r1", 100).await?;

    // Four purges and four housekeeping runs, all at once.
    let purges = (0..4).map(|_| {
        Box::pin(async {
            entrants::erase(db, "r1", None, 100)
                .await
                .map(|_| ())
                .map_err(fail)
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>>
    });
    let runs = (0..4).map(|_| {
        Box::pin(async {
            retention::erase_due(db, 100, 900)
                .await
                .map(|_| ())
                .map_err(fail)
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>>
    });
    for result in futures_util::future::join_all(purges.chain(runs)).await {
        result?;
    }

    check(
        entrants::list(db, "r1")
            .await
            .map_err(fail)?
            .iter()
            .all(|e| e.erased),
        "everyone is erased",
    )?;
    let logged: Vec<(String,)> = db
        .query("SELECT entrant_id FROM erasures WHERE raffle_id = 'r1' ORDER BY entrant_id")
        .all()
        .await
        .map_err(fail)?;
    let logged: Vec<String> = logged.into_iter().map(|(id,)| id).collect();
    check(
        logged == ids,
        &format!(
            "each entrant logged once: {} rows for {} entrants",
            logged.len(),
            ids.len()
        ),
    )?;
    let audited: Vec<(String,)> = db
        .query("SELECT detail FROM audit WHERE raffle_id = 'r1' AND action = 'auto_erase'")
        .all()
        .await
        .map_err(fail)?;
    let counted: i64 = audited
        .iter()
        .map(|(d,)| d.parse::<i64>().unwrap_or(0))
        .sum();
    check(
        counted <= ids.len() as i64,
        &format!("the audit counts don't exceed the entrants: {audited:?}"),
    )
}

/// Marking every winner delivered, and the owner leaving: one batch that
/// deletes what was never frozen, erases and purges what is over, leaves
/// what is being drawn and other owners alone, and forgets the defaults.
pub(in crate::conformance) async fn delivers_all_and_removes_the_organizer(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    for (id, email) in [("n", "n@x"), ("w", "w@x"), ("d", "d@x")] {
        check(join(db, "r1", id, email, 10).await?, "joining")?;
    }
    claimed(db, "r1", 1, "w").await?;
    claimed(db, "r1", 2, "d").await?;
    patch(
        db,
        "r1",
        "d",
        entrants::Patch {
            delivered_at: Some(Some(10)),
            ..Default::default()
        },
    )
    .await?;
    force_state(db, "r1", RaffleState::Finished).await?;
    check(
        entrants::deliver_all(db, "r1", 50).await.map_err(fail)? == 1,
        "only the undelivered winner",
    )?;
    check(
        entrant(db, "r1", "w").await?.delivered_at == Some(50)
            && entrant(db, "r1", "d").await?.delivered_at == Some(10)
            && entrant(db, "r1", "n").await?.delivered_at.is_none(),
        "delivered at: w now, d as it was, n never",
    )?;
    check(
        entrants::deliver_all(db, "r1", 60).await.map_err(fail)? == 0,
        "idempotent",
    )?;

    // The owner leaves: r1 is over, r2 was never frozen, r3 is being drawn
    // (the app refuses then; the store leaves it alone), r4 is someone
    // else's.
    db.query(
        "UPDATE raffles SET organizer_name = 'Org', contact_email = 'a@b.c', privacy_email = 'p@b.c',
         privacy_text = 't', privacy_url = 'https://b.c' WHERE id = 'r1'",
    )
    .run()
    .await
    .map_err(fail)?;
    raffle_in(db, "r2", RaffleState::Open).await?;
    check(join(db, "r2", "z", "z@x", 10).await?, "joining r2")?;
    raffle_in(db, "r3", RaffleState::Open).await?;
    check(join(db, "r3", "y", "y@x", 10).await?, "joining r3")?;
    force_state(db, "r3", RaffleState::Drawing).await?;
    raffles::insert(
        db,
        &raffles::NewRaffle {
            id: "r4",
            owner_id: "p",
            slug: "slug-r4",
            join_code: "code-r4",
            title: "t",
        },
        &settings_with(""),
        NOW,
    )
    .await
    .map_err(fail)?;
    defaults::save(
        db,
        "o",
        &defaults::Defaults {
            organizer_name: "Org".into(),
            ..Default::default()
        },
        NOW,
    )
    .await
    .map_err(fail)?;

    // Nothing while a raffle is being drawn: all or nothing.
    let admin = audit::Actor {
        owner: "o",
        user: "u-admin",
    };
    let left = raffles::owner_leaves(db, admin, 69).await.map_err(fail)?;
    check(
        left.refused && left.deleted.is_empty() && left.purged.is_empty(),
        &format!("refused while r3 is drawing: {left:?}"),
    )?;
    check(
        raffle(db, "r1").await?.organizer_name == "Org"
            && raffles::get(db, "r2").await.map_err(fail)?.is_some()
            && !entrant(db, "r1", "n").await?.erased
            && defaults::get(db, "o").await.map_err(fail)?.is_some(),
        "nothing happened",
    )?;
    force_state(db, "r3", RaffleState::Finished).await?;

    let mut left = raffles::owner_leaves(db, admin, 70).await.map_err(fail)?;
    left.purged.sort();
    check(
        left.deleted == ["r2"] && left.purged == ["r1", "r3"] && !left.refused,
        &format!("what went: {left:?}"),
    )?;
    check(
        raffles::get(db, "r2").await.map_err(fail)?.is_none()
            && entrants::list(db, "r2").await.map_err(fail)?.is_empty(),
        "r2 and its entrants are deleted",
    )?;
    let r = raffle(db, "r1").await?;
    check(
        r.organizer_name.is_empty()
            && r.contact_email.is_empty()
            && r.privacy_email.is_empty()
            && r.privacy_text.is_empty()
            && r.privacy_url.is_empty()
            && r.state == RaffleState::Purged
            && r.purged_at == Some(70)
            && r.organizer_left_at == Some(70)
            && r.live_ended_at == Some(70),
        "the organizer is gone from r1 and it is purged",
    )?;
    check(
        entrants::list(db, "r1")
            .await
            .map_err(fail)?
            .iter()
            .all(|e| e.erased && e.email_raw.is_none()),
        "r1's entrants are erased",
    )?;
    check(
        audit_of(db, "r1").await?.last() == Some(&("purge".to_owned(), "owner left".to_owned()))
            && audit_of(db, "r2").await?.last()
                == Some(&("delete".to_owned(), "owner left".to_owned())),
        "both audited",
    )?;
    let actors: Vec<(String, String)> = db
        .query("SELECT DISTINCT actor, user_id FROM audit WHERE raffle_id IN ('r1', 'r2') AND detail = 'owner left'")
        .all()
        .await
        .map_err(fail)?;
    check(
        actors == [("o".to_owned(), "u-admin".to_owned())],
        &format!("the owner and the admin who pressed it are both recorded: {actors:?}"),
    )?;
    let logged: Vec<(String,)> = db
        .query("SELECT entrant_id FROM erasures WHERE raffle_id = 'r1' ORDER BY entrant_id")
        .all()
        .await
        .map_err(fail)?;
    check(
        logged.len() == 3,
        &format!("three erasures logged: {logged:?}"),
    )?;
    check(
        raffle(db, "r3").await?.state == RaffleState::Purged
            && entrant(db, "r3", "y").await?.erased
            && raffle(db, "r4").await?.state == RaffleState::Draft
            && raffle(db, "r4").await?.organizer_left_at.is_none(),
        "r3 went with r1; another owner's raffle is left alone",
    )?;
    check(
        defaults::get(db, "o").await.map_err(fail)?.is_none(),
        "the defaults are gone",
    )?;

    // A restore names the organizer again; the tool blanks them again.
    db.query("UPDATE raffles SET organizer_name = 'Org', contact_email = 'a@b.c' WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    check(
        raffles::reapply_organizer_left(db).await.map_err(fail)? == 1
            && raffle(db, "r1").await?.organizer_name.is_empty()
            && raffle(db, "r1").await?.contact_email.is_empty(),
        "the organizer is blanked again",
    )?;
    check(
        raffles::reapply_organizer_left(db).await.map_err(fail)? == 0,
        "idempotent",
    )
}
