//! The use cases on their own: in-memory SQLite, the in-process raffle
//! engine, mock drand, and no HTTP.

use std::{sync::Arc, time::Duration};

use rafi_api::{
    AddRequest, AddResult, AddRow, EntrantPatch, LifecycleCommand, Origin, PrizeInput, RowError,
    RowProblem, SkipReason, StartStep,
};
use rafi_app::{
    self as app, App, Clock, Error, EventDate, Limiter, Policy, ProofOfWork, Refusal,
    RetentionPolicy,
};
use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_engine::local::LocalEngine;
use rafi_lifecycle::prize::Delivery;
use rafi_live_server::{LiveUpdates, Polling};
use rafi_raffle::{Decision, DrawRequest, ItemStatus, Resolution};
use rafi_store::{Db, RaffleWriter, entrants, raffles};

const OWNER: &str = "organizer";
/// The test owner acting for themselves (a personal account).
const ACTOR: app::Actor<'static> = app::Actor::solo(OWNER);

async fn app() -> App {
    let db = Db::connect("sqlite::memory:").await.expect("database");
    let drand = DrandClient::new(DrandConfig {
        relays: vec![],
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
    })
    .expect("drand");
    let chain = drand.chain_hash();
    // A test's clock, which the engine's writes follow too: it says now
    // until a test moves it.
    let clock = Clock::fixed(Clock::system().now());
    let engine = LocalEngine::new(
        drand,
        RaffleWriter::with_clock(db.clone(), clock.clone()),
        Polling,
    );
    App {
        db,
        chain,
        engine: Some(Arc::new(engine)),
        live_updates: LiveUpdates::polling(),
        limiter: Arc::new(Limiter::default()),
        clock,
        policy: Policy {
            public_url: "http://localhost".into(),
            max_entrants: 100,
            proof_of_work: ProofOfWork::new("test-key", 1).unwrap(),
            retention: RetentionPolicy {
                days: 7,
                max_days: 30,
                winner_hold_days: 90,
            },
            min_age: 16,
        },
    }
}

/// An open raffle with one prize and `people` entered by hand, named so the
/// notice can open registration; returns its ID and theirs.
async fn open_raffle(app: &App, people: &[&str]) -> (String, Vec<String>) {
    open_raffle_with(app, people, Delivery::Later).await
}

/// [`open_raffle`], its prize reaching its winner by `delivery`.
async fn open_raffle_with(app: &App, people: &[&str], delivery: Delivery) -> (String, Vec<String>) {
    let id = app::create(app, ACTOR, "Meetup", &EventDate::today(app))
        .await
        .unwrap();
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.organizer_name = "Meetup".into();
    settings.contact_email = "hello@example.com".into();
    app::update_settings(app, ACTOR, &id, settings)
        .await
        .unwrap();
    app::upsert_prize(
        app,
        ACTOR,
        &id,
        PrizeInput {
            id: None,
            name: "Book".into(),
            sponsor: String::new(),
            description: String::new(),
            qty: 1,
            position: 0,
            delivery,
        },
    )
    .await
    .unwrap();
    app::lifecycle(app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    app::lifecycle(app, ACTOR, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for name in people {
        ids.push(add_one(app, &id, name, None).await.unwrap());
    }
    (id, ids)
}

/// An add of one person, `name` with `email`, in a batch of its own.
fn one(name: &str, email: Option<&str>) -> AddRequest {
    AddRequest {
        batch_id: app::ids::id(),
        origin: Origin::One,
        rows: vec![AddRow {
            name: name.into(),
            email: email.map(Into::into),
            list_ref: None,
        }],
    }
}

/// Adds `name` (and `email`) with the one-person form; returns their
/// entrant ID.
async fn add_one(app: &App, id: &str, name: &str, email: Option<&str>) -> app::Result<String> {
    let result = app::add_entrants(app, ACTOR, id, one(name, email)).await?;
    assert_eq!(result.added.len(), 1, "{result:?}");
    Ok(result.added[0].id.clone())
}

/// Waits until the raffle's one item is drawn; returns the winner.
async fn drawn(app: &App, id: &str) -> String {
    for _ in 0..200 {
        let raffle = raffles::get(&app.db, id).await.unwrap().unwrap();
        let drawn = raffles::drawn(&app.db, &raffle).await.unwrap();
        if let Some(winner) = drawn.and_then(|d| {
            d.items()
                .iter()
                .find(|i| i.status() == ItemStatus::Drawn)
                .and_then(|i| i.winner().cloned())
        }) {
            return winner.to_string();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never drawn");
}

#[tokio::test]
async fn a_refused_patch_changes_nothing() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna"]).await;
    // Renamed and delivered at once: nobody won anything yet, so neither.
    let patch = EntrantPatch {
        display_name: Some("Bea".into()),
        delivered: Some(true),
        ..Default::default()
    };
    let err = app::patch_entrant(&app, ACTOR, &id, &people[0], patch)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Refused(Refusal::NotDeliverable)),
        "{err:?}"
    );
    let anna = entrants::get(&app.db, &id, &people[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(anna.display_name, "Anna", "the rename didn't land either");
    assert!(anna.delivered_at.is_none());
}

#[tokio::test]
async fn only_a_claimed_winner_is_delivered_to() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna", "Bence"]).await;
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let unknown = DrawRequest {
        item: "nope".into(),
    };
    assert!(matches!(
        app::command(&app, ACTOR, &id, unknown, "d0").await,
        Err(Error::NotFound)
    ));
    let item = {
        let admin = app::admin(&app, OWNER, &id).await.unwrap();
        admin.view.items[0].item.clone()
    };
    app::command(&app, ACTOR, &id, DrawRequest { item: item.clone() }, "d1")
        .await
        .unwrap();
    let winner = drawn(&app, &id).await;
    let loser = people.iter().find(|p| **p != winner).unwrap().clone();
    let deliver = || EntrantPatch {
        delivered: Some(true),
        ..Default::default()
    };
    assert!(
        matches!(
            app::patch_entrant(&app, ACTOR, &id, &winner, deliver()).await,
            Err(Error::Refused(Refusal::NotDeliverable))
        ),
        "drawn, but not claimed yet"
    );
    app::command(
        &app,
        ACTOR,
        &id,
        Decision::Resolve {
            item,
            resolution: Resolution::Claimed,
        },
        "c1",
    )
    .await
    .unwrap();
    let delivered_at = |who: String| {
        let (app, id) = (&app, &id);
        async move {
            entrants::get(&app.db, id, &who)
                .await
                .unwrap()
                .unwrap()
                .delivered_at
        }
    };
    assert_eq!(
        delivered_at(winner.clone()).await,
        None,
        "a prize sent later isn't delivered by its claim"
    );
    app::patch_entrant(&app, ACTOR, &id, &winner, deliver())
        .await
        .unwrap();
    assert!(delivered_at(winner.clone()).await.is_some());
    assert!(matches!(
        app::patch_entrant(&app, ACTOR, &id, &loser, deliver()).await,
        Err(Error::Refused(Refusal::NotDeliverable))
    ));
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    let flags: Vec<(bool, bool)> = admin
        .entrants
        .iter()
        .map(|e| (e.id == winner, e.deliverable))
        .collect();
    assert!(
        flags.iter().all(|(won, deliverable)| won == deliverable),
        "the admin page offers delivery to the winner only: {flags:?}"
    );
}

#[tokio::test]
async fn claiming_a_prize_handed_over_on_the_spot_delivers_it() {
    let app = app().await;
    let (id, _) = open_raffle_with(&app, &["Anna", "Bence"], Delivery::Later).await;
    let prize = app::admin(&app, OWNER, &id).await.unwrap().prizes[0]
        .id
        .clone();
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    // Found out during the draw: past the freeze, the delivery still changes.
    app::set_prize_delivery(&app, ACTOR, &id, &prize, Delivery::OnTheSpot)
        .await
        .unwrap();
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert_eq!(admin.prizes[0].delivery, Delivery::OnTheSpot);
    let item = admin.view.items[0].item.clone();
    app::command(&app, ACTOR, &id, DrawRequest { item: item.clone() }, "d1")
        .await
        .unwrap();
    let winner = drawn(&app, &id).await;
    let claim = Decision::Resolve {
        item,
        resolution: Resolution::Claimed,
    };
    app::command(&app, ACTOR, &id, claim, "c1").await.unwrap();

    let winner_row = entrants::get(&app.db, &id, &winner).await.unwrap().unwrap();
    assert_eq!(
        winner_row.delivered_at,
        Some(app.clock.now()),
        "the claim delivered it"
    );
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    let anna = admin.entrants.iter().find(|e| e.id == winner).unwrap();
    assert!(anna.delivered && anna.deliverable && anna.held_until.is_none());
    assert_eq!(admin.retention.held_winners, 0, "nobody's data is held");
    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&id)
        .all()
        .await
        .unwrap();
    for row in [
        (
            "set_prize_delivery".to_owned(),
            format!("{prize} on_the_spot"),
        ),
        ("deliver".to_owned(), winner.clone()),
    ] {
        assert!(audit.contains(&row), "{row:?} in {audit:?}");
    }
    // A mistake can be undone, as any delivery.
    let undeliver = EntrantPatch {
        delivered: Some(false),
        ..Default::default()
    };
    app::patch_entrant(&app, ACTOR, &id, &winner, undeliver)
        .await
        .unwrap();

    app::command(&app, ACTOR, &id, Decision::Finish, "f1")
        .await
        .unwrap();
    assert!(matches!(
        app::set_prize_delivery(&app, ACTOR, &id, &prize, Delivery::Later).await,
        Err(Error::Refused(Refusal::PrizeDeliveryFixed))
    ));
    assert!(matches!(
        app::set_prize_delivery(&app, ACTOR, &id, "nope", Delivery::Later).await,
        Err(Error::Refused(Refusal::PrizeDeliveryFixed))
    ));
}

#[tokio::test]
async fn a_prizes_delivery_needs_the_prize() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna"]).await;
    assert!(matches!(
        app::set_prize_delivery(&app, ACTOR, &id, "nope", Delivery::OnTheSpot).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        app::set_prize_delivery(
            &app,
            app::Actor::solo("someone"),
            &id,
            "nope",
            Delivery::OnTheSpot
        )
        .await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_admin_page_offers_what_the_rules_allow() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna"]).await;
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert_eq!(actions.start, Some(StartStep::Freeze));
    assert!(!actions.draw && !actions.export && !actions.purge);

    // A freeze that stopped before its manifest was written.
    assert!(
        raffles::freeze(
            &app.db,
            &id,
            rafi_lifecycle::RaffleState::Open,
            app.clock.now(),
        )
        .await
        .unwrap()
    );
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert_eq!(
        actions.start,
        Some(StartStep::Retry),
        "the page can recover it"
    );
    assert!(actions.draw && actions.export && !actions.purge);

    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert_eq!(actions.start, Some(StartStep::Started));

    app::command(&app, ACTOR, &id, Decision::Finish, "f1")
        .await
        .unwrap();
    for _ in 0..200 {
        let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
        if actions.purge {
            assert_eq!(actions.start, None, "a finished raffle doesn't start");
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a finished raffle offers erasure");
}

#[tokio::test]
async fn joining_follows_the_raffle_rules() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let join = |name: &'static str, email: &'static str, rules: bool| {
        let (app, raffle) = (app.clone(), raffle.clone());
        async move {
            let proof = solved(&app);
            let req = app::JoinRequest {
                name,
                email,
                rules,
                proof: &proof,
            };
            app::join(&app, &raffle, &req).await
        }
    };
    assert_eq!(
        join("Anna", "anna@x.org", false).await,
        Err(app::JoinError::Rules)
    );
    assert_eq!(
        join("Anna", "not an email", true).await,
        Err(app::JoinError::Email)
    );
    assert!(matches!(
        join("Anna", "anna@x.org", true).await,
        Ok(app::JoinOutcome::Joined { .. })
    ));
    assert_eq!(
        join("Anna", "Anna@X.org", true).await,
        Ok(app::JoinOutcome::AlreadyIn)
    );
}

/// A solved proof-of-work, as the join form posts it.
fn solved(app: &App) -> String {
    let challenge = app::pow::challenge(app).unwrap();
    let solution = altcha::solve_challenge(altcha::SolveChallengeOptions::new(&challenge))
        .unwrap()
        .expect("solvable");
    app::pow::encode(&altcha::Payload {
        challenge,
        solution,
    })
}

#[tokio::test]
async fn a_join_needs_a_fresh_proof_of_work() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let proof = solved(&app);
    let req = |name, email, proof| app::JoinRequest {
        name,
        email,
        rules: true,
        proof,
    };
    assert_eq!(
        app::join(&app, &raffle, &req("Bot", "bot@x.org", "")).await,
        Err(app::JoinError::Proof)
    );
    assert!(matches!(
        app::join(&app, &raffle, &req("Anna", "anna@x.org", &proof)).await,
        Ok(app::JoinOutcome::Joined { .. })
    ));
    assert_eq!(
        app::join(&app, &raffle, &req("Bea", "bea@x.org", &proof)).await,
        Err(app::JoinError::Proof),
        "a proof-of-work is redeemed once"
    );
}

#[tokio::test]
async fn joins_are_rate_limited_per_email() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let (budget, _) = app::Limit::JoinPerEmail.budget();
    for _ in 0..budget {
        let proof = solved(&app);
        let req = app::JoinRequest {
            name: "Anna",
            email: "anna@x.org",
            rules: true,
            proof: &proof,
        };
        app::join(&app, &raffle, &req).await.unwrap();
    }
    let proof = solved(&app);
    let req = app::JoinRequest {
        name: "Anna",
        email: "ANNA@x.org",
        rules: true,
        proof: &proof,
    };
    assert_eq!(
        app::join(&app, &raffle, &req).await,
        Err(app::JoinError::Limited)
    );
}

/// A list of `rows`: a name, an email and a registration ID each.
fn list(rows: &[(&str, Option<&str>, Option<&str>)]) -> AddRequest {
    AddRequest {
        batch_id: app::ids::id(),
        origin: Origin::List,
        rows: rows
            .iter()
            .map(|(name, email, list_ref)| AddRow {
                name: (*name).into(),
                email: email.map(Into::into),
                list_ref: list_ref.map(Into::into),
            })
            .collect(),
    }
}

/// The audit rows of raffle `id` with `action`: their details, oldest first.
async fn audited(app: &App, id: &str, action: &str) -> Vec<String> {
    app.db
        .query("SELECT detail FROM audit WHERE raffle_id = ? AND action = ? ORDER BY id")
        .bind(id)
        .bind(action)
        .all::<(String,)>()
        .await
        .unwrap()
        .into_iter()
        .map(|(d,)| d)
        .collect()
}

/// Why `result` was refused.
fn refusal<T: std::fmt::Debug>(result: app::Result<T>) -> Refusal {
    match result {
        Err(Error::Refused(r)) => r,
        other => panic!("not refused: {other:?}"),
    }
}

/// The display names of raffle `id`'s entrants, in the order they're listed.
async fn names(app: &App, id: &str) -> Vec<String> {
    entrants::list(&app.db, id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.display_name)
        .collect()
}

#[tokio::test]
async fn adding_one_person_is_a_batch_of_one() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let add = one("Anna", Some("Anna@X.org"));
    let batch = add.batch_id.clone();
    let result = app::add_entrants(&app, ACTOR, &id, add.clone())
        .await
        .unwrap();
    assert_eq!(result.batch_id, batch);
    assert!(result.skipped.is_empty());
    let [added] = &result.added[..] else {
        panic!("{result:?}")
    };
    assert_eq!(added.row, 0);
    let anna = entrants::get(&app.db, &id, &added.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(anna.source, rafi_lifecycle::entrant::Source::Manual);
    assert_eq!(anna.email_raw.as_deref(), Some("Anna@X.org"));
    assert_eq!(anna.email_norm.as_deref(), Some("anna@x.org"));
    assert!(
        anna.consent_version.is_none() && anna.consent_rules_at.is_none(),
        "they accepted nothing"
    );
    assert!(anna.token_hash.is_none(), "no device holds the entry");
    assert_eq!(
        (anna.batch_id.as_deref(), anna.batch_row),
        (Some(batch.as_str()), None),
        "one person has no row in a list"
    );
    assert_eq!(anna.created_at, app.clock.now());

    // A retry (the reply was lost) answers the same, and adds nobody.
    assert_eq!(
        app::add_entrants(&app, ACTOR, &id, add).await.unwrap(),
        result
    );
    assert_eq!(names(&app, &id).await, ["Anna"]);

    // Claim codes are unique in the raffle.
    for name in ["Bence", "Csilla", "Dora"] {
        add_one(&app, &id, name, None).await.unwrap();
    }
    let codes: std::collections::HashSet<String> = entrants::list(&app.db, &id)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|e| e.claim_code)
        .collect();
    assert_eq!(codes.len(), 4);

    // The same email again is already in: skipped, not refused.
    let again = app::add_entrants(&app, ACTOR, &id, one("Anna K.", Some("anna+2@x.org")))
        .await
        .unwrap();
    assert!(again.added.is_empty());
    assert_eq!(
        again.skipped,
        [rafi_api::SkippedRow {
            row: 0,
            reason: SkipReason::AlreadyIn
        }]
    );

    // One audit row per add, retries not, and nothing personal in them.
    let adds = audited(&app, &id, "add").await;
    assert_eq!(adds.len(), 5, "{adds:?}");
    assert_eq!(
        adds[0],
        format!("batch={batch} origin=one added=1 skipped=0")
    );
    assert!(
        adds[4].ends_with(" origin=one added=0 skipped=1"),
        "{adds:?}"
    );
    assert!(
        adds.iter().all(|d| !d.contains("Anna") && !d.contains('@')),
        "{adds:?}"
    );

    // One person's name reaches the screen after the usual delay.
    let recent = |app: &App| {
        let (app, id) = (app.clone(), id.clone());
        async move {
            app::view::build(&app, &id)
                .await
                .unwrap()
                .unwrap()
                .recent_names
        }
    };
    assert!(recent(&app).await.is_empty());
    app.clock.advance(5);
    let mut shown = recent(&app).await;
    shown.sort();
    assert_eq!(shown, ["Anna", "Bence", "Csilla", "Dora"]);
}

#[tokio::test]
async fn adding_a_list_keeps_its_order_and_skips_who_is_in() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Joined Before"]).await;
    app.clock.advance(1);
    add_one(&app, &id, "Anna", Some("anna@x.org"))
        .await
        .unwrap();
    let add = list(&[
        ("Zoe", Some("zoe@x.org"), None),
        ("Anna Again", Some("ANNA@x.org"), None),
        ("Bence", None, Some("T-1")),
        ("Adam", None, None),
    ]);
    let batch = add.batch_id.clone();
    let result: AddResult = app::add_entrants(&app, ACTOR, &id, add).await.unwrap();
    let rows: Vec<u32> = result.added.iter().map(|a| a.row).collect();
    assert_eq!(rows, [0, 2, 3]);
    assert_eq!(
        result.skipped,
        [rafi_api::SkippedRow {
            row: 1,
            reason: SkipReason::AlreadyIn
        }]
    );
    // Stamped so their names show at once, in the list's order (not by
    // name or ID).
    assert_eq!(
        names(&app, &id).await,
        ["Zoe", "Bence", "Adam", "Joined Before", "Anna"]
    );
    let view = app::view::build(&app, &id).await.unwrap().unwrap();
    assert_eq!(view.recent_names, ["Adam", "Bence", "Zoe"], "newest first");
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    let bence = admin
        .entrants
        .iter()
        .find(|e| e.display_name == "Bence")
        .unwrap();
    assert_eq!(bence.source, rafi_lifecycle::entrant::Source::Manual);
    assert_eq!(bence.list_ref.as_deref(), Some("T-1"));
    assert_eq!(bence.batch_id.as_deref(), Some(batch.as_str()));
    assert_eq!(
        audited(&app, &id, "add").await.last().unwrap(),
        &format!("batch={batch} origin=list added=3 skipped=1")
    );

    // An updated list adds only who is new, by email or registration ID.
    let update = list(&[
        ("Zoe", Some("zoe@x.org"), None),
        ("Bence B.", None, Some("T-1")),
        ("Csilla", None, None),
    ]);
    let result = app::add_entrants(&app, ACTOR, &id, update).await.unwrap();
    assert_eq!(result.added.iter().map(|a| a.row).collect::<Vec<_>>(), [2]);
    assert_eq!(result.skipped.len(), 2);
    assert_eq!(names(&app, &id).await.len(), 6);
}

#[tokio::test]
async fn adds_the_raffle_doesnt_take_are_refused() {
    let app = app().await;
    let draft = app::create(&app, ACTOR, "Draft", &EventDate::today(&app))
        .await
        .unwrap();
    let anna = || list(&[("Anna", None, None)]);
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &draft, anna()).await),
        Refusal::Add(rafi_lifecycle::entrant::AddRefusal::Unpublished),
        "no list before publishing"
    );
    add_one(&app, &draft, "Anna", None).await.unwrap();

    let (id, _) = open_raffle(&app, &[]).await;
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.test_mode = true;
    app::update_settings(&app, ACTOR, &id, settings)
        .await
        .unwrap();
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, anna()).await),
        Refusal::Add(rafi_lifecycle::entrant::AddRefusal::TestMode),
        "no list in test mode"
    );
    add_one(&app, &id, "Anna", None).await.unwrap();

    // After the freeze, nobody is added, one person or a list.
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    for add in [one("Bence", None), anna()] {
        assert_eq!(
            refusal(app::add_entrants(&app, ACTOR, &id, add).await),
            Refusal::Add(rafi_lifecycle::entrant::AddRefusal::Frozen)
        );
    }
    assert_eq!(names(&app, &id).await, ["Anna"]);
}

#[tokio::test]
async fn invalid_adds_are_refused_whole_with_their_rows() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let add = list(&[
        ("Anna", Some("anna@x.org"), None),
        ("Bence", Some("not an email"), None),
        ("", None, None),
        ("Dora", Some("ANNA@x.org"), None),
        ("Emil", None, Some("1")),
        ("Feri", None, Some(" 1 ")),
    ]);
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, add).await);
    let problem = |row, problem| RowProblem { row, problem };
    assert_eq!(
        refused,
        Refusal::Rows {
            origin: Origin::List,
            rows: vec![
                problem(1, RowError::Email),
                problem(2, RowError::Name),
                problem(3, RowError::DuplicateEmail),
                problem(5, RowError::DuplicateListRef),
            ],
        }
    );
    assert!(refused.invalid());
    assert!(
        refused.to_string().starts_with(
            "4 rows can't be added, so nothing was: row 2: not an email address; row 3: "
        ),
        "{refused}"
    );
    // One person: the problem itself.
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, one(" ", None)).await);
    assert_eq!(
        refused.to_string(),
        "The name must be 1–64 characters, with at least one letter or digit and nothing invisible."
    );
    assert!(names(&app, &id).await.is_empty(), "nothing was added");

    // The form adds exactly one person; a list 1 to MAX_ADD_ROWS.
    let mut two = one("Anna", None);
    two.rows.push(two.rows[0].clone());
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, two).await),
        Refusal::RowCount(Origin::One)
    );
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, list(&[])).await),
        Refusal::RowCount(Origin::List)
    );
    let names_: Vec<String> = (0..=rafi_lifecycle::entrant::MAX_ADD_ROWS)
        .map(|i| format!("P{i}"))
        .collect();
    let rows: Vec<_> = names_.iter().map(|n| (n.as_str(), None, None)).collect();
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, list(&rows)).await),
        Refusal::RowCount(Origin::List),
        "too many rows"
    );
    let mut bad_batch = one("Anna", None);
    bad_batch.batch_id = "not an id".into();
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, bad_batch).await),
        Refusal::BatchId
    );
    assert!(names(&app, &id).await.is_empty());
}

#[tokio::test]
async fn an_add_past_the_cap_is_refused_whole() {
    let mut app = app().await;
    app.policy.max_entrants = 5;
    let (id, _) = open_raffle(&app, &["Anna", "Bence"]).await;
    let four = list(&[
        ("Csilla", None, None),
        ("Dora", None, None),
        ("Emil", None, None),
        ("Feri", None, None),
    ]);
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, four).await);
    assert_eq!(refused, Refusal::Room { adding: 4, left: 3 });
    assert!(refused.to_string().contains("Only 3 more"), "{refused}");
    assert_eq!(names(&app, &id).await.len(), 2, "nothing was added");
    // Who is in already takes no room.
    let three = list(&[
        ("Csilla", None, None),
        ("Dora", None, None),
        ("Emil", None, None),
    ]);
    app::add_entrants(&app, ACTOR, &id, three).await.unwrap();
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, one("Gabi", None)).await);
    assert_eq!(refused, Refusal::Room { adding: 1, left: 0 });
    assert!(
        refused.to_string().starts_with("The raffle is full"),
        "{refused}"
    );
    assert_eq!(names(&app, &id).await.len(), 5);
}

#[tokio::test]
async fn undoing_an_add_takes_it_back_until_the_freeze() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna"]).await;
    let anna = entrants::get(&app.db, &id, &people[0])
        .await
        .unwrap()
        .unwrap();
    let anna_batch = anna.batch_id.unwrap();
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(!admin.actions.edit_rules, "someone added fixes the rules");
    assert_eq!(
        admin.actions.undo,
        Some(rafi_api::UndoBatch {
            batch_id: anna_batch.clone(),
            count: 1
        })
    );

    // A list added later is the one to undo, though its names are stamped
    // early to show at once.
    let add = list(&[("Bence", None, None), ("Csilla", None, None)]);
    let list_batch = add.batch_id.clone();
    app.clock.advance(1);
    app::add_entrants(&app, ACTOR, &id, add).await.unwrap();
    let undo = app::admin(&app, OWNER, &id).await.unwrap().actions.undo;
    assert_eq!(
        undo,
        Some(rafi_api::UndoBatch {
            batch_id: list_batch.clone(),
            count: 2
        })
    );
    assert_eq!(
        app::undo_add(&app, ACTOR, &id, &list_batch).await.unwrap(),
        2
    );
    assert_eq!(names(&app, &id).await, ["Anna"]);
    assert_eq!(
        app::undo_add(&app, ACTOR, &id, &list_batch).await.unwrap(),
        0,
        "undone already: nothing to do"
    );

    // Undoing the last real entrant's add unlocks the rules.
    assert_eq!(
        app::undo_add(&app, ACTOR, &id, &anna_batch).await.unwrap(),
        1
    );
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(admin.entrants.is_empty());
    assert!(
        admin.actions.edit_rules,
        "nobody accepted the rules any more"
    );
    assert_eq!(admin.actions.undo, None);
    assert_eq!(
        audited(&app, &id, "undo_add").await,
        [
            format!("batch={list_batch} removed=2"),
            format!("batch={anna_batch} removed=1")
        ]
    );
    assert_eq!(
        refusal(app::undo_add(&app, ACTOR, &id, "not an id").await),
        Refusal::BatchId
    );

    // Once frozen, an add stays.
    let dora = add_one(&app, &id, "Dora", None).await.unwrap();
    let dora_batch = entrants::get(&app.db, &id, &dora)
        .await
        .unwrap()
        .unwrap()
        .batch_id
        .unwrap();
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    assert_eq!(
        refusal(app::undo_add(&app, ACTOR, &id, &dora_batch).await),
        Refusal::EntrantsFrozen
    );
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert_eq!(admin.actions.undo, None, "nothing to offer once frozen");
    assert_eq!(names(&app, &id).await, ["Dora"]);
}

#[tokio::test]
async fn adds_and_undos_are_rate_limited_per_owner() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let (budget, _) = app::Limit::AddPerOwner.budget();
    for _ in 0..budget {
        assert_eq!(app::undo_add(&app, ACTOR, &id, "nothing").await.unwrap(), 0);
    }
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, one("Anna", None)).await),
        Refusal::Limited
    );
    assert_eq!(
        refusal(app::undo_add(&app, ACTOR, &id, "nothing").await),
        Refusal::Limited
    );
    assert!(names(&app, &id).await.is_empty());
    // Another organizer has their own budget.
    let someone = app::Actor::solo("someone");
    let theirs = app::create(&app, someone, "Theirs", &EventDate::today(&app))
        .await
        .unwrap();
    app::add_entrants(&app, someone, &theirs, one("Anna", None))
        .await
        .unwrap();
}
