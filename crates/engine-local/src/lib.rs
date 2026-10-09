//! An in-process draw engine, for tests and development only.
//!
//! [`LocalEngine`] keeps the [`DrawingEngine`] contract the way the `Drawing`
//! Restate virtual object does, without its durability:
//!
//! - one queue per drawing, worked off in arrival order by one task, so
//!   commands for a drawing apply one at a time and a decision sent while a
//!   draw waits for its beacon applies after the draw settles;
//! - draws are accepted at once (queued) and settle in the background;
//! - every command ID is applied once;
//! - every new state is written to the [`DrawStore`] (retried), then clients
//!   are notified, best effort;
//! - the drawing is forgotten once it finishes.
//!
//! All decisions are `rafi_drawing`'s ([`Drawing::open_or_check`],
//! [`Drawing::announce`], [`Drawing::settle`], [`Drawing::decide`]); the
//! committed round comes from [`DrandClient::target_round`]. This engine only
//! does the I/O, with an injectable clock.
//!
//! It is never a production engine (nothing survives a restart, and two
//! engines owning one drawing could announce one draw on two rounds): only
//! dev-dependencies use this crate.
//!
//! With the `conformance` feature, [`conformance`] is the scenario suite every
//! engine must pass.

#![cfg(not(target_arch = "wasm32"))]

#[cfg(feature = "conformance")]
pub mod conformance;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rafi_drand::DrandClient;
use rafi_drawing::{
    BeaconFetch, BeaconSource, Decision, DrawRequest, Drawing, Error as DrawingError, ErrorKind,
    Manifest,
};
use rafi_platform::{
    BoxFuture, DrawStore, DrawingEngine, EngineError, EngineInfo, NOT_OPEN, Notifier,
};
use tokio::sync::{mpsc, watch};

/// Unix seconds now.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// How a job ended: refusals keep their kind and message.
type Outcome = Result<(), (ErrorKind, String)>;

/// The draw engine, in this process. Clones share the engine.
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
    queues: Mutex<HashMap<String, mpsc::UnboundedSender<Job>>>,
    /// The outcome of every call by idempotency key, set once applied.
    seen: Mutex<HashMap<String, watch::Sender<Option<Outcome>>>>,
}

/// What a raffle's task needs.
struct Io {
    drand: DrandClient,
    store: Arc<dyn DrawStore>,
    notifier: Arc<dyn Notifier>,
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
    /// through `notifier`. System clock; no deadline.
    pub fn new(drand: DrandClient, store: impl DrawStore, notifier: impl Notifier) -> Self {
        Self::builder(drand, store, notifier).build()
    }

    /// [`LocalEngine::new`], configured further.
    pub fn builder(drand: DrandClient, store: impl DrawStore, notifier: impl Notifier) -> Builder {
        Builder {
            io: Io {
                drand,
                store: Arc::new(store),
                notifier: Arc::new(notifier),
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
    fn submit(&self, raffle_id: &str, key: String, work: Work) -> watch::Receiver<Option<Outcome>> {
        let mut seen = self.shared.seen.lock().expect("seen lock");
        if let Some(done) = seen.get(&key) {
            return done.subscribe();
        }
        let (done, rx) = watch::channel(None);
        seen.insert(key, done.clone());
        drop(seen);
        let job = Job { work, done };
        let mut queues = self.shared.queues.lock().expect("queues lock");
        let queue = queues.entry(raffle_id.to_owned()).or_insert_with(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(run(self.shared.io.clone(), raffle_id.to_owned(), rx));
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

impl DrawingEngine for LocalEngine {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        let beacon = BeaconSource::Drand(self.shared.io.drand.chain_hash());
        Box::pin(async move { Ok(EngineInfo { beacon }) })
    }

    fn open<'a>(&'a self, doc: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            let key = format!("{}/{}", doc.drawing, doc.open_key());
            let rx = self.submit(doc.drawing.as_str(), key, Work::Open(doc.clone()));
            self.outcome(rx).await
        })
    }

    fn draw<'a>(
        &'a self,
        drawing_id: &'a str,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            // Accepted once queued: progress shows in the projection.
            let key = format!("{drawing_id}/{command_id}");
            self.submit(drawing_id, key, Work::Draw(request.clone()));
            Ok(())
        })
    }

    fn decide<'a>(
        &'a self,
        drawing_id: &'a str,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            let key = format!("{drawing_id}/{command_id}");
            let rx = self.submit(drawing_id, key, Work::Decide(decision.clone()));
            self.outcome(rx).await
        })
    }
}

/// A drawing's task: applies its jobs one at a time, in order.
async fn run(io: Arc<Io>, raffle_id: String, mut jobs: mpsc::UnboundedReceiver<Job>) {
    let mut state: Option<Drawing> = None;
    while let Some(job) = jobs.recv().await {
        let outcome = io.apply(&mut state, job.work).await;
        if let Err((_, message)) = &outcome {
            tracing::debug!(raffle_id, message, "refused");
        }
        job.done.send_replace(Some(outcome));
    }
}

/// A refusal by the draw rules, with its kind.
fn refused(e: DrawingError) -> (ErrorKind, String) {
    (e.kind(), e.to_string())
}

impl Io {
    async fn apply(&self, state: &mut Option<Drawing>, work: Work) -> Outcome {
        let beacon = BeaconSource::Drand(self.drand.chain_hash());
        let not_open = || (ErrorKind::NotFound, NOT_OPEN.to_owned());
        match work {
            Work::Open(doc) => {
                if let Some(fresh) =
                    Drawing::open_or_check(state.as_ref(), &doc, &beacon).map_err(refused)?
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
                    // The app keeps the record; the engine forgets the drawing.
                    *state = None;
                }
                Ok(())
            }
        }
    }

    /// Writes the state, retried with backoff (the database is the record),
    /// then tells clients, best effort.
    async fn project(&self, state: &Drawing) {
        let mut delay = Duration::from_millis(20);
        for attempt in 1.. {
            match self.store.save_drawing(state).await {
                Ok(_) => break,
                Err(e) if attempt < 10 => {
                    tracing::warn!(raffle_id = %state.id(), attempt, error = %e, "projecting failed; retrying");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
                Err(e) => {
                    tracing::error!(raffle_id = %state.id(), error = %e, "projecting failed; giving up");
                    return;
                }
            }
        }
        self.notifier.notify(state.id().as_str()).await;
    }
}

#[cfg(test)]
mod tests {
    use rafi_drand::DrandConfig;
    use rafi_drawing::{ItemRef, ItemStatus};
    use rafi_platform::{MemoryDrawStore, Recording};

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
            vec![ItemRef {
                id: "u-1".into(),
                name: "Prize".into(),
            }],
        )
    }

    async fn projected(store: &MemoryDrawStore, f: impl Fn(&Drawing) -> bool) -> Drawing {
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
    async fn a_drawing_from_opening_to_finish() {
        let drand = mock_drand();
        let doc = doc(&drand);
        let store = MemoryDrawStore::default();
        let notifier = Arc::new(Recording::default());
        let engine = LocalEngine::new(drand, store.clone(), notifier.clone());

        assert_eq!(engine.info().await.unwrap().beacon, doc.beacon);
        engine.open(&doc).await.unwrap();
        engine.open(&doc).await.unwrap();
        let draw = DrawRequest { item: "u-1".into() };
        engine.draw("r1", &draw, "d1").await.unwrap();
        projected(&store, |s| {
            s.item(&"u-1".into()).unwrap().status() == ItemStatus::Drawn
        })
        .await;
        engine.decide("r1", &Decision::Finish, "f1").await.unwrap();
        let finished = projected(&store, |s| s.is_finished()).await;
        assert!(!notifier.notified().is_empty());

        // Forgotten: further commands are refused as not found.
        let err = engine
            .decide("r1", &Decision::Finish, "f2")
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
    }
}
