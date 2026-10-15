//! The `domain.*` scenarios: the domain functions, on every backend.

use rafi_lifecycle::RaffleState;
use rafi_raffle::{BeaconSource, Decision, Event, Manifest, ManifestItem, Raffle, Resolution};

use super::{Outcome, check, fail};
use crate::{
    Db, EntrantRow, PurgeAfter, RaffleRow, altcha, entrants, live, prizes, raffles, retention,
    save_raffle, view::Full,
};

const PURGE: PurgeAfter = PurgeAfter {
    nonwinners_secs: 30,
    winners_secs: 90,
};

/// A frozen raffle `id` with two entrants and one prize, and its draw state.
async fn frozen(db: &Db, id: &str) -> Result<Raffle, String> {
    let state = Raffle::open(&manifest(id, ["a".repeat(32), "b".repeat(32)])).map_err(fail)?;
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, manifest_hash, created_at, updated_at)
         VALUES (?, 'o', ?, ?, 't', 'frozen', ?, 0, 0)",
    )
    .bind(id)
    .bind(format!("slug-{id}"))
    .bind(format!("code-{id}"))
    .bind(state.manifest_hash().to_string())
    .run()
    .await
    .map_err(fail)?;
    Ok(state)
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
    let dates = || async {
        db.query("SELECT state, finished_at, purge_nonwinners_at, purge_winners_at FROM raffles WHERE id = 'r1'")
            .one::<(String, i64, Option<i64>, Option<i64>)>()
            .await
            .map_err(fail)
    };
    let (lifecycle, finished_at, nonwinners, winners) = dates().await?;
    check(lifecycle == "finished", "finished state")?;
    check(
        nonwinners.is_none() && winners.is_none(),
        "the engine's write dates no erasure: that's the app's policy",
    )?;
    check(
        retention::date_purges(db, PURGE).await.map_err(fail)?,
        "the retention run dates it",
    )?;
    let (_, _, nonwinners, winners) = dates().await?;
    check(
        nonwinners == Some(finished_at + 30),
        "non-winner purge date",
    )?;
    check(winners == Some(finished_at + 90), "winner purge date")?;
    check(
        !retention::date_purges(db, PurgeAfter::days(1, 1))
            .await
            .map_err(fail)?,
        "dated once: new settings don't move it",
    )?;

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

/// A new raffle, moved straight to `state` (the store doesn't check the
/// lifecycle; the app does).
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
        consent_marketing: false,
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
        entrants::erase(db, "r1", Some("c")).await.map_err(fail)?,
        "erasing one",
    )?;
    check(
        entrants::count_active(db, "r1").await.map_err(fail)? == 2,
        "erased entrants don't count",
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
        ids == ["a", "b"],
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

pub(super) async fn adds_fakes_in_chunks(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let fake = |i: usize| entrants::Fake {
        id: format!("f{i:03}"),
        display_name: format!("Fake {i}"),
        claim_code: format!("C{i:03}"),
    };
    // More rows than fit in one statement's parameters on D1.
    let fakes: Vec<_> = (0..70).map(fake).collect();
    entrants::insert_fakes(db, "r1", "v1", 5, &fakes)
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
        entrants::insert_fakes(db, "r1", "v1", 6, &again)
            .await
            .is_err(),
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
        !entrants::erase(db, "r1", None).await.map_err(fail)?,
        "nobody to erase yet",
    )?;
    for id in ["a", "b", "c"] {
        join(db, "r1", id, &format!("{id}@x"), 10).await?;
    }
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

    check(
        entrants::erase(db, "r1", Some("a")).await.map_err(fail)?,
        "erasing one",
    )?;
    check(
        !entrants::erase(db, "r1", Some("a")).await.map_err(fail)?,
        "erasing them again",
    )?;
    let a = entrant(db, "r1", "a").await?;
    check(
        a.purged
            && a.display_name == "#a"
            && a.email_raw.is_none()
            && a.email_norm.is_none()
            && a.token_hash.is_none(),
        "their personal data is gone",
    )?;
    check(
        !entrant(db, "r1", "b").await?.purged,
        "nobody else was erased",
    )?;
    check(
        entrants::erase(db, "r1", None).await.map_err(fail)?,
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
    )?;

    // Before the freeze, removing deletes.
    entrants::delete(db, "r1", "a").await.map_err(fail)?;
    check(
        entrants::list(db, "r1").await.map_err(fail)?.len() == 2,
        "removed",
    )
}

pub(super) async fn retention_erases_what_is_due(db: &Db) -> Outcome {
    // A finished raffle: w claimed a prize, d claimed one that was delivered,
    // n won nothing.
    raffle_in(db, "r1", RaffleState::Open).await?;
    for id in ["w", "d", "n"] {
        join(db, "r1", id, &format!("{id}@x"), 10).await?;
    }
    patch(
        db,
        "r1",
        "d",
        entrants::Patch {
            delivered_at: Some(Some(1)),
            ..Default::default()
        },
    )
    .await?;
    let manifest = Manifest::new(
        "r1",
        BeaconSource::QUICKNET,
        vec!["w".into(), "d".into(), "n".into()],
        ["u1", "u2"]
            .map(|u| ManifestItem {
                id: u.into(),
                name: "Book".into(),
            })
            .into(),
    );
    // "w" and "d" won and claimed, as their engine logged it.
    let mut events = Vec::new();
    for (k, (item, winner)) in [(1, ("u1", "w")), (2, ("u2", "d"))] {
        events.push(Event::Drawn {
            k,
            item: item.into(),
            round: k,
            signature: String::new(),
            randomness: String::new(),
            pass: 1,
            pool_size: 3,
            index: 0,
            entrant: winner.into(),
        });
        events.push(Event::Resolved {
            k,
            resolution: Resolution::Claimed,
        });
    }
    let state = Raffle::restore(&manifest, &events).map_err(fail)?;
    let stored = crate::StoredRaffle::of(&state).map_err(fail)?;
    db.query(
        "UPDATE raffles SET state = 'finished', log = ?, winners = ?, finished_at = 100,
         purge_nonwinners_at = 130, purge_winners_at = 190 WHERE id = 'r1'",
    )
    .bind(stored.json)
    .bind(stored.winners)
    .run()
    .await
    .map_err(fail)?;
    // A raffle that hasn't finished is never due.
    raffle_in(db, "r2", RaffleState::Draft).await?;
    entrants::insert_manual(db, &manual("r2", "x", None))
        .await
        .map_err(fail)?;
    force_state(db, "r2", RaffleState::Drawing).await?;
    db.query("UPDATE raffles SET purge_nonwinners_at = 0, purge_winners_at = 0 WHERE id = 'r2'")
        .run()
        .await
        .map_err(fail)?;

    let erase = |now| retention::erase_due(db, now);
    check(
        erase(129).await.map_err(fail)? == 0,
        "nothing before the non-winners' date",
    )?;
    check(
        erase(130).await.map_err(fail)? == 2,
        "the non-winner and the delivered winner",
    )?;
    let purged = |e: EntrantRow| e.purged;
    check(
        purged(entrant(db, "r1", "n").await?)
            && purged(entrant(db, "r1", "d").await?)
            && !purged(entrant(db, "r1", "w").await?),
        "only those two",
    )?;
    check(
        raffle(db, "r1").await?.state == RaffleState::Finished,
        "the raffle isn't purged before the winners' date",
    )?;
    check(erase(189).await.map_err(fail)? == 0, "nobody else yet")?;
    check(erase(190).await.map_err(fail)? == 1, "the winner")?;
    let r = raffle(db, "r1").await?;
    check(
        r.state == RaffleState::Purged && r.purged_at == Some(190),
        "the raffle is purged",
    )?;
    check(
        erase(10_000).await.map_err(fail)? == 0,
        "a purged raffle is done",
    )?;
    check(
        !purged(entrant(db, "r2", "x").await?),
        "the unfinished raffle is untouched",
    )?;
    let log: Vec<(String, i64)> = db
        .query("SELECT entrant_id, at FROM erasures ORDER BY entrant_id")
        .all()
        .await
        .map_err(fail)?;
    check(
        log == [
            ("d".to_owned(), 130),
            ("n".to_owned(), 130),
            ("w".to_owned(), 190),
        ],
        &format!("the erasure log: {log:?}"),
    )?;

    // Live channels end a while after the raffle finished, once.
    check(
        retention::live_channels_due(db, 99)
            .await
            .map_err(fail)?
            .is_empty(),
        "no channel to end yet",
    )?;
    check(
        retention::live_channels_due(db, 100).await.map_err(fail)? == ["r1"],
        "the finished raffle's channel",
    )?;
    retention::live_ended(db, "r1", 200).await.map_err(fail)?;
    retention::live_ended(db, "r1", 300).await.map_err(fail)?;
    check(
        retention::live_channels_due(db, 1000)
            .await
            .map_err(fail)?
            .is_empty()
            && raffle(db, "r1").await?.live_ended_at == Some(200),
        "ended once",
    )?;

    check(
        retention::last_ok(db).await.map_err(fail)?.is_none(),
        "no run yet",
    )?;
    retention::record_ok(db, 5).await.map_err(fail)?;
    retention::record_ok(db, 7).await.map_err(fail)?;
    check(
        retention::last_ok(db).await.map_err(fail)? == Some(7),
        "the last run",
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
    entrants::insert_fakes(
        db,
        "r1",
        "v1",
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
    check(
        live::snapshot(db, "nope").await.map_err(fail)?.is_none(),
        "no such raffle",
    )
}

pub(super) async fn the_list_and_prizes_are_fixed_once_frozen(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Draft).await?;
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
        entrants::insert_fakes(db, "r1", "v1", 0, &[fake("f1"), fake("f2")])
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
        !entrants::insert_fakes(db, "r1", "v1", 0, &[fake("f3")])
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
    entrants::erase(db, "r1", Some("m1")).await.map_err(fail)?;
    check(
        !entrants::patch(db, "r1", "m1", &patch)
            .await
            .map_err(fail)?,
        "an erased entrant isn't patched",
    )
}
