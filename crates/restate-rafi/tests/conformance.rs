//! The engine conformance suite on the `Raffle` object: each profile deploys
//! the service to its own `restate-server`, and the scenarios drive it through
//! `restate-rafi-client`, as the app does.
//!
//! Ignored: run with `RESTATE_SERVER_BIN` set and `-- --ignored`.

#![cfg(unix)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rafi_engine_local::conformance::{self, Deployed, Deployment, EngineHarness};
use rafi_platform::{BoxFuture, DrawingEngine};
use restate_e2e_harness::{Restate, ReusePolicy, ServerSpec, launcher_or_skip};
use restate_rafi::{INFO_SERVICE_NAME, Raffle, RaffleEngine, SERVICE_NAME};
use restate_rafi_client::{ClientConfig, RestateClient};
use restate_sdk::endpoint::Endpoint;

struct OnRestate {
    servers: AtomicUsize,
}

struct Running {
    restate: Restate,
}

impl Deployed for Running {
    fn engine(&self, deadline: Option<Duration>) -> Arc<dyn DrawingEngine> {
        Arc::new(RestateClient::new(ClientConfig {
            ingress_url: self.restate.ingress_url().to_owned(),
            api_key: String::new(),
            timeout_secs: deadline
                .map(|d| d.as_secs().max(1))
                .unwrap_or(ClientConfig::DEFAULT_TIMEOUT_SECS),
        }))
    }
}

impl EngineHarness for OnRestate {
    fn name(&self) -> &str {
        "the Raffle object on restate-server"
    }

    fn deploy(&self, d: Deployment) -> BoxFuture<'_, Result<Arc<dyn Deployed>, String>> {
        Box::pin(async move {
            let launcher = launcher_or_skip(ReusePolicy::Never).ok_or("no RESTATE_SERVER_BIN")?;
            // A server per profile: a second deployment of the same services on
            // one server would take over its invocations.
            let n = self.servers.fetch_add(1, Ordering::Relaxed);
            let name: &'static str = Box::leak(format!("conformance-{n}").into_boxed_str());
            let spec: &'static ServerSpec = Box::leak(Box::new(ServerSpec {
                name,
                features: &[],
                env: &[],
            }));
            let restate = launcher.launch(spec).await;
            restate
                .deploy(
                    Endpoint::builder()
                        .bind(RaffleEngine::new(&d.drand))
                        .bind(Raffle::new(d.drand, d.store, d.notifier))
                        .build(),
                )
                .await;
            Ok(Arc::new(Running { restate }) as Arc<dyn Deployed>)
        })
    }
}

/// The client calls the handlers by these names.
#[test]
fn the_client_knows_the_services() {
    assert_eq!(restate_rafi_client::RAFFLE_SERVICE, SERVICE_NAME);
    assert_eq!(restate_rafi_client::INFO_SERVICE, INFO_SERVICE_NAME);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs RESTATE_SERVER_BIN"]
async fn the_raffle_object_conforms() {
    if launcher_or_skip(ReusePolicy::Never).is_none() {
        return;
    }
    conformance::run(&OnRestate {
        servers: AtomicUsize::new(0),
    })
    .await;
}
