//! Rafi's live channel.
//!
//! Phones get the [`ParticipantView`](rafi_live::ParticipantView), pushed by
//! one Durable Object per raffle ([`RaffleChannel`]) over a partyline Latest
//! channel. Only the object builds the view: the app and the draw engine
//! [`nudge`] it after writing to the database, and it reads the raffle back in
//! one snapshot ([`Snapshot`]) and publishes by [`rule::decide`].
//!
//! The snapshot and the rule are plain Rust and tested natively; the object
//! and the Worker helpers exist on `wasm32` only.

pub mod rule;
pub mod snapshot;

#[cfg(target_arch = "wasm32")]
mod gate;
#[cfg(target_arch = "wasm32")]
mod object;

#[cfg(target_arch = "wasm32")]
pub use gate::{BINDING, CONNECT_LIMITER, JURISDICTION_VAR, connect, destroy, namespace, nudge};
#[cfg(target_arch = "wasm32")]
pub use object::RaffleChannel;
pub use snapshot::{HIDDEN_NAME, Snapshot};

/// The D1 binding the object reads the raffle from.
pub const D1_BINDING: &str = "DB";
