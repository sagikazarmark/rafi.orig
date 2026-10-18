//! A [`ReplayStore`] in memory: for one process, a demo, or tests.

use std::{collections::HashMap, convert::Infallible, sync::Mutex};

use crate::ReplayStore;

/// Redeemed nonces in a map, until they expire. Only for one process: with
/// more than one server, use a shared store (a database table).
#[derive(Debug, Default)]
pub struct MemoryStore {
    used: Mutex<HashMap<String, i64>>,
}

impl MemoryStore {
    /// How many nonces it holds.
    pub fn len(&self) -> usize {
        self.used.lock().expect("not poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ReplayStore for MemoryStore {
    type Error = Infallible;

    async fn redeem(&self, nonce: &str, expires_at: i64) -> Result<bool, Infallible> {
        let mut used = self.used.lock().expect("not poisoned");
        if used.contains_key(nonce) {
            return Ok(false);
        }
        used.insert(nonce.to_owned(), expires_at);
        Ok(true)
    }

    async fn forget_expired(&self, now: i64) -> Result<(), Infallible> {
        self.used
            .lock()
            .expect("not poisoned")
            .retain(|_, expires_at| *expires_at >= now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn once_until_forgotten() {
        let store = MemoryStore::default();
        assert!(store.redeem("a", 10).await.unwrap());
        assert!(!store.redeem("a", 10).await.unwrap());
        assert!(store.redeem("b", 20).await.unwrap());
        store.forget_expired(11).await.unwrap();
        assert_eq!(store.len(), 1, "a expired before 11");
        assert!(!store.redeem("b", 20).await.unwrap());
    }
}
