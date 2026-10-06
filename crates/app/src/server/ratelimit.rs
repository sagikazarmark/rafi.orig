//! Tiny keyed fixed-window rate limiter.
//!
//! Keys are never IP addresses: a whole meetup venue shares one NAT address.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

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
}
