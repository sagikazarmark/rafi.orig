//! The engine conformance suite on the local runner.

#![cfg(not(target_arch = "wasm32"))]

use std::{sync::Arc, time::Duration};

use rafi_engine_local::{
    LocalEngine,
    conformance::{self, Deployed, Deployment, EngineHarness},
};
use rafi_platform::{BoxFuture, DrawEngine};

struct Local;

struct Running(LocalEngine);

impl Deployed for Running {
    fn engine(&self, deadline: Option<Duration>) -> Arc<dyn DrawEngine> {
        Arc::new(self.0.with_deadline(deadline))
    }
}

impl EngineHarness for Local {
    fn name(&self) -> &str {
        "the local runner"
    }

    fn deploy(&self, d: Deployment) -> BoxFuture<'_, Result<Arc<dyn Deployed>, String>> {
        Box::pin(async move {
            let engine = LocalEngine::new(d.drand, d.store, d.notifier);
            Ok(Arc::new(Running(engine)) as Arc<dyn Deployed>)
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_local_runner_conforms() {
    conformance::run(&Local).await;
}
