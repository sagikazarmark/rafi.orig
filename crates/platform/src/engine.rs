//! The draw engine, as the app sees it.

use std::sync::Arc;

use rafi_core::{
    commit::CommitDoc,
    drand::ChainHash,
    engine::{Command, ErrorKind},
    state::DrawError,
};

use crate::BoxFuture;

/// What the draw engine says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    /// The drand chain the engine draws from. The app decides which chain a
    /// raffle commits to; it only checks this before freezing, and the engine
    /// refuses a mismatch itself.
    pub chain: ChainHash,
}

/// Why the draw engine didn't (or didn't yet) confirm a call.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine refused the command; it was not applied.
    #[error("{message}")]
    Rejected { kind: ErrorKind, message: String },
    /// The engine accepted the command but didn't settle it before the
    /// caller's deadline (it was queued behind a draw waiting for its beacon).
    /// It applies once; its outcome shows in the projection.
    #[error("the draw engine accepted the command; it applies after the current draw")]
    Pending,
    /// The engine couldn't be reached, or failed. The command may still have
    /// been applied: retry with the same command ID.
    #[error("the draw engine is unavailable: {0:#}")]
    Unavailable(anyhow::Error),
}

impl EngineError {
    /// A refusal by the draw rules.
    pub fn rejected(e: &DrawError) -> Self {
        Self::Rejected {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

impl From<DrawError> for EngineError {
    fn from(e: DrawError) -> Self {
        Self::rejected(&e)
    }
}

/// The draw engine: owns each raffle's draw state from freeze to finish,
/// applies the draw rules (`rafi_core`) and writes every new state to the
/// projection ([`DrawStore`](crate::DrawStore)), then notifies clients.
///
/// The contract every engine keeps:
///
/// - **State is read from the projection**, never from the engine.
/// - **Freeze** is synchronous and idempotent per list
///   ([`CommitDoc::freeze_key`]): the same list again is a no-op, another list
///   is refused, and so is another drand chain than the engine's.
/// - **Draws are accepted asynchronously**: `command(Draw)` returns once the
///   draw is accepted, not when it is settled. A draw the engine refuses is
///   silent (only the projection shows progress), so callers validate against
///   the projection first.
/// - **Commands for one raffle apply one at a time**, in arrival order. A
///   command sent while a draw waits for its beacon is queued and applies
///   after the draw settles, against the settled state. A synchronous command
///   may therefore wait up to the engine's drand lead plus fetch timeout; past
///   the adapter's own deadline it returns [`EngineError::Pending`].
/// - **Idempotency**: the same `command_id` (within the engine's retention
///   window) is applied once, so retrying after [`EngineError::Pending`] or
///   [`EngineError::Unavailable`] is safe.
/// - **After finish** the engine forgets the raffle: further commands are
///   refused as not found. A freeze replayed after that never changes the
///   published projection (the store keeps the newest version).
pub trait DrawEngine: Send + Sync + 'static {
    /// The engine's drand chain; also tells whether it is reachable.
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>>;

    /// Freezes the raffle's entrant list: the engine takes over its draw state.
    fn freeze<'a>(&'a self, doc: &'a CommitDoc) -> BoxFuture<'a, Result<(), EngineError>>;

    /// Sends a command (`command_id` deduplicates it).
    fn command<'a>(
        &'a self,
        raffle_id: &'a str,
        cmd: &'a Command,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>>;
}

impl<T: DrawEngine + ?Sized> DrawEngine for Arc<T> {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        (**self).info()
    }

    fn freeze<'a>(&'a self, doc: &'a CommitDoc) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).freeze(doc)
    }

    fn command<'a>(
        &'a self,
        raffle_id: &'a str,
        cmd: &'a Command,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).command(raffle_id, cmd, command_id)
    }
}
