//! A deployment's live updates: the notifier writers tell, and how clients
//! follow each raffle meanwhile.

use std::{fmt, sync::Arc};

use rafi_live::Follow;
use rafi_platform::{BoxFuture, Notifier, Polling};

/// How a deployment pushes raffles' changes to clients, and how clients
/// follow them. Built from one notifier and a function of that notifier, so
/// clients are never told to follow a channel this deployment doesn't feed.
#[derive(Clone)]
pub struct LiveUpdates {
    notifier: Arc<dyn Notifier>,
    follow: Arc<dyn Fn(&str) -> Follow + Send + Sync>,
}

impl LiveUpdates {
    /// Live updates through `notifier`; clients follow a raffle as `follow`
    /// says for it (a Mercure topic, say, names the raffle).
    pub fn new<N: Notifier>(
        notifier: Arc<N>,
        follow: impl Fn(&N, &str) -> Follow + Send + Sync + 'static,
    ) -> Self {
        let n = notifier.clone();
        Self {
            notifier,
            follow: Arc::new(move |raffle_id| follow(&n, raffle_id)),
        }
    }

    /// No push: clients poll ([`Follow::poll`]).
    pub fn polling() -> Self {
        Self::new(Arc::new(Polling), |_, _| Follow::poll())
    }

    /// What phones see of `raffle_id` may have changed ([`Notifier::notify`]).
    pub fn notify<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()> {
        self.notifier.notify(raffle_id)
    }

    /// Ends the raffle's live updates ([`Notifier::end`]).
    pub fn end<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.notifier.end(raffle_id)
    }

    /// How clients follow `raffle_id` while its live updates run.
    pub fn follow(&self, raffle_id: &str) -> Follow {
        (self.follow)(raffle_id)
    }
}

impl fmt::Debug for LiveUpdates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveUpdates").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use rafi_platform::Recording;

    use super::*;

    #[tokio::test]
    async fn the_notifier_and_how_to_follow() {
        let recording = Arc::new(Recording::default());
        let live = LiveUpdates::new(recording.clone(), |_, id| Follow::Sse {
            url: format!("/hub?topic={id}"),
        });
        live.notify("a").await;
        live.end("b").await.unwrap();
        assert_eq!(recording.notified(), ["a"]);
        assert_eq!(recording.ended(), ["b"]);
        assert_eq!(
            live.follow("a"),
            Follow::Sse {
                url: "/hub?topic=a".into()
            }
        );
        assert_eq!(LiveUpdates::polling().follow("a"), Follow::poll());
    }
}
