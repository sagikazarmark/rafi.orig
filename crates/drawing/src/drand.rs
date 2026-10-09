//! The drand side of a draw: which chain, which round.
//!
//! Fetching, BLS verification and round timing (including the round to commit
//! to, [`Schedule::commit_round`]) live in the `drand` crate; this module
//! re-exports what the domain model needs.

pub use drand::{
    Beacon, ChainHash, ChainInfo, Error as BeaconError, Schedule, UnixTime, VerifiedBeacon,
    Verifier, chains,
};
