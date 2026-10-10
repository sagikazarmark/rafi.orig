//! Telling clients that a drawing changed.

use std::sync::Arc;

use rafi_drawing::DrawingId;
use rafi_platform::{BoxFuture, Notifier};

/// Tells clients a drawing changed, after it is stored. Engines name
/// drawings; what clients follow is the host's to say ([`Notify`]).
/// Best effort: failures are logged, never returned (clients re-read on their
/// own).
pub trait DrawingNotifier: Send + Sync + 'static {
    fn changed<'a>(&'a self, drawing: &'a DrawingId) -> BoxFuture<'a, ()>;
}

impl<T: DrawingNotifier + ?Sized> DrawingNotifier for Arc<T> {
    fn changed<'a>(&'a self, drawing: &'a DrawingId) -> BoxFuture<'a, ()> {
        (**self).changed(drawing)
    }
}

/// Notifies a [`Notifier`] of the subject each drawing belongs to: the
/// host, which knows what its drawings are for, names it (for rafi,
/// `rafi_raffle::drawing::raffle_id`).
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
