//! Where the draw engine writes draw states.

use std::sync::{Arc, Mutex};

use rafi_drawing::drawing::DrawState;

use crate::BoxFuture;

/// The draw engine's only write: a raffle's draw state, after every change.
pub trait DrawStore: Send + Sync + 'static {
    /// Stores `state`. Called at least once per change (the engine retries),
    /// so it must be idempotent: a state is written only when it is newer
    /// ([`DrawState::version`]) than the stored one, and for the entrant list
    /// the raffle was frozen with. Returns whether it was written.
    fn save_draw_state<'a>(&'a self, state: &'a DrawState) -> BoxFuture<'a, anyhow::Result<bool>>;
}

impl<T: DrawStore + ?Sized> DrawStore for Arc<T> {
    fn save_draw_state<'a>(&'a self, state: &'a DrawState) -> BoxFuture<'a, anyhow::Result<bool>> {
        (**self).save_draw_state(state)
    }
}

/// Keeps every draw state it is handed, in memory (tests, local experiments).
#[derive(Clone, Default)]
pub struct MemoryDrawStore {
    states: Arc<Mutex<Vec<DrawState>>>,
}

impl MemoryDrawStore {
    /// Every state received so far, in order (retries included).
    pub fn states(&self) -> Vec<DrawState> {
        self.states.lock().expect("store lock").clone()
    }

    /// The latest state of `raffle_id`.
    pub fn latest(&self, raffle_id: &str) -> Option<DrawState> {
        self.states()
            .into_iter()
            .filter(|s| s.raffle_id == raffle_id)
            .max_by_key(|s| s.version)
    }
}

impl DrawStore for MemoryDrawStore {
    fn save_draw_state<'a>(&'a self, state: &'a DrawState) -> BoxFuture<'a, anyhow::Result<bool>> {
        let newer = self
            .latest(&state.raffle_id)
            .is_none_or(|s| s.version < state.version);
        self.states.lock().expect("store lock").push(state.clone());
        Box::pin(async move { Ok(newer) })
    }
}
