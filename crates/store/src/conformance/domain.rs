//! The `domain.*` scenarios: the domain functions, on every backend.

use rafi_lifecycle::RaffleState;
use rafi_raffle::{BeaconSource, Decision, Event, Manifest, ManifestItem, Raffle, Resolution};

use super::{Outcome, check, fail};
use crate::{
    Db, EntrantRow, RaffleRow, altcha, defaults, entrants, housekeeping, legal, live, prizes,
    raffles, retention, save_raffle, view::Full,
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
                              event_ends_at, retention_days, erase_at, created_at, updated_at)
         VALUES (?, 'o', ?, ?, 't', 'frozen', ?, ?, '2100-01-01', 4102444800, 7, 4102444800, 0, 0)",
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
    check(save_raffle(db, &state).await.map_err(fail)?, "first write")?;
    check(
        !save_raffle(db, &state).await.map_err(fail)?,
        "a retry is a no-op",
    )?;

    state.decide(&Decision::Finish).map_err(fail)?;
    check(
        save_raffle(db, &state).await.map_err(fail)?,
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
        !save_raffle(db, &other).await.map_err(fail)?,
        "a different entrant list",
    )
}

/// A date no scenario reaches: fixtures' erasure dates, so nothing is ever
/// due by accident.
const FAR: i64 = 4_102_444_800;

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
    )
    .await
    .map_err(fail)?;
    if state != RaffleState::Draft {
        check(
            raffles::transition(db, id, RaffleState::Draft, state)
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
    entrants::join(db, &joiner(raffle_id, id, email), cap)
        .await
        .map_err(fail)
}

fn manual<'a>(raffle_id: &'a str, id: &'a str, email: Option<&'a str>) -> entrants::Manual<'a> {
    entrants::Manual {
        id,
        raffle_id,
        display_name: id,
        email_raw: email,
        email_norm: email,
        claim_code: "Q4M7",
        consent_version: "v1",
    }
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
        !raffles::freeze(db, "r1", RaffleState::Open)
            .await
            .map_err(fail)?,
        "a raffle that isn't in the state it froze from",
    )?;
    check(
        raffles::transition(db, "r1", RaffleState::Draft, RaffleState::Open)
            .await
            .map_err(fail)?,
        "opening",
    )?;
    check(
        !raffles::transition(db, "r1", RaffleState::Draft, RaffleState::Open)
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
        raffles::freeze(db, "r1", RaffleState::Open)
            .await
            .map_err(fail)?,
        "freezing",
    )?;
    check(
        !raffles::freeze(db, "r1", RaffleState::Open)
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
        raffles::set_manifest(db, "r1", "{}", "h1")
            .await
            .map_err(fail)?,
        "writing the manifest",
    )?;
    check(
        !raffles::set_manifest(db, "r1", "{}", "h2")
            .await
            .map_err(fail)?,
        "writing the manifest again",
    )?;
    raffles::unfreeze(db, "r1", RaffleState::Closed)
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
        raffles::freeze(db, "r2", RaffleState::Closed)
            .await
            .map_err(fail)?,
        "freezing a closed raffle",
    )?;
    raffles::unfreeze(db, "r2", RaffleState::Closed)
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
    match entrants::join(db, &joiner("r1", "a2", "a@x"), 2).await {
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
        entrants::insert_manual(db, &manual("r1", id, None))
            .await
            .map_err(fail)?;
    }
    let taken = entrants::insert_manual(db, &manual("r1", "m3", Some("b@x"))).await;
    check(
        taken.is_err_and(|e| e.is_unique_violation()),
        "a manual entry with a taken email",
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
    let joins = joiners.iter().map(|j| entrants::join(db, j, 5));
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
        erase_at: FAR,
        test_mode: true,
    }
}

/// The rules change only until someone accepted them (fakes don't); the
/// other settings always do.
pub(super) async fn rules_are_fixed_once_accepted(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let save = async |s: &raffles::Settings<'_>| {
        raffles::update_settings(db, "r1", s)
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
    entrants::insert_fakes(db, "r1", 0, &[fake])
        .await
        .map_err(fail)?;
    check(
        save(&settings_with("two")).await.map_err(fail)?,
        "only fakes",
    )?;
    check(
        entrants::insert_manual(db, &manual("r1", "m1", None))
            .await
            .map_err(fail)?,
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
                erase_at: FAR + 86_400,
                ..settings_with("two")
            },
        ),
        (
            "the period",
            raffles::Settings {
                retention_days: 3,
                erase_at: FAR - 4 * 86_400,
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
        !entrants::insert_fakes(db, "r1", 0, &[fake("f0")])
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
        entrants::insert_fakes(db, "r1", 0, &[fake("f1"), fake("f2")])
            .await
            .map_err(fail)?,
        "fakes in test mode",
    )?;
    entrants::insert_manual(db, &manual("r1", "m1", None))
        .await
        .map_err(fail)?;
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
    entrants::insert_fakes(db, "r1", 0, &[fake("f3")])
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
    defaults::save(db, "o", &first).await.map_err(fail)?;
    let second = defaults::Defaults {
        rules: "Two.".into(),
        retention_days: None,
        ..first.clone()
    };
    defaults::save(db, "o", &second).await.map_err(fail)?;
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
    entrants::insert_fakes(db, "r1", 5, &fakes)
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
        entrants::insert_fakes(db, "r1", 6, &again).await.is_err(),
        "a duplicate ID fails",
    )?;
    check(
        entrants::list(db, "r1").await.map_err(fail)?.len() == 70,
        "nothing was added",
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

pub(super) async fn altcha_refuses_replays(db: &Db) -> Outcome {
    let redeem = |challenge, expires_at| altcha::redeem(db, challenge, expires_at);
    check(
        redeem("c1", 100).await.map_err(fail)?,
        "the first redemption",
    )?;
    check(!redeem("c1", 100).await.map_err(fail)?, "a replay")?;
    check(redeem("c2", 50).await.map_err(fail)?, "another challenge")?;
    altcha::purge_expired(db, 60).await.map_err(fail)?;
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
        prizes::update(db, "i2", "r1", &prize("T-shirt", 1))
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
        entrants::insert_manual(db, &manual("r1", id, None))
            .await
            .map_err(fail)?;
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
        save_raffle(db, &state).await.map_err(fail)?,
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
        save_raffle(db, &state).await.map_err(fail)?,
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
    };
    let fake = |id: &str| entrants::Fake {
        id: id.into(),
        display_name: "Fake".into(),
        claim_code: id.to_uppercase(),
    };
    check(
        entrants::insert_manual(db, &manual("r1", "m1", None))
            .await
            .map_err(fail)?,
        "added by hand while closed",
    )?;
    check(
        entrants::insert_fakes(db, "r1", 0, &[fake("f1"), fake("f2")])
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
        !entrants::insert_manual(db, &manual("r1", "m2", None))
            .await
            .map_err(fail)?,
        "nobody is added by hand once frozen",
    )?;
    check(
        !entrants::insert_fakes(db, "r1", 0, &[fake("f3")])
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
    )
}

pub(super) async fn patches_an_entrant_at_once(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    entrants::insert_manual(db, &manual("r1", "m1", None))
        .await
        .map_err(fail)?;
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
            save_raffle(db, &state).await.map_err(fail)?,
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
        save_raffle(db, &state).await.map_err(fail)?,
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
        save_raffle(db, &state).await.map_err(fail)?,
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
    check(save_raffle(db, &state).await.map_err(fail)?, "opening it")?;
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
        check(save_raffle(db, state).await.map_err(fail)?, what)?;
    }
    let unchanged = || async {
        let r = raffle(db, "r1").await?;
        Ok::<_, String>((r.version, r.opened_at, stored_log(db, "r1").await?))
    };
    let before = unchanged().await?;
    check(
        !save_raffle(db, &claimed).await.map_err(fail)?,
        "a repeated save",
    )?;
    for (state, what) in [(&drawn, "drawn"), (&opened, "opened")] {
        check(
            !save_raffle(db, state).await.map_err(fail)?,
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
            !crate::write_raffle(db, state, 0).await.map_err(fail)?,
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
        !save_raffle(db, &other).await.map_err(fail)?,
        "another entrant list",
    )?;
    let ghost = Raffle::open(&manifest("r9", ["a".repeat(32)])).map_err(fail)?;
    check(
        !save_raffle(db, &ghost).await.map_err(fail)?,
        "a raffle that isn't there",
    )?;
    check(unchanged().await? == before, "nothing written for either")
}

pub(super) async fn raffle_events_are_append_only(db: &Db) -> Outcome {
    let [_, _, claimed] = three_states(&frozen(db, "r1").await?)?;
    check(save_raffle(db, &claimed).await.map_err(fail)?, "two events")?;
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

/// Dates raffle `id`'s erasure at `erase_at`, past the lifecycle's rules.
async fn erase_at(db: &Db, id: &str, erase_at: i64) -> Outcome {
    db.query("UPDATE raffles SET erase_at = ? WHERE id = ?")
        .bind(erase_at)
        .bind(id)
        .run()
        .await
        .map_err(fail)?;
    Ok(())
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
    // Open, never frozen: z.
    raffle_in(db, "op", RaffleState::Open).await?;
    check(join(db, "op", "z", "z@x", 10).await?, "joining op")?;
    erase_at(db, "op", 100).await?;
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

    // The date: op expires; n, d and x are erased; w and y are held.
    check(
        retention::expire_due(db, 100).await.map_err(fail)? == ["op"],
        "op expires",
    )?;
    check(
        entrants::list(db, "op").await.map_err(fail)?.is_empty(),
        "op's entrants are deleted",
    )?;
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
        retention::reapply_purges(db).await.map_err(fail)? == 1
            && raffle(db, "fin").await?.state == RaffleState::Purged,
        "the purge re-applied",
    )?;
    check(
        entrants::reapply_erasures(db).await.map_err(fail)? == 0
            && retention::reapply_purges(db).await.map_err(fail)? == 0,
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
    )
    .await
    .map_err(fail)?;

    let left = raffles::owner_leaves(db, "o", 70).await.map_err(fail)?;
    check(
        left.deleted == ["r2"] && left.purged == ["r1"],
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
    let logged: Vec<(String,)> = db
        .query("SELECT entrant_id FROM erasures WHERE raffle_id = 'r1' ORDER BY entrant_id")
        .all()
        .await
        .map_err(fail)?;
    check(
        logged.len() == 3,
        &format!("three erasures logged: {logged:?}"),
    )?;
    let r3 = raffle(db, "r3").await?;
    check(
        r3.state == RaffleState::Drawing
            && !entrant(db, "r3", "y").await?.erased
            && raffle(db, "r4").await?.state == RaffleState::Draft,
        "a drawing raffle and another owner's are left alone",
    )?;
    check(
        defaults::get(db, "o").await.map_err(fail)?.is_none(),
        "the defaults are gone",
    )
}

/// Which terms an owner accepted.
pub(super) async fn records_legal_acceptances(db: &Db) -> Outcome {
    check(
        !legal::accepted(db, "o", legal::TERMS, "v1")
            .await
            .map_err(fail)?,
        "nothing accepted yet",
    )?;
    legal::record(db, "o", "u1", legal::TERMS, "v1")
        .await
        .map_err(fail)?;
    legal::record(db, "o", "u2", legal::TERMS, "v1")
        .await
        .map_err(fail)?;
    check(
        legal::accepted(db, "o", legal::TERMS, "v1")
            .await
            .map_err(fail)?,
        "v1 accepted",
    )?;
    check(
        !legal::accepted(db, "o", legal::TERMS, "v2")
            .await
            .map_err(fail)?
            && !legal::accepted(db, "p", legal::TERMS, "v1")
                .await
                .map_err(fail)?,
        "not another version, not another owner",
    )?;
    let list = legal::list(db, "o").await.map_err(fail)?;
    check(
        list.len() == 1 && list[0].user_id == "u1",
        &format!("the first acceptance stays: {list:?}"),
    )
}
