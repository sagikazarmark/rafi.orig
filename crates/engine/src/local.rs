//! An in-process raffle engine, for tests and development only.
//!
//! [`LocalEngine`] keeps the [`Engine`] contract the way the `Raffle`
//! Restate virtual object does, without its durability:
//!
//! - one queue per raffle, worked off in arrival order by one task, so
//!   commands for a raffle apply one at a time and a decision sent while a
//!   draw waits for its beacon applies after the draw settles;
//! - draws are accepted at once (queued) and settle in the background;
//! - every command ID is applied once;
//! - every new state is written to the [`Store`] (retried), then clients
//!   are notified, best effort;
//! - the raffle is forgotten once it finishes.
//!
//! All decisions are `rafi_raffle`'s ([`Raffle::open_or_check`],
//! [`Raffle::announce`], [`Raffle::settle`], [`Raffle::decide`]); the
//! committed round comes from [`DrandClient::target_round`]. This engine only
//! does the I/O, with an injectable clock.
//!
//! It is never a production engine (nothing survives a restart, and two
//! engines owning one raffle could announce one draw on two rounds): only
//! dev-dependencies enable the `local` feature.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::BoxFuture;
use crate::drand::DrandClient;
use rafi_raffle::{
    BeaconFetch, BeaconSource, Decision, DrawRequest, Error as RaffleError, ErrorKind, Manifest,
    Raffle, RaffleId,
};

use crate::{Engine, EngineError, EngineInfo, NOT_OPEN, Observer, Store};
use tokio::sync::{mpsc, watch};

/// Unix seconds now.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// How a job ended: refusals keep their kind and message.
type Outcome = Result<(), (ErrorKind, String)>;

/// The raffle engine, in this process. Clones share the engine.
#[derive(Clone)]
pub struct LocalEngine {
    shared: Arc<Shared>,
    /// How long a synchronous call waits before it counts as
    /// [`EngineError::Pending`]; `None` waits for the outcome.
    deadline: Option<Duration>,
}

struct Shared {
    io: Arc<Io>,
    /// Each raffle's queue; its task ends when the engine is dropped.
    queues: Mutex<HashMap<RaffleId, mpsc::UnboundedSender<Job>>>,
    /// The outcome of every call by idempotency key, set once applied.
    seen: Mutex<HashMap<String, watch::Sender<Option<Outcome>>>>,
}

/// What a raffle's task needs.
struct Io {
    drand: DrandClient,
    store: Arc<dyn Store>,
    observer: Arc<dyn Observer>,
    clock: Clock,
}

enum Work {
    Open(Manifest),
    Draw(DrawRequest),
    Decide(Decision),
}

struct Job {
    work: Work,
    done: watch::Sender<Option<Outcome>>,
}

/// Configures a [`LocalEngine`].
pub struct Builder {
    io: Io,
    deadline: Option<Duration>,
}

impl Builder {
    /// Reads the time (Unix seconds) from `clock` when committing a round.
    pub fn clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.io.clock = Arc::new(clock);
        self
    }

    /// Gives up waiting for a synchronous call after `deadline`: it returns
    /// [`EngineError::Pending`] and still applies (once).
    pub fn deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn build(self) -> LocalEngine {
        LocalEngine {
            shared: Arc::new(Shared {
                io: Arc::new(self.io),
                queues: Mutex::default(),
                seen: Mutex::default(),
            }),
            deadline: self.deadline,
        }
    }
}

impl LocalEngine {
    /// Draws with `drand`, writes every state to `store` and tells clients
    /// through `observer`. System clock; no deadline.
    pub fn new(drand: DrandClient, store: impl Store, observer: impl Observer) -> Self {
        Self::builder(drand, store, observer).build()
    }

    /// [`LocalEngine::new`], configured further.
    pub fn builder(drand: DrandClient, store: impl Store, observer: impl Observer) -> Builder {
        Builder {
            io: Io {
                drand,
                store: Arc::new(store),
                observer: Arc::new(observer),
                clock: Arc::new(|| {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or_default()
                }),
            },
            deadline: None,
        }
    }

    /// The same engine, but synchronous calls give up waiting after `deadline`
    /// ([`EngineError::Pending`]; they still apply, once).
    pub fn with_deadline(&self, deadline: Option<Duration>) -> Self {
        Self {
            shared: self.shared.clone(),
            deadline,
        }
    }

    /// Queues `work` for `raffle_id` under `key`, once per key. Returns the
    /// key's outcome channel.
    fn submit(
        &self,
        raffle_id: &RaffleId,
        key: String,
        work: Work,
    ) -> watch::Receiver<Option<Outcome>> {
        let mut seen = self.shared.seen.lock().expect("seen lock");
        if let Some(done) = seen.get(&key) {
            return done.subscribe();
        }
        let (done, rx) = watch::channel(None);
        seen.insert(key, done.clone());
        drop(seen);
        let job = Job { work, done };
        let mut queues = self.shared.queues.lock().expect("queues lock");
        let queue = queues.entry(raffle_id.clone()).or_insert_with(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(run(self.shared.io.clone(), raffle_id.clone(), rx));
            tx
        });
        if let Err(mpsc::error::SendError(job)) = queue.send(job) {
            job.done.send_replace(Some(Err((
                ErrorKind::Invariant,
                "the raffle's task ended".into(),
            ))));
        }
        rx
    }

    /// Waits for a synchronous call's outcome, up to the deadline.
    async fn outcome(&self, mut rx: watch::Receiver<Option<Outcome>>) -> Result<(), EngineError> {
        let wait = async {
            match rx.wait_for(Option::is_some).await {
                Ok(outcome) => outcome.clone().expect("an outcome"),
                Err(_) => Err((ErrorKind::Invariant, "the raffle's task ended".into())),
            }
        };
        let outcome = match self.deadline {
            Some(deadline) => match tokio::time::timeout(deadline, wait).await {
                Ok(outcome) => outcome,
                Err(_) => return Err(EngineError::Pending),
            },
            None => wait.await,
        };
        outcome.map_err(|(kind, message)| EngineError::Rejected { kind, message })
    }
}

impl Engine for LocalEngine {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        let beacon = BeaconSource::Drand(self.shared.io.drand.chain_hash());
        Box::pin(async move { Ok(EngineInfo { beacon }) })
    }

    fn open<'a>(&'a self, doc: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            let key = format!("{}/{}", doc.raffle, doc.open_key());
            let rx = self.submit(&doc.raffle, key, Work::Open(doc.clone()));
            self.outcome(rx).await
        })
    }

    fn draw<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            // Accepted once queued: progress shows in the projection.
            let key = format!("{raffle_id}/{command_id}");
            self.submit(raffle_id, key, Work::Draw(request.clone()));
            Ok(())
        })
    }

    fn decide<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            let key = format!("{raffle_id}/{command_id}");
            let rx = self.submit(raffle_id, key, Work::Decide(decision.clone()));
            self.outcome(rx).await
        })
    }
}

/// A raffle's task: applies its jobs one at a time, in order. Command IDs
/// outlive the raffle, as Restate's idempotency retention does: a retried
/// finish answers as the first one did.
async fn run(io: Arc<Io>, raffle_id: RaffleId, mut jobs: mpsc::UnboundedReceiver<Job>) {
    let mut state: Option<Raffle> = None;
    while let Some(job) = jobs.recv().await {
        let outcome = io.apply(&mut state, job.work).await;
        if let Err((_, message)) = &outcome {
            tracing::debug!(%raffle_id, message, "refused");
        }
        job.done.send_replace(Some(outcome));
    }
}

/// A refusal by the draw rules, with its kind.
fn refused(e: RaffleError) -> (ErrorKind, String) {
    (e.kind(), e.to_string())
}

impl Io {
    async fn apply(&self, state: &mut Option<Raffle>, work: Work) -> Outcome {
        let beacon = BeaconSource::Drand(self.drand.chain_hash());
        let not_open = || (ErrorKind::NotFound, NOT_OPEN.to_owned());
        match work {
            Work::Open(doc) => {
                if let Some(fresh) =
                    Raffle::open_or_check(state.as_ref(), &doc, &beacon).map_err(refused)?
                {
                    self.project(&fresh).await;
                    *state = Some(fresh);
                }
                Ok(())
            }
            Work::Draw(request) => {
                let s = state.as_mut().ok_or_else(not_open)?;
                let (round, round_time) = self.drand.target_round((self.clock)());
                // A pending draw for this item (e.g. stalled) is re-entered on its round.
                let pending = s
                    .announce(&request, &beacon, round, round_time)
                    .map_err(refused)?;
                // The commitment is recorded before the beacon is asked for.
                self.project(s).await;
                let outcome = match self.drand.wait_for(pending.round).await {
                    Ok(beacon) => BeaconFetch::Verified(beacon),
                    Err(e) => BeaconFetch::Unavailable(e.to_string()),
                };
                s.settle(outcome).map_err(refused)?;
                self.project(s).await;
                Ok(())
            }
            Work::Decide(decision) => {
                let s = state.as_mut().ok_or_else(not_open)?;
                s.decide(&decision).map_err(refused)?;
                self.project(s).await;
                if s.is_finished() {
                    // The app keeps the record; the engine forgets the raffle.
                    *state = None;
                }
                Ok(())
            }
        }
    }

    /// Writes the state, retried with backoff until it succeeds (the database
    /// is the record), as Restate retries its journaled step; then tells
    /// clients, best effort.
    async fn project(&self, state: &Raffle) {
        let mut delay = Duration::from_millis(20);
        for attempt in 1u32.. {
            match self.store.save_raffle(state).await {
                Ok(_) => break,
                Err(e) => {
                    tracing::warn!(raffle_id = %state.id(), attempt, error = %e, "projecting failed; retrying");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
            }
        }
        self.observer.changed(state.id()).await;
    }
}

#[cfg(test)]
mod tests {
    use crate::drand::DrandConfig;
    use crate::{MemoryStore, RecordingObserver};
    use rafi_raffle::{ItemStatus, ManifestItem};

    use super::*;

    fn mock_drand() -> DrandClient {
        DrandClient::new(DrandConfig {
            mock: true,
            lead_secs: 0,
            timeout_secs: 5,
            ..DrandConfig::default()
        })
        .unwrap()
    }

    fn doc(drand: &DrandClient) -> Manifest {
        Manifest::new(
            "r1",
            BeaconSource::Drand(drand.chain_hash()),
            vec!["a".into(), "b".into(), "c".into()],
            vec![ManifestItem {
                id: "item-1".into(),
                name: "Item 1".into(),
            }],
        )
    }

    async fn projected(store: &MemoryStore, f: impl Fn(&Raffle) -> bool) -> Raffle {
        for _ in 0..200 {
            if let Some(s) = store.latest("r1")
                && f(&s)
            {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("not projected: {:#?}", store.latest("r1"));
    }

    #[tokio::test]
    async fn a_raffle_from_opening_to_finish() {
        let drand = mock_drand();
        let doc = doc(&drand);
        let store = MemoryStore::default();
        let observer = Arc::new(RecordingObserver::default());
        let engine = LocalEngine::new(drand, store.clone(), observer.clone());

        assert_eq!(engine.info().await.unwrap().beacon, doc.beacon);
        engine.open(&doc).await.unwrap();
        engine.open(&doc).await.unwrap();
        let draw = DrawRequest {
            item: "item-1".into(),
        };
        engine.draw(&"r1".into(), &draw, "d1").await.unwrap();
        projected(&store, |s| {
            s.item(&"item-1".into()).unwrap().status() == ItemStatus::Drawn
        })
        .await;
        engine
            .decide(&"r1".into(), &Decision::Finish, "f1")
            .await
            .unwrap();
        let finished = projected(&store, |s| s.is_finished()).await;
        assert!(!observer.notified().is_empty());

        // Forgotten: further commands are refused as not found.
        let err = engine
            .decide(&"r1".into(), &Decision::Finish, "f2")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            EngineError::Rejected {
                kind: ErrorKind::NotFound,
                ..
            }
        ));
        assert_eq!(store.latest("r1").unwrap().version(), finished.version());
        // A retried finish answers as the first one did.
        engine
            .decide(&"r1".into(), &Decision::Finish, "f1")
            .await
            .unwrap();
    }
}
