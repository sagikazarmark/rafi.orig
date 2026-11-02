//! Entrants joining and added: joins up to the cap, adds and their undo,
//! fakes, patches.

use super::*;

pub(in crate::conformance) async fn joins_up_to_the_cap_once_per_email(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    check(join(db, "r1", "a", "a@x", 2).await?, "the first entry")?;
    match entrants::join(db, &joiner("r1", "a2", "a@x"), 2, NOW).await {
        Err(e) if e.is_unique_violation() => {}
        other => return Err(format!("the same email again: {other:?}")),
    }
    check(
        entrants::email_taken(db, "r1", "a@x").await.map_err(fail)?,
        "the email is taken",
    )?;
    check(
        !entrants::email_taken(db, "r1", "z@x").await.map_err(fail)?,
        "another email isn't",
    )?;
    check(join(db, "r1", "b", "b@x", 2).await?, "the second entry")?;
    check(!join(db, "r1", "c", "c@x", 2).await?, "the raffle is full")?;

    // The organizer can add people without an email, but not a taken one.
    for id in ["m1", "m2"] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    let m1 = entrant(db, "r1", "m1").await?;
    check(
        m1.source == rafi_lifecycle::entrant::Source::Manual
            && m1.consent_version.is_none()
            && m1.consent_rules_at.is_none()
            && m1.token_hash.is_none(),
        "someone added saw no form: no consent recorded, no device",
    )?;
    let taken = add(
        db,
        "r1",
        "b-m3",
        Origin::One,
        &[("m3", Some("b@x"), None)],
        CAP,
    )
    .await?;
    check(
        taken && entrants::get(db, "r1", "m3").await.map_err(fail)?.is_none(),
        "an added entry with a taken email is left out",
    )?;
    check(
        entrants::claim_codes(db, "r1").await.map_err(fail)?.len() == 4,
        "claim codes",
    )?;
    let a = entrants::by_token(db, "r1", "a").await.map_err(fail)?;
    check(
        a.is_some_and(|a| {
            a.id == "a"
                && a.source == rafi_lifecycle::entrant::Source::SelfService
                && a.consent_rules_at.is_some()
                && !a.hidden
        }),
        "found by token",
    )?;

    // Each raffle has its own emails; a closed one takes nobody.
    raffle_in(db, "r2", RaffleState::Open).await?;
    check(
        join(db, "r2", "d", "a@x", 10).await?,
        "the email in another raffle",
    )?;
    raffle_in(db, "r3", RaffleState::Closed).await?;
    check(
        !join(db, "r3", "e", "e@x", 10).await?,
        "registration is closed",
    )
}

/// Joins that race (each on its own connection) still stop at the cap: the
/// check and the insert are one statement, and the backend's transactions
/// must keep it exact (Postgres: serializable, retried on conflict).
pub(in crate::conformance) async fn racing_joins_respect_the_cap(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let ids: Vec<(String, String)> = (0..16)
        .map(|n| (format!("{n:032}"), format!("{n}@example.com")))
        .collect();
    let joiners: Vec<_> = ids
        .iter()
        .map(|(id, email)| joiner("r1", id, email))
        .collect();
    let joins = joiners.iter().map(|j| entrants::join(db, j, 5, NOW));
    let mut joined = 0;
    for result in futures_util::future::join_all(joins).await {
        joined += usize::from(result.map_err(fail)?);
    }
    check(joined == 5, &format!("{joined} of 16 racing joins went in"))?;
    check(
        entrants::count_active(db, "r1").await.map_err(fail)? == 5,
        "five entrants",
    )
}

/// An add is one batch: its rows across statements, in the add's order,
/// left out where a join took the email, all of them or none under the cap,
/// once per batch ID, and a registration ID once per raffle.
pub(in crate::conformance) async fn adds_all_or_nothing(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    check(join(db, "r1", "j", "j@x", CAP).await?, "a join")?;
    // More rows than one statement takes; IDs against the list's order, so
    // only the row number can keep it. Row 7 is the joiner's email.
    let ids: Vec<String> = (0..30).map(|i| format!("p{:02}", 29 - i)).collect();
    let emails: Vec<String> = (0..30).map(|i| format!("{i}@x")).collect();
    let refs: Vec<String> = (0..30).map(|i| format!("R{i}")).collect();
    let rows: Vec<Row<'_>> = (0..30)
        .map(|i| {
            let email = if i == 7 { "j@x" } else { emails[i].as_str() };
            // Every third row has no email, every fifth no registration ID.
            (
                ids[i].as_str(),
                (i % 3 != 0).then_some(email),
                (i % 5 != 0).then_some(refs[i].as_str()),
            )
        })
        .collect();
    check(
        add_at(db, "r1", "b1", Origin::List, &rows, CAP, NOW - 5)
            .await
            .map_err(fail)?,
        "a list of thirty",
    )?;
    no_guard_left(db).await?;
    let want: Vec<String> = (0..30)
        .filter(|&i| i != 7)
        .map(|i| ids[i].clone())
        .chain(["j".to_owned()])
        .collect();
    check(
        listed(db, "r1").await? == want,
        &format!(
            "in the list's order, before the joiner who entered after their names' time; the clash left out: {:?}",
            listed(db, "r1").await?
        ),
    )?;
    let batch = entrants::of_batch(db, "r1", "b1").await.map_err(fail)?;
    check(batch.len() == 29, "the batch, read back")?;
    for e in &batch {
        let i = 29 - e.id[1..].parse::<usize>().map_err(fail)?;
        check(
            e.source == Source::Manual
                && e.batch_id.as_deref() == Some("b1")
                && e.batch_row == Some(i as i64)
                && e.list_ref == rows[i].2.map(str::to_owned)
                && e.email_norm == rows[i].1.map(str::to_owned)
                && e.created_at == NOW - 5
                && e.consent_version.is_none()
                && e.consent_rules_at.is_none()
                && e.token_hash.is_none()
                && e.claim_code.is_some(),
            &format!("row {i} as added: {e:?}"),
        )?;
    }

    // A retry with the same batch ID adds nothing, whatever it carries.
    check(
        !add(db, "r1", "b1", Origin::List, &[("q1", None, None)], CAP).await?,
        "the batch ID was used",
    )?;
    no_guard_left(db).await?;

    // The cap holds for the whole add, the add counted in.
    let five: Vec<Row<'_>> = ["c1", "c2", "c3", "c4", "c5"]
        .into_iter()
        .map(|id| (id, None, None))
        .collect();
    check(
        !add(db, "r1", "b2", Origin::List, &five, 34).await?,
        "30 in and 5 more don't fit under 34",
    )?;
    no_guard_left(db).await?;
    check(
        entrants::count_active(db, "r1").await.map_err(fail)? == 30,
        "nothing of a refused add",
    )?;
    check(
        add(db, "r1", "b2", Origin::List, &five, 35).await?,
        "they fit under 35",
    )?;
    check(
        !add(db, "r1", "b3", Origin::One, &[("d1", None, None)], 35).await?,
        "one person past the cap is refused too",
    )?;

    // A registration ID once per raffle: a taken one fails the whole add.
    let clash = add_at(
        db,
        "r1",
        "b4",
        Origin::List,
        &[("e1", None, None), ("e2", None, Some("R1"))],
        CAP,
        NOW,
    )
    .await;
    check(
        matches!(&clash, Err(e) if e.is_unique_violation()),
        &format!("a taken registration ID: {clash:?}"),
    )?;
    no_guard_left(db).await?;
    check(
        entrants::of_batch(db, "r1", "b4")
            .await
            .map_err(fail)?
            .is_empty(),
        "nothing of it",
    )?;
    raffle_in(db, "r2", RaffleState::Open).await?;
    check(
        add(
            db,
            "r2",
            "b5",
            Origin::List,
            &[("f1", None, Some("R1"))],
            CAP,
        )
        .await?,
        "the same registration ID in another raffle",
    )?;
    let taken = entrants::taken(db, "r1").await.map_err(fail)?;
    check(
        taken.emails.contains("j@x")
            && taken.emails.contains("1@x")
            && taken.list_refs.contains("R29")
            && taken.claim_codes.contains("K2P9")
            && taken.claim_codes.contains("C0029")
            && taken.names.len() == taken.entrants
            && taken.names.iter().any(|n| n == "j")
            && taken.names.iter().any(|n| n == "p00"),
        &format!("what an add must not repeat: {taken:?}"),
    )
}

/// Racing adds (each on its own connection) stop at the cap together: the
/// guard row serializes them, and the count is the batch's own.
pub(in crate::conformance) async fn racing_adds_respect_the_cap(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let ids: Vec<String> = (0..8).map(|n| format!("{n:032}")).collect();
    let adds = ids.iter().map(|id| async move {
        add(
            db,
            "r1",
            &format!("b{id}"),
            Origin::One,
            &[(id, None, None)],
            5,
        )
        .await
    });
    let mut added = 0;
    for result in futures_util::future::join_all(adds).await {
        added += usize::from(result?);
    }
    check(added == 5, &format!("{added} of 8 racing adds went in"))?;
    no_guard_left(db).await?;
    check(
        entrants::count_active(db, "r1").await.map_err(fail)? == 5,
        "five entrants",
    )
}

/// Which raffles take an add, by its origin: the same rules as the
/// lifecycle's, in the add's own SQL.
pub(in crate::conformance) async fn adds_where_the_raffle_takes_them(db: &Db) -> Outcome {
    let one = |id: &'static str| [(id, None, None)];
    raffle_in(db, "draft", RaffleState::Draft).await?;
    check(
        !add(db, "draft", "l1", Origin::List, &one("x1"), CAP).await?,
        "no list before publishing",
    )?;
    check(
        add(db, "draft", "o1", Origin::One, &one("x2"), CAP).await?,
        "one person in a draft",
    )?;
    raffle_in(db, "test", RaffleState::Ready).await?;
    test_mode_on(db, "test").await?;
    check(
        !add(db, "test", "l2", Origin::List, &one("x3"), CAP).await?,
        "no list in test mode",
    )?;
    check(
        add(db, "test", "o2", Origin::One, &one("x4"), CAP).await?,
        "one person in test mode",
    )?;
    raffle_in(db, "closed", RaffleState::Closed).await?;
    check(
        add(db, "closed", "l3", Origin::List, &one("x5"), CAP).await?,
        "a list once registration closed",
    )?;
    for state in [
        RaffleState::Frozen,
        RaffleState::Drawing,
        RaffleState::Finished,
    ] {
        force_state(db, "closed", state).await?;
        for (origin, batch) in [(Origin::One, "o3"), (Origin::List, "l4")] {
            check(
                !add(db, "closed", batch, origin, &one("x6"), CAP).await?,
                &format!("nobody is added once {}", state.as_str()),
            )?;
        }
    }
    raffle_in(db, "late", RaffleState::Open).await?;
    erase_at(db, "late", NOW).await?;
    check(
        !add(db, "late", "o4", Origin::One, &one("x7"), CAP).await?,
        "nobody is added once the erasure date has come",
    )?;
    no_guard_left(db).await?;
    for (raffle, want) in [
        ("draft", vec!["x2"]),
        ("test", vec!["x4"]),
        ("closed", vec!["x5"]),
        ("late", vec![]),
    ] {
        let ids = listed(db, raffle).await?;
        check(ids == want, &format!("{raffle}: {ids:?}"))?;
    }
    Ok(())
}

/// Undoing an add removes its entrants, says how many, and only while the
/// list can change.
pub(in crate::conformance) async fn deletes_a_batch(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    check(join(db, "r1", "j", "j@x", CAP).await?, "a join")?;
    let rows = [
        ("a", Some("a@x"), None),
        ("b", None, Some("7")),
        ("c", None, None),
    ];
    add(db, "r1", "b1", Origin::List, &rows, CAP).await?;
    add(db, "r1", "b2", Origin::One, &[("d", None, None)], CAP).await?;
    let delete = |batch: &'static str| entrants::delete_batch(db, "r1", batch);
    check(
        delete("b1").await.map_err(fail)? == 3,
        "the three of the first add",
    )?;
    check(
        listed(db, "r1").await? == ["d", "j"],
        "the joiner and the other add stay (entered at once: by ID)",
    )?;
    check(delete("b1").await.map_err(fail)? == 0, "undone already")?;
    check(delete("nope").await.map_err(fail)? == 0, "no such add")?;
    check(
        add(db, "r1", "b3", Origin::List, &rows, CAP).await?,
        "the same people again, in a new add",
    )?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    check(
        delete("b2").await.map_err(fail)? == 0,
        "nothing is undone once frozen",
    )?;
    check(listed(db, "r1").await?.len() == 5, "the list as frozen")?;

    // Erasing an added entrant erases their registration ID too; the add
    // they came in with is nothing personal and stays.
    force_state(db, "r1", RaffleState::Finished).await?;
    check(
        entrants::erase(db, "r1", Some("b"), NOW)
            .await
            .map_err(fail)?,
        "erasing one",
    )?;
    let b = entrant(db, "r1", "b").await?;
    check(
        b.erased && b.list_ref.is_none() && b.batch_id.as_deref() == Some("b3"),
        &format!("erased: {b:?}"),
    )?;
    check(
        entrants::taken(db, "r1")
            .await
            .map_err(fail)?
            .list_refs
            .is_empty(),
        "no registration ID left",
    )
}

pub(in crate::conformance) async fn adds_fakes_in_chunks(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    test_mode_on(db, "r1").await?;
    let fake = |i: usize| entrants::Fake {
        id: format!("f{i:03}"),
        display_name: format!("Fake {i}"),
        claim_code: format!("C{i:03}"),
    };
    // More rows than fit in one statement's parameters on D1.
    let fakes: Vec<_> = (0..70).map(fake).collect();
    entrants::insert_fakes(db, "r1", 5, &fakes, CAP)
        .await
        .map_err(fail)?;
    let list = entrants::list(db, "r1").await.map_err(fail)?;
    check(
        list.len() == 70
            && list.iter().all(|e| {
                e.source == rafi_lifecycle::entrant::Source::Fake
                    && e.created_at == 5
                    && e.email_norm.is_none()
            }),
        "seventy fakes",
    )?;
    check(
        list.windows(2).all(|w| w[0].id < w[1].id),
        "entered at the same time: listed by ID",
    )?;
    // All or nothing: a duplicate in the last statement fails every one.
    let mut again: Vec<_> = (100..140).map(fake).collect();
    again.push(fake(0));
    check(
        entrants::insert_fakes(db, "r1", 6, &again, CAP)
            .await
            .is_err(),
        "a duplicate ID fails",
    )?;
    check(
        entrants::list(db, "r1").await.map_err(fail)?.len() == 70,
        "nothing was added",
    )?;
    no_guard_left(db).await?;
    // The cap holds for fakes too, in the same batch.
    let more: Vec<_> = (200..205).map(fake).collect();
    check(
        !entrants::insert_fakes(db, "r1", 7, &more, 74)
            .await
            .map_err(fail)?,
        "70 in and 5 more don't fit under 74",
    )?;
    no_guard_left(db).await?;
    check(
        entrants::list(db, "r1").await.map_err(fail)?.len() == 70,
        "nothing of it",
    )?;
    check(
        entrants::insert_fakes(db, "r1", 7, &more, 75)
            .await
            .map_err(fail)?,
        "they fit under 75",
    )
}

pub(in crate::conformance) async fn patches_an_entrant_at_once(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    add_one(db, "r1", "m1").await.map_err(fail)?;
    let patch = entrants::Patch {
        display_name: Some("Anna"),
        hidden: Some(true),
        delivered_at: Some(Some(7)),
    };
    check(
        entrants::patch(db, "r1", "m1", &patch)
            .await
            .map_err(fail)?,
        "patched",
    )?;
    let e = entrants::get(db, "r1", "m1")
        .await
        .map_err(fail)?
        .ok_or("the entrant is gone")?;
    check(
        e.display_name == "Anna" && e.hidden && e.delivered_at == Some(7),
        "every field at once",
    )?;
    // Absent fields keep their values; delivery can be undone.
    let undeliver = entrants::Patch {
        delivered_at: Some(None),
        ..Default::default()
    };
    entrants::patch(db, "r1", "m1", &undeliver)
        .await
        .map_err(fail)?;
    let e = entrants::get(db, "r1", "m1")
        .await
        .map_err(fail)?
        .ok_or("the entrant is gone")?;
    check(
        e.display_name == "Anna" && e.hidden && e.delivered_at.is_none(),
        "only delivery changed",
    )?;
    check(
        !entrants::patch(db, "r1", "nobody", &patch)
            .await
            .map_err(fail)?,
        "no such entrant",
    )?;
    force_state(db, "r1", RaffleState::Finished).await?;
    entrants::erase(db, "r1", Some("m1"), 0)
        .await
        .map_err(fail)?;
    check(
        !entrants::patch(db, "r1", "m1", &patch)
            .await
            .map_err(fail)?,
        "an erased entrant isn't patched",
    )
}

/// An add's receipt outlives its undo, so a retry with the same batch ID is
/// answered with it rather than adding the people again; recorded once,
/// only before the manifest, and forgotten with the manifest's write.
pub(in crate::conformance) async fn an_add_receipt_outlives_its_undo(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    check(add_one(db, "r1", "m1").await?, "the add")?;
    let first = r#"{"batch_id":"b-m1","added":[{"row":0,"id":"m1"}],"skipped":[]}"#;
    check(
        entrants::record_add(db, "r1", "b-m1", first, NOW)
            .await
            .map_err(fail)?,
        "its receipt",
    )?;
    check(
        !entrants::record_add(db, "r1", "b-m1", "{}", NOW + 1)
            .await
            .map_err(fail)?,
        "recorded once",
    )?;
    check(
        entrants::delete_batch(db, "r1", "b-m1")
            .await
            .map_err(fail)?
            == 1,
        "the undo",
    )?;
    check(listed(db, "r1").await?.is_empty(), "nobody left")?;
    check(
        entrants::add_receipt(db, "r1", "b-m1")
            .await
            .map_err(fail)?
            .as_deref()
            == Some(first),
        "the first result, after the undo",
    )?;
    check(
        entrants::add_receipt(db, "r1", "b-other")
            .await
            .map_err(fail)?
            .is_none(),
        "no receipt for another batch",
    )?;

    // The manifest's write forgets them; none is recorded after it.
    force_state(db, "r1", RaffleState::Frozen).await?;
    check(
        raffles::set_manifest(db, "r1", "{}", "hash", NOW)
            .await
            .map_err(fail)?,
        "the manifest",
    )?;
    check(
        entrants::add_receipt(db, "r1", "b-m1")
            .await
            .map_err(fail)?
            .is_none(),
        "forgotten with the manifest",
    )?;
    check(
        !entrants::record_add(db, "r1", "b-late", first, NOW)
            .await
            .map_err(fail)?,
        "none after it",
    )
}
