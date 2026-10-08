//! Rate limits on joining.
//!
//! Keys are never IP addresses: a whole meetup venue shares one NAT address.
//! These limits are soft (Cloudflare's rate-limiting binding counts per
//! location); the hard limit is the per-raffle entrant cap in the join's
//! `INSERT` (`RAFI_MAX_ENTRANTS`).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::live::BoxFuture;

/// The limits the app checks. Each has its own budget (on Workers, its own
/// rate-limiting binding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Joins per raffle (key: the raffle ID).
    JoinPerRaffle,
    /// Joins per raffle and email address (key: raffle ID and normalized email).
    JoinPerEmail,
}

impl Limit {
    /// The native budget: hits per window.
    pub fn budget(self) -> (u32, Duration) {
        match self {
            Self::JoinPerRaffle => (600, Duration::from_secs(60)),
            Self::JoinPerEmail => (5, Duration::from_secs(60)),
        }
    }
}

/// Counts hits against a [`Limit`].
pub trait RateLimit: Send + Sync + 'static {
    /// Records a hit for `key`; `false` if it's over the limit.
    fn allow<'a>(&'a self, limit: Limit, key: &'a str) -> BoxFuture<'a, bool>;
}

/// Tiny keyed fixed-window rate limiter, in memory (one native process).
#[derive(Clone, Default)]
pub struct Limiter {
    windows: Arc<Mutex<HashMap<String, (Instant, u32)>>>,
}

impl Limiter {
    /// Records a hit for `key`; returns `false` if more than `limit` hits
    /// happened within `window`.
    pub fn check(&self, key: &str, limit: u32, window: Duration) -> bool {
        let now = Instant::now();
        let mut map = self.windows.lock().expect("limiter lock");
        if map.len() > 10_000 {
            map.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(600));
        }
        let entry = map.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= limit
    }
}

impl RateLimit for Limiter {
    fn allow<'a>(&'a self, limit: Limit, key: &'a str) -> BoxFuture<'a, bool> {
        let (max, window) = limit.budget();
        let allowed = self.check(&format!("{limit:?}:{key}"), max, window);
        Box::pin(async move { allowed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        let l = Limiter::default();
        for _ in 0..3 {
            assert!(l.check("a", 3, Duration::from_secs(60)));
        }
        assert!(!l.check("a", 3, Duration::from_secs(60)));
        assert!(l.check("b", 3, Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn limits_are_separate() {
        let l = Limiter::default();
        for _ in 0..5 {
            assert!(l.allow(Limit::JoinPerEmail, "r:a@x").await);
        }
        assert!(!l.allow(Limit::JoinPerEmail, "r:a@x").await);
        assert!(l.allow(Limit::JoinPerRaffle, "r:a@x").await);
    }
}
