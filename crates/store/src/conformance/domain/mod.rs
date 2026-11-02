//! The `domain.*` scenarios: the domain functions, on every backend.
//!
//! This module holds the fixtures the scenarios share and re-exports the
//! scenarios, which are in its submodules, one per concern.

use rafi_lifecycle::{
    RaffleState,
    entrant::{Origin, Source},
    prize::Delivery,
};
use rafi_raffle::{BeaconSource, Decision, Event, Manifest, ManifestItem, Raffle, Resolution};

use super::{Outcome, check, fail};
use crate::{
    Db, EntrantRow, RaffleRow, audit, defaults, entrants, housekeeping, live, pow, prizes, raffles,
    retention, save_raffle,
    view::{Full, Lite, Summary},
};

mod delivery;
mod erasure;
mod joins;
mod misc;
mod raffle_log;
mod removals;
mod settings;
mod views;

pub(super) use delivery::{a_claim_keeps_its_delivery, delivers_on_the_spot_with_the_claim};
pub(super) use erasure::{
    delivers_all_and_removes_the_organizer, erases_and_logs_erasures,
    racing_erasures_log_each_entrant_once, retention_erases_what_is_due,
};
pub(super) use joins::{
    adds_all_or_nothing, adds_fakes_in_chunks, adds_where_the_raffle_takes_them,
    an_add_receipt_outlives_its_undo, deletes_a_batch, joins_up_to_the_cap_once_per_email,
    patches_an_entrant_at_once, racing_adds_respect_the_cap, racing_joins_respect_the_cap,
};
pub(super) use misc::{live_channels_end_once, pow_refuses_replays};
pub(super) use raffle_log::{
    an_opened_raffle_may_have_no_events, keeps_the_newest_state, logs_a_whole_evening,
    raffle_events_are_append_only, saves_only_newer_states,
};
pub(super) use removals::{
    an_objected_winner_waits_for_the_hold, objections_hide_and_erase_at_the_end,
    removals_keep_people_out_until_lifted,
};
pub(super) use settings::{
    deletes_only_undrawn_raffles, fakes_go_with_test_mode, freezes_in_steps,
    registration_guards_the_lifecycle, rules_are_fixed_once_an_entrant_locks_them,
    saves_owner_defaults, the_list_and_prizes_are_fixed_once_frozen,
};
pub(super) use views::{
    phones_see_only_drawn_names, reads_the_full_view, summarizes_the_entrants_as_the_rows_do,
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
/// Deletes raffle `id` whatever it holds, as the schema allows (the
/// app's [`raffles::delete`] keeps a raffle that has drawn): to start a
/// scenario's raffle again, or to see what deleting takes with it.
async fn drop_raffle(db: &Db, id: &str) -> Outcome {
    db.query("DELETE FROM raffles WHERE id = ?")
        .bind(id)
        .run()
        .await
        .map_err(fail)?;
    Ok(())
}

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
            raffles::transition(db, id, RaffleState::Draft, state, true, NOW)
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
            lifted_emails: &[],
            lifted_list_refs: &[],
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
        registration: true,
    }
}

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
        serde_json::from_str(include_str!("../../../../../testdata/draw-vectors.json"))
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

/// Raffle `raffle_id`'s removals at their request, as `(email key,
/// registration ID, when)`.
async fn removed(
    db: &Db,
    raffle_id: &str,
) -> Result<Vec<(Option<String>, Option<String>, i64)>, String> {
    Ok(entrants::removals(db, raffle_id)
        .await
        .map_err(fail)?
        .into_iter()
        .map(|r| (r.email_norm, r.list_ref, r.removed_at))
        .collect())
}
