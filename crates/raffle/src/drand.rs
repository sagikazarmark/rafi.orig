//! The drand types this crate's API is made of (a [`BeaconSource`] names a
//! drand chain; a draw completes with a verified beacon), the public chains
//! the verifier knows, and the mock chain.
//!
//! The mock chain (the `mock-chain` feature) is a signing test chain with a
//! public seed: its beacons are real BLS signatures, but anyone can forge
//! them, and its chain hash marks every raffle using it as not publicly
//! verifiable. For development and tests only.
//!
//! [`BeaconSource`]: crate::BeaconSource

pub use ::drand::{Beacon, ChainHash, ChainInfo, VerifiedBeacon, Verifier, chains};

/// The public seed of the mock chain. Anyone can sign its beacons.
pub const MOCK_SEED: [u8; 32] = *b"rafi mock drand: not verifiable!";

/// The mock chain: pinned parameters (quicknet's scheme, period and genesis),
/// so the app, the engines and the verifier agree on its hash.
#[cfg(feature = "mock-chain")]
pub fn mock_chain() -> ::drand::TestChain {
    let q = chains::quicknet();
    let period = u32::try_from(q.period().as_secs())
        .ok()
        .and_then(std::num::NonZeroU32::new)
        .expect("quicknet period");
    ::drand::TestChain::new(
        MOCK_SEED,
        ::drand::Scheme::BlsUnchainedG1Rfc9380,
        period,
        q.genesis_time().as_secs(),
        "rafi-test",
    )
    .expect("supported scheme")
}

/// Chain info for a chain hash found in a manifest: the public drand chains,
/// and the mock chain when this build has it. A resolver for
/// [`verify`](crate::verify).
pub fn resolve_chain(hash: &ChainHash) -> Option<ChainInfo> {
    if let Some(info) = chains::by_hash(hash) {
        return Some(info);
    }
    #[cfg(feature = "mock-chain")]
    {
        let mock = mock_chain().chain().clone();
        if mock.hash() == *hash {
            return Some(mock);
        }
    }
    None
}

#[cfg(all(test, feature = "mock-chain"))]
mod tests {
    use super::*;

    #[test]
    fn the_mock_chain_resolves_but_is_not_public() {
        let mock = mock_chain().chain().clone();
        assert_ne!(mock.hash(), chains::QUICKNET_HASH);
        assert_eq!(resolve_chain(&mock.hash()), Some(mock.clone()));
        assert!(!crate::BeaconSource::Drand(mock.hash()).publicly_verifiable());
        assert_eq!(
            mock_chain().chain().hash(),
            mock.hash(),
            "the mock chain is pinned"
        );
    }
}
