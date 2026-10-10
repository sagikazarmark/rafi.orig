//! The raffle engine, as the app sees it.

use std::sync::Arc;

use rafi_raffle::{
    BeaconSource, Decision, DrawRequest, Error as RaffleError, ErrorKind, Manifest, RaffleId,
};

use crate::BoxFuture;

/// What the raffle engine says about itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EngineInfo {
    /// Where the engine's randomness comes from. The app decides which
    /// source a raffle commits to; it only checks this before opening one,
    /// and the engine refuses a mismatch itself.
    pub beacon: BeaconSource,
}

/// Why the raffle engine didn't (or didn't yet) confirm a call.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine refused the command; it was not applied.
    #[error("{message}")]
    Rejected { kind: ErrorKind, message: String },
    /// The engine accepted the command but didn't settle it before the
    /// caller's deadline (it was queued behind a draw waiting for its beacon).
    /// It applies once; its outcome shows in the projection.
    #[error("the raffle engine accepted the command; it applies after the current draw")]
    Pending,
    /// The engine couldn't be reached, or failed. The command may still have
    /// been applied: retry with the same command ID.
    #[error("the raffle engine is unavailable: {0:#}")]
    Unavailable(anyhow::Error),
}

impl EngineError {
    /// A refusal by the draw rules.
    pub fn rejected(e: &RaffleError) -> Self {
        Self::Rejected {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

/// Why a command for a raffle the engine doesn't hold (never opened, or
/// finished and forgotten) is refused, as not found.
pub const NOT_OPEN: &str = "no raffle is open";

impl From<RaffleError> for EngineError {
    fn from(e: RaffleError) -> Self {
        Self::rejected(&e)
    }
}

/// The raffle engine: owns each raffle from opening to finish, applies the
/// draw rules (`rafi_raffle`) and writes every new state to the
/// projection ([`RaffleStore`](crate::RaffleStore)), then notifies clients.
///
/// The contract every engine keeps:
///
/// - **State is read from the projection**, never from the engine.
/// - **Opening** is synchronous and idempotent per manifest
///   ([`Manifest::open_key`]): the same manifest again is a no-op, another
///   manifest is refused, and so is another drand chain than the engine's.
/// - **Draws are accepted asynchronously**: [`draw`](Self::draw) returns once
///   the draw is accepted, not when it is settled. A draw the engine refuses is
///   silent (only the projection shows progress), so callers validate against
///   the projection first.
/// - **Decisions are applied synchronously**: [`decide`](Self::decide) returns
///   once the decision is applied.
/// - **Commands for one raffle apply one at a time**, in arrival order. A
///   decision sent while a draw waits for its beacon is queued and applies
///   after the draw settles, against the settled state. It may therefore wait
///   up to the engine's drand lead plus fetch timeout; past the adapter's own
///   deadline it returns [`EngineError::Pending`].
/// - **Idempotency**: the same `command_id` (within the engine's retention
///   window) is applied once, even when sent twice at once, so retrying after
///   [`EngineError::Pending`] or [`EngineError::Unavailable`] is safe. A
///   command ID names one command: sent again with another command, the
///   engine answers as it did the first time and applies nothing.
/// - **The projection is the record**: an engine retries a failed write until
///   it succeeds, and applies nothing more to that raffle meanwhile (an
///   abandon included: it too must be recorded).
/// - **After finish** the engine forgets the raffle: further commands are
///   refused as not found. An opening replayed after that never changes the
///   published projection (the store keeps the newest version), but past the
///   retention window it would open the raffle again, unpublished. Callers
///   must therefore not open a raffle they finished: rafi's lifecycle
///   opens one only from a frozen raffle (`rafi_lifecycle::start::next`).
pub trait RaffleEngine: Send + Sync + 'static {
    /// The engine's beacon source; also tells whether it is reachable.
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>>;

    /// Opens a raffle from its manifest: the engine owns it from here.
    fn open<'a>(&'a self, manifest: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>>;

    /// Starts a draw: returns once it is accepted; it settles in the
    /// background (`command_id` deduplicates it).
    fn draw<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>>;

    /// Applies a decision (`command_id` deduplicates it).
    fn decide<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>>;
}

impl<T: RaffleEngine + ?Sized> RaffleEngine for Arc<T> {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        (**self).info()
    }

    fn open<'a>(&'a self, manifest: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).open(manifest)
    }

    fn draw<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).draw(raffle_id, request, command_id)
    }

    fn decide<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).decide(raffle_id, decision, command_id)
    }
}
