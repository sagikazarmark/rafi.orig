//! The app and the separate `restate-rafi` service, end to end: the app calls
//! the `Raffle` object through a real `restate-server`'s ingress, the object
//! writes every state into the app's SQLite file, or Postgres schema with
//! `RAFI_TEST_POSTGRES_URL` set (through its own connection pool, as the
//! separate process does), and the screens' view shows it.
//!
//! Ignored: run with `RESTATE_SERVER_BIN` set and `-- --ignored`.

#![cfg(unix)]

use std::time::Duration;

use rafi_drand::{DrandClient, DrandConfig};
use rafi_engine::RaffleEngine;
use rafi_engine_restate::RestateClient;
use rafi_lifecycle::RaffleState;
use rafi_platform::Polling;
use rafi_raffle::{Decision, DrawRequest};
use rafi_raffle::{ItemStatus, Resolution};
use rafi_store::{Db, DrawWriter, RaffleRow, raffles};
use restate_e2e_harness::{Call, ReusePolicy, ServerSpec, launcher_or_skip};
use restate_sdk::endpoint::Endpoint;

use super::{
    AppState,
    config::{Config, RestateConfig},
    service, view,
};
use crate::model::{Lifecycle, PrizeInput};

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
    let store = DrawWriter::new(Db::connect_lazy(&url).unwrap());
    let chain = drand.chain_hash();
    // As the native endpoint binds them.
    let (drawing, info) = rafi_engine_restate::services(
        drand,
        store,
        rafi_platform::Notify::new(Polling::default(), rafi_raffle::RaffleId::as_str),
    );
    restate
        .deploy(Endpoint::builder().bind(drawing).bind(info).build())
        .await;
    cfg.restate = RestateConfig {
        ingress_url: restate.ingress_url().to_owned(),
        api_key: String::new(),
        timeout_secs: RestateConfig::DEFAULT_TIMEOUT_SECS,
    };
    let client = RestateClient::new(cfg.restate.clone());
    let info = client.info().await.expect("the engine's info");
    assert_eq!(
        info.beacon,
        rafi_raffle::BeaconSource::Drand(chain),
        "the engine reports its beacon source"
    );
    let app = AppState::new(cfg).await.unwrap();

    let id = service::create(&app, OWNER, "Shared database")
        .await
        .unwrap();
    service::upsert_prize(
        &app,
        OWNER,
        &id,
        PrizeInput {
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
    // Freeze: the service writes the frozen state.
    service::start_draw(&app, OWNER, &id)
        .await
        .expect("freeze via restate-rafi");
    let r = wait_for(&app, &id, |r| r.drawing.is_some()).await;
    let item = r.drawing().unwrap().unwrap().items()[0].id.clone();

    service::command(
        &app,
        OWNER,
        &id,
        DrawRequest { item: item.clone() },
        "cmd-1",
    )
    .await
    .unwrap();
    wait_for(&app, &id, |r| {
        r.drawing()
            .unwrap()
            .is_some_and(|s| s.item(&item).unwrap().status() == ItemStatus::Drawn)
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
        Decision::Resolve {
            item: item.clone(),
            resolution: Resolution::Claimed,
        },
        "cmd-2",
    )
    .await
    .unwrap();
    service::command(&app, OWNER, &id, Decision::Finish, "cmd-3")
        .await
        .unwrap();
    wait_for(&app, &id, |r| r.state == RaffleState::Finished).await;
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert!(r.finished_at.is_some(), "the finish is timed");

    // The object forgot everything; the app keeps the verifiable record.
    let state = restate
        .invoke(
            &Call::object(rafi_engine_restate::SERVICE_NAME, &id, "get"),
            None,
            None,
        )
        .await;
    assert_eq!(state.status, 200, "{}", state.body);
    assert!(
        state.body.is_null(),
        "the object's state is cleared: {}",
        state.body
    );
    let data = service::verify_data(&app, &r.slug).await.unwrap();
    let report = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), 1);

    std::fs::remove_dir_all(&dir).ok();
}
