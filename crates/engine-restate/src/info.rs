//! The `EngineInfo` service: what the drawing engine says about itself.

use rafi_drand::DrandClient;
use rafi_raffle::BeaconSource;
use restate_sdk::prelude::*;

/// The Restate service name.
pub const INFO_SERVICE_NAME: &str = "EngineInfo";

/// Keyless: answers for the whole engine, never blocked by a drawing's draw.
pub struct EngineInfo {
    beacon: BeaconSource,
}

impl EngineInfo {
    /// The engine that draws with `drand` (the same client as its `Drawing`).
    pub fn new(drand: &DrandClient) -> Self {
        Self {
            beacon: BeaconSource::Drand(drand.chain_hash()),
        }
    }
}

#[restate_sdk::service(name = "EngineInfo")]
impl EngineInfo {
    /// The engine's beacon source. The app checks it before opening a
    /// drawing; the `Drawing` object refuses another source itself.
    #[handler]
    async fn info(&self, _ctx: Context<'_>) -> HandlerResult<Json<rafi_engine::EngineInfo>> {
        Ok(Json(rafi_engine::EngineInfo {
            beacon: self.beacon,
        }))
    }
}
