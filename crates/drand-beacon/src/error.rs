use crate::{ChainHash, Scheme};

/// Errors from parsing and verification.
///
/// `Clone + Eq + Send + Sync + 'static`, and available without `std`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Malformed input: bad JSON, hex, URL or field value.
    #[error("invalid {what}")]
    #[non_exhaustive]
    Parse {
        /// What could not be parsed.
        what: &'static str,
    },
    /// A byte string has the wrong length.
    #[error("{what} must be {expected} bytes, got {got}")]
    #[non_exhaustive]
    Length {
        /// What has the wrong length.
        what: &'static str,
        /// The required length.
        expected: usize,
        /// The actual length.
        got: usize,
    },
    /// Round 0 does not exist (relays answer `/public/0` with the latest round).
    #[error("round 0 does not exist")]
    RoundZero,
    /// A `randomness` field is not `sha256(signature)`.
    #[error("randomness does not match sha256(signature)")]
    RandomnessMismatch,
    /// The beacon is for a different round than requested.
    #[error("expected round {expected}, got {got}")]
    #[non_exhaustive]
    RoundMismatch {
        /// The requested round.
        expected: u64,
        /// The round in the beacon.
        got: u64,
    },
    /// Chain info does not hash to the expected chain hash.
    #[error("chain hash mismatch: expected {expected}, got {got}")]
    #[non_exhaustive]
    ChainHashMismatch {
        /// The expected hash.
        expected: ChainHash,
        /// The hash computed from the chain info.
        got: ChainHash,
    },
    /// The scheme cannot be verified (unknown, deprecated, or its curve feature is off).
    #[error("unsupported scheme {0}")]
    UnsupportedScheme(Scheme),
    /// The chain's public key is malformed, non-canonical, infinity, or outside the subgroup.
    #[error("invalid public key")]
    BadPublicKey,
    /// The signature is not a canonical encoding of a valid, non-infinity subgroup point.
    #[error("signature is not a canonical point encoding")]
    NonCanonicalSignature,
    /// The signature does not verify.
    #[error("invalid signature")]
    InvalidSignature,
    /// A chained beacon lacks a valid previous signature.
    #[error("missing or invalid previous signature")]
    MissingPreviousSignature,
}

impl Error {
    pub(crate) const fn parse(what: &'static str) -> Self {
        Self::Parse { what }
    }

    pub(crate) fn length(what: &'static str, expected: usize, got: usize) -> Self {
        Self::Length {
            what,
            expected,
            got,
        }
    }
}
