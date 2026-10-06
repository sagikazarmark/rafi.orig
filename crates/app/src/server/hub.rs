//! In-memory fan-out of [`RaffleView`]s to SSE subscribers.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::sync::watch;

use crate::model::RaffleView;

pub type ViewRx = watch::Receiver<Option<Arc<RaffleView>>>;

type Channels = Arc<Mutex<HashMap<String, watch::Sender<Option<Arc<RaffleView>>>>>>;

#[derive(Clone, Default)]
pub struct Hub {
    channels: Channels,
    version: Arc<AtomicU64>,
}

impl Hub {
    pub fn next_version(&self) -> u64 {
        self.version.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn sender(&self, raffle_id: &str) -> watch::Sender<Option<Arc<RaffleView>>> {
        let mut channels = self.channels.lock().expect("hub lock");
        channels
            .entry(raffle_id.to_owned())
            .or_insert_with(|| watch::channel(None).0)
            .clone()
    }

    /// Subscribes to a raffle; the receiver yields the latest view first.
    pub fn subscribe(&self, raffle_id: &str) -> ViewRx {
        self.sender(raffle_id).subscribe()
    }

    pub fn current(&self, raffle_id: &str) -> Option<Arc<RaffleView>> {
        self.sender(raffle_id).borrow().clone()
    }

    /// Publishes a view unless a newer one has already been published.
    pub fn publish(&self, view: RaffleView) {
        let tx = self.sender(&view.raffle_id);
        tx.send_if_modified(|cur| {
            if cur.as_ref().is_some_and(|c| c.v >= view.v) {
                return false;
            }
            *cur = Some(Arc::new(view));
            true
        });
    }

    /// Drops channels nobody listens to (memory hygiene for long-running servers).
    pub fn gc(&self) {
        let mut channels = self.channels.lock().expect("hub lock");
        channels.retain(|_, tx| tx.receiver_count() > 0);
    }
}
