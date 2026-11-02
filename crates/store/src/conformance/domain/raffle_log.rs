//! The raffle's log: its saved states, its events and its draws.

use super::*;

pub(in crate::conformance) async fn keeps_the_newest_state(db: &Db) -> Outcome {
    let mut state = frozen(db, "r1").await?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "first write",
    )?;
    check(
        !save_raffle(db, &state, NOW).await.map_err(fail)?,
        "a retry is a no-op",
    )?;

    state.decide(&Decision::Finish).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "finished write",
    )?;
    let (lifecycle, finished_at): (String, Option<i64>) = db
        .query("SELECT state, finished_at FROM raffles WHERE id = 'r1'")
        .one()
        .await
        .map_err(fail)?;
    check(lifecycle == "finished", "finished state")?;
    check(finished_at.is_some(), "the finish is timed")?;

    // Newer, but opened from another entrant list.
    let mut other = Raffle::open(&manifest("r1", ["c".repeat(32)])).map_err(fail)?;
    for decision in [Decision::Void { item: "u1".into() }, Decision::Finish] {
        other.decide(&decision).map_err(fail)?;
    }
    check(
        other.version() > state.version(),
        "the other raffle is newer",
    )?;
    check(
        !save_raffle(db, &other, NOW).await.map_err(fail)?,
        "a different entrant list",
    )
}

pub(in crate::conformance) async fn logs_a_whole_evening(db: &Db) -> Outcome {
    let (manifest, events) = evening()?;
    let id = manifest.raffle.as_str();
    frozen_with(db, &manifest).await?;
    for n in 0..=events.len() {
        let state = Raffle::restore(&manifest, &events[..n]).map_err(fail)?;
        check(
            save_raffle(db, &state, NOW).await.map_err(fail)?,
            &format!("saving after {n} events"),
        )?;
        holds(db, &state, &format!("after {n} events")).await?;
    }
    check(
        raffle(db, id).await?.state == RaffleState::Finished,
        "the evening is finished",
    )?;

    // Deleting the raffle takes its log with it.
    drop_raffle(db, id).await?;
    let (events_left, draws_left) = stored_log(db, id).await?;
    check(
        events_left.is_empty() && draws_left.is_empty(),
        "a deleted raffle's events and draws are gone",
    )?;

    // The same evening in one save.
    frozen_with(db, &manifest).await?;
    let state = Raffle::restore(&manifest, &events).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "saving it at once",
    )?;
    holds(db, &state, "saved at once").await?;

    // A draw that stalls on and on, saved at once: more events than one
    // statement takes, many of them alike.
    let opened = frozen(db, "r1").await?;
    let mut events = vec![Event::Announced {
        k: 1,
        item: "u1".into(),
        round: 1,
        round_time: 0,
    }];
    events.extend((0..44).map(|_| Event::Stalled {
        k: 1,
        item: "u1".into(),
        round: 1,
        reason: "drand unavailable".into(),
    }));
    let state = Raffle::restore(opened.manifest(), &events).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "saving a long stall",
    )?;
    holds(db, &state, "a long stall").await
}

pub(in crate::conformance) async fn an_opened_raffle_may_have_no_events(db: &Db) -> Outcome {
    let state = frozen(db, "r1").await?;
    let row = raffle(db, "r1").await?;
    check(
        !row.opened() && raffles::drawn(db, &row).await.map_err(fail)?.is_none(),
        "not opened before the engine writes it",
    )?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "opening it",
    )?;
    check(raffle(db, "r1").await?.opened(), "opened")?;
    holds(db, &state, "opened").await?;

    // Opened, without its manifest: an error, never "not opened".
    db.query("UPDATE raffles SET manifest = NULL WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    check(
        raffles::drawn(db, &raffle(db, "r1").await?).await.is_err(),
        "the raffle can't be read",
    )?;
    check(
        Full::read(db, "r1").await.is_err(),
        "the full view can't be read",
    )?;
    check(
        live::snapshot(db, "r1").await.is_err(),
        "the snapshot can't be read",
    )
}

/// Raffle `r1`'s states: opened, then Anna drawn, then Anna claimed.
fn three_states(opened: &Raffle) -> Result<[Raffle; 3], String> {
    let drawn = Event::Drawn {
        k: 1,
        item: "u1".into(),
        round: 1,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 2,
        index: 0,
        entrant: "a".repeat(32).into(),
    };
    let claimed = Event::Resolved {
        k: 1,
        resolution: Resolution::Claimed,
    };
    let restore = |events: &[Event]| Raffle::restore(opened.manifest(), events).map_err(fail);
    Ok([
        opened.clone(),
        restore(std::slice::from_ref(&drawn))?,
        restore(&[drawn, claimed])?,
    ])
}

pub(in crate::conformance) async fn saves_only_newer_states(db: &Db) -> Outcome {
    let [opened, drawn, claimed] = three_states(&frozen(db, "r1").await?)?;
    for (state, what) in [
        (&opened, "opened"),
        (&drawn, "drawn"),
        (&claimed, "claimed"),
    ] {
        check(save_raffle(db, state, NOW).await.map_err(fail)?, what)?;
    }
    let unchanged = || async {
        let r = raffle(db, "r1").await?;
        Ok::<_, String>((r.version, r.opened_at, stored_log(db, "r1").await?))
    };
    let before = unchanged().await?;
    check(
        !save_raffle(db, &claimed, NOW).await.map_err(fail)?,
        "a repeated save",
    )?;
    for (state, what) in [(&drawn, "drawn"), (&opened, "opened")] {
        check(
            !save_raffle(db, state, NOW).await.map_err(fail)?,
            &format!("a stale save ({what})"),
        )?;
    }
    check(unchanged().await? == before, "nothing written")?;
    holds(db, &claimed, "after stale saves").await?;

    // Stale saves that get past the read, as if the newer one landed between
    // their read and their write. The drawn state re-sends its draw, which
    // would turn the claim back into a draw: the write's guard leaves the
    // events, the draws and the row alone.
    for (state, what) in [(&drawn, "drawn"), (&opened, "opened")] {
        check(
            !crate::write_raffle(db, state, 0, NOW).await.map_err(fail)?,
            &format!("a stale write ({what})"),
        )?;
        check(
            unchanged().await? == before,
            &format!("nothing written past the read ({what})"),
        )?;
    }
    holds(db, &claimed, "after stale writes").await?;

    // Another entrant list, newer, and a raffle that isn't there.
    let mut other = Raffle::open(&manifest("r1", ["c".repeat(32)])).map_err(fail)?;
    for decision in [Decision::Void { item: "u1".into() }, Decision::Finish] {
        other.decide(&decision).map_err(fail)?;
    }
    check(
        !save_raffle(db, &other, NOW).await.map_err(fail)?,
        "another entrant list",
    )?;
    let ghost = Raffle::open(&manifest("r9", ["a".repeat(32)])).map_err(fail)?;
    check(
        !save_raffle(db, &ghost, NOW).await.map_err(fail)?,
        "a raffle that isn't there",
    )?;
    check(unchanged().await? == before, "nothing written for either")
}

pub(in crate::conformance) async fn raffle_events_are_append_only(db: &Db) -> Outcome {
    let [_, _, claimed] = three_states(&frozen(db, "r1").await?)?;
    check(
        save_raffle(db, &claimed, NOW).await.map_err(fail)?,
        "two events",
    )?;
    let refused = |result: crate::Result<u64>, why: &str, what: &str| match result {
        Err(e) if e.to_string().contains(why) => Ok(()),
        other => Err(format!("{what}: {other:?}")),
    };
    refused(
        db.query("UPDATE raffle_events SET body = body WHERE raffle_id = 'r1'")
            .run()
            .await,
        "append-only",
        "updating an event",
    )?;
    refused(
        db.query("DELETE FROM raffle_events WHERE raffle_id = 'r1' AND seq = 2")
            .run()
            .await,
        "append-only",
        "deleting an event",
    )?;
    let insert = |seqs: &[i64]| {
        let mut q = db.query(format!(
            "INSERT INTO raffle_events (raffle_id, seq, type, body, recorded_at) VALUES {}
             ON CONFLICT DO NOTHING",
            vec!["('r1', ?, 'finished', '{\"type\":\"finished\"}', 0)"; seqs.len()].join(", ")
        ));
        for seq in seqs {
            q = q.bind(*seq);
        }
        q.run()
    };
    refused(insert(&[4]).await, "contiguous", "an event after a gap")?;
    refused(
        insert(&[3, 5]).await,
        "contiguous",
        "a statement with a gap",
    )?;
    insert(&[2]).await.map_err(fail)?;
    check(
        stored_log(db, "r1").await?.0.len() == 2,
        "a gap inserts nothing, a stored event is ignored",
    )?;
    check(
        db.query(
            "INSERT INTO raffle_events (raffle_id, seq, type, body, recorded_at)
             VALUES ('r9', 1, 'finished', '{}', 0)",
        )
        .run()
        .await
        .is_err(),
        "an event of a raffle that isn't there",
    )?;
    holds(db, &claimed, "after refused writes").await?;

    drop_raffle(db, "r1").await?;
    let (events, draws) = stored_log(db, "r1").await?;
    check(
        events.is_empty() && draws.is_empty(),
        "deleting the raffle deletes its events and draws",
    )
}
