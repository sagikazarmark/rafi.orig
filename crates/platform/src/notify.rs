//! A deployment's [`Notifier`] as the drawing engine's notifier.

use rafi_drawing::DrawingId;
use rafi_engine::DrawingNotifier;

use crate::{BoxFuture, Notifier};

/// Notifies a [`Notifier`] of the subject each drawing belongs to: the
/// host, which knows what its drawings are for, names it (for rafi,
/// `rafi_lifecycle::drawing::raffle_id`).
pub struct Notify<N> {
    notifier: N,
    subject: fn(&DrawingId) -> &str,
}

impl<N: Notifier> Notify<N> {
    pub fn new(notifier: N, subject: fn(&DrawingId) -> &str) -> Self {
        Self { notifier, subject }
    }
}

impl<N: Notifier> DrawingNotifier for Notify<N> {
    fn changed<'a>(&'a self, drawing: &'a DrawingId) -> BoxFuture<'a, ()> {
        self.notifier.notify((self.subject)(drawing))
    }
}
