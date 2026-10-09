//! The `RaffleEngine` service: what the draw engine says about itself.

use rafi_core::drand::ChainHash;
use rafi_drand::DrandClient;
use restate_sdk::prelude::*;
use serde::Serialize;

/// The Restate service name.
pub const INFO_SERVICE_NAME: &str = "RaffleEngine";

/// The `info` handler's answer.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    /// The drand chain the engine draws from (hex).
    pub chain: String,
}

/// Keyless: answers for the whole engine, never blocked by a raffle's draw.
pub struct RaffleEngine {
    chain: ChainHash,
}

impl RaffleEngine {
    /// The engine that draws with `drand` (the same client as its `Raffle`).
    pub fn new(drand: &DrandClient) -> Self {
        Self {
            chain: drand.chain_hash(),
        }
    }
}

#[restate_sdk::service(name = "RaffleEngine")]
impl RaffleEngine {
    /// The engine's drand chain. The app checks it before freezing a raffle;
    /// the `Raffle` object refuses another chain itself.
    #[handler]
    async fn info(&self, _ctx: Context<'_>) -> HandlerResult<Json<Info>> {
        Ok(Json(Info {
            chain: self.chain.to_string(),
        }))
    }
}
