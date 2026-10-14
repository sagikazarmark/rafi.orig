//! The use cases on their own: in-memory SQLite, the in-process raffle
//! engine, mock drand, and no HTTP.

use std::{sync::Arc, time::Duration};

use rafi_app_core::{self as app, App, Error, Limiter, Policy, ProofOfWork, Refusal, Retention};
use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_engine::local::LocalEngine;
use rafi_live_server::{LiveUpdates, Polling};
use rafi_model::{EntrantPatch, LifecycleCommand, PrizeInput, StartStep};
use rafi_raffle::{Decision, DrawRequest, ItemStatus, Resolution};
use rafi_store::{Db, RaffleWriter, entrants, raffles};

const OWNER: &str = "organizer";

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
    let engine = LocalEngine::new(drand, RaffleWriter::new(db.clone()), Polling);
    App {
        db,
        chain,
        engine: Some(Arc::new(engine)),
        live_updates: LiveUpdates::polling(),
        limiter: Arc::new(Limiter::default()),
        policy: Policy {
            public_url: "http://localhost".into(),
            max_entrants: 100,
            proof_of_work: ProofOfWork {
                key: b"test-key".to_vec(),
                max: 1000,
            },
            retention: Retention {
                enabled: true,
                nonwinners_days: 30,
                winners_days: 90,
            },
        },
    }
}

/// An open raffle with one prize and `people` entered by hand; returns its
/// ID and theirs.
async fn open_raffle(app: &App, people: &[&str]) -> (String, Vec<String>) {
    let id = app::create(app, OWNER, "Meetup").await.unwrap();
    app::upsert_prize(
        app,
        OWNER,
        &id,
        PrizeInput {
            id: None,
            name: "Book".into(),
            sponsor: String::new(),
            description: String::new(),
            qty: 1,
            position: 0,
        },
    )
    .await
    .unwrap();
    app::lifecycle(app, OWNER, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    app::lifecycle(app, OWNER, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for name in people {
        ids.push(app::add_manual(app, OWNER, &id, name, None).await.unwrap());
    }
    (id, ids)
}

/// Waits until the raffle's one item is drawn; returns the winner.
async fn drawn(app: &App, id: &str) -> String {
    for _ in 0..200 {
        let raffle = raffles::get(&app.db, id).await.unwrap().unwrap();
        if let Some(winner) = raffle.drawn().unwrap().and_then(|d| {
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
    let err = app::patch_entrant(&app, OWNER, &id, &people[0], patch)
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
    app::start_draw(&app, OWNER, &id).await.unwrap();
    let unknown = DrawRequest {
        item: "nope".into(),
    };
    assert!(matches!(
        app::command(&app, OWNER, &id, unknown, "d0").await,
        Err(Error::NotFound)
    ));
    let item = {
        let admin = app::admin(&app, OWNER, &id).await.unwrap();
        admin.view.items[0].item.clone()
    };
    app::command(&app, OWNER, &id, DrawRequest { item: item.clone() }, "d1")
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
            app::patch_entrant(&app, OWNER, &id, &winner, deliver()).await,
            Err(Error::Refused(Refusal::NotDeliverable))
        ),
        "drawn, but not claimed yet"
    );
    app::command(
        &app,
        OWNER,
        &id,
        Decision::Resolve {
            item,
            resolution: Resolution::Claimed,
        },
        "c1",
    )
    .await
    .unwrap();
    app::patch_entrant(&app, OWNER, &id, &winner, deliver())
        .await
        .unwrap();
    assert!(matches!(
        app::patch_entrant(&app, OWNER, &id, &loser, deliver()).await,
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
async fn the_admin_page_offers_what_the_rules_allow() {
    let app = app().await;
    let (id, _) = open_raffle(&app, &["Anna"]).await;
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert_eq!(actions.start, Some(StartStep::Freeze));
    assert!(!actions.draw && !actions.export && !actions.purge);

    // A freeze that stopped before its manifest was written.
    assert!(
        raffles::freeze(&app.db, &id, rafi_lifecycle::RaffleState::Open)
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

    app::start_draw(&app, OWNER, &id).await.unwrap();
    let actions = app::admin(&app, OWNER, &id).await.unwrap().actions;
    assert_eq!(actions.start, Some(StartStep::Started));

    app::command(&app, OWNER, &id, Decision::Finish, "f1")
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
                marketing: false,
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
    rafi_altcha::solve(&app::altcha::challenge(app))
        .expect("solvable")
        .to_payload()
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
        marketing: false,
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
            marketing: false,
            proof: &proof,
        };
        app::join(&app, &raffle, &req).await.unwrap();
    }
    let proof = solved(&app);
    let req = app::JoinRequest {
        name: "Anna",
        email: "ANNA@x.org",
        rules: true,
        marketing: false,
        proof: &proof,
    };
    assert_eq!(
        app::join(&app, &raffle, &req).await,
        Err(app::JoinError::Limited)
    );
}
