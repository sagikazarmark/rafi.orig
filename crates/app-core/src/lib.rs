//! What the Rafi app does, without HTTP: the organizer's use cases (raffles,
//! prizes, entrants, starting and running the draw, the reads the admin page
//! shows), the participants' (joining) and the public reads (the full view,
//! the verification data).
//!
//! Each use case is a function over an [`App`]: the database, the raffle
//! engine and the [`LiveUpdates`] the deployment plugs in, and the
//! deployment's [`Policy`]. The rules come from `rafi-lifecycle` and
//! `rafi-raffle`; a use case sequences the reads and writes around them. A
//! refusal is a typed [`Refusal`]; the HTTP layer only maps it.
//!
//! Every write that changes what phones see (the participant view) notifies
//! clients ([`App::live_updates`]). Joins don't: they never change it. The
//! raffle engine notifies after its own writes.

pub mod codes;
mod draw;
mod entrants;
mod error;
pub mod ids;
mod prizes;
mod raffles;
mod ratelimit;
mod reads;
pub mod view;

use std::sync::Arc;

use rafi_engine::Engine;
use rafi_live_server::LiveUpdates;
use rafi_raffle::drand::ChainHash;
use rafi_store::{Db, RaffleRow, raffles as store};

pub use draw::{command, start_draw};
pub use entrants::{
    CONSENT_VERSION, JoinError, JoinOutcome, add_fakes, add_manual, end_live, join, patch_entrant,
    purge, remove_entrant,
};
pub use error::{Error, Refusal, Result};
pub use prizes::{delete_prize, upsert_prize};
pub use raffles::{create, delete, duplicate, lifecycle, list, settings_of, update_settings};
#[cfg(not(target_arch = "wasm32"))]
pub use ratelimit::Limiter;
pub use ratelimit::{Limit, RateLimit};
pub use reads::{admin, export_csv, verify_data};

/// What the use cases run against.
#[derive(Clone)]
pub struct App {
    pub db: Db,
    /// The drand chain new raffles commit to.
    pub chain: ChainHash,
    /// The raffle engine; `None` on a server without one.
    pub engine: Option<Arc<dyn Engine>>,
    /// Live updates to clients (natively: none, clients poll).
    pub live_updates: LiveUpdates,
    pub policy: Policy,
}

/// The deployment's choices the use cases follow.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Where the app is served: join links point here.
    pub public_url: String,
    /// The most entrants a raffle takes: the hard limit behind the join's
    /// (soft) rate limits.
    pub max_entrants: i64,
}

impl App {
    /// How clients follow `raffle_id`: as the deployment's live updates say
    /// while they run, plain polling once they've ended.
    pub fn follow(&self, raffle_id: &str, live_ended: bool) -> rafi_live_client::Follow {
        if live_ended {
            rafi_live_client::Follow::poll()
        } else {
            self.live_updates.follow(raffle_id)
        }
    }

    /// The raffle engine, or a refusal if this server has none.
    fn engine(&self) -> Result<&dyn Engine> {
        self.engine
            .as_deref()
            .ok_or(Error::Refused(Refusal::NoEngine))
    }
}

/// Raffle `id`, if `owner` owns it; not found otherwise (another organizer's
/// raffle doesn't exist for them).
pub async fn owned(app: &App, owner: &str, id: &str) -> Result<RaffleRow> {
    let raffle = store::get(&app.db, id).await?.ok_or(Error::NotFound)?;
    if raffle.owner_id != owner {
        return Err(Error::NotFound);
    }
    Ok(raffle)
}
