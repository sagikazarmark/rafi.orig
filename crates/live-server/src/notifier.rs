//! Pushing raffles' changes to clients.

use std::sync::{Arc, Mutex};

use rafi_engine::{BoxFuture, Observer};
use rafi_raffle::RaffleId;

/// Pushes live updates of raffles to clients, however the deployment does it.
///
/// It is the raffle engine's [`Observer`]: the engine tells it after every
/// write, and the app's writers do after every write that changes what
/// phones see ([`Observer::changed`]). The implementation reads the raffle
/// back itself (nothing is pushed from the caller), logs its failures and
/// never returns them, since clients recover on their own (re-reads,
/// polling). Where there's no push at all, clients poll and [`Polling`] does
/// nothing. How clients follow a raffle is the app's to tell them, from the
/// deployment's notifier (`rafi_app_core::LiveUpdates`).
pub trait Notifier: Observer {
    /// Ends the raffle's live updates: clients get the final view, then are
    /// disconnected, and anything kept for the raffle is deleted. Set
    /// `live_ended_at` first, so it can't come back. Idempotent.
    fn end<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, anyhow::Result<()>>;
}

impl<T: Notifier + ?Sized> Notifier for Arc<T> {
    fn end<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, anyhow::Result<()>> {
        (**self).end(raffle)
    }
}

/// No push: clients poll.
#[derive(Debug, Default, Clone, Copy)]
pub struct Polling;

impl Observer for Polling {
    fn changed<'a>(&'a self, _raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

impl Notifier for Polling {
    fn end<'a>(&'a self, _raffle: &'a RaffleId) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Records every call (tests).
#[derive(Debug, Default)]
pub struct Recording {
    calls: Mutex<Vec<(&'static str, RaffleId)>>,
}

impl Recording {
    /// The raffles notified so far, in order.
    pub fn notified(&self) -> Vec<RaffleId> {
        self.calls("changed")
    }

    /// The raffles whose live updates were ended, in order.
    pub fn ended(&self) -> Vec<RaffleId> {
        self.calls("end")
    }

    fn calls(&self, kind: &str) -> Vec<RaffleId> {
        self.calls
            .lock()
            .expect("recording lock")
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, id)| id.clone())
            .collect()
    }

    fn record(&self, kind: &'static str, raffle: &RaffleId) {
        self.calls
            .lock()
            .expect("recording lock")
            .push((kind, raffle.clone()));
    }
}

impl Observer for Recording {
    fn changed<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, ()> {
        self.record("changed", raffle);
        Box::pin(async {})
    }
}

impl Notifier for Recording {
    fn end<'a>(&'a self, raffle: &'a RaffleId) -> BoxFuture<'a, anyhow::Result<()>> {
        self.record("end", raffle);
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recording_records() {
        let r = Arc::new(Recording::default());
        let id = |s: &str| RaffleId::new(s);
        r.changed(&id("a")).await;
        r.end(&id("b")).await.unwrap();
        r.changed(&id("c")).await;
        assert_eq!(r.notified(), [id("a"), id("c")]);
        assert_eq!(r.ended(), [id("b")]);
    }
}
