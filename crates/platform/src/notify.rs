//! A deployment's [`Notifier`] as the drawing engine's notifier.

use rafi_engine::DrawingNotifier;
use rafi_raffle::RaffleId;

use crate::{BoxFuture, Notifier};

/// Notifies a [`Notifier`] of the subject each drawing belongs to: the
/// host, which knows what its drawings are for, names it (for rafi,
/// `rafi_lifecycle::drawing::raffle_id`).
pub struct Notify<N> {
    notifier: N,
    subject: fn(&RaffleId) -> &str,
}

impl<N: Notifier> Notify<N> {
    pub fn new(notifier: N, subject: fn(&RaffleId) -> &str) -> Self {
        Self { notifier, subject }
    }
}

impl<N: Notifier> DrawingNotifier for Notify<N> {
    fn changed<'a>(&'a self, drawing: &'a RaffleId) -> BoxFuture<'a, ()> {
        self.notifier.notify((self.subject)(drawing))
    }
}
