//! What the Rafi app does, without HTTP: the organizer's use cases (raffles,
//! prizes, entrants, starting and running the draw, the reads the admin page
//! shows), the participants' (joining) and the public reads (the full view,
//! the verification data).
//!
//! Each use case is a function over an [`App`]: the database, the raffle
//! engine, the [`LiveUpdates`] and the [`RateLimit`]s the deployment
//! plugs in, its [`Clock`], and the deployment's [`Policy`]. The rules come
//! from `rafi-lifecycle` and `rafi-raffle`; a use case sequences the reads
//! and writes around them. A refusal is a typed [`Refusal`]; the HTTP layer
//! only maps it.
//!
//! A use case reads the clock once and goes by that time throughout: its
//! checks against the dates and every row it writes ([`rafi_store`] takes
//! `now`). Tests set the clock rather than date the data back.
//!
//! Every write that changes what phones see (the participant view) notifies
//! clients ([`App::live_updates`]). Joins don't: they never change it. The
//! raffle engine notifies after its own writes.

pub mod codes;
mod draw;
mod entrants;
mod error;
pub mod housekeeping;
pub mod ids;
mod owners;
pub mod pow;
mod prizes;
pub mod public;
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
    CONSENT_VERSION, JoinError, JoinOutcome, JoinRequest, add_entrants, add_fakes, deliver_all,
    end_live, join, name_ideas, object_entrant, patch_entrant, purge, remove_entrant, undo_add,
    unobject_entrant,
};
pub use error::{Error, Kind as RefusalKind, Refusal, Result, Unfinished};
pub use owners::{reapply_erasures, remove_owner_data};
pub use pow::ProofOfWork;
pub use prizes::{delete_prize, set_prize_delivery, upsert_prize};
pub use raffles::{
    EventDate, JOIN_CODE_LEN, create, delete, duplicate, get_defaults, lifecycle, list,
    save_defaults, settings_of, update_settings,
};
pub use rafi_lifecycle::settings::RetentionPolicy;
pub use rafi_store::Clock;
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
    /// The rate limits: on joining, and on the organizer's adds.
    pub limiter: Arc<dyn RateLimit>,
    /// What time it is, for the use cases: the system's, or a test's.
    pub clock: Clock,
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
    /// The join form's proof-of-work.
    pub proof_of_work: ProofOfWork,
    /// When personal data is erased: the default period, the longest one an
    /// organizer may choose, and the winner hold.
    pub retention: RetentionPolicy,
    /// The youngest an entrant may be: part of what the join form says, so
    /// part of the consent version it records.
    pub min_age: u32,
}

impl Policy {
    /// The version of what the join form shows an entrant: its text
    /// ([`CONSENT_VERSION`]) and the age it asks them to confirm.
    pub fn consent_version(&self) -> String {
        format!("{CONSENT_VERSION}/age{}", self.min_age)
    }
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

/// Who acts: the owner key the raffles belong to (a Clerk organization, or
/// else the user), the user acting for it, both recorded with every audited
/// action, and what the user may do for the owner, which the use cases that
/// need a tier check ([`Actor::allow`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor<'a> {
    pub owner: &'a str,
    pub user: &'a str,
    pub access: rafi_api::Access,
}

impl<'a> Actor<'a> {
    /// A personal account, or a test's owner: the user is the owner, and may
    /// do everything.
    pub const fn solo(owner: &'a str) -> Self {
        Self {
            owner,
            user: owner,
            access: rafi_api::Access::Admin,
        }
    }

    /// The same user with another tier (a test's organization member).
    pub const fn with_access(self, access: rafi_api::Access) -> Self {
        Self { access, ..self }
    }

    /// Refuses unless this actor's tier allows what `needs`. Running a raffle
    /// needs nothing beyond membership; the tiers guard creating, configuring
    /// and deleting raffles ([`Access::Manager`](rafi_api::Access::Manager))
    /// and removing the owner's data ([`Access::Admin`](rafi_api::Access::Admin)).
    pub fn allow(&self, needs: rafi_api::Access) -> Result<()> {
        if self.access.allows(needs) {
            Ok(())
        } else {
            Err(Refusal::Forbidden(needs).into())
        }
    }
}

/// The audit log's actor: who, without the tier.
impl<'a> From<Actor<'a>> for rafi_store::audit::Actor<'a> {
    fn from(a: Actor<'a>) -> Self {
        Self {
            owner: a.owner,
            user: a.user,
        }
    }
}

/// Why a write guarded on the raffle's state changed nothing: the raffle as
/// it is now, asked of `refuses` (the use case's own pre-check, as it reads
/// the raffle now) for the refusal; when it has none, `otherwise` (what else
/// the guard could have failed on: the row gone, or simply "it changed").
/// A raffle gone is not found.
pub(crate) async fn unwritten(
    app: &App,
    raffle_id: &str,
    refuses: impl FnOnce(&RaffleRow) -> Option<Refusal>,
    otherwise: Error,
) -> Error {
    match store::get(&app.db, raffle_id).await {
        Ok(Some(raffle)) => refuses(&raffle).map_or(otherwise, Into::into),
        Ok(None) => Error::NotFound,
        Err(e) => e.into(),
    }
}

/// Who is acting, from who is calling the admin API.
pub fn actor(c: &rafi_api::Caller) -> Actor<'_> {
    Actor {
        owner: &c.owner,
        user: &c.user,
        access: c.access,
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
