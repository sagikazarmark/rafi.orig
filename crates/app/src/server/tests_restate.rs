//! The app and the separate `restate-rafi` service, end to end: the app calls
//! the `Raffle` object through a real `restate-server`'s ingress, the object
//! writes every state into the app's SQLite file (through its own connection
//! pool, as the separate process does), and the app's watcher pushes it to the
//! screens.
//!
//! Ignored: run with `RESTATE_SERVER_BIN` set and `-- --ignored`.

#![cfg(unix)]

use std::time::Duration;

use rafi_core::state::{Outcome, UnitStatus};
use rafi_drand::{DrandClient, DrandConfig};
use restate_e2e_harness::{ReusePolicy, ServerSpec, launcher_or_skip};
use restate_rafi::{DatabaseConfig, Raffle, SqliteProjector};
use restate_sdk::endpoint::Endpoint;

use super::{
    AppState,
    config::{Config, EngineKind, RestateConfig},
    db,
    engine::{Command, RestateClient},
    service, watch,
};
use crate::model::{ItemInput, Lifecycle};

const OWNER: &str = "user_restate";
const SERVER: ServerSpec = ServerSpec {
    name: "rafi-app",
    features: &[],
    env: &[],
};

async fn wait_for<F: Fn(&db::RaffleRow) -> bool>(app: &AppState, id: &str, f: F) -> db::RaffleRow {
    for _ in 0..300 {
        let r = db::raffle(&app.db, id).await.unwrap().unwrap();
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

    // A real database file, opened by the app and, separately, by the service.
    let dir = std::env::temp_dir().join(format!("rafi-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}", dir.join("rafi.db").display());
    let mut cfg = Config::for_tests();
    cfg.database_url = url.clone();
    cfg.engine = EngineKind::Restate;
    let app = AppState::new(cfg).await.unwrap();

    let drand = DrandClient::new(DrandConfig {
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
        ..DrandConfig::default()
    })
    .unwrap();
    let projector = SqliteProjector::new(&DatabaseConfig { url }).unwrap();
    restate
        .deploy(
            Endpoint::builder()
                .bind(Raffle::new(drand, projector))
                .build(),
        )
        .await;
    let client = RestateClient::new(RestateConfig {
        ingress_url: restate.ingress_url().trim_end_matches('/').to_owned(),
        api_key: String::new(),
    });
    assert!(client.healthy().await, "Restate ingress health endpoint");
    app.restate.set(client).ok().expect("client set once");
    let mut watcher = watch::Watcher::default();

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
        db::raffle(&app.db, &id).await.unwrap().unwrap().engine,
        "restate"
    );

    // Freeze: the service writes the frozen state.
    service::start_draw(&app, OWNER, &id)
        .await
        .expect("freeze via restate-rafi");
    let r = wait_for(&app, &id, |r| r.draw_state.is_some()).await;
    let unit = r.draw_state().unwrap().units[0].unit_id.clone();
    watcher.poll(&app).await.unwrap();

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

    // The watcher notices the service's write and pushes it to the screens.
    assert_eq!(watcher.poll(&app).await.unwrap(), std::slice::from_ref(&id));
    assert!(
        watcher.poll(&app).await.unwrap().is_empty(),
        "nothing new since"
    );
    let view = app.hub.current(&id).expect("a view was published");
    assert!(
        serde_json::to_string(&*view).unwrap().contains("Person"),
        "the winner's name is on the screens"
    );

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
    watcher.poll(&app).await.unwrap();
    let r = db::raffle(&app.db, &id).await.unwrap().unwrap();
    assert!(
        r.purge_nonwinners_at.is_some(),
        "erasure is scheduled once finished"
    );

    // The object forgot everything; the app keeps the verifiable record.
    assert_eq!(app.restate.get().unwrap().get(&id).await.unwrap(), None);
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
