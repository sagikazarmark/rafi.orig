//! Telling clients that a raffle changed.

use std::sync::{Arc, Mutex};

use rafi_live::LiveTransport;

use crate::BoxFuture;

/// Pushes live updates of raffles to clients, however the deployment does it.
///
/// Writers call [`notify`](Notifier::notify) after every write that changes
/// what phones see; the implementation reads the raffle back itself (nothing
/// is pushed from the caller). Where there's no push at all, clients poll and
/// [`Polling`] does nothing.
pub trait Notifier: Send + Sync + 'static {
    /// What phones see of `raffle_id` may have changed. Best effort: the
    /// implementation logs failures and never returns them, since clients
    /// recover on their own (re-reads, polling).
    fn notify<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()>;

    /// Ends the raffle's live updates: clients get the final view, then are
    /// disconnected, and anything kept for the raffle is deleted. Set
    /// `live_ended_at` first, so it can't come back. Idempotent.
    fn end<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;

    /// How clients should follow `raffle_id` while its live updates run.
    fn transport(&self, raffle_id: &str) -> LiveTransport;
}

impl<T: Notifier + ?Sized> Notifier for Arc<T> {
    fn notify<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()> {
        (**self).notify(raffle_id)
    }

    fn end<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        (**self).end(raffle_id)
    }

    fn transport(&self, raffle_id: &str) -> LiveTransport {
        (**self).transport(raffle_id)
    }
}

/// No push: clients poll every [`every_ms`](Self::every_ms).
#[derive(Debug, Clone, Copy)]
pub struct Polling {
    pub every_ms: u32,
}

impl Default for Polling {
    fn default() -> Self {
        Self {
            every_ms: rafi_live::DEFAULT_POLL_MS,
        }
    }
}

impl Notifier for Polling {
    fn notify<'a>(&'a self, _raffle_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn end<'a>(&'a self, _raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn transport(&self, _raffle_id: &str) -> LiveTransport {
        LiveTransport::Poll {
            every_ms: self.every_ms,
        }
    }
}

/// Records every call, and tells clients to subscribe over partyline (tests).
#[derive(Debug, Default)]
pub struct Recording {
    calls: Mutex<Vec<(&'static str, String)>>,
}

impl Recording {
    /// The raffles notified so far, in order.
    pub fn notified(&self) -> Vec<String> {
        self.calls("notify")
    }

    /// The raffles whose live updates were ended, in order.
    pub fn ended(&self) -> Vec<String> {
        self.calls("end")
    }

    fn calls(&self, kind: &str) -> Vec<String> {
        self.calls
            .lock()
            .expect("recording lock")
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, id)| id.clone())
            .collect()
    }

    fn record(&self, kind: &'static str, raffle_id: &str) {
        self.calls
            .lock()
            .expect("recording lock")
            .push((kind, raffle_id.to_owned()));
    }
}

impl Notifier for Recording {
    fn notify<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()> {
        self.record("notify", raffle_id);
        Box::pin(async {})
    }

    fn end<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.record("end", raffle_id);
        Box::pin(async { Ok(()) })
    }

    fn transport(&self, _raffle_id: &str) -> LiveTransport {
        LiveTransport::Partyline { base_url: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recording_records() {
        let r = Arc::new(Recording::default());
        r.notify("a").await;
        r.end("b").await.unwrap();
        r.notify("c").await;
        assert_eq!(r.notified(), ["a", "c"]);
        assert_eq!(r.ended(), ["b"]);
        assert_eq!(
            Polling::default().transport("a"),
            LiveTransport::poll(),
            "polling tells clients to poll"
        );
    }
}
