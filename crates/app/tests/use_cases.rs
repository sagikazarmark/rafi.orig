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
            ..AddRow::default()
        }],
        name_format: None,
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
            k: None,
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
        k: None,
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
            true,
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
                ..AddRow::default()
            })
            .collect(),
        name_format: None,
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
        format!("batch={batch} origin=one added=1 skipped=0 approved=0 lifted=0")
    );
    assert!(
        adds[4].ends_with(" origin=one added=0 skipped=1 approved=0 lifted=0"),
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
        &format!("batch={batch} origin=list added=3 skipped=1 approved=0 lifted=0")
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
    // The admin's view says whether a list can be added, and why not: the
    // list tab derives nothing.
    let list_tab = |id: String| {
        let app = app.clone();
        async move {
            let a = app::admin(&app, OWNER, &id).await.unwrap().actions;
            (a.add_list, a.add_list_refusal)
        }
    };
    assert_eq!(
        list_tab(draft.clone()).await,
        (
            false,
            Some(rafi_lifecycle::entrant::AddRefusal::Unpublished)
        )
    );
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &draft, anna()).await),
        Refusal::Add(rafi_lifecycle::entrant::AddRefusal::Unpublished),
        "no list before publishing"
    );
    add_one(&app, &draft, "Anna", None).await.unwrap();

    let (id, _) = open_raffle(&app, &[]).await;
    assert_eq!(list_tab(id.clone()).await, (true, None));
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.test_mode = true;
    app::update_settings(&app, ACTOR, &id, settings)
        .await
        .unwrap();
    assert_eq!(
        list_tab(id.clone()).await,
        (false, Some(rafi_lifecycle::entrant::AddRefusal::TestMode))
    );
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, anna()).await),
        Refusal::Add(rafi_lifecycle::entrant::AddRefusal::TestMode),
        "no list in test mode"
    );
    add_one(&app, &id, "Anna", None).await.unwrap();

    // After the freeze, nobody is added, one person or a list.
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    assert_eq!(
        list_tab(id.clone()).await,
        (false, Some(rafi_lifecycle::entrant::AddRefusal::Frozen))
    );
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
    let problem = |row, problem| RowProblem {
        row,
        problem,
        shown_as: None,
    };
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
    assert_eq!(refused.kind(), app::RefusalKind::Invalid);
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

/// The browser keeps an add's batch ID across a network error and sends it
/// again: after an undo too, which deleted the people, the retry is
/// answered with the first result and adds nobody.
#[tokio::test]
async fn a_retry_after_an_undo_adds_nobody() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let add = list(&[("Bence", None, None), ("Csilla", None, None)]);
    let first = app::add_entrants(&app, ACTOR, &id, add.clone())
        .await
        .unwrap();
    assert_eq!(first.added.len(), 2);
    assert_eq!(
        app::undo_add(&app, ACTOR, &id, &add.batch_id)
            .await
            .unwrap(),
        2
    );
    let retried = app::add_entrants(&app, ACTOR, &id, add).await.unwrap();
    assert_eq!(retried, first, "the first result");
    assert!(names(&app, &id).await.is_empty(), "nobody added again");
    assert_eq!(audited(&app, &id, "add").await.len(), 1, "one add");
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

/// Joins `raffle_id` on the participant page as `name` with `email`.
async fn join_as(app: &App, raffle_id: &str, name: &str, email: &str) {
    let raffle = raffles::get(&app.db, raffle_id).await.unwrap().unwrap();
    let proof = solved(app);
    let req = app::JoinRequest {
        name,
        email,
        rules: true,
        proof: &proof,
    };
    assert!(matches!(
        app::join(app, &raffle, &req).await,
        Ok(app::JoinOutcome::Joined { .. })
    ));
}

/// Nobody the organizer adds is shown with a name someone else has, by
/// the key (case, spacing and composition aside): not someone who joined,
/// not someone added before, not another row. The whole add is refused,
/// with the rows and whom they clash with; the one-person form too. Joins
/// aren't held to it.
#[tokio::test]
async fn added_names_are_unique() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Bence Nagy"]).await;
    join_as(&app, &id, "Anna K.", "anna@x.org").await;

    let clash = |row, problem, shown_as: &str| RowProblem {
        row,
        problem,
        shown_as: Some(shown_as.to_owned()),
    };
    let add = list(&[
        ("Csilla", None, None),
        ("anna  k.", None, None),
        ("BENCE NAGY", Some("bence@x.org"), None),
        ("Dora", None, None),
        ("Csilla", None, None),
        ("Do\u{301}ra", None, None),
        ("Dóra", None, None),
    ]);
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, add).await);
    assert_eq!(
        refused,
        Refusal::Rows {
            origin: Origin::List,
            rows: vec![
                clash(1, RowError::NameTaken, "Anna K."),
                clash(2, RowError::NameTaken, "Bence Nagy"),
                clash(4, RowError::DuplicateName, "Csilla"),
                clash(6, RowError::DuplicateName, "Dóra"),
            ],
        },
        "against who is in, joined or added, and the earlier rows; composed first"
    );
    assert_eq!(refused.kind(), app::RefusalKind::Invalid);
    assert!(
        refused.to_string().starts_with(
            "4 rows can't be added, so nothing was: row 2: someone is already shown as “Anna K.”; row 3: someone is already shown as “Bence Nagy”; row 5: an earlier row is shown as “Csilla” too; and 1 more."
        ),
        "{refused}"
    );
    assert_eq!(names(&app, &id).await.len(), 2, "nothing was added");

    // One person too: the organizer adds a surname or a number.
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, one("Anna K.", None)).await);
    assert_eq!(
        refused.to_string(),
        "Someone is already shown as “Anna K.”."
    );
    add_one(&app, &id, "Anna K. (2)", None).await.unwrap();

    // A row already in (by its email) keeps its own name: an updated list
    // goes in, the new rows checked against everyone.
    add_one(&app, &id, "Emil", Some("emil@x.org"))
        .await
        .unwrap();
    let update = list(&[("Emil", Some("emil@x.org"), None), ("Feri", None, None)]);
    let result = app::add_entrants(&app, ACTOR, &id, update).await.unwrap();
    assert_eq!(result.added.iter().map(|a| a.row).collect::<Vec<_>>(), [1]);
    assert_eq!(result.skipped.len(), 1);
    let refused = refusal(
        app::add_entrants(
            &app,
            ACTOR,
            &id,
            list(&[("Gabi", Some("gabi@x.org"), None), ("emil", None, None)]),
        )
        .await,
    );
    assert_eq!(
        refused,
        Refusal::Rows {
            origin: Origin::List,
            rows: vec![clash(1, RowError::NameTaken, "Emil")],
        },
        "a new row with an added name"
    );

    // Joins and renames aren't held to it.
    join_as(&app, &id, "Feri", "feri@x.org").await;
    let anna2 = entrants::list(&app.db, &id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.display_name == "Anna K. (2)")
        .unwrap();
    app::patch_entrant(
        &app,
        ACTOR,
        &id,
        &anna2.id,
        EntrantPatch {
            display_name: Some("Anna K.".into()),
            ..EntrantPatch::default()
        },
    )
    .await
    .unwrap();
    let mut shown = names(&app, &id).await;
    shown.sort();
    assert_eq!(
        shown,
        ["Anna K.", "Anna K.", "Bence Nagy", "Emil", "Feri", "Feri"]
    );
}

/// A name the blocklist refuses goes in when the organizer approved it, and
/// the approval is counted in the audit; it lifts the blocklist only.
#[tokio::test]
async fn blocked_names_go_in_when_approved() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let mut add = list(&[
        ("Nazir Ahmed", None, None),
        ("Gábor Buzinkay", None, None),
        ("Anna", None, None),
    ]);
    let refused = refusal(app::add_entrants(&app, ACTOR, &id, add.clone()).await);
    assert_eq!(
        refused,
        Refusal::Rows {
            origin: Origin::List,
            rows: [0, 1]
                .map(|row| RowProblem {
                    row,
                    problem: RowError::BlockedName,
                    shown_as: None,
                })
                .to_vec(),
        },
        "refused without the approval"
    );
    assert!(
        refused.to_string().contains(
            "row 1: the name has a word the blocklist refuses; approve it if it is their real name"
        ),
        "{refused}"
    );

    // Approval lifts the blocklist, not the other rules.
    let mut sneaky = add.clone();
    sneaky.rows[0].name = "Nazir\u{200B}".into();
    sneaky.rows[0].approved = true;
    sneaky.rows[1].approved = true;
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, sneaky).await),
        Refusal::Rows {
            origin: Origin::List,
            rows: vec![RowProblem {
                row: 0,
                problem: RowError::Name,
                shown_as: None,
            }],
        },
        "nothing invisible, approved or not"
    );
    assert!(names(&app, &id).await.is_empty());

    for row in &mut add.rows {
        row.approved = true;
    }
    add.name_format = Some(rafi_api::NameFormat::Full);
    let batch = add.batch_id.clone();
    app::add_entrants(&app, ACTOR, &id, add).await.unwrap();
    assert_eq!(
        names(&app, &id).await,
        ["Nazir Ahmed", "Gábor Buzinkay", "Anna"]
    );
    assert_eq!(
        audited(&app, &id, "add").await.last().unwrap(),
        &format!("batch={batch} origin=list added=3 skipped=0 approved=2 lifted=0 format=full"),
        "an approval of a name the blocklist takes approves nothing"
    );

    // The one-person form takes an approval too.
    let mut nazir = one("Nazir Ali", None);
    assert_eq!(
        refusal(app::add_entrants(&app, ACTOR, &id, nazir.clone()).await).to_string(),
        "The name has a word the blocklist refuses; approve it if it is their real name."
    );
    nazir.rows[0].approved = true;
    app::add_entrants(&app, ACTOR, &id, nazir).await.unwrap();
    assert!(
        audited(&app, &id, "add")
            .await
            .last()
            .unwrap()
            .ends_with(" origin=one added=1 skipped=0 approved=1 lifted=0"),
        "no format for one person"
    );
}

/// The add's audit row names the format the browser made the names in,
/// when it says; never a name.
#[tokio::test]
async fn the_name_format_is_audited() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let mut add = list(&[("Anna K.", None, None), ("Bence N.", None, None)]);
    add.name_format = Some(rafi_api::NameFormat::FirstInitial);
    let batch = add.batch_id.clone();
    app::add_entrants(&app, ACTOR, &id, add).await.unwrap();
    assert_eq!(
        audited(&app, &id, "add").await,
        [format!(
            "batch={batch} origin=list added=2 skipped=0 approved=0 lifted=0 format=first_initial"
        )]
    );
}

/// The entrant of raffle `id` who is `name`, as the admin page has them.
async fn admin_entrant(app: &App, id: &str, name: &str) -> rafi_api::AdminEntrant {
    app::admin(app, OWNER, id)
        .await
        .unwrap()
        .entrants
        .into_iter()
        .find(|e| e.display_name == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

/// Someone removed at their request before the freeze stays out of later
/// adds, by email key or registration ID, the one-person form's too, until
/// the organizer lifts it for their row; the removal is forgotten with the
/// freeze. A plain removal remembers nothing.
#[tokio::test]
async fn removed_at_their_request_they_stay_out_until_lifted() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    app::add_entrants(
        &app,
        ACTOR,
        &id,
        list(&[
            ("Anna", Some("anna@x.org"), Some("1")),
            ("Bea", Some("bea@x.org"), None),
            ("Cili", None, Some("3")),
        ]),
    )
    .await
    .unwrap();
    let (anna, bea, cili) = (
        admin_entrant(&app, &id, "Anna").await.id,
        admin_entrant(&app, &id, "Bea").await.id,
        admin_entrant(&app, &id, "Cili").await.id,
    );
    app.clock.advance(1);
    app::remove_entrant(&app, ACTOR, &id, &anna, true)
        .await
        .unwrap();
    app::remove_entrant(&app, ACTOR, &id, &bea, false)
        .await
        .unwrap();
    app.clock.advance(1);
    app::remove_entrant(&app, ACTOR, &id, &cili, true)
        .await
        .unwrap();
    assert_eq!(
        audited(&app, &id, "remove_entrant").await,
        [
            format!("{anna} at_request"),
            bea.clone(),
            format!("{cili} at_request")
        ],
        "the reason, never who"
    );
    let removed = app::admin(&app, OWNER, &id).await.unwrap().removed;
    let now = app.clock.now();
    assert_eq!(
        removed,
        [
            rafi_api::RemovedAtRequest {
                email_key: Some("anna@x.org".into()),
                list_ref: Some("1".into()),
                removed_at: now - 1,
            },
            rafi_api::RemovedAtRequest {
                email_key: None,
                list_ref: Some("3".into()),
                removed_at: now,
            },
        ]
    );

    // The list again: Anna (by her email, written otherwise) and Cili (by
    // her ID) stay out; Bea, removed without asking, is back.
    let again = list(&[
        ("Anna", Some("ANNA+meetup@x.org"), None),
        ("Bea", Some("bea@x.org"), None),
        ("Cili", None, Some("3")),
    ]);
    let batch = again.batch_id.clone();
    let result = app::add_entrants(&app, ACTOR, &id, again.clone())
        .await
        .unwrap();
    assert_eq!(result.added.len(), 1);
    assert_eq!(
        result.skipped,
        [
            rafi_api::SkippedRow {
                row: 0,
                reason: SkipReason::RemovedAtRequest
            },
            rafi_api::SkippedRow {
                row: 2,
                reason: SkipReason::RemovedAtRequest
            },
        ]
    );
    assert_eq!(names(&app, &id).await, ["Bea"]);
    assert_eq!(
        app::add_entrants(&app, ACTOR, &id, again)
            .await
            .unwrap()
            .skipped,
        result.skipped,
        "a retry answers the same"
    );
    assert_eq!(
        audited(&app, &id, "add").await.last().unwrap(),
        &format!("batch={batch} origin=list added=1 skipped=2 approved=0 lifted=0")
    );
    // The one-person form too.
    let one = app::add_entrants(&app, ACTOR, &id, one("Anna K.", Some("anna@x.org")))
        .await
        .unwrap();
    assert!(one.added.is_empty());
    assert_eq!(one.skipped[0].reason, SkipReason::RemovedAtRequest);

    // Lifted for Anna's row: she is added, her removal goes, the add
    // counts it; Cili's stays.
    let mut lifting = list(&[
        ("Anna", Some("anna@x.org"), None),
        ("Cili", None, Some("3")),
    ]);
    lifting.rows[0].lift_removal = true;
    let batch = lifting.batch_id.clone();
    let result = app::add_entrants(&app, ACTOR, &id, lifting).await.unwrap();
    assert_eq!(result.added.len(), 1);
    assert_eq!(result.skipped[0].reason, SkipReason::RemovedAtRequest);
    assert_eq!(
        audited(&app, &id, "add").await.last().unwrap(),
        &format!("batch={batch} origin=list added=1 skipped=1 approved=0 lifted=1")
    );
    let removed = app::admin(&app, OWNER, &id).await.unwrap().removed;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].list_ref.as_deref(), Some("3"));
    assert!(!removed[0].matches(Some("anna@x.org"), None));

    // The freeze fixes the list for good: nobody is removed now, and the
    // removals serve nothing any more.
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let anna = admin_entrant(&app, &id, "Anna").await.id;
    assert_eq!(
        refusal(app::remove_entrant(&app, ACTOR, &id, &anna, true).await),
        Refusal::NotRemovable
    );
    assert!(
        app::admin(&app, OWNER, &id)
            .await
            .unwrap()
            .removed
            .is_empty()
    );
}

/// Waits until raffle `id` is finished.
async fn finished(app: &App, id: &str) {
    for _ in 0..200 {
        let raffle = raffles::get(&app.db, id).await.unwrap().unwrap();
        if raffle.state == rafi_lifecycle::RaffleState::Finished {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never finished");
}

/// After the freeze, someone the organizer added can take themselves off:
/// their name is hidden and stays so, the admin page offers it and its
/// undo per entrant, it is audited, and once the raffle is over
/// housekeeping erases them, not a non-winner who didn't object. Someone
/// who joined can't, and neither before the freeze.
#[tokio::test]
async fn added_people_take_themselves_off_after_the_freeze() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna"]).await;
    let anna = people[0].clone();
    join_as(&app, &id, "Jo", "jo@x.org").await;
    let jo = admin_entrant(&app, &id, "Jo").await;
    assert_eq!(
        refusal(app::object_entrant(&app, ACTOR, &id, &anna).await),
        Refusal::Objection(rafi_lifecycle::entrant::ObjectionRefusal::State),
        "before the freeze, they're removed at their request"
    );
    assert!(!admin_entrant(&app, &id, "Anna").await.actions.object);

    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let a = admin_entrant(&app, &id, "Anna").await;
    assert!(a.actions.object && !a.actions.unobject && !a.objected);
    assert!(!admin_entrant(&app, &id, "Jo").await.actions.object);
    assert_eq!(
        refusal(app::object_entrant(&app, ACTOR, &id, &jo.id).await),
        Refusal::Objection(rafi_lifecycle::entrant::ObjectionRefusal::Source),
        "a joiner accepted the rules"
    );
    app::object_entrant(&app, ACTOR, &id, &anna).await.unwrap();
    app::object_entrant(&app, ACTOR, &id, &anna)
        .await
        .expect("again: nothing to do");
    let a = admin_entrant(&app, &id, "Anna").await;
    assert!(a.objected && a.hidden && a.actions.unobject && !a.actions.object);
    let show = EntrantPatch {
        hidden: Some(false),
        ..Default::default()
    };
    assert_eq!(
        refusal(app::patch_entrant(&app, ACTOR, &id, &anna, show.clone()).await),
        Refusal::ObjectedHidden
    );
    // A mistake undone: no longer objected, still hidden until shown.
    app::unobject_entrant(&app, ACTOR, &id, &anna)
        .await
        .unwrap();
    let a = admin_entrant(&app, &id, "Anna").await;
    assert!(!a.objected && a.hidden && a.actions.object);
    app::object_entrant(&app, ACTOR, &id, &anna).await.unwrap();
    assert_eq!(
        audited(&app, &id, "object").await,
        [anna.clone(), anna.clone()]
    );
    assert_eq!(
        audited(&app, &id, "unobject").await,
        std::slice::from_ref(&anna)
    );
    let view = app::admin(&app, OWNER, &id).await.unwrap().view;
    assert_eq!(view.state, rafi_lifecycle::RaffleState::Frozen);

    // The raffle is over: housekeeping erases Anna, not Jo, whose data waits
    // for the erasure date.
    app::command(&app, ACTOR, &id, Decision::Finish, "f1")
        .await
        .unwrap();
    finished(&app, &id).await;
    assert_eq!(
        refusal(app::unobject_entrant(&app, ACTOR, &id, &anna).await),
        Refusal::Objection(rafi_lifecycle::entrant::ObjectionRefusal::State)
    );
    let report = app::housekeeping::run(&app).await.unwrap();
    assert_eq!((report.objected, report.erased), (1, 1));
    let a = admin_entrant(&app, &id, &format!("#{}", &anna[..6])).await;
    assert!(a.erased && a.objected && a.email.is_none());
    assert!(!admin_entrant(&app, &id, "Jo").await.erased);
    assert_eq!(audited(&app, &id, "auto_erase").await, ["1 objected"]);
    assert_eq!(
        app::housekeeping::run(&app).await.unwrap().objected,
        0,
        "once"
    );
}

/// A winner who took themselves off can only be declined: the server
/// refuses "Claimed" and "Not here" before the engine is asked. A winner
/// who claimed can't take themselves off: the result stands.
#[tokio::test]
async fn a_winner_who_took_themselves_off_is_declined() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Zoe"]).await;
    let zoe = people[0].clone();
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let item = app::admin(&app, OWNER, &id).await.unwrap().view.items[0]
        .item
        .clone();
    app::command(&app, ACTOR, &id, DrawRequest { item: item.clone() }, "d1")
        .await
        .unwrap();
    assert_eq!(drawn(&app, &id).await, zoe);
    app::object_entrant(&app, ACTOR, &id, &zoe).await.unwrap();
    for (resolution, command_id) in [(Resolution::Claimed, "c1"), (Resolution::NotHere, "n1")] {
        let decide = Decision::Resolve {
            k: None,
            item: item.clone(),
            resolution,
        };
        assert_eq!(
            refusal(app::command(&app, ACTOR, &id, decide, command_id).await),
            Refusal::ObjectedWinner,
            "{resolution:?}"
        );
    }
    assert!(audited(&app, &id, "resolve").await.is_empty());
    let decline = Decision::Resolve {
        k: None,
        item: item.clone(),
        resolution: Resolution::Declined,
    };
    app::command(&app, ACTOR, &id, decline, "x1").await.unwrap();
    assert_eq!(
        audited(&app, &id, "resolve").await,
        [format!("{item} declined")]
    );

    // A claimed winner's result stands.
    let (other, people) = open_raffle(&app, &["Kim"]).await;
    let kim = people[0].clone();
    app::start_draw(&app, ACTOR, &other).await.unwrap();
    let item = app::admin(&app, OWNER, &other).await.unwrap().view.items[0]
        .item
        .clone();
    app::command(
        &app,
        ACTOR,
        &other,
        DrawRequest { item: item.clone() },
        "d2",
    )
    .await
    .unwrap();
    assert_eq!(drawn(&app, &other).await, kim);
    let claim = Decision::Resolve {
        k: None,
        item,
        resolution: Resolution::Claimed,
    };
    app::command(&app, ACTOR, &other, claim, "c2")
        .await
        .unwrap();
    for _ in 0..200 {
        if !admin_entrant(&app, &other, "Kim").await.actions.object {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!admin_entrant(&app, &other, "Kim").await.actions.object);
    assert_eq!(
        refusal(app::object_entrant(&app, ACTOR, &other, &kim).await),
        Refusal::Objection(rafi_lifecycle::entrant::ObjectionRefusal::Claimed)
    );
}

/// A raffle with one prize and registration off, its organizer named or
/// not, published; returns its ID.
async fn published_without_registration(app: &App, named: bool) -> String {
    let id = app::create(app, ACTOR, "Gala", &EventDate::today(app))
        .await
        .unwrap();
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.registration = false;
    if named {
        settings.organizer_name = "Gala".into();
        settings.contact_email = "hello@example.com".into();
    }
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
            delivery: Delivery::Later,
        },
    )
    .await
    .unwrap();
    app::lifecycle(app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    id
}

/// Without registration a raffle doesn't open: it freezes from Published,
/// once the notice names a controller and a contact, with the people the
/// organizer added; the setting is fixed from then on.
#[tokio::test]
async fn without_registration_the_list_freezes_from_published() {
    let app = app().await;
    let id = published_without_registration(&app, false).await;
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(!admin.settings.registration && !admin.view.registration);
    assert_eq!(admin.actions.lifecycle, [LifecycleCommand::Unpublish]);
    assert_eq!(admin.actions.start, Some(StartStep::Freeze));
    assert!(admin.actions.edit_registration);
    assert_eq!(
        refusal(app::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open).await),
        Refusal::NoRegistration
    );
    add_one(&app, &id, "Anna", None).await.unwrap();

    // The checks opening would have made.
    let refused = refusal(app::start_draw(&app, ACTOR, &id).await);
    assert_eq!(
        refused,
        Refusal::OrganizerMissing {
            fields: vec!["the organizer's name", "a contact or privacy email"],
            registration: false,
        }
    );
    assert!(
        refused
            .to_string()
            .starts_with("Before the list can be frozen, fill in")
    );
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(raffle.state, rafi_lifecycle::RaffleState::Ready);
    let mut settings = app::settings_of(&raffle);
    settings.organizer_name = "Gala".into();
    settings.contact_email = "hello@example.com".into();
    app::update_settings(&app, ACTOR, &id, settings.clone())
        .await
        .unwrap();

    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert!(raffle.state.drawable() && raffle.manifest.is_some());
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert!(actions.lifecycle.is_empty() && !actions.edit_registration);
    settings.registration = true;
    assert_eq!(
        refusal(app::update_settings(&app, ACTOR, &id, settings).await),
        Refusal::RegistrationLocked
    );

    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? AND action = 'settings' ORDER BY id")
        .bind(&id)
        .all()
        .await
        .unwrap();
    assert_eq!(audit[0].1, "registration", "{audit:?}");
}

/// A freeze from Published that finds nobody to draw from goes back to
/// Published, where the next one starts.
#[tokio::test]
async fn a_freeze_from_published_unfreezes_to_it() {
    let app = app().await;
    let id = published_without_registration(&app, true).await;
    // The freeze's first step, everyone removed meanwhile.
    assert!(
        raffles::freeze(
            &app.db,
            &id,
            rafi_lifecycle::RaffleState::Ready,
            false,
            app.clock.now(),
        )
        .await
        .unwrap()
    );
    assert_eq!(
        refusal(app::start_draw(&app, ACTOR, &id).await),
        Refusal::Start(rafi_lifecycle::start::Refusal::NoEntrants)
    );
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(raffle.state, rafi_lifecycle::RaffleState::Ready);
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert_eq!(actions.start, Some(StartStep::Freeze));
}

/// Registration switches before the freeze, but not while it is open; a
/// raffle switched off once closed doesn't reopen, and freezes from Closed.
#[tokio::test]
async fn registration_switches_only_while_nobody_is_joining() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna"]).await;
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(
        admin.settings.registration && admin.view.registration,
        "on by default"
    );
    assert_eq!(admin.actions.lifecycle, [LifecycleCommand::Close]);
    assert!(!admin.actions.edit_registration);
    let mut off = admin.settings.clone();
    off.registration = false;
    assert_eq!(
        refusal(app::update_settings(&app, ACTOR, &id, off.clone()).await),
        Refusal::RegistrationLocked
    );

    app::lifecycle(&app, ACTOR, &id, LifecycleCommand::Close)
        .await
        .unwrap();
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert_eq!(admin.actions.lifecycle, [LifecycleCommand::Open], "reopen");
    app::update_settings(&app, ACTOR, &id, off).await.unwrap();
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(
        admin.actions.lifecycle.is_empty(),
        "no reopening: {:?}",
        admin.actions.lifecycle
    );
    assert_eq!(admin.actions.start, Some(StartStep::Freeze));
    assert_eq!(
        refusal(app::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open).await),
        Refusal::NoRegistration
    );
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    assert!(
        raffles::get(&app.db, &id)
            .await
            .unwrap()
            .unwrap()
            .state
            .drawable()
    );
}

/// A copy keeps the original's registration setting.
#[tokio::test]
async fn a_copy_keeps_registration() {
    let app = app().await;
    let id = published_without_registration(&app, true).await;
    let copy = app::duplicate(&app, ACTOR, &id, &EventDate::today(&app))
        .await
        .unwrap();
    let copy = raffles::get(&app.db, &copy).await.unwrap().unwrap();
    assert!(!copy.registration);
    assert_eq!(copy.state, rafi_lifecycle::RaffleState::Draft);
}

/// What the admin says is missing goes by the step ahead: while opening (or,
/// without registration, the freeze from Published) is to come, the
/// organizer's name and contact; nothing once it is behind, though the
/// settings still lack them.
#[tokio::test]
async fn the_admin_says_what_the_next_step_misses() {
    let app = app().await;
    let both = ["the organizer's name", "a contact or privacy email"];
    for registration in [true, false] {
        let id = app::create(&app, ACTOR, "Gala", &EventDate::today(&app))
            .await
            .unwrap();
        let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
        settings.registration = registration;
        app::update_settings(&app, ACTOR, &id, settings)
            .await
            .unwrap();
        let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
        assert_eq!(actions.missing, both, "Draft, registration {registration}");
        app::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
            .await
            .unwrap();
        let admin = app::admin(&app, OWNER, &id).await.unwrap();
        assert_eq!(admin.actions.missing, both, "Published, {registration}");

        let mut settings = admin.settings;
        settings.organizer_name = "Gala".into();
        app::update_settings(&app, ACTOR, &id, settings.clone())
            .await
            .unwrap();
        assert_eq!(
            app::admin(&app, OWNER, &id).await.unwrap().actions.missing,
            ["a contact or privacy email"]
        );
        settings.contact_email = "hello@example.com".into();
        app::update_settings(&app, ACTOR, &id, settings)
            .await
            .unwrap();
        assert!(
            app::admin(&app, OWNER, &id)
                .await
                .unwrap()
                .actions
                .missing
                .is_empty()
        );
    }

    // Opened, then the contact cleared: opening is behind and nothing blocks
    // the freeze.
    let (id, _) = open_raffle(&app, &["Anna"]).await;
    raffles_set_contact(&app, &id, "").await;
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(admin.actions.missing.is_empty(), "{:?}", admin.actions);
}

/// Clears or sets raffle `id`'s contact email directly: the settings form
/// refuses a blank one once registration is open.
async fn raffles_set_contact(app: &App, id: &str, email: &str) {
    app.db
        .query("UPDATE raffles SET contact_email = ? WHERE id = ?")
        .bind(email)
        .bind(id)
        .run()
        .await
        .unwrap();
}

/// The evening's flags: whom no phone tells they won (someone the
/// organizer added, not a joiner), on the projector's draws and in the
/// admin; whether people were added from a list (not one by one); and the
/// manifest's entrants counted by source for the verification page.
#[tokio::test]
async fn the_evening_knows_who_has_no_phone() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna"]).await;
    let anna = people[0].clone();
    let view = app::admin(&app, OWNER, &id).await.unwrap().view;
    assert!(!view.list_added, "one by one isn't a list");
    assert_eq!(view.organizer, "Meetup");
    let slug = view.slug.clone();
    assert_eq!(view.privacy_url, format!("http://localhost/privacy/{slug}"));
    app::add_entrants(
        &app,
        ACTOR,
        &id,
        list(&[("Bence", Some("bence@x.org"), None)]),
    )
    .await
    .unwrap();
    join_as(&app, &id, "Jo", "jo@x.org").await;
    let admin = app::admin(&app, OWNER, &id).await.unwrap();
    assert!(admin.view.list_added);
    for (name, no_phone) in [("Anna", true), ("Bence", true), ("Jo", false)] {
        assert_eq!(
            admin_entrant(&app, &id, name).await.no_phone,
            no_phone,
            "{name}"
        );
    }
    assert!(
        matches!(app::verify_data(&app, &slug).await, Err(Error::NotFound)),
        "nothing to count before the freeze"
    );

    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let data = app::verify_data(&app, &slug).await.unwrap();
    assert_eq!(
        data.sources,
        Some(rafi_api::SourceCounts {
            joined: 1,
            added: 2,
            fake: 0
        })
    );
    let item = app::admin(&app, OWNER, &id).await.unwrap().view.items[0]
        .item
        .clone();
    app::command(&app, ACTOR, &id, DrawRequest { item }, "d1")
        .await
        .unwrap();
    let winner = drawn(&app, &id).await;
    let view = app::admin(&app, OWNER, &id).await.unwrap().view;
    let draw = view.draws.last().unwrap();
    assert_eq!(draw.entrant.as_str(), winner);
    let jo = admin_entrant(&app, &id, "Jo").await.id;
    assert_eq!(draw.no_phone, winner != jo, "{winner} ({anna}, {jo})");
    // The phones' view says the same of the latest winner.
    let snapshot = rafi_store::live::snapshot(&app.db, &id)
        .await
        .unwrap()
        .unwrap();
    let phones = rafi_live_server::raffle::view(&snapshot);
    assert_eq!(phones.last.unwrap().no_phone, winner != jo);
}

/// In test mode the counts include the fakes, and an older reader of
/// `verify.json` (no counts) still reads it.
#[tokio::test]
async fn a_rehearsal_counts_its_fakes() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna"]).await;
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.test_mode = true;
    app::update_settings(&app, ACTOR, &id, settings)
        .await
        .unwrap();
    app::add_fakes(&app, ACTOR, &id, 3).await.unwrap();
    join_as(&app, &id, "Jo", "jo@x.org").await;
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let slug = app::admin(&app, OWNER, &id).await.unwrap().view.slug;
    let data = app::verify_data(&app, &slug).await.unwrap();
    assert!(data.test_mode);
    let sources = data.sources.unwrap();
    assert_eq!((sources.joined, sources.added, sources.fake), (1, 1, 3));
    assert_eq!(
        sources.text(),
        "5 entrants: 1 joined on their phone, 1 added by the organizer, 3 made up for the test"
    );

    // Published beside the envelope's fields, under the same format: a
    // reader that doesn't know them reads the rest as before.
    let mut json = serde_json::to_value(&data).unwrap();
    assert_eq!(
        json["sources"],
        serde_json::json!({ "joined": 1, "added": 1, "fake": 3 })
    );
    assert_eq!(json["format"], rafi_api::VERIFY_FORMAT);
    json.as_object_mut().unwrap().remove("sources");
    let older: rafi_api::VerifyData = serde_json::from_value(json).unwrap();
    assert_eq!(older.sources, None);
    assert_eq!(older.manifest, data.manifest);
}

// ---------------------------------------------------------------------------
// Interleavings: a second actor between two of a use case's statements.
// ---------------------------------------------------------------------------

/// Where [`paused_at`] holds a use case: [`Pause::reached`] returns once it
/// is about to send the statement, [`Pause::resume`] lets it go on.
struct Pause {
    reached: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
}

impl Pause {
    async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.reached.notified())
            .await
            .expect("the paused statement was never reached");
    }

    fn resume(&self) {
        self.resume.notify_one();
    }
}

/// `app` with its database paused before the first statement `at` matches
/// (by its SQL), for a use case run on it; another actor uses `app` itself,
/// which doesn't pause. Both share one database.
fn paused_at(app: &App, at: fn(&str) -> bool) -> (App, Pause) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let reached = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let fired = Arc::new(AtomicBool::new(false));
    let hook: rafi_store::Hook = {
        let (reached, resume) = (reached.clone(), resume.clone());
        Arc::new(move |sql: &str| {
            let hit = at(sql) && !fired.swap(true, Ordering::SeqCst);
            let (reached, resume) = (reached.clone(), resume.clone());
            Box::pin(async move {
                if hit {
                    reached.notify_one();
                    resume.notified().await;
                }
            })
        })
    };
    let paused = App {
        db: app.db.with_hook(hook),
        ..app.clone()
    };
    (paused, Pause { reached, resume })
}

/// The freeze decides from the prizes it reads, then locks them: a prize
/// saved by another organizer in between is in the manifest, which reads
/// them again once they are locked.
#[tokio::test]
async fn a_prize_saved_while_freezing_is_in_the_manifest() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna", "Bence"]).await;
    // Paused at the freeze's first write, after its checks.
    let (paused, pause) = paused_at(&app, |sql| {
        sql.contains("UPDATE raffles SET state") && sql.contains("registration = ?")
    });
    let freeze = tokio::spawn({
        let id = id.clone();
        async move { app::start_draw(&paused, ACTOR, &id).await }
    });
    pause.reached().await;
    app::upsert_prize(
        &app,
        ACTOR,
        &id,
        PrizeInput {
            id: None,
            name: "Mug".into(),
            sponsor: String::new(),
            description: String::new(),
            qty: 2,
            position: 1,
            delivery: Delivery::OnTheSpot,
        },
    )
    .await
    .unwrap();
    pause.resume();
    freeze.await.unwrap().unwrap();

    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let manifest: rafi_raffle::Manifest =
        serde_json::from_str(raffle.manifest.as_deref().unwrap()).unwrap();
    let names: Vec<&str> = manifest.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        ["Book", "Mug", "Mug"],
        "the prizes as they were locked"
    );
}

/// `app` with its database paused twice: before the first statement
/// `write` matches (by its SQL), then before the next statement after it,
/// for a use case run on it; another actor uses `app` itself. Around a
/// guarded write: something changed before it makes it change nothing,
/// something changed after it is what the use case's re-read finds.
fn paused_around(app: &App, write: fn(&str) -> bool) -> (App, Pause, Pause) {
    use std::sync::atomic::{AtomicU8, Ordering};

    let pauses = [(); 2].map(|()| Pause {
        reached: Arc::new(tokio::sync::Notify::new()),
        resume: Arc::new(tokio::sync::Notify::new()),
    });
    // 0: waiting for the write; 1: waiting for the statement after it; 2: done.
    let stage = Arc::new(AtomicU8::new(0));
    let hook: rafi_store::Hook = {
        let notifies = pauses
            .each_ref()
            .map(|p| (p.reached.clone(), p.resume.clone()));
        Arc::new(move |sql: &str| {
            let hit = if write(sql) {
                stage.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            } else {
                stage.compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            };
            let pause = hit.ok().map(|i| notifies[usize::from(i)].clone());
            Box::pin(async move {
                if let Some((reached, resume)) = pause {
                    reached.notify_one();
                    resume.notified().await;
                }
            })
        })
    };
    let paused = App {
        db: app.db.with_hook(hook),
        ..app.clone()
    };
    let [before, after] = pauses;
    (paused, before, after)
}

/// A raffle's settings refused by a real entrant who came in meanwhile,
/// then the raffle deleted before the refusal is explained: not found,
/// not fixed settings naming none.
#[tokio::test]
async fn settings_refused_on_a_raffle_deleted_meanwhile_are_not_found() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &[]).await;
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.rules = "Bring a mug.".into();
    let (paused, before, after) =
        paused_around(&app, |sql| sql.starts_with("UPDATE raffles SET title"));
    let saving = tokio::spawn({
        let id = id.clone();
        async move { app::update_settings(&paused, ACTOR, &id, settings).await }
    });
    before.reached().await;
    add_one(&app, &id, "Anna", None).await.unwrap();
    before.resume();
    after.reached().await;
    app::delete(&app, ACTOR, &id).await.unwrap();
    after.resume();
    let err = saving.await.unwrap().unwrap_err();
    assert!(matches!(err, Error::NotFound), "{err:?}");
}

/// An entrant shown again after they took themselves off meanwhile, then
/// gone with their raffle before the refusal is explained: not found, not
/// erased.
#[tokio::test]
async fn a_patch_of_an_entrant_gone_meanwhile_is_not_found() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna", "Bence"]).await;
    let anna = people[0].clone();
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let (paused, before, after) =
        paused_around(&app, |sql| sql.contains("display_name = COALESCE"));
    let patching = tokio::spawn({
        let (id, anna) = (id.clone(), anna.clone());
        let show = EntrantPatch {
            hidden: Some(false),
            ..Default::default()
        };
        async move { app::patch_entrant(&paused, ACTOR, &id, &anna, show).await }
    });
    before.reached().await;
    app::object_entrant(&app, ACTOR, &id, &anna).await.unwrap();
    before.resume();
    after.reached().await;
    app::delete(&app, ACTOR, &id).await.unwrap();
    after.resume();
    let err = patching.await.unwrap().unwrap_err();
    assert!(matches!(err, Error::NotFound), "{err:?}");
}

/// An objection undone by someone else meanwhile, then the raffle deleted
/// before the refusal is explained: not found, not the objection refused
/// for the raffle's state.
#[tokio::test]
async fn an_unobject_on_a_raffle_deleted_meanwhile_is_not_found() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna", "Bence"]).await;
    let anna = people[0].clone();
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    app::object_entrant(&app, ACTOR, &id, &anna).await.unwrap();
    let (paused, before, after) = paused_around(&app, |sql| sql.contains("SET objected_at = NULL"));
    let undoing = tokio::spawn({
        let (id, anna) = (id.clone(), anna.clone());
        async move { app::unobject_entrant(&paused, ACTOR, &id, &anna).await }
    });
    before.reached().await;
    app::unobject_entrant(&app, ACTOR, &id, &anna)
        .await
        .unwrap();
    before.resume();
    after.reached().await;
    app::delete(&app, ACTOR, &id).await.unwrap();
    after.resume();
    let err = undoing.await.unwrap().unwrap_err();
    assert!(matches!(err, Error::NotFound), "{err:?}");
}

/// An objection undone by someone else meanwhile, then the entrant erased
/// (the erasure date has passed) before the refusal is explained: erased,
/// not "it changed".
#[tokio::test]
async fn an_unobject_of_an_entrant_erased_meanwhile_is_erased() {
    let app = app().await;
    let (id, people) = open_raffle(&app, &["Anna", "Bence"]).await;
    let anna = people[0].clone();
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    app::object_entrant(&app, ACTOR, &id, &anna).await.unwrap();
    // Past the erasure date: a frozen raffle's entrants can be erased.
    app.clock.advance(200 * 86_400);
    let (paused, before, after) = paused_around(&app, |sql| sql.contains("SET objected_at = NULL"));
    let undoing = tokio::spawn({
        let (id, anna) = (id.clone(), anna.clone());
        async move { app::unobject_entrant(&paused, ACTOR, &id, &anna).await }
    });
    before.reached().await;
    app::unobject_entrant(&app, ACTOR, &id, &anna)
        .await
        .unwrap();
    before.resume();
    after.reached().await;
    app::remove_entrant(&app, ACTOR, &id, &anna, false)
        .await
        .unwrap();
    assert!(
        entrants::get(&app.db, &id, &anna)
            .await
            .unwrap()
            .unwrap()
            .erased
    );
    after.resume();
    assert_eq!(refusal(undoing.await.unwrap()), Refusal::Erased);
}

/// The raffle engine, with `gate` awaited before each decision it passes
/// on: a pause point between a use case's reads and its call to the engine.
struct Gated {
    inner: Arc<dyn rafi_engine::Engine>,
    gate: Gate,
}

/// What [`Gated`] awaits before passing a decision on.
type Gate = Arc<dyn Fn(&Decision) -> rafi_engine::BoxFuture<'static, ()> + Send + Sync>;

impl rafi_engine::Engine for Gated {
    fn info(
        &self,
    ) -> rafi_engine::BoxFuture<'_, Result<rafi_engine::EngineInfo, rafi_engine::EngineError>> {
        self.inner.info()
    }

    fn open<'a>(
        &'a self,
        manifest: &'a rafi_raffle::Manifest,
    ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
        self.inner.open(manifest)
    }

    fn draw<'a>(
        &'a self,
        raffle_id: &'a rafi_raffle::RaffleId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
        self.inner.draw(raffle_id, request, command_id)
    }

    fn decide<'a>(
        &'a self,
        raffle_id: &'a rafi_raffle::RaffleId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
        Box::pin(async move {
            (self.gate)(decision).await;
            self.inner.decide(raffle_id, decision, command_id).await
        })
    }
}

/// `app` with its engine paused before the first `Finish` it is sent.
fn paused_before_finish(app: &App) -> (App, Pause) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let reached = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let fired = Arc::new(AtomicBool::new(false));
    let gate = {
        let (reached, resume) = (reached.clone(), resume.clone());
        Arc::new(move |decision: &Decision| {
            let hit = *decision == Decision::Finish && !fired.swap(true, Ordering::SeqCst);
            let (reached, resume) = (reached.clone(), resume.clone());
            Box::pin(async move {
                if hit {
                    reached.notify_one();
                    resume.notified().await;
                }
            }) as rafi_engine::BoxFuture<'static, ()>
        })
    };
    let engine = Gated {
        inner: app.engine.clone().expect("an engine"),
        gate,
    };
    let paused = App {
        engine: Some(Arc::new(engine)),
        ..app.clone()
    };
    (paused, Pause { reached, resume })
}

/// Deleting a frozen raffle checks that nothing was drawn, then asks the
/// engine to forget it. Another organizer's draw in between keeps the
/// raffle: its record (the announced draw) is what makes it verifiable.
#[tokio::test]
async fn a_draw_announced_while_deleting_keeps_the_raffle() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna", "Bence"]).await;
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    let item = app::admin(&app, OWNER, &id).await.unwrap().view.items[0]
        .item
        .clone();

    let (paused, pause) = paused_before_finish(&app);
    let deleting = tokio::spawn({
        let id = id.clone();
        async move { app::delete(&paused, ACTOR, &id).await }
    });
    pause.reached().await;
    app::command(&app, ACTOR, &id, DrawRequest { item }, "d1")
        .await
        .unwrap();
    drawn(&app, &id).await;
    pause.resume();
    let err = deleting.await.unwrap().unwrap_err();
    assert!(matches!(err, Error::Refused(Refusal::Drawn)), "{err:?}");

    assert!(
        raffles::get(&app.db, &id).await.unwrap().is_some(),
        "the raffle stays"
    );
    let (announced,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM raffle_events WHERE raffle_id = ? AND type = 'announced'")
        .bind(&id)
        .one()
        .await
        .unwrap();
    assert_eq!(announced, 1, "with its log");
}

/// The engine refusing the finish (a draw it holds, not yet stored) is a
/// refusal, not an outage: nothing is deleted.
#[tokio::test]
async fn an_engine_refusing_the_finish_keeps_the_raffle() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna", "Bence"]).await;
    app::start_draw(&app, ACTOR, &id).await.unwrap();
    struct Refusing(Arc<dyn rafi_engine::Engine>);
    impl rafi_engine::Engine for Refusing {
        fn info(
            &self,
        ) -> rafi_engine::BoxFuture<'_, Result<rafi_engine::EngineInfo, rafi_engine::EngineError>>
        {
            self.0.info()
        }
        fn open<'a>(
            &'a self,
            manifest: &'a rafi_raffle::Manifest,
        ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
            self.0.open(manifest)
        }
        fn draw<'a>(
            &'a self,
            raffle_id: &'a rafi_raffle::RaffleId,
            request: &'a DrawRequest,
            command_id: &'a str,
        ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
            self.0.draw(raffle_id, request, command_id)
        }
        fn decide<'a>(
            &'a self,
            _: &'a rafi_raffle::RaffleId,
            _: &'a Decision,
            _: &'a str,
        ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
            Box::pin(async {
                Err(rafi_engine::EngineError::rejected(
                    &rafi_raffle::Error::DrawInProgress,
                ))
            })
        }
    }
    let refused = App {
        engine: Some(Arc::new(Refusing(app.engine.clone().unwrap()))),
        ..app.clone()
    };
    let err = app::delete(&refused, ACTOR, &id).await.unwrap_err();
    assert!(matches!(err, Error::Refused(Refusal::Drawn)), "{err:?}");
    assert!(raffles::get(&app.db, &id).await.unwrap().is_some());

    // Without a draw, the engine forgets it and the raffle goes.
    app::delete(&app, ACTOR, &id).await.unwrap();
    assert!(raffles::get(&app.db, &id).await.unwrap().is_none());
}

/// An engine that refuses every finish, as one holding a draw would, and
/// records which raffles it was asked to finish.
struct RefusesFinish {
    inner: Arc<dyn rafi_engine::Engine>,
    asked: std::sync::Mutex<Vec<String>>,
}

impl rafi_engine::Engine for RefusesFinish {
    fn info(
        &self,
    ) -> rafi_engine::BoxFuture<'_, Result<rafi_engine::EngineInfo, rafi_engine::EngineError>> {
        self.inner.info()
    }

    fn open<'a>(
        &'a self,
        manifest: &'a rafi_raffle::Manifest,
    ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
        self.inner.open(manifest)
    }

    fn draw<'a>(
        &'a self,
        raffle_id: &'a rafi_raffle::RaffleId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
        self.inner.draw(raffle_id, request, command_id)
    }

    fn decide<'a>(
        &'a self,
        raffle_id: &'a rafi_raffle::RaffleId,
        _: &'a Decision,
        _: &'a str,
    ) -> rafi_engine::BoxFuture<'a, Result<(), rafi_engine::EngineError>> {
        self.asked
            .lock()
            .unwrap()
            .push(raffle_id.as_str().to_owned());
        Box::pin(async {
            Err(rafi_engine::EngineError::rejected(
                &rafi_raffle::Error::DrawInProgress,
            ))
        })
    }
}

/// Raffles never finished, past the ceiling: a run asks the engine about a
/// few, and the next starts after the last it asked about, so the ones the
/// engine keeps refusing don't keep the rest waiting.
#[tokio::test]
async fn ceiling_finishes_take_turns() {
    let app = app().await;
    let mut ids = Vec::new();
    for _ in 0..7 {
        let (id, _) = open_raffle(&app, &["Anna"]).await;
        app::start_draw(&app, ACTOR, &id).await.unwrap();
        ids.push(id);
    }
    ids.sort();
    let engine = Arc::new(RefusesFinish {
        inner: app.engine.clone().unwrap(),
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let refusing = App {
        engine: Some(engine.clone()),
        ..app.clone()
    };
    // Past the erasure date and the winner hold.
    app.clock.advance(200 * 86_400);

    app::housekeeping::run(&refusing).await.unwrap();
    let first: Vec<String> = std::mem::take(&mut *engine.asked.lock().unwrap());
    assert_eq!(first, ids[..5], "five per run, in ID order");
    app::housekeeping::run(&refusing).await.unwrap();
    let second: Vec<String> = std::mem::take(&mut *engine.asked.lock().unwrap());
    assert_eq!(
        second[..2],
        ids[5..],
        "the next run starts with the two left"
    );
    assert_eq!(second[2..], ids[..3], "then around again");
}

/// The tiers are the use cases' to check: a member can't create a raffle, a
/// manager can't remove the owner's data; an admin can do both.
#[tokio::test]
async fn the_tiers_are_checked_by_the_use_cases() {
    use rafi_api::Access;
    let app = app().await;
    let member = ACTOR.with_access(Access::Member);
    let manager = ACTOR.with_access(Access::Manager);
    let refused = app::create(&app, member, "Meetup", &EventDate::today(&app)).await;
    assert!(
        matches!(
            refused,
            Err(Error::Refused(Refusal::Forbidden(Access::Manager)))
        ),
        "{refused:?}"
    );
    app::create(&app, manager, "Meetup", &EventDate::today(&app))
        .await
        .unwrap();
    let refused = app::remove_owner_data(&app, manager).await;
    assert!(
        matches!(
            refused,
            Err(Error::Refused(Refusal::Forbidden(Access::Admin)))
        ),
        "{refused:?}"
    );
    app::remove_owner_data(&app, ACTOR).await.unwrap();
}
