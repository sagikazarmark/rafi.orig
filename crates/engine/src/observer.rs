//! Who hears that a raffle changed.

use std::sync::{Arc, Mutex};

use rafi_raffle::RaffleId;

use crate::BoxFuture;

/// Told each time an engine has stored a raffle, so the host can tell
/// clients it changed (`rafi_platform::Notify` maps a raffle to what they
/// follow). Best effort: failures are logged, never returned (clients
/// re-read on their own).
///
/// Engines wait for it before the raffle's next command, and impose no
/// deadline (Workers have no timer an engine could share), so an
/// implementation bounds its own time: a few attempts, then drop.
pub trait Observer: Send + Sync + 'static {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()>;
}

impl<T: Observer + ?Sized> Observer for Arc<T> {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        (**self).changed(raffle)
    }
}

/// Records every raffle it is told about, in order (tests).
#[derive(Debug, Default)]
pub struct RecordingObserver {
    changed: Mutex<Vec<RaffleId>>,
}

impl RecordingObserver {
    /// The raffles notified so far, in order.
    pub fn notified(&self) -> Vec<RaffleId> {
        self.changed.lock().expect("recording lock").clone()
    }
}

impl Observer for RecordingObserver {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        self.changed
            .lock()
            .expect("recording lock")
            .push(raffle.clone());
        Box::pin(async {})
    }
}
