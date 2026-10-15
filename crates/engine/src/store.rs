//! Where the raffle engine writes raffles.

use std::sync::{Arc, Mutex};

use rafi_raffle::Raffle;

use crate::BoxFuture;

/// The raffle engine's only write: a raffle, after every change.
pub trait Store: Send + Sync + 'static {
    /// Stores `state`. Called at least once per change (the engine retries),
    /// so it must be idempotent: a state is written only when it is newer
    /// ([`Raffle::version`]) than the stored one, and for the manifest the
    /// raffle was opened with. Returns whether it was written.
    fn save_raffle<'a>(&'a self, state: &'a Raffle) -> BoxFuture<'a, anyhow::Result<bool>>;
}

impl<T: Store + ?Sized> Store for Arc<T> {
    fn save_raffle<'a>(&'a self, state: &'a Raffle) -> BoxFuture<'a, anyhow::Result<bool>> {
        (**self).save_raffle(state)
    }
}

/// Keeps every raffle it is handed, in memory (tests, local experiments).
#[derive(Clone, Default)]
pub struct MemoryStore {
    states: Arc<Mutex<Vec<Raffle>>>,
}

impl MemoryStore {
    /// Every state received so far, in order (retries and refused ones
    /// included).
    pub fn states(&self) -> Vec<Raffle> {
        self.states.lock().expect("store lock").clone()
    }

    /// The latest stored state of raffle `raffle_id`.
    pub fn latest(&self, raffle_id: &str) -> Option<Raffle> {
        latest(&self.states.lock().expect("store lock"), raffle_id).cloned()
    }
}

/// The newest of `states` for raffle `raffle_id` that was opened with the
/// raffle's first manifest: what a store would hold.
fn latest<'a>(states: &'a [Raffle], raffle_id: &str) -> Option<&'a Raffle> {
    let mut same = states.iter().filter(|s| s.id() == raffle_id);
    let opened_with = *same.next()?.manifest_hash();
    states
        .iter()
        .filter(|s| s.id() == raffle_id && *s.manifest_hash() == opened_with)
        .max_by_key(|s| s.version())
}

impl Store for MemoryStore {
    /// Keeps every state; writes it (as [`latest`](Self::latest) sees it)
    /// only when it is newer, and for the manifest the raffle's first state
    /// had: a stored raffle belongs to the entrant list it was opened with.
    fn save_raffle<'a>(&'a self, state: &'a Raffle) -> BoxFuture<'a, anyhow::Result<bool>> {
        let mut states = self.states.lock().expect("store lock");
        let written = match latest(&states, state.id().as_str()) {
            None => true,
            Some(stored) => {
                stored.manifest_hash() == state.manifest_hash()
                    && stored.version() < state.version()
            }
        };
        states.push(state.clone());
        Box::pin(async move { Ok(written) })
    }
}

#[cfg(test)]
mod tests {
    use rafi_raffle::{BeaconSource, Decision, Manifest, ManifestItem};

    use super::*;

    fn raffle(entrant: &str) -> Raffle {
        Raffle::open(&Manifest::new(
            "d1",
            BeaconSource::QUICKNET,
            vec![entrant.into()],
            vec![ManifestItem {
                id: "i1".into(),
                name: "Book".into(),
            }],
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn writes_newer_states_of_the_opened_manifest_only() {
        let store = MemoryStore::default();
        let mut first = raffle("a");
        assert!(store.save_raffle(&first).await.unwrap());
        assert!(!store.save_raffle(&first).await.unwrap(), "a retry");
        first.decide(&Decision::Finish).unwrap();
        assert!(store.save_raffle(&first).await.unwrap());

        let mut other = raffle("b");
        for decision in [Decision::Void { item: "i1".into() }, Decision::Finish] {
            other.decide(&decision).unwrap();
        }
        assert!(other.version() > first.version());
        assert!(
            !store.save_raffle(&other).await.unwrap(),
            "another entrant list"
        );
    }
}
