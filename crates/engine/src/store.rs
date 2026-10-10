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
    /// Every state received so far, in order (retries included).
    pub fn states(&self) -> Vec<Drawing> {
        self.states.lock().expect("store lock").clone()
    }

    /// The latest state of drawing `drawing_id`.
    pub fn latest(&self, drawing_id: &str) -> Option<Drawing> {
        self.states()
            .into_iter()
            .filter(|s| s.id() == drawing_id)
            .max_by_key(Drawing::version)
    }
}

impl DrawStore for MemoryDrawStore {
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>> {
        let newer = self
            .latest(state.id().as_str())
            .is_none_or(|s| s.version() < state.version());
        self.states.lock().expect("store lock").push(state.clone());
        Box::pin(async move { Ok(newer) })
    }
}
