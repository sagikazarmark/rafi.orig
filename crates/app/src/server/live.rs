//! Telling the live channel what changed.
//!
//! Phones follow a raffle's participant view. Where the server runs a live
//! channel per raffle (Cloudflare Workers: a Durable Object), the app
//! [nudges](Live::nudge) it after every write that changes what phones see,
//! and [destroys](Live::destroy) it when live updates end. The channel then
//! reads the database itself; nothing is pushed from here.
//!
//! The native server has no channel: phones poll `/api/r/{slug}/participant`
//! and screens poll the full view, so [`Polling`] does nothing.

use std::{future::Future, pin::Pin};

/// A boxed future, so [`Live`] can be a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The raffles' live channels.
pub trait Live: Send + Sync + 'static {
    /// Whether this server runs live channels (phones and screens subscribe),
    /// or phones and screens only poll.
    fn has_channel(&self) -> bool;

    /// What phones see of `raffle_id` may have changed: the channel re-reads
    /// it. Best effort: failures are logged, never returned, since the channel
    /// re-reads on its own every few seconds.
    fn nudge<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()>;

    /// Ends the raffle's channel: it publishes the final view, closes every
    /// socket and deletes its storage. Idempotent.
    fn destroy<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;
}

/// No live channel: phones and screens poll.
#[derive(Debug, Clone, Copy, Default)]
pub struct Polling;

impl Live for Polling {
    fn has_channel(&self) -> bool {
        false
    }

    fn nudge<'a>(&'a self, _raffle_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn destroy<'a>(&'a self, _raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Records every call (tests).
#[cfg(test)]
#[derive(Debug, Default)]
pub struct Recording {
    pub calls: std::sync::Mutex<Vec<(&'static str, String)>>,
}

#[cfg(test)]
impl Recording {
    pub fn calls(&self, kind: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, id)| id.clone())
            .collect()
    }

    fn record(&self, kind: &'static str, raffle_id: &str) {
        self.calls
            .lock()
            .unwrap()
            .push((kind, raffle_id.to_owned()));
    }
}

#[cfg(test)]
impl Live for Recording {
    fn has_channel(&self) -> bool {
        true
    }

    fn nudge<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()> {
        self.record("nudge", raffle_id);
        Box::pin(async {})
    }

    fn destroy<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.record("destroy", raffle_id);
        Box::pin(async { Ok(()) })
    }
}
