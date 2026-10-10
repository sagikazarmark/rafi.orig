//! Telling clients that a raffle changed.

use std::sync::{Arc, Mutex};

use rafi_raffle::RaffleId;

use crate::BoxFuture;

/// Tells clients a raffle changed, after it is stored. Engines name
/// raffles; what clients follow is the host's to say (`rafi_platform::Notify`
/// maps a raffle to a subject). Best effort: failures are logged, never
/// returned (clients re-read on their own).
///
/// Engines wait for it before the raffle's next command, and impose no
/// deadline (Workers have no timer an engine could share), so an
/// implementation bounds its own time: a few attempts, then drop.
pub trait RaffleNotifier: Send + Sync + 'static {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()>;
}

impl<T: RaffleNotifier + ?Sized> RaffleNotifier for Arc<T> {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        (**self).changed(raffle)
    }
}

/// Records every raffle it is told about, in order (tests).
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    changed: Mutex<Vec<RaffleId>>,
}

impl RecordingNotifier {
    /// The raffles notified so far, in order.
    pub fn notified(&self) -> Vec<RaffleId> {
        self.changed.lock().expect("recording lock").clone()
    }
}

impl RaffleNotifier for RecordingNotifier {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        self.changed
            .lock()
            .expect("recording lock")
            .push(raffle.clone());
        Box::pin(async {})
    }
}
