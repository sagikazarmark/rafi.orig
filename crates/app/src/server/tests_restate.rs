//! The app and the separate `restate-rafi` service, end to end: the app calls
//! the `Raffle` object through a real `restate-server`'s ingress, the object
//! writes every state into the app's SQLite file, or Postgres schema with
//! `RAFI_TEST_POSTGRES_URL` set (through its own connection pool, as the
//! separate process does), and the screens' view shows it.
//!
//! Ignored: run with `RESTATE_SERVER_BIN` set and `-- --ignored`.

#![cfg(unix)]

use std::time::Duration;

use rafi_core::state::{Outcome, UnitStatus};
use rafi_drand::{DrandClient, DrandConfig};
use rafi_platform::Polling;
use rafi_store::{Db, DrawWriter, PurgeAfter, RaffleRow, raffles};
use restate_e2e_harness::{ReusePolicy, ServerSpec, launcher_or_skip};
use restate_rafi::Raffle;
use restate_sdk::endpoint::Endpoint;

use super::{
    AppState,
    config::{Config, RestateConfig},
    engine::{Command, RestateClient},
    service, view,
};
use crate::model::{ItemInput, Lifecycle};

const OWNER: &str = "user_restate";
const SERVER: ServerSpec = ServerSpec {
    name: "rafi-app",
    features: &[],
    env: &[],
};

async fn wait_for<F: Fn(&RaffleRow) -> bool>(app: &AppState, id: &str, f: F) -> RaffleRow {
    for _ in 0..300 {
        let r = raffles::get(&app.db, id).await.unwrap().unwrap();
        if f(&r) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not met in time");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs RESTATE_SERVER_BIN"]
async fn app_and_restate_rafi_share_the_database() {
    let Some(launcher) = launcher_or_skip(ReusePolicy::Never) else {
        return;
    };
    let restate = launcher.launch(&SERVER).await;

    // A real database file, opened by the app and, separately, by the service;
    // with `RAFI_TEST_POSTGRES_URL` set, a fresh schema there.
    let dir = std::env::temp_dir().join(format!("rafi-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = match rafi_store::testing::postgres_url().await {
        Some(url) => url,
        None => format!("sqlite://{}", dir.join("rafi.db").display()),
    };
    let mut cfg = Config::for_tests();
    cfg.database_url = url.clone();

    let drand = DrandClient::new(DrandConfig {
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
        ..DrandConfig::default()
    })
    .unwrap();
    // As the native endpoint opens it: lazily, without creating it.
    let store = DrawWriter::new(Db::connect_lazy(&url).unwrap(), PurgeAfter::days(30, 90));
    restate
        .deploy(
            Endpoint::builder()
                .bind(Raffle::new(drand, store, Polling::default()))
                .build(),
        )
        .await;
    let client = RestateClient::new(RestateConfig {
        ingress_url: restate.ingress_url().trim_end_matches('/').to_owned(),
        api_key: String::new(),
    });
    assert!(client.healthy().await, "Restate ingress health endpoint");
    cfg.restate = RestateConfig {
        ingress_url: restate.ingress_url().trim_end_matches('/').to_owned(),
        api_key: String::new(),
    };
    let app = AppState::new(cfg).await.unwrap();

    let id = service::create(&app, OWNER, "Shared database")
        .await
        .unwrap();
    service::upsert_item(
        &app,
        OWNER,
        &id,
        ItemInput {
            name: "Ticket".into(),
            qty: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::lifecycle(&app, OWNER, &id, Lifecycle::Publish)
        .await
        .unwrap();
    service::lifecycle(&app, OWNER, &id, Lifecycle::Open)
        .await
        .unwrap();
    for i in 0..3 {
        service::add_manual(&app, OWNER, &id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().engine,
        "restate"
    );

    // Freeze: the service writes the frozen state.
    service::start_draw(&app, OWNER, &id)
        .await
        .expect("freeze via restate-rafi");
    let r = wait_for(&app, &id, |r| r.draw_state.is_some()).await;
    let unit = r.draw_state().unwrap().units[0].unit_id.clone();

    service::command(
        &app,
        OWNER,
        &id,
        Command::Draw {
            unit_id: unit.clone(),
            offline: false,
        },
        "cmd-1",
    )
    .await
    .unwrap();
    wait_for(&app, &id, |r| {
        r.draw_state()
            .is_some_and(|s| s.unit(&unit).unwrap().status == UnitStatus::Drawn)
    })
    .await;

    // The screens' view shows the service's write, versioned by the database.
    let view = view::build(&app, &id).await.unwrap().expect("a view");
    assert!(
        view.draws
            .iter()
            .any(|d| d.display_name.starts_with("Person")),
        "the winner's name is on the screens"
    );
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(view.v, r.version as u64);

    service::command(
        &app,
        OWNER,
        &id,
        Command::Resolve {
            unit_id: unit.clone(),
            outcome: Outcome::Claimed,
        },
        "cmd-2",
    )
    .await
    .unwrap();
    service::command(&app, OWNER, &id, Command::Finish, "cmd-3")
        .await
        .unwrap();
    wait_for(&app, &id, |r| r.state == "finished").await;
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert!(
        r.purge_nonwinners_at.is_some(),
        "erasure is scheduled once finished"
    );

    // The object forgot everything; the app keeps the verifiable record.
    assert_eq!(app.restate.as_ref().unwrap().get(&id).await.unwrap(), None);
    let data = service::verify_data(&app, &r.slug).await.unwrap();
    let report = rafi_core::verify::verify(
        &data.commit_doc,
        &data.list_hash,
        &data.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), 1);

    std::fs::remove_dir_all(&dir).ok();
}
