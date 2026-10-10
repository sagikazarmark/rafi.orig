//! The drawing engine, as the app sees it.

use std::sync::Arc;

use rafi_drawing::{
    BeaconSource, Decision, DrawRequest, DrawingId, Error as DrawingError, ErrorKind, Manifest,
};

use rafi_platform::BoxFuture;

/// What the drawing engine says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    /// Where the engine's randomness comes from. The app decides which
    /// source a drawing commits to; it only checks this before opening one,
    /// and the engine refuses a mismatch itself.
    pub beacon: BeaconSource,
}

/// Why the drawing engine didn't (or didn't yet) confirm a call.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine refused the command; it was not applied.
    #[error("{message}")]
    Rejected { kind: ErrorKind, message: String },
    /// The engine accepted the command but didn't settle it before the
    /// caller's deadline (it was queued behind a draw waiting for its beacon).
    /// It applies once; its outcome shows in the projection.
    #[error("the drawing engine accepted the command; it applies after the current draw")]
    Pending,
    /// The engine couldn't be reached, or failed. The command may still have
    /// been applied: retry with the same command ID.
    #[error("the drawing engine is unavailable: {0:#}")]
    Unavailable(anyhow::Error),
}

impl EngineError {
    /// A refusal by the draw rules.
    pub fn rejected(e: &DrawingError) -> Self {
        Self::Rejected {
            kind: e.kind(),
            message: e.to_string(),
        }
    }

    /// The engine has no open drawing by that ID (never opened, or finished
    /// and forgotten).
    pub fn not_open() -> Self {
        Self::Rejected {
            kind: ErrorKind::NotFound,
            message: NOT_OPEN.to_owned(),
        }
    }
}

/// Why a command for a drawing the engine doesn't hold is refused.
pub const NOT_OPEN: &str = "no drawing is open";

/// The HTTP status code an engine answers a refusal of this kind with.
pub fn status_code(kind: ErrorKind) -> u16 {
    match kind {
        ErrorKind::NotFound => 404,
        ErrorKind::BadRequest => 400,
        ErrorKind::Conflict => 409,
        ErrorKind::Invariant => 500,
    }
}

/// The refusal kind an engine's status code stands for; `None` for anything
/// an engine never refuses with (the engine is unavailable).
pub fn refusal_kind(status: u16) -> Option<ErrorKind> {
    match status {
        404 => Some(ErrorKind::NotFound),
        400 => Some(ErrorKind::BadRequest),
        409 => Some(ErrorKind::Conflict),
        500 => Some(ErrorKind::Invariant),
        _ => None,
    }
}

impl From<DrawingError> for EngineError {
    fn from(e: DrawingError) -> Self {
        Self::rejected(&e)
    }
}

/// The drawing engine: owns each drawing from opening to finish, applies the
/// draw rules (`rafi_drawing`) and writes every new state to the
/// projection ([`DrawStore`](crate::DrawStore)), then notifies clients.
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
/// - **Commands for one drawing apply one at a time**, in arrival order. A
///   decision sent while a draw waits for its beacon is queued and applies
///   after the draw settles, against the settled state. It may therefore wait
///   up to the engine's drand lead plus fetch timeout; past the adapter's own
///   deadline it returns [`EngineError::Pending`].
/// - **Idempotency**: the same `command_id` (within the engine's retention
///   window) is applied once, so retrying after [`EngineError::Pending`] or
///   [`EngineError::Unavailable`] is safe.
/// - **After finish** the engine forgets the drawing: further commands are
///   refused as not found. An opening replayed after that never changes the
///   published projection (the store keeps the newest version).
pub trait DrawingEngine: Send + Sync + 'static {
    /// The engine's beacon source; also tells whether it is reachable.
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>>;

    /// Opens a drawing from its manifest: the engine owns it from here.
    fn open<'a>(&'a self, manifest: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>>;

    /// Starts a draw: returns once it is accepted; it settles in the
    /// background (`command_id` deduplicates it).
    fn draw<'a>(
        &'a self,
        drawing_id: &'a DrawingId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>>;

    /// Applies a decision (`command_id` deduplicates it).
    fn decide<'a>(
        &'a self,
        drawing_id: &'a DrawingId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>>;
}

impl<T: DrawingEngine + ?Sized> DrawingEngine for Arc<T> {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        (**self).info()
    }

    fn open<'a>(&'a self, manifest: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).open(manifest)
    }

    fn draw<'a>(
        &'a self,
        drawing_id: &'a DrawingId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).draw(drawing_id, request, command_id)
    }

    fn decide<'a>(
        &'a self,
        drawing_id: &'a DrawingId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        (**self).decide(drawing_id, decision, command_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_kinds_round_trip_through_status_codes() {
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::BadRequest,
            ErrorKind::Conflict,
            ErrorKind::Invariant,
        ] {
            assert_eq!(refusal_kind(status_code(kind)), Some(kind));
        }
        assert_eq!(refusal_kind(502), None);
    }
}
