//! The server side of live updates, over `rafi-live-client`'s view and
//! channel:
//!
//! - [`raffle::view`]: the participant view of a raffle as drawn;
//! - [`publish::decide`]: when a live channel publishes a participant view;
//! - [`altcha`]: issuing and verifying the join form's proof-of-work.
//!
//! It reads no clock and no randomness of its own, so it runs unchanged on
//! `wasm32-unknown-unknown` (Workers).

pub mod altcha;
pub mod publish;
pub mod raffle;
