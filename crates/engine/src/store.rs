//! Where the drawing engine writes drawings.

use std::sync::{Arc, Mutex};

use rafi_drawing::Drawing;

use rafi_platform::BoxFuture;

/// The drawing engine's only write: a drawing, after every change.
pub trait DrawStore: Send + Sync + 'static {
    /// Stores `state`. Called at least once per change (the engine retries),
    /// so it must be idempotent: a state is written only when it is newer
    /// ([`Drawing::version`]) than the stored one, and for the manifest the
    /// drawing was opened with. Returns whether it was written.
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>>;
}

impl<T: DrawStore + ?Sized> DrawStore for Arc<T> {
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>> {
        (**self).save_drawing(state)
    }
}

/// Keeps every drawing it is handed, in memory (tests, local experiments).
#[derive(Clone, Default)]
pub struct MemoryDrawStore {
    states: Arc<Mutex<Vec<Drawing>>>,
}

impl MemoryDrawStore {
    /// Every state received so far, in order (retries and refused ones
    /// included).
    pub fn states(&self) -> Vec<Drawing> {
        self.states.lock().expect("store lock").clone()
    }

    /// The latest stored state of drawing `drawing_id`.
    pub fn latest(&self, drawing_id: &str) -> Option<Drawing> {
        latest(&self.states.lock().expect("store lock"), drawing_id).cloned()
    }
}

/// The newest of `states` for drawing `drawing_id` that was opened with the
/// drawing's first manifest: what a store would hold.
fn latest<'a>(states: &'a [Drawing], drawing_id: &str) -> Option<&'a Drawing> {
    let mut same = states.iter().filter(|s| s.id() == drawing_id);
    let opened_with = *same.next()?.manifest_hash();
    states
        .iter()
        .filter(|s| s.id() == drawing_id && *s.manifest_hash() == opened_with)
        .max_by_key(|s| s.version())
}

impl DrawStore for MemoryDrawStore {
    /// Keeps every state; writes it (as [`latest`](Self::latest) sees it)
    /// only when it is newer, and for the manifest the drawing's first state
    /// had: a stored drawing belongs to the entrant list it was opened with.
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>> {
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
    use rafi_drawing::{BeaconSource, Decision, Manifest, ManifestItem};

    use super::*;

    fn drawing(entrant: &str) -> Drawing {
        Drawing::open(&Manifest::new(
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
        let store = MemoryDrawStore::default();
        let mut first = drawing("a");
        assert!(store.save_drawing(&first).await.unwrap());
        assert!(!store.save_drawing(&first).await.unwrap(), "a retry");
        first.decide(&Decision::Finish).unwrap();
        assert!(store.save_drawing(&first).await.unwrap());

        let mut other = drawing("b");
        for decision in [Decision::Void { item: "i1".into() }, Decision::Finish] {
            other.decide(&decision).unwrap();
        }
        assert!(other.version() > first.version());
        assert!(
            !store.save_drawing(&other).await.unwrap(),
            "another entrant list"
        );
    }
}
