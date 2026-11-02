//! Removals at someone's request and objections: who stays out, until when, and
//! their erasure.

use super::*;

/// Raffle `raffle_id`'s list add `batch_id` of `rows` (as [`add`]), lifting
/// the removals of `emails` and `refs`: whether the guard admitted it.
async fn add_lifting(
    db: &Db,
    raffle_id: &str,
    batch_id: &str,
    rows: &[Row<'_>],
    emails: &[&str],
    refs: &[&str],
) -> Result<bool, String> {
    let codes: Vec<String> = (0..rows.len()).map(|i| format!("L{i:04}")).collect();
    let added: Vec<entrants::Added<'_>> = rows
        .iter()
        .zip(&codes)
        .enumerate()
        .map(|(i, ((id, email, list_ref), code))| entrants::Added {
            id,
            display_name: id,
            email_raw: *email,
            email_norm: *email,
            claim_code: code,
            row: Some(i as i64),
            list_ref: *list_ref,
        })
        .collect();
    entrants::insert_added(
        db,
        &entrants::AddBatch {
            raffle_id,
            batch_id,
            origin: Origin::List,
            created_at: NOW,
            rows: &added,
            lifted_emails: emails,
            lifted_list_refs: refs,
            max_entrants: CAP,
            now: NOW,
        },
    )
    .await
    .map_err(fail)
}

/// How many removals at their request are left, in every raffle.
async fn removals_left(db: &Db) -> Result<i64, String> {
    let (n,): (i64,) = db
        .query("SELECT COUNT(*) FROM add_removals")
        .one()
        .await
        .map_err(fail)?;
    Ok(n)
}

/// A removal remembered as is, past the domain functions: what a race or
/// a restored backup leaves.
async fn remembered(
    db: &Db,
    raffle_id: &str,
    email: Option<&str>,
    list_ref: Option<&str>,
) -> Outcome {
    db.query(
        "INSERT INTO add_removals (raffle_id, email_norm, list_ref, removed_at) VALUES (?, ?, ?, 5)",
    )
    .bind(raffle_id)
    .bind(email)
    .bind(list_ref)
    .run()
    .await
    .map_err(fail)?;
    Ok(())
}

/// Removing someone at their request before the freeze remembers their
/// email key and registration ID in the same write as the delete; a later
/// add leaves them out unless it lifts the removal, which it deletes under
/// its guard; removals go when the list is frozen for good, the raffle
/// expires, is erased, purged or deleted, its owner leaves, or
/// housekeeping finds them past their use.
pub(in crate::conformance) async fn removals_keep_people_out_until_lifted(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    add(
        db,
        "r1",
        "b1",
        Origin::List,
        &[
            ("a", Some("a@x"), Some("1")),
            ("b", Some("b@x"), None),
            ("c", None, Some("3")),
            ("d", None, None),
        ],
        CAP,
    )
    .await?;
    check(join(db, "r1", "j", "j@x", CAP).await?, "a join")?;
    for (id, at) in [("a", 10), ("b", 11), ("c", 12), ("d", 13), ("j", 14)] {
        check(
            entrants::delete(db, "r1", id, Some(at))
                .await
                .map_err(fail)?,
            &format!("removing {id} at their request"),
        )?;
    }
    let want = vec![
        (Some("a@x".to_owned()), Some("1".to_owned()), 10),
        (Some("b@x".to_owned()), None, 11),
        (None, Some("3".to_owned()), 12),
        (Some("j@x".to_owned()), None, 14),
    ];
    let got = removed(db, "r1").await?;
    check(
        got == want,
        &format!("both keys in one row, nobody without a key, a joiner too: {got:?}"),
    )?;
    let taken = entrants::taken(db, "r1").await.map_err(fail)?;
    check(
        taken.removed(Some("a@x"), None)
            && taken.removed(None, Some("3"))
            && !taken.removed(Some("c@x"), Some("9"))
            && taken.entrants == 0,
        &format!("what an add leaves out: {taken:?}"),
    )?;

    // A later add: the removed stay out, by either key, unless lifted.
    check(
        add(
            db,
            "r1",
            "b2",
            Origin::List,
            &[
                ("a2", Some("a@x"), None),
                ("c2", Some("c@x"), Some("3")),
                ("e", Some("e@x"), None),
            ],
            CAP,
        )
        .await?,
        "the add goes in",
    )?;
    check(
        listed(db, "r1").await? == ["e"],
        "only the one never removed is added",
    )?;
    check(
        add_lifting(
            db,
            "r1",
            "b3",
            &[("a3", Some("a@x"), Some("1")), ("c3", None, Some("3"))],
            &["a@x"],
            &["3"],
        )
        .await?,
        "an add that lifts",
    )?;
    check(
        listed(db, "r1").await? == ["a3", "c3", "e"],
        "the lifted rows are added (entered at once: by their row)",
    )?;
    let got = removed(db, "r1").await?;
    check(
        got == [
            (Some("b@x".to_owned()), None, 11),
            (Some("j@x".to_owned()), None, 14),
        ],
        &format!("their removals are gone: {got:?}"),
    )?;
    no_guard_left(db).await?;
    // A lift whose add the guard refuses lifts nothing: all or nothing.
    check(
        !add_lifting(db, "r1", "b3", &[("b4", Some("b@x"), None)], &["b@x"], &[]).await?,
        "a retried batch ID is refused",
    )?;
    check(
        removed(db, "r1").await?.len() == 2,
        "b's removal stays with the refused add",
    )?;

    // A joiner isn't held to it: their own choice. Removed again, nothing
    // new is remembered, nothing lost.
    check(
        join(db, "r1", "j2", "j@x", CAP).await?,
        "joining with j's email",
    )?;
    check(
        entrants::delete(db, "r1", "j2", Some(20))
            .await
            .map_err(fail)?,
        "removed again",
    )?;
    check(removed(db, "r1").await?.len() == 2, "j@x once")?;
    // One key remembered already (a race): the other gets its own row.
    add(
        db,
        "r1",
        "b5",
        Origin::List,
        &[("k", Some("k@x"), Some("9"))],
        CAP,
    )
    .await?;
    remembered(db, "r1", Some("k@x"), None).await?;
    check(
        entrants::delete(db, "r1", "k", Some(21))
            .await
            .map_err(fail)?,
        "k removed",
    )?;
    let got = removed(db, "r1").await?;
    check(
        got.contains(&(Some("k@x".to_owned()), None, 5))
            && got.contains(&(None, Some("9".to_owned()), 21)),
        &format!("neither key is lost: {got:?}"),
    )?;
    // A plain removal (a duplicate, a troll) remembers nothing; nor does a
    // fake's at their request.
    check(
        entrants::delete(db, "r1", "e", None).await.map_err(fail)?,
        "e removed",
    )?;
    test_mode_on(db, "r1").await?;
    let fake = entrants::Fake {
        id: "f".into(),
        display_name: "Fake".into(),
        claim_code: "F001".into(),
    };
    check(
        entrants::insert_fakes(db, "r1", NOW, &[fake], CAP)
            .await
            .map_err(fail)?,
        "a fake",
    )?;
    check(
        entrants::delete(db, "r1", "f", Some(22))
            .await
            .map_err(fail)?,
        "the fake removed",
    )?;
    let before = removed(db, "r1").await?;
    check(
        before.len() == 4 && !before.iter().any(|r| r.0.as_deref() == Some("e@x")),
        &format!("nothing new: {before:?}"),
    )?;

    // Frozen: nobody is removed, nothing is remembered.
    check(
        raffles::freeze(db, "r1", RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "freezing r1",
    )?;
    check(
        !entrants::delete(db, "r1", "a3", Some(30))
            .await
            .map_err(fail)?
            && removed(db, "r1").await? == before,
        "after the freeze, neither",
    )?;
    // A freeze that found nobody would go back: the removals wait for the
    // manifest, which fixes the list for good and forgets them.
    check(
        raffles::set_manifest(db, "r1", "{}", "h", NOW)
            .await
            .map_err(fail)?,
        "the manifest",
    )?;
    check(
        removed(db, "r1").await?.is_empty(),
        "forgotten with the manifest",
    )?;

    // Expiry forgets them with the entrants.
    raffle_in(db, "old", RaffleState::Open).await?;
    remembered(db, "old", Some("o@x"), None).await?;
    erase_at(db, "old", 100).await?;
    check(
        retention::expire_due(db, 100).await.map_err(fail)? == ["old"],
        "old expires",
    )?;
    check(removed(db, "old").await?.is_empty(), "with its removals")?;

    // A restore brings removals back to a raffle past their use: erasing
    // everyone forgets them, and so does housekeeping's sweep.
    remembered(db, "r1", Some("back@x"), None).await?;
    force_state(db, "r1", RaffleState::Finished).await?;
    check(
        entrants::erase(db, "r1", None, NOW).await.map_err(fail)?,
        "erasing r1",
    )?;
    check(
        removed(db, "r1").await?.is_empty(),
        "erasing everyone forgets them",
    )?;
    remembered(db, "r1", Some("back@x"), None).await?;
    raffle_in(db, "live", RaffleState::Open).await?;
    remembered(db, "live", Some("l@x"), None).await?;
    raffle_in(db, "late", RaffleState::Open).await?;
    remembered(db, "late", Some("t@x"), None).await?;
    erase_at(db, "late", 100).await?;
    check(
        retention::forget_removals(db, 100).await.map_err(fail)? == 2,
        "the sweep forgets r1's (a manifest) and late's (past the date)",
    )?;
    check(
        removed(db, "live").await?.len() == 1
            && retention::forget_removals(db, 100).await.map_err(fail)? == 0,
        "a raffle still taking adds keeps its own; idempotent",
    )?;

    // Deleting a raffle, or its owner leaving, takes them along.
    check(
        raffles::delete(db, "live").await.map_err(fail)?,
        "a raffle never drawn is deleted",
    )?;
    check(removals_left(db).await? == 0, "deleted with the raffle")?;
    raffle_in(db, "mine", RaffleState::Open).await?;
    remembered(db, "mine", Some("m@x"), None).await?;
    remembered(db, "r1", Some("back@x"), None).await?;
    let left = raffles::owner_leaves(db, audit::Actor::solo("o"), NOW)
        .await
        .map_err(fail)?;
    check(!left.refused, &format!("the owner left: {left:?}"))?;
    check(
        removals_left(db).await? == 0,
        "gone with a never-frozen raffle, and from one that is over",
    )
}

/// Someone the organizer added takes themselves off after the freeze: one
/// guarded write hides their name and records it, only between the freeze
/// and the end, only for whom the organizer added, never for a claimed
/// winner; the name stays hidden until they're put back; once the raffle
/// is over, housekeeping erases them, audited per raffle as objected.
pub(in crate::conformance) async fn objections_hide_and_erase_at_the_end(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    for id in ["m1", "m2", "m3", "m4"] {
        check(add_one(db, "r1", id).await?, "adding")?;
    }
    check(join(db, "r1", "j", "j@x", CAP).await?, "a join")?;
    let object = |id: &'static str, at: i64| entrants::object(db, "r1", id, at);
    check(
        !object("m1", 40).await.map_err(fail)?,
        "not before the freeze: removed instead",
    )?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    check(
        !object("j", 50).await.map_err(fail)?,
        "a joiner accepted the rules",
    )?;
    check(
        object("m1", 50).await.map_err(fail)?,
        "m1 takes themselves off",
    )?;
    let m1 = entrant(db, "r1", "m1").await?;
    check(
        m1.hidden && m1.objected_at == Some(50),
        &format!("hidden and recorded: {m1:?}"),
    )?;
    check(!object("m1", 51).await.map_err(fail)?, "once")?;
    check(
        !entrants::patch(
            db,
            "r1",
            "m1",
            &entrants::Patch {
                hidden: Some(false),
                ..Default::default()
            },
        )
        .await
        .map_err(fail)?
            && entrant(db, "r1", "m1").await?.hidden,
        "their name stays hidden",
    )?;
    patch(
        db,
        "r1",
        "m1",
        entrants::Patch {
            display_name: Some("Renamed"),
            ..Default::default()
        },
    )
    .await?;
    claimed(db, "r1", 1, "m2").await?;
    check(
        !object("m2", 52).await.map_err(fail)?,
        "a claimed winner's result stands",
    )?;
    force_state(db, "r1", RaffleState::Drawing).await?;
    check(object("m3", 53).await.map_err(fail)?, "while drawing too")?;
    check(
        entrants::unobject(db, "r1", "m3").await.map_err(fail)?,
        "m3 put back",
    )?;
    let m3 = entrant(db, "r1", "m3").await?;
    check(
        m3.objected_at.is_none() && m3.hidden,
        &format!("no longer objected, still hidden until shown: {m3:?}"),
    )?;
    check(
        !entrants::unobject(db, "r1", "m3").await.map_err(fail)?,
        "once",
    )?;
    check(object("m3", 54).await.map_err(fail)?, "m3 again")?;

    // Nothing is erased before the end.
    check(
        retention::erase_objected(db, 60).await.map_err(fail)? == retention::Erased::default(),
        "nothing while drawing",
    )?;
    force_state(db, "r1", RaffleState::Finished).await?;
    check(
        !object("m4", 70).await.map_err(fail)?
            && !entrants::unobject(db, "r1", "m1").await.map_err(fail)?,
        "neither once it is over",
    )?;
    let erased = retention::erase_objected(db, 80).await.map_err(fail)?;
    check(
        erased
            == retention::Erased {
                entrants: 2,
                raffles: vec!["r1".into()],
                purged: vec![],
            },
        &format!("m1 and m3 at the end: {erased:?}"),
    )?;
    erased_are(db, "r1", &["m1", "m3"]).await?;
    check(
        audit_of(db, "r1").await? == [("auto_erase".to_owned(), "2 objected".to_owned())],
        "audited per raffle, as objected",
    )?;
    let logged: Vec<(String, i64)> = db
        .query("SELECT entrant_id, at FROM erasures WHERE raffle_id = 'r1' ORDER BY entrant_id")
        .all()
        .await
        .map_err(fail)?;
    check(
        logged == [("m1".to_owned(), 80), ("m3".to_owned(), 80)],
        &format!("each erasure logged: {logged:?}"),
    )?;
    check(
        retention::erase_objected(db, 90).await.map_err(fail)? == retention::Erased::default(),
        "idempotent",
    )?;
    check(
        entrant(db, "r1", "m1").await?.objected_at == Some(50),
        "the anonymous entry still says so",
    )
}

/// Took themselves off and claimed, both written (the objection first, the
/// claim landing within the engine's round trip): once the raffle is over,
/// the end-of-raffle erasure of objections leaves them, as a winner a prize
/// may still be owed to; the winner hold erases them.
pub(in crate::conformance) async fn an_objected_winner_waits_for_the_hold(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    for id in ["m1", "m2"] {
        check(add_one(db, "r1", id).await?, "adding")?;
    }
    force_state(db, "r1", RaffleState::Frozen).await?;
    check(
        entrants::object(db, "r1", "m1", 50).await.map_err(fail)?,
        "m1 takes themselves off",
    )?;
    claimed(db, "r1", 1, "m1").await?;
    check(
        entrants::object(db, "r1", "m2", 51).await.map_err(fail)?,
        "m2 takes themselves off",
    )?;
    force_state(db, "r1", RaffleState::Finished).await?;
    erase_at(db, "r1", 100).await?;

    let erased = retention::erase_objected(db, 80).await.map_err(fail)?;
    check(
        erased.entrants == 1,
        &format!("m2 only, at the end: {erased:?}"),
    )?;
    erased_are(db, "r1", &["m2"]).await?;
    check(
        entrant(db, "r1", "m1").await?.objected_at == Some(50),
        "m1 is still objected",
    )?;

    const HOLD: i64 = 1000;
    retention::erase_due(db, 200, HOLD).await.map_err(fail)?;
    erased_are(db, "r1", &["m2"]).await?;
    retention::erase_due(db, 100 + HOLD, HOLD)
        .await
        .map_err(fail)?;
    erased_are(db, "r1", &["m1", "m2"]).await
}
