//! Where a drawing's randomness comes from.

use std::fmt;

use serde::{Deserialize, Serialize};

use drand::{ChainHash, chains};

/// The source of a drawing's randomness, as its manifest names it:
/// `{"drand": "<chain hash>"}`.
///
/// drand is the only source. The tag leaves room for another one if every
/// hosted drand network went away; the events and the verifier would still
/// need a way to check its beacons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeaconSource {
    /// A drand network, by chain hash: quicknet, the mock test chain, or one
    /// run privately (only a public chain is publicly verifiable).
    Drand(ChainHash),
}

impl BeaconSource {
    /// drand's quicknet: the public chain drawings use.
    pub const QUICKNET: Self = Self::Drand(chains::QUICKNET_HASH);

    /// Whether anyone can verify this source's beacons: a public drand chain
    /// this build knows ([`verify_public`](crate::verify_public) checks
    /// against these).
    pub fn publicly_verifiable(&self) -> bool {
        match self {
            Self::Drand(chain) => chains::by_hash(chain).is_some(),
        }
    }

    /// The drand chain, when the source is drand.
    pub fn drand_chain(&self) -> Option<&ChainHash> {
        match self {
            Self::Drand(chain) => Some(chain),
        }
    }
}

impl fmt::Display for BeaconSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drand(chain) => write!(f, "drand chain {chain}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drand::chains::QUICKNET_HASH;

    #[test]
    fn quicknet_is_publicly_verifiable() {
        assert!(BeaconSource::QUICKNET.publicly_verifiable());
        assert!(!BeaconSource::Drand(ChainHash::from_bytes([7; 32])).publicly_verifiable());
    }

    #[test]
    fn serializes_tagged_by_kind() {
        let source = BeaconSource::Drand(QUICKNET_HASH);
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, format!(r#"{{"drand":"{QUICKNET_HASH}"}}"#));
        assert_eq!(serde_json::from_str::<BeaconSource>(&json).unwrap(), source);
    }
}
