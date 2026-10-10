//! A deployment's [`Notifier`] as the raffle engine's notifier.

use rafi_engine::RaffleNotifier;
use rafi_raffle::RaffleId;

use crate::{BoxFuture, Notifier};

/// Notifies a [`Notifier`] of the subject each raffle's clients follow: the
/// host names it (for rafi, the raffle's own ID: `RaffleId::as_str`).
pub struct Notify<N> {
    notifier: N,
    subject: fn(&RaffleId) -> &str,
}

impl<N: Notifier> Notify<N> {
    pub fn new(notifier: N, subject: fn(&RaffleId) -> &str) -> Self {
        Self { notifier, subject }
    }
}

impl<N: Notifier> RaffleNotifier for Notify<N> {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        self.notifier.notify((self.subject)(raffle))
    }
}
