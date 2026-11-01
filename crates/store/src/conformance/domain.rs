//! The `domain.*` scenarios: the domain functions, on every backend.

use rafi_lifecycle::{
    RaffleState,
    entrant::{Origin, Source},
    prize::Delivery,
};
use rafi_raffle::{BeaconSource, Decision, Event, Manifest, ManifestItem, Raffle, Resolution};

use super::{Outcome, check, fail};
use crate::{
    Db, EntrantRow, RaffleRow, audit, defaults, entrants, housekeeping, live, pow, prizes, raffles,
    retention, save_raffle, view::Full,
};

/// A frozen raffle `id` with two entrants and one prize, and its draw state.
async fn frozen(db: &Db, id: &str) -> Result<Raffle, String> {
    let state = Raffle::open(&manifest(id, ["a".repeat(32), "b".repeat(32)])).map_err(fail)?;
    frozen_with(db, state.manifest()).await?;
    Ok(state)
}

/// A frozen raffle with `manifest`, as the app stores it: the manifest and
/// its hash.
async fn frozen_with(db: &Db, manifest: &Manifest) -> Outcome {
    let id = manifest.raffle.as_str();
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, manifest, manifest_hash, event_date,
                              event_ends_at, retention_days, created_at, updated_at)
         VALUES (?, 'o', ?, ?, 't', 'frozen', ?, ?, '2100-01-01', 4102444800, 7, 0, 0)",
    )
    .bind(id)
    .bind(format!("slug-{id}"))
    .bind(format!("code-{id}"))
    .bind(manifest.canonical_json())
    .bind(manifest.hash().to_string())
    .run()
    .await
    .map_err(fail)?;
    Ok(())
}

/// Applies `p` to an entrant that must exist.
async fn patch(db: &Db, raffle_id: &str, id: &str, p: entrants::Patch<'_>) -> Outcome {
    check(
        entrants::patch(db, raffle_id, id, &p).await.map_err(fail)?,
        &format!("patching {id}"),
    )
}

/// Puts raffle `id` in `state` as is, past the lifecycle's rules: fixtures
/// that need entrants in a raffle whose list no longer changes add them
/// first, then move it on.
async fn force_state(db: &Db, id: &str, state: RaffleState) -> Outcome {
    db.query("UPDATE raffles SET state = ? WHERE id = ?")
        .bind(state.as_str())
        .bind(id)
        .run()
        .await
        .map_err(fail)?;
    Ok(())
}

/// A manifest for raffle `id`: `entrants`, and one item `u1`.
fn manifest<const N: usize>(id: &str, entrants: [String; N]) -> Manifest {
    Manifest::new(
        id,
        BeaconSource::QUICKNET,
        entrants.map(Into::into).into(),
        vec![ManifestItem {
            id: "u1".into(),
            name: "Book".into(),
        }],
    )
}

pub(super) async fn keeps_the_newest_state(db: &Db) -> Outcome {
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

/// A date no scenario reaches: fixtures' erasure dates, so nothing is ever
/// due by accident.
const FAR: i64 = 4_102_444_800;

/// When the writes happen whose time a scenario doesn't test: after every
/// date the scenarios move through, before [`FAR`].
const NOW: i64 = 1_800_000_000;

/// A new raffle, moved straight to `state` (the store doesn't check the
/// lifecycle; the app does), with an erasure date far away.
async fn raffle_in(db: &Db, id: &str, state: RaffleState) -> Outcome {
    raffles::insert(
        db,
        &raffles::NewRaffle {
            id,
            owner_id: "o",
            slug: &format!("slug-{id}"),
            join_code: &format!("code-{id}"),
            title: "t",
        },
        &raffles::Settings {
            test_mode: false,
            ..settings_with("")
        },
        NOW,
    )
    .await
    .map_err(fail)?;
    if state != RaffleState::Draft {
        check(
            raffles::transition(db, id, RaffleState::Draft, state, NOW)
                .await
                .map_err(fail)?,
            "moving a new raffle",
        )?;
    }
    Ok(())
}

/// A self-service entry whose token hash is its ID.
fn joiner<'a>(raffle_id: &'a str, id: &'a str, email: &'a str) -> entrants::Join<'a> {
    entrants::Join {
        id,
        raffle_id,
        display_name: id,
        email_raw: email,
        email_norm: email,
        claim_code: "K2P9",
        token_hash: id,
        consent_version: "v1",
    }
}

async fn join(db: &Db, raffle_id: &str, id: &str, email: &str, cap: i64) -> Result<bool, String> {
    entrants::join(db, &joiner(raffle_id, id, email), cap, NOW)
        .await
        .map_err(fail)
}

/// A cap no fixture reaches.
const CAP: i64 = 1000;

/// A row of an add in a fixture: an ID (its display name too), an email
/// (its key as well) and a registration ID.
type Row<'a> = (&'a str, Option<&'a str>, Option<&'a str>);

/// Raffle `raffle_id`'s add `batch_id` of `rows` from `origin`, under the
/// entrant cap `cap`, entered at `created_at` (now [`NOW`]): whether the
/// guard admitted it. Row `n` is the add's row `n`.
async fn add_at(
    db: &Db,
    raffle_id: &str,
    batch_id: &str,
    origin: Origin,
    rows: &[Row<'_>],
    cap: i64,
    created_at: i64,
) -> crate::Result<bool> {
    let codes: Vec<String> = (0..rows.len()).map(|i| format!("C{i:04}")).collect();
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
            row: (origin == Origin::List).then_some(i as i64),
            list_ref: *list_ref,
        })
        .collect();
    entrants::insert_added(
        db,
        &entrants::AddBatch {
            raffle_id,
            batch_id,
            origin,
            created_at,
            rows: &added,
            max_entrants: cap,
            now: NOW,
        },
    )
    .await
}

/// [`add_at`], entered now.
async fn add(
    db: &Db,
    raffle_id: &str,
    batch_id: &str,
    origin: Origin,
    rows: &[Row<'_>],
    cap: i64,
) -> Result<bool, String> {
    add_at(db, raffle_id, batch_id, origin, rows, cap, NOW)
        .await
        .map_err(fail)
}

/// Adds `id`, without an email, as the one-person form does (its own batch,
/// `b-<id>`).
async fn add_one(db: &Db, raffle_id: &str, id: &str) -> Result<bool, String> {
    add(
        db,
        raffle_id,
        &format!("b-{id}"),
        Origin::One,
        &[(id, None, None)],
        CAP,
    )
    .await
}

/// The IDs of raffle `raffle_id`'s entrants, in the order they are listed.
async fn listed(db: &Db, raffle_id: &str) -> Result<Vec<String>, String> {
    Ok(entrants::list(db, raffle_id)
        .await
        .map_err(fail)?
        .into_iter()
        .map(|e| e.id)
        .collect())
}

/// Whether `batch_guards` is empty, as it must be outside a batch.
async fn no_guard_left(db: &Db) -> Outcome {
    let (n,): (i64,) = db
        .query("SELECT COUNT(*) FROM batch_guards")
        .one()
        .await
        .map_err(fail)?;
    check(n == 0, &format!("{n} guard rows left behind"))
}

async fn raffle(db: &Db, id: &str) -> Result<RaffleRow, String> {
    raffles::get(db, id)
        .await
        .map_err(fail)?
        .ok_or_else(|| format!("raffle {id} is gone"))
}

async fn entrant(db: &Db, raffle_id: &str, id: &str) -> Result<EntrantRow, String> {
    entrants::get(db, raffle_id, id)
        .await
        .map_err(fail)?
        .ok_or_else(|| format!("entrant {id} is gone"))
}

/// Puts raffle `id` in test mode as is: fixtures that add fake entrants.
async fn test_mode_on(db: &Db, id: &str) -> Outcome {
    db.query("UPDATE raffles SET test_mode = 1 WHERE id = ?")
        .bind(id)
        .run()
        .await
        .map_err(fail)?;
    Ok(())
}

pub(super) async fn freezes_in_steps(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Draft).await?;
    check(
        !raffles::freeze(db, "r1", RaffleState::Open, NOW)
            .await
            .map_err(fail)?,
        "a raffle that isn't in the state it froze from",
    )?;
    check(
        raffles::transition(db, "r1", RaffleState::Draft, RaffleState::Open, NOW)
            .await
            .map_err(fail)?,
        "opening",
    )?;
    check(
        !raffles::transition(db, "r1", RaffleState::Draft, RaffleState::Open, NOW)
            .await
            .map_err(fail)?,
        "a transition from a state the raffle has left",
    )?;
    for id in ["a", "b", "c"] {
        check(join(db, "r1", id, &format!("{id}@x"), 10).await?, "joining")?;
    }
    check(
        !entrants::erase(db, "r1", Some("c"), 0)
            .await
            .map_err(fail)?,
        "nobody is erased while the raffle runs",
    )?;
    check(
        entrants::count_active(db, "r1").await.map_err(fail)? == 3,
        "everyone counts",
    )?;

    check(
        raffles::freeze(db, "r1", RaffleState::Open, NOW)
            .await
            .map_err(fail)?,
        "freezing",
    )?;
    check(
        !raffles::freeze(db, "r1", RaffleState::Open, NOW)
            .await
            .map_err(fail)?,
        "freezing twice",
    )?;
    check(
        !join(db, "r1", "d", "d@x", 10).await?,
        "no joins once frozen",
    )?;
    let mut ids = entrants::active_ids(db, "r1").await.map_err(fail)?;
    ids.sort();
    check(
        ids == ["a", "b", "c"],
        &format!("the entrants for the manifest: {ids:?}"),
    )?;

    check(
        raffles::set_manifest(db, "r1", "{}", "h1", NOW)
            .await
            .map_err(fail)?,
        "writing the manifest",
    )?;
    check(
        !raffles::set_manifest(db, "r1", "{}", "h2", NOW)
            .await
            .map_err(fail)?,
        "writing the manifest again",
    )?;
    raffles::unfreeze(db, "r1", RaffleState::Closed, NOW)
        .await
        .map_err(fail)?;
    let r = raffle(db, "r1").await?;
    check(
        r.state == RaffleState::Frozen
            && r.manifest_hash.as_deref() == Some("h1")
            && r.manifest.is_some()
            && r.frozen_at.is_some(),
        "the first commit stays, and a committed raffle stays frozen",
    )?;

    // A freeze that found nobody to draw from goes back.
    raffle_in(db, "r2", RaffleState::Closed).await?;
    check(
        raffles::freeze(db, "r2", RaffleState::Closed, NOW)
            .await
            .map_err(fail)?,
        "freezing a closed raffle",
    )?;
    raffles::unfreeze(db, "r2", RaffleState::Closed, NOW)
        .await
        .map_err(fail)?;
    check(
        raffle(db, "r2").await?.state == RaffleState::Closed,
        "unfrozen",
    )
}

pub(super) async fn joins_up_to_the_cap_once_per_email(db: &Db) -> Outcome {
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
pub(super) async fn racing_joins_respect_the_cap(db: &Db) -> Outcome {
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
pub(super) async fn adds_all_or_nothing(db: &Db) -> Outcome {
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
pub(super) async fn racing_adds_respect_the_cap(db: &Db) -> Outcome {
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
pub(super) async fn adds_where_the_raffle_takes_them(db: &Db) -> Outcome {
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
pub(super) async fn deletes_a_batch(db: &Db) -> Outcome {
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

/// Settings with `rules`, in test mode, the rest fixed; the erasure date far
/// away ([`FAR`]).
fn settings_with(rules: &str) -> raffles::Settings<'_> {
    raffles::Settings {
        title: "t",
        description: "",
        organizer_name: "",
        contact_email: "",
        privacy_email: "",
        privacy_text: "",
        privacy_url: "",
        rules,
        event_date: "2100-01-01",
        event_ends_at: FAR,
        retention_days: 7,
        test_mode: true,
    }
}

/// The rules change only until someone accepted them (fakes don't); the
/// other settings always do.
pub(super) async fn rules_are_fixed_once_accepted(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let save = async |s: &raffles::Settings<'_>| {
        raffles::update_settings(db, "r1", s, NOW)
            .await
            .map(|w| w.written)
    };
    check(
        save(&settings_with("one")).await.map_err(fail)?,
        "no entrants",
    )?;
    let fake = entrants::Fake {
        id: "f1".into(),
        display_name: "Fake".into(),
        claim_code: "F001".into(),
    };
    entrants::insert_fakes(db, "r1", 0, &[fake], CAP)
        .await
        .map_err(fail)?;
    check(
        save(&settings_with("two")).await.map_err(fail)?,
        "only fakes",
    )?;
    check(
        add_one(db, "r1", "m1").await.map_err(fail)?,
        "adding someone",
    )?;
    check(
        !save(&settings_with("three")).await.map_err(fail)?,
        "accepted: the rules are fixed",
    )?;
    // So are the event date, the period, and the privacy text and link.
    for (what, fixed) in [
        (
            "the event date",
            raffles::Settings {
                event_date: "2100-01-02",
                event_ends_at: FAR + 86_400,
                ..settings_with("two")
            },
        ),
        (
            "the period",
            raffles::Settings {
                retention_days: 3,
                ..settings_with("two")
            },
        ),
        (
            "the privacy text",
            raffles::Settings {
                privacy_text: "ours",
                ..settings_with("two")
            },
        ),
        (
            "the privacy link",
            raffles::Settings {
                privacy_url: "https://x",
                ..settings_with("two")
            },
        ),
    ] {
        check(
            !save(&fixed).await.map_err(fail)?,
            &format!("accepted: {what} is fixed"),
        )?;
    }
    let renamed = raffles::Settings {
        title: "renamed",
        privacy_email: "dpo@x",
        ..settings_with("two")
    };
    check(
        save(&renamed).await.map_err(fail)?,
        "the same fixed settings, another title and privacy contact",
    )?;
    let r = raffle(db, "r1").await?;
    check(
        r.rules == "two" && r.title == "renamed",
        &format!("stored {:?}, {:?}", r.rules, r.title),
    )
}

/// Fakes only go into a raffle in test mode; switching it off removes them in
/// the same write, and test mode stays as it is once the list is frozen.
pub(super) async fn fakes_go_with_test_mode(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let fake = |id: &str| entrants::Fake {
        id: id.into(),
        display_name: "Fake".into(),
        claim_code: id.to_uppercase(),
    };
    check(
        !entrants::insert_fakes(db, "r1", 0, &[fake("f0")], CAP)
            .await
            .map_err(fail)?,
        "no fakes outside test mode",
    )?;
    let save = async |test_mode: bool| {
        raffles::update_settings(
            db,
            "r1",
            &raffles::Settings {
                test_mode,
                ..settings_with("")
            },
            NOW,
        )
        .await
        .map_err(fail)
    };
    let on = save(true).await?;
    check(
        on.written && !on.fakes_removed,
        &format!("test mode on: {on:?}"),
    )?;
    check(
        entrants::insert_fakes(db, "r1", 0, &[fake("f1"), fake("f2")], CAP)
            .await
            .map_err(fail)?,
        "fakes in test mode",
    )?;
    add_one(db, "r1", "m1").await.map_err(fail)?;
    check(
        entrants::count_fakes(db, "r1").await.map_err(fail)? == 2,
        "two fakes",
    )?;
    let fakes = entrants::list(db, "r1").await.map_err(fail)?;
    check(
        fakes
            .iter()
            .filter(|e| e.source == rafi_lifecycle::entrant::Source::Fake)
            .all(|e| e.consent_version.is_none() && e.consent_rules_at.is_none()),
        "fakes consented to nothing",
    )?;

    let off = save(false).await?;
    check(
        off.written && off.fakes_removed,
        &format!("test mode off: {off:?}"),
    )?;
    let ids: Vec<String> = entrants::list(db, "r1")
        .await
        .map_err(fail)?
        .into_iter()
        .map(|e| e.id)
        .collect();
    check(
        ids == ["m1"],
        &format!("the fakes went, the person stayed: {ids:?}"),
    )?;

    // Frozen: test mode no longer changes, so nothing is removed.
    save(true).await?;
    entrants::insert_fakes(db, "r1", 0, &[fake("f3")], CAP)
        .await
        .map_err(fail)?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    let off = save(false).await?;
    check(
        !off.written && !off.fakes_removed,
        &format!("test mode off once frozen: {off:?}"),
    )?;
    check(raffle(db, "r1").await?.test_mode, "still in test mode")?;
    check(
        save(true).await?.written,
        "the other settings save once frozen",
    )?;
    check(
        entrants::count_fakes(db, "r1").await.map_err(fail)? == 1,
        "the frozen fake stays",
    )
}

/// An owner's defaults: none until saved, then replaced as a whole; other
/// owners have their own.
pub(super) async fn saves_owner_defaults(db: &Db) -> Outcome {
    check(
        defaults::get(db, "o").await.map_err(fail)?.is_none(),
        "none yet",
    )?;
    let first = defaults::Defaults {
        organizer_name: "Org".into(),
        contact_email: "a@b.c".into(),
        privacy_email: "dpo@b.c".into(),
        privacy_text: "Ours.".into(),
        privacy_url: "https://b.c/p".into(),
        rules: "One.".into(),
        retention_days: Some(3),
    };
    defaults::save(db, "o", &first, NOW).await.map_err(fail)?;
    let second = defaults::Defaults {
        rules: "Two.".into(),
        retention_days: None,
        ..first.clone()
    };
    defaults::save(db, "o", &second, NOW).await.map_err(fail)?;
    check(
        defaults::get(db, "o").await.map_err(fail)? == Some(second),
        "the second save replaces the first, an unset period included",
    )?;
    check(
        defaults::get(db, "p").await.map_err(fail)?.is_none(),
        "another owner's",
    )?;
    defaults::delete(db, "o").await.map_err(fail)?;
    check(
        defaults::get(db, "o").await.map_err(fail)?.is_none(),
        "deleted",
    )
}

pub(super) async fn adds_fakes_in_chunks(db: &Db) -> Outcome {
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

pub(super) async fn erases_and_logs_erasures(db: &Db) -> Outcome {
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
    entrants::delete(db, "r1", "d").await.map_err(fail)?;
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

pub(super) async fn live_channels_end_once(db: &Db) -> Outcome {
    // A raffle that finished at 100, and one that hasn't finished.
    raffle_in(db, "r1", RaffleState::Draft).await?;
    db.query("UPDATE raffles SET state = 'finished', finished_at = 100 WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    raffle_in(db, "r2", RaffleState::Open).await?;

    // Live channels end a while after the raffle finished, once.
    check(
        housekeeping::live_channels_due(db, 99)
            .await
            .map_err(fail)?
            .is_empty(),
        "no channel to end yet",
    )?;
    check(
        housekeeping::live_channels_due(db, 100)
            .await
            .map_err(fail)?
            == ["r1"],
        "the finished raffle's channel, not the unfinished one's",
    )?;
    housekeeping::live_ended(db, "r1", 200)
        .await
        .map_err(fail)?;
    housekeeping::live_ended(db, "r1", 300)
        .await
        .map_err(fail)?;
    check(
        housekeeping::live_channels_due(db, 1000)
            .await
            .map_err(fail)?
            .is_empty()
            && raffle(db, "r1").await?.live_ended_at == Some(200),
        "ended once",
    )
}

pub(super) async fn pow_refuses_replays(db: &Db) -> Outcome {
    let redeem = |challenge, expires_at| pow::redeem(db, challenge, expires_at);
    check(
        redeem("c1", 100).await.map_err(fail)?,
        "the first redemption",
    )?;
    check(!redeem("c1", 100).await.map_err(fail)?, "a replay")?;
    check(redeem("c2", 50).await.map_err(fail)?, "another challenge")?;
    pow::purge_expired(db, 60).await.map_err(fail)?;
    check(
        !redeem("c1", 100).await.map_err(fail)?,
        "an unexpired challenge stays redeemed",
    )?;
    check(
        redeem("c2", 50).await.map_err(fail)?,
        "an expired one is forgotten",
    )
}

pub(super) async fn reads_the_full_view(db: &Db) -> Outcome {
    check(
        Full::read(db, "nope").await.map_err(fail)?.is_none(),
        "no such raffle",
    )?;
    raffle_in(db, "r1", RaffleState::Open).await?;
    let prize = |name, position| prizes::PrizeFields {
        name,
        sponsor: "",
        description: "",
        qty: 1,
        position,
        delivery: Delivery::Later,
    };
    prizes::insert(db, "i2", "r1", &prize("Shirt", 1))
        .await
        .map_err(fail)?;
    prizes::insert(db, "i1", "r1", &prize("Book", 0))
        .await
        .map_err(fail)?;
    prizes::insert(db, "i0", "r1", &prize("Mug", 1))
        .await
        .map_err(fail)?;
    check(
        prizes::update(
            db,
            "i2",
            "r1",
            &prizes::PrizeFields {
                delivery: Delivery::OnTheSpot,
                ..prize("T-shirt", 1)
            },
        )
        .await
        .map_err(fail)?,
        "updating a prize",
    )?;
    check(
        !prizes::update(db, "i9", "r1", &prize("Hat", 2))
            .await
            .map_err(fail)?,
        "updating a missing prize",
    )?;
    join(db, "r1", "z", "z@x", 10).await?;
    test_mode_on(db, "r1").await?;
    entrants::insert_fakes(
        db,
        "r1",
        5,
        &[entrants::Fake {
            id: "y".into(),
            display_name: "Yves".into(),
            claim_code: "Y1".into(),
        }],
        CAP,
    )
    .await
    .map_err(fail)?;

    let full = Full::read(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    let prize_ids: Vec<_> = full.prizes.iter().map(|i| i.id.as_str()).collect();
    check(
        prize_ids == ["i1", "i0", "i2"] && full.prizes[2].name == "T-shirt",
        &format!("prizes by position, then ID: {prize_ids:?}"),
    )?;
    let deliveries: Vec<_> = full.prizes.iter().map(|i| i.delivery).collect();
    check(
        deliveries == [Delivery::Later, Delivery::Later, Delivery::OnTheSpot],
        &format!("how each prize is delivered: {deliveries:?}"),
    )?;
    let entrant_ids: Vec<_> = full.entrants.iter().map(|e| e.id.as_str()).collect();
    check(
        entrant_ids == ["y", "z"],
        &format!("entrants in the order they entered: {entrant_ids:?}"),
    )?;
    check(
        full.raffle.version == raffle(db, "r1").await?.version && full.raffle.version > 0,
        "the raffle's version",
    )?;
    check(
        prizes::list(db, "r1").await.map_err(fail)?.len() == 3,
        "the same prizes",
    )?;
    prizes::delete(db, "i1", "r1").await.map_err(fail)?;
    check(
        Full::read(db, "r1")
            .await
            .map_err(fail)?
            .is_some_and(|f| f.prizes.len() == 2 && f.raffle.version > full.raffle.version),
        "a deleted prize is gone, and the version moved",
    )
}

pub(super) async fn phones_see_only_drawn_names(db: &Db) -> Outcome {
    frozen(db, "r1").await?;
    let (anna, bence) = ("a".repeat(32), "b".repeat(32));
    force_state(db, "r1", RaffleState::Closed).await?;
    for id in [&anna, &bence] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    force_state(db, "r1", RaffleState::Frozen).await?;
    patch(
        db,
        "r1",
        &anna,
        entrants::Patch {
            display_name: Some("Anna"),
            ..Default::default()
        },
    )
    .await?;
    let bence_hidden = entrants::Patch {
        display_name: Some("Bence"),
        hidden: Some(true),
        ..Default::default()
    };
    patch(db, "r1", &bence, bence_hidden).await?;
    // Bence drawn, as the engine logged it.
    let draw = Event::Drawn {
        k: 1,
        item: "u1".into(),
        round: 1,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 2,
        index: 0,
        entrant: bence.clone().into(),
    };
    let state =
        Raffle::restore(&manifest("r1", [anna.clone(), bence.clone()]), &[draw]).map_err(fail)?;
    check(state.version() == 2, "one event")?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "writing the draw",
    )?;

    let snap = live::snapshot(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(snap.state == RaffleState::Drawing, "drawing")?;
    check(
        snap.names.len() == 1 && snap.names.get(&bence) == Some(&None),
        "only the drawn entrant, and their hidden name isn't read",
    )?;
    check(
        snap.drawn.as_ref().is_some_and(|d| d.draws().len() == 1),
        "the raffle as drawn",
    )?;
    check(
        snap.version == raffle(db, "r1").await?.version as u64,
        "the raffle's version",
    )?;

    // Bence isn't here; the prize goes to Anna. Everyone drawn is named,
    // forfeits too.
    let mut state = state;
    state
        .decide(&Decision::Resolve {
            item: "u1".into(),
            resolution: Resolution::NotHere,
        })
        .map_err(fail)?;
    let state = Raffle::restore(
        state.manifest(),
        &[
            state.events(),
            &[Event::Drawn {
                k: 2,
                item: "u1".into(),
                round: 2,
                signature: String::new(),
                randomness: String::new(),
                pass: 1,
                pool_size: 1,
                index: 0,
                entrant: anna.clone().into(),
            }],
        ]
        .concat(),
    )
    .map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "writing the forfeit and the redraw",
    )?;
    let snap = live::snapshot(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(
        snap.names.len() == 2
            && snap.names.get(&bence) == Some(&None)
            && snap.names.get(&anna) == Some(&Some("Anna".to_owned())),
        &format!("both drawn entrants: {:?}", snap.names),
    )?;
    check(snap.drawn.as_ref() == Some(&state), "the raffle as drawn")?;

    // Once erased, a drawn entrant has no name to show.
    force_state(db, "r1", RaffleState::Finished).await?;
    entrants::erase(db, "r1", Some(&anna), 0)
        .await
        .map_err(fail)?;
    let snap = live::snapshot(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(
        snap.names.len() == 1 && snap.names.get(&bence) == Some(&None),
        &format!("the erased entrant's name isn't read: {:?}", snap.names),
    )?;
    check(
        live::snapshot(db, "nope").await.map_err(fail)?.is_none(),
        "no such raffle",
    )
}

pub(super) async fn the_list_and_prizes_are_fixed_once_frozen(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Draft).await?;
    test_mode_on(db, "r1").await?;
    let prize = |name, qty| prizes::PrizeFields {
        name,
        sponsor: "",
        description: "",
        qty,
        position: 0,
        delivery: Delivery::Later,
    };
    let fake = |id: &str| entrants::Fake {
        id: id.into(),
        display_name: "Fake".into(),
        claim_code: id.to_uppercase(),
    };
    check(
        add_one(db, "r1", "m1").await.map_err(fail)?,
        "added by hand while closed",
    )?;
    check(
        entrants::insert_fakes(db, "r1", 0, &[fake("f1"), fake("f2")], CAP)
            .await
            .map_err(fail)?,
        "fakes while closed",
    )?;
    check(
        prizes::insert(db, "i1", "r1", &prize("Book", 1))
            .await
            .map_err(fail)?,
        "a prize while closed",
    )?;
    check(
        entrants::delete(db, "r1", "f2").await.map_err(fail)?,
        "removed while closed",
    )?;

    force_state(db, "r1", RaffleState::Frozen).await?;
    check(
        !add_one(db, "r1", "m2").await.map_err(fail)?,
        "nobody is added by hand once frozen",
    )?;
    check(
        !entrants::insert_fakes(db, "r1", 0, &[fake("f3")], CAP)
            .await
            .map_err(fail)?,
        "no fakes once frozen",
    )?;
    check(
        !entrants::delete(db, "r1", "m1").await.map_err(fail)?,
        "nobody is removed once frozen",
    )?;
    check(
        !prizes::insert(db, "i2", "r1", &prize("Mug", 1))
            .await
            .map_err(fail)?,
        "no prize is added once frozen",
    )?;
    check(
        !prizes::update(db, "i1", "r1", &prize("Big book", 1))
            .await
            .map_err(fail)?,
        "no prize changes once frozen",
    )?;
    check(
        !prizes::delete(db, "i1", "r1").await.map_err(fail)?,
        "no prize is removed once frozen",
    )?;
    let mut ids: Vec<String> = entrants::list(db, "r1")
        .await
        .map_err(fail)?
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    check(
        ids == ["f1", "m1"],
        &format!("the list as it was frozen: {ids:?}"),
    )?;
    check(
        prizes::list(db, "r1").await.map_err(fail)?.len() == 1,
        "the prizes as they were frozen",
    )?;

    // How a prize is delivered isn't in the manifest: it changes until the
    // raffle is over.
    for state in [RaffleState::Frozen, RaffleState::Drawing] {
        force_state(db, "r1", state).await?;
        check(
            prizes::set_delivery(db, "i1", "r1", Delivery::OnTheSpot)
                .await
                .map_err(fail)?,
            &format!("the delivery changes while {}", state.as_str()),
        )?;
    }
    check(
        prizes::set_delivery(db, "i1", "r1", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "setting it again counts as a change",
    )?;
    check(
        !prizes::set_delivery(db, "i9", "r1", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "no such prize",
    )?;
    check(
        !prizes::set_delivery(db, "i1", "r2", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "not that raffle's prize",
    )?;
    for state in [RaffleState::Finished, RaffleState::Purged] {
        force_state(db, "r1", state).await?;
        check(
            !prizes::set_delivery(db, "i1", "r1", Delivery::Later)
                .await
                .map_err(fail)?,
            &format!("not once {}", state.as_str()),
        )?;
    }
    check(
        prizes::list(db, "r1").await.map_err(fail)?[0].delivery == Delivery::OnTheSpot,
        "the delivery as last set",
    )
}

/// Claiming an item of a prize handed over on the spot delivers it, in the
/// save that stores the claim: a book handed over, a voucher sent later.
/// Only the claim does (not the draw, not a forfeit), only once, never for an
/// erased winner, and a stale save delivers nothing.
pub(super) async fn delivers_on_the_spot_with_the_claim(db: &Db) -> Outcome {
    let (anna, bence, csilla) = ("a".repeat(32), "b".repeat(32), "c".repeat(32));
    let manifest = Manifest::new(
        "r1",
        BeaconSource::QUICKNET,
        vec![
            anna.clone().into(),
            bence.clone().into(),
            csilla.clone().into(),
        ],
        vec![
            ManifestItem {
                id: "book-1".into(),
                name: "Book".into(),
            },
            ManifestItem {
                id: "voucher-1".into(),
                name: "Voucher".into(),
            },
        ],
    );
    frozen_with(db, &manifest).await?;
    force_state(db, "r1", RaffleState::Closed).await?;
    for id in [&anna, &bence, &csilla] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    for (id, name, delivery) in [
        ("book", "Book", Delivery::OnTheSpot),
        ("voucher", "Voucher", Delivery::Later),
    ] {
        let fields = prizes::PrizeFields {
            name,
            sponsor: "",
            description: "",
            qty: 1,
            position: 0,
            delivery,
        };
        prizes::insert(db, id, "r1", &fields).await.map_err(fail)?;
    }
    force_state(db, "r1", RaffleState::Frozen).await?;
    let drawn = |k, item: &str, entrant: &str| Event::Drawn {
        k,
        item: item.into(),
        round: k,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 3,
        index: 0,
        entrant: entrant.to_owned().into(),
    };
    let resolved = |k, resolution| Event::Resolved { k, resolution };
    let delivered_at = |id: String| async move {
        entrants::get(db, "r1", &id)
            .await
            .map_err(fail)?
            .ok_or_else(|| format!("entrant {id} is gone"))
            .map(|e| e.delivered_at)
    };
    let save = |events: Vec<Event>, now: i64| {
        let manifest = manifest.clone();
        async move {
            let state = Raffle::restore(&manifest, &events).map_err(fail)?;
            save_raffle(db, &state, now).await.map_err(fail)
        }
    };

    // Bence is drawn for the book and isn't here; Anna is drawn instead.
    let mut events = vec![drawn(1, "book-1", &bence)];
    check(save(events.clone(), NOW).await?, "the first draw")?;
    check(
        delivered_at(bence.clone()).await?.is_none(),
        "a draw delivers nothing",
    )?;
    events.extend([resolved(1, Resolution::NotHere), drawn(2, "book-1", &anna)]);
    check(
        save(events.clone(), NOW).await?,
        "the forfeit and the redraw",
    )?;
    check(
        delivered_at(bence.clone()).await?.is_none(),
        "a forfeit delivers nothing",
    )?;

    // Anna claims the book; Csilla, drawn for the voucher, claims it.
    let stale = events.clone();
    events.extend([
        resolved(2, Resolution::Claimed),
        drawn(3, "voucher-1", &csilla),
        resolved(3, Resolution::Claimed),
    ]);
    check(save(events.clone(), NOW + 5).await?, "the claims")?;
    check(
        delivered_at(anna.clone()).await? == Some(NOW + 5),
        "the book left with Anna when she claimed it",
    )?;
    check(
        delivered_at(csilla.clone()).await?.is_none(),
        "the voucher is still to be sent",
    )?;
    check(!save(stale, NOW + 9).await?, "a stale save is refused")?;

    // Undelivered by hand, it stays so: the claim isn't stored again.
    patch(
        db,
        "r1",
        &anna,
        entrants::Patch {
            delivered_at: Some(None),
            ..Default::default()
        },
    )
    .await?;
    events.push(Event::Finished);
    check(save(events.clone(), NOW + 9).await?, "the finish")?;
    check(
        delivered_at(anna.clone()).await?.is_none(),
        "a later save doesn't deliver again",
    )?;

    // The same evening in one save, the winner of the book erased first.
    raffles::delete(db, "r1").await.map_err(fail)?;
    frozen_with(db, &manifest).await?;
    force_state(db, "r1", RaffleState::Closed).await?;
    for id in [&anna, &bence, &csilla] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    let fields = prizes::PrizeFields {
        name: "Book",
        sponsor: "",
        description: "",
        qty: 1,
        position: 0,
        delivery: Delivery::OnTheSpot,
    };
    prizes::insert(db, "book", "r1", &fields)
        .await
        .map_err(fail)?;
    force_state(db, "r1", RaffleState::Finished).await?;
    entrants::erase(db, "r1", Some(&anna), NOW)
        .await
        .map_err(fail)?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    check(save(events, NOW).await?, "the evening at once")?;
    check(
        delivered_at(anna).await?.is_none(),
        "an erased winner isn't delivered to",
    )?;
    delivers_once_every_claimed_prize_is(db).await
}

/// More items than people: Dora alone claims a book handed over on the
/// spot, then in pass 2 a voucher sent later, then a mug handed over. She is
/// delivered only while every prize she claimed is, saved claim by claim or
/// at once.
async fn delivers_once_every_claimed_prize_is(db: &Db) -> Outcome {
    let dora = "d".repeat(32);
    let manifest = Manifest::new(
        "r2",
        BeaconSource::QUICKNET,
        vec![dora.clone().into()],
        ["book", "voucher", "mug"]
            .map(|p| ManifestItem {
                id: format!("dora-{p}-1").into(),
                name: p.into(),
            })
            .into(),
    );
    let setup = || async {
        frozen_with(db, &manifest).await?;
        force_state(db, "r2", RaffleState::Closed).await?;
        add_one(db, "r2", &dora).await.map_err(fail)?;
        for (id, delivery) in [
            ("book", Delivery::OnTheSpot),
            ("voucher", Delivery::Later),
            ("mug", Delivery::OnTheSpot),
        ] {
            let fields = prizes::PrizeFields {
                name: id,
                sponsor: "",
                description: "",
                qty: 1,
                position: 0,
                delivery,
            };
            prizes::insert(db, &format!("dora-{id}"), "r2", &fields)
                .await
                .map_err(fail)?;
        }
        force_state(db, "r2", RaffleState::Frozen).await
    };
    let claim = |k: u64, item: &str| {
        [
            Event::Drawn {
                k,
                item: format!("dora-{item}-1").into(),
                round: k,
                signature: String::new(),
                randomness: String::new(),
                pass: if k == 1 { 1 } else { 2 },
                pool_size: 1,
                index: 0,
                entrant: dora.clone().into(),
            },
            Event::Resolved {
                k,
                resolution: Resolution::Claimed,
            },
        ]
    };
    let delivered =
        || async { Ok::<_, String>(entrant(db, "r2", &dora).await?.delivered_at.is_some()) };

    setup().await?;
    let mut events = Vec::new();
    for (k, item, want, what) in [
        (1, "book", true, "the book was handed over"),
        (2, "voucher", false, "the voucher is still to be sent"),
        (
            3,
            "mug",
            false,
            "the mug was handed over, the voucher still isn't",
        ),
    ] {
        events.extend(claim(k, item));
        let state = Raffle::restore(&manifest, &events).map_err(fail)?;
        check(
            save_raffle(db, &state, NOW).await.map_err(fail)?,
            &format!("saving the {item}"),
        )?;
        check(delivered().await? == want, what)?;
    }

    raffles::delete(db, "r2").await.map_err(fail)?;
    setup().await?;
    let state = Raffle::restore(&manifest, &events).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "saving it at once",
    )?;
    check(!delivered().await?, "at once, the voucher still isn't")
}

pub(super) async fn patches_an_entrant_at_once(db: &Db) -> Outcome {
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

// ---------------------------------------------------------------------------
// The raffle's log: its events and its draws
// ---------------------------------------------------------------------------

/// The test vectors' evening (`testdata/draw-vectors.json`): every kind of
/// event, an abandon and forfeits included. Built in, so D1 runs it too.
fn evening() -> Result<(Manifest, Vec<Event>), String> {
    #[derive(serde::Deserialize)]
    struct Vectors {
        scenario: Scenario,
    }
    #[derive(serde::Deserialize)]
    struct Scenario {
        manifest: String,
        events: Vec<Event>,
    }
    let vectors: Vectors =
        serde_json::from_str(include_str!("../../../../testdata/draw-vectors.json"))
            .map_err(fail)?;
    let manifest = serde_json::from_str(&vectors.scenario.manifest).map_err(fail)?;
    Ok((manifest, vectors.scenario.events))
}

/// An event row: `(seq, type, body)`.
type EventRow = (i64, String, String);
/// A draw row: `(k, item_id, entrant_id, outcome)`.
type DrawRow = (i64, String, String, String);

/// What the store holds of raffle `id`'s log: its events and its draws.
async fn stored_log(db: &Db, id: &str) -> Result<(Vec<EventRow>, Vec<DrawRow>), String> {
    let events = db
        .query("SELECT seq, type, body FROM raffle_events WHERE raffle_id = ? ORDER BY seq")
        .bind(id)
        .all()
        .await
        .map_err(fail)?;
    let draws = db
        .query("SELECT k, item_id, entrant_id, outcome FROM draws WHERE raffle_id = ? ORDER BY k")
        .bind(id)
        .all()
        .await
        .map_err(fail)?;
    Ok((events, draws))
}

/// Checks that the store holds exactly `state`: every reader reads it back,
/// and its events and draws are rows, as the raffle says.
async fn holds(db: &Db, state: &Raffle, when: &str) -> Outcome {
    let id = state.id().as_str();
    let row = raffle(db, id).await?;
    check(
        raffles::drawn(db, &row).await.map_err(fail)?.as_ref() == Some(state),
        &format!("{when}: the raffle read back"),
    )?;
    let full = Full::read(db, id)
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(
        full.drawn.as_ref() == Some(state),
        &format!("{when}: the full view's raffle"),
    )?;
    let snap = live::snapshot(db, id)
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(
        snap.drawn.as_ref() == Some(state),
        &format!("{when}: the snapshot's raffle"),
    )?;

    let (events, draws) = stored_log(db, id).await?;
    let expected: Vec<EventRow> = state
        .events()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let body = serde_json::to_string(e).map_err(fail)?;
            Ok((i as i64 + 1, e.kind().to_owned(), body))
        })
        .collect::<Result<_, String>>()?;
    check(
        events == expected,
        &format!("{when}: the events, in order: {events:?}"),
    )?;
    let expected: Vec<DrawRow> = state
        .draws()
        .iter()
        .map(|d| {
            (
                d.k as i64,
                d.item.as_str().to_owned(),
                d.entrant.as_str().to_owned(),
                d.outcome.as_str().to_owned(),
            )
        })
        .collect();
    check(
        draws == expected,
        &format!("{when}: the draws: {draws:?}, not {expected:?}"),
    )
}

pub(super) async fn logs_a_whole_evening(db: &Db) -> Outcome {
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
    raffles::delete(db, id).await.map_err(fail)?;
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

pub(super) async fn an_opened_raffle_may_have_no_events(db: &Db) -> Outcome {
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

pub(super) async fn saves_only_newer_states(db: &Db) -> Outcome {
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

pub(super) async fn raffle_events_are_append_only(db: &Db) -> Outcome {
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

    raffles::delete(db, "r1").await.map_err(fail)?;
    let (events, draws) = stored_log(db, "r1").await?;
    check(
        events.is_empty() && draws.is_empty(),
        "deleting the raffle deletes its events and draws",
    )
}

/// Dates raffle `id`'s erasure at `erase_at`, past the lifecycle's rules:
/// the end of the event day is moved so that, with the period, the database
/// computes that date.
async fn erase_at(db: &Db, id: &str, erase_at: i64) -> Outcome {
    db.query("UPDATE raffles SET event_ends_at = ? - retention_days * 86400 WHERE id = ?")
        .bind(erase_at)
        .bind(id)
        .run()
        .await
        .map_err(fail)?;
    check(
        raffle(db, id).await?.erase_at == erase_at,
        "the database computes the erasure date",
    )
}

/// A claimed draw of item `k` for `entrant` in raffle `id`.
async fn claimed(db: &Db, id: &str, k: i64, entrant: &str) -> Outcome {
    db.query(
        "INSERT INTO draws (raffle_id, k, item_id, entrant_id, outcome) VALUES (?, ?, ?, ?, 'claimed')",
    )
    .bind(id)
    .bind(k)
    .bind(format!("u{k}"))
    .bind(entrant)
    .run()
    .await
    .map_err(fail)?;
    Ok(())
}

/// The audit rows of a raffle, `(action, detail)`, oldest first.
async fn audit_of(db: &Db, id: &str) -> Result<Vec<(String, String)>, String> {
    db.query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(id)
        .all()
        .await
        .map_err(fail)
}

/// Whether every entrant of `id` whose ID is in `erased` is erased, and
/// nobody else.
async fn erased_are(db: &Db, id: &str, erased: &[&str]) -> Outcome {
    for e in entrants::list(db, id).await.map_err(fail)? {
        let want = erased.contains(&e.id.as_str());
        check(
            e.erased == want && (e.email_raw.is_none() == want || e.email_raw.is_none()),
            &format!("{id}/{}: erased {} (wanted {want})", e.id, e.erased),
        )?;
    }
    Ok(())
}

/// Housekeeping's clocks, on every backend: a never-frozen raffle expires,
/// non-winners and delivered winners go on the erasure date, held winners
/// after the hold, a finished raffle is then purged and a drawing one isn't
/// moved; every write is logged and idempotent.
pub(super) async fn retention_erases_what_is_due(db: &Db) -> Outcome {
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
        raffles::freeze(db, "stuck", RaffleState::Open, NOW)
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
        retention::unfinished_at_ceiling(db, 999, 900)
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
        retention::unfinished_at_ceiling(db, 1000, 900)
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
pub(super) async fn racing_erasures_log_each_entrant_once(db: &Db) -> Outcome {
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
pub(super) async fn delivers_all_and_removes_the_organizer(db: &Db) -> Outcome {
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
