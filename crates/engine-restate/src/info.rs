//! The `EngineInfo` service: what the drawing engine says about itself.

use rafi_drand::DrandClient;
use rafi_drawing::BeaconSource;
use restate_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// The Restate service name.
pub const INFO_SERVICE_NAME: &str = "EngineInfo";

/// The `info` handler's answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    /// Where the engine's randomness comes from.
    pub beacon: BeaconSource,
}

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
    async fn info(&self, _ctx: Context<'_>) -> HandlerResult<Json<Info>> {
        Ok(Json(Info {
            beacon: self.beacon,
        }))
    }
}
