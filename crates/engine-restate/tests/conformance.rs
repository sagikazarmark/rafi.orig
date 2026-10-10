//! The engine conformance suite on the `Drawing` object: each profile deploys
//! the service to its own `restate-server`, and the scenarios drive it through
//! [`RestateClient`](rafi_engine_restate::RestateClient), as the app does.
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

use rafi_engine::BoxFuture;
use rafi_engine::{
    DrawingEngine,
    conformance::{self, Deployed, Deployment, EngineHarness},
};
use rafi_engine_restate::{ClientConfig, RestateClient};
use restate_e2e_harness::{Restate, ReusePolicy, ServerSpec, launcher_or_skip};
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
        "the Drawing object on restate-server"
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
            // Bound as every host binds them.
            let (drawing, info) = rafi_engine_restate::services(d.drand, d.store, d.notifier);
            restate
                .deploy(Endpoint::builder().bind(drawing).bind(info).build())
                .await;
            Ok(Arc::new(Running { restate }) as Arc<dyn Deployed>)
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs RESTATE_SERVER_BIN"]
async fn the_drawing_object_conforms() {
    if launcher_or_skip(ReusePolicy::Never).is_none() {
        return;
    }
    conformance::run(&OnRestate {
        servers: AtomicUsize::new(0),
    })
    .await;
}
