//! Telling clients that a drawing changed.

use std::sync::{Arc, Mutex};

use rafi_raffle::DrawingId;

use crate::BoxFuture;

/// Tells clients a drawing changed, after it is stored. Engines name
/// drawings; what clients follow is the host's to say (`rafi_platform::Notify`
/// maps a drawing to its raffle). Best effort: failures are logged, never
/// returned (clients re-read on their own).
///
/// Engines wait for it before the drawing's next command, and impose no
/// deadline (Workers have no timer an engine could share), so an
/// implementation bounds its own time: a few attempts, then drop.
pub trait DrawingNotifier: Send + Sync + 'static {
    fn changed<'a>(&'a self, drawing: &'a DrawingId) -> BoxFuture<'a, ()>;
}

impl<T: DrawingNotifier + ?Sized> DrawingNotifier for Arc<T> {
    fn changed<'a>(&'a self, drawing: &'a DrawingId) -> BoxFuture<'a, ()> {
        (**self).changed(drawing)
    }
}

/// Records every drawing it is told about, in order (tests).
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    changed: Mutex<Vec<DrawingId>>,
}

impl RecordingNotifier {
    /// The drawings notified so far, in order.
    pub fn notified(&self) -> Vec<DrawingId> {
        self.changed.lock().expect("recording lock").clone()
    }
}

impl DrawingNotifier for RecordingNotifier {
    fn changed<'a>(&'a self, drawing: &'a DrawingId) -> BoxFuture<'a, ()> {
        self.changed
            .lock()
            .expect("recording lock")
            .push(drawing.clone());
        Box::pin(async {})
    }
}
