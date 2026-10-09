//! The `RaffleEngine` service: what the draw engine says about itself.

use rafi_drand::DrandClient;
use rafi_drawing::BeaconSource;
use restate_sdk::prelude::*;
use serde::Serialize;

/// The Restate service name.
pub const INFO_SERVICE_NAME: &str = "RaffleEngine";

/// The `info` handler's answer.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    /// Where the engine's randomness comes from.
    pub beacon: BeaconSource,
}

/// Keyless: answers for the whole engine, never blocked by a raffle's draw.
pub struct RaffleEngine {
    beacon: BeaconSource,
}

impl RaffleEngine {
    /// The engine that draws with `drand` (the same client as its `Raffle`).
    pub fn new(drand: &DrandClient) -> Self {
        Self {
            beacon: BeaconSource::Drand(drand.chain_hash()),
        }
    }
}

#[restate_sdk::service(name = "RaffleEngine")]
impl RaffleEngine {
    /// The engine's beacon source. The app checks it before opening a
    /// drawing; the `Raffle` object refuses another source itself.
    #[handler]
    async fn info(&self, _ctx: Context<'_>) -> HandlerResult<Json<Info>> {
        Ok(Json(Info {
            beacon: self.beacon,
        }))
    }
}
