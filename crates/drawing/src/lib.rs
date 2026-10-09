//! Rafi drawing: a verifiable selection of winners for a fixed set of items
//! from a fixed list of entrants. The state machine, the draw algorithm, the
//! manifest and the verifier; it knows nothing of raffles.
//!
//! Everything in this crate is pure (no IO, no clocks, no randomness) so it can
//! run in the server, inside the Restate virtual object, in the browser
//! verifier (WASM) and in tests, and always produce identical results.

pub mod beacon;
pub mod draw;
pub mod drawing;
pub mod engine;
pub mod error;
pub mod manifest;
pub mod verify;

/// The drand crate this one is built on: its types are part of this crate's
/// API (`BeaconSource::Drand(ChainHash)`, the verifier's chain lookup).
pub use drand;

pub use beacon::BeaconSource;
pub use drawing::Drawing;
pub use error::{Error, ErrorKind};
pub use manifest::Manifest;

/// A SHA-256 digest.
pub type Hash = [u8; 32];

pub(crate) fn sha256(parts: &[&[u8]]) -> Hash {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// Decodes a 32-byte hex string.
pub fn hash_from_hex(s: &str) -> Option<Hash> {
    let bytes = hex::decode(s).ok()?;
    bytes.try_into().ok()
}
