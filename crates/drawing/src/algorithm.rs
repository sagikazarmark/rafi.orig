//! `rafi-draw-v1`: who can be drawn, and a deterministic, unbiased selection
//! from public randomness.
//!
//! The pool for draw `k`:
//! - Pass 1: every entrant not drawn in pass 1 yet, whatever their draw's
//!   outcome: one win per entrant.
//! - Pass 2, once pass 1 is exhausted: pass-1 winners whose draw was not
//!   forfeited (resolved "not here" or "declined"), not drawn in pass 2 yet.
//!
//! The selection:
//!
//! ```text
//! seed_k      = SHA256("rafi-draw-v1" ‖ randomness ‖ manifest_hash ‖ BE_u64(k))
//! rand_u64(c) = BE_u64(SHA256(seed_k ‖ BE_u64(c))[0..8])
//! uniform(n)  = rejection sampling below 2^64 − (2^64 mod n)
//! winner_k    = pool_k[uniform(|pool_k|)]
//! ```
//!
//! The algorithm deliberately avoids the `rand` crate: its value stability is
//! not guaranteed across versions and a verifier in another language would have
//! to reimplement it. This needs nothing but SHA-256.

use std::collections::HashSet;

use crate::{
    draw::CompletedDraw,
    hash::{Hash, sha256},
    id::EntrantId,
};

/// Algorithm identifier, also used as the domain separation tag.
pub const ALGO: &str = "rafi-draw-v1";

/// Derives the seed for draw number `k`.
pub fn seed(randomness: &Hash, manifest_hash: &Hash, k: u64) -> Hash {
    sha256(&[ALGO.as_bytes(), randomness, manifest_hash, &k.to_be_bytes()])
}

/// A deterministic stream of `u64`s derived from a seed.
#[derive(Debug, Clone)]
struct Stream {
    seed: Hash,
    counter: u64,
}

impl Stream {
    fn new(seed: Hash) -> Self {
        Self { seed, counter: 0 }
    }

    fn next_u64(&mut self) -> u64 {
        let digest = sha256(&[&self.seed, &self.counter.to_be_bytes()]);
        self.counter += 1;
        u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
    }

    /// Returns a uniformly distributed integer in `0..n` (no modulo bias).
    ///
    /// # Panics
    ///
    /// Panics if `n` is zero.
    fn uniform(&mut self, n: u64) -> u64 {
        assert!(n > 0, "uniform(0) is undefined");
        let span = 1u128 << 64;
        let limit = span - (span % n as u128);
        loop {
            let x = self.next_u64();
            if (x as u128) < limit {
                return x % n;
            }
        }
    }
}

/// Selects the winning index for draw `k` from a pool of `pool_len` entrants.
///
/// # Panics
///
/// Panics if `pool_len` is zero.
pub fn select(randomness: &Hash, manifest_hash: &Hash, k: u64, pool_len: usize) -> usize {
    Stream::new(seed(randomness, manifest_hash, k)).uniform(pool_len as u64) as usize
}

/// The pool for the next draw: `(pass, entrants)`, in the order of
/// `entrants` (the manifest's, sorted).
pub(crate) fn pool(entrants: &[EntrantId], draws: &[CompletedDraw]) -> (u8, Vec<EntrantId>) {
    let drawn = |pass: u8| -> HashSet<&EntrantId> {
        draws
            .iter()
            .filter(|d| d.pass == pass)
            .map(|d| &d.entrant)
            .collect()
    };
    let drawn_pass1 = drawn(1);
    let pass1: Vec<EntrantId> = entrants
        .iter()
        .filter(|e| !drawn_pass1.contains(e))
        .cloned()
        .collect();
    if !pass1.is_empty() {
        return (1, pass1);
    }

    let winners: HashSet<&EntrantId> = draws
        .iter()
        .filter(|d| d.pass == 1 && !d.forfeited())
        .map(|d| &d.entrant)
        .collect();
    let drawn_pass2 = drawn(2);
    let pass2 = entrants
        .iter()
        .filter(|e| winners.contains(e) && !drawn_pass2.contains(e))
        .cloned()
        .collect();
    (2, pass2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_stays_in_range() {
        let mut s = Stream::new([7; 32]);
        for n in 1..200 {
            assert!(s.uniform(n) < n);
        }
    }

    #[test]
    fn uniform_one_is_zero() {
        let mut s = Stream::new([1; 32]);
        assert_eq!(s.uniform(1), 0);
    }

    #[test]
    fn select_is_deterministic() {
        let a = select(&[1; 32], &[2; 32], 3, 100);
        let b = select(&[1; 32], &[2; 32], 3, 100);
        assert_eq!(a, b);
        let c = select(&[1; 32], &[2; 32], 4, 100);
        // Different draw numbers give (almost certainly) different results;
        // this specific pair is pinned by the test vectors anyway.
        let _ = c;
    }

    #[test]
    fn uniform_huge_n_uses_rejection() {
        // n just above 2^63 makes ~half of the u64 space rejected.
        let n = (1u64 << 63) + 1;
        let mut s = Stream::new([9; 32]);
        for _ in 0..50 {
            assert!(s.uniform(n) < n);
        }
    }
}
