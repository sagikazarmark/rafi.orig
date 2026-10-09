//! An in-process draw engine, for tests and development only.
//!
//! [`LocalEngine`] keeps the [`DrawingEngine`] contract the way the `Raffle`
//! Restate virtual object does, without its durability:
//!
//! - one queue per raffle, worked off in arrival order by one task, so commands
//!   for a raffle apply one at a time and a command sent while a draw waits
//!   for its beacon applies after the draw settles;
//! - draws are accepted at once (queued) and settle in the background;
//! - every command ID is applied once;
//! - every new state is written to the [`DrawStore`] (retried), then clients
//!   are notified, best effort;
//! - the raffle is forgotten once it finishes.
//!
//! All decisions are `rafi_drawing`'s ([`Drawing::open_or_check`],
//! [`Drawing::begin`], [`Drawing::conclude`], [`Drawing::apply`]); the
//! committed round comes from [`DrandClient::target_round`]. This engine only
//! does the I/O, with an injectable clock and randomness.
//!
//! It is never a production engine (nothing survives a restart, and two
//! engines owning one raffle could announce one draw on two rounds): only
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
    Error as DrawingError, ErrorKind,
    drawing::Drawing,
    engine::{BeaconOutcome, Command},
    manifest::Manifest,
};
use rafi_platform::{BoxFuture, DrawStore, DrawingEngine, EngineError, EngineInfo, Notifier};
use tokio::sync::{mpsc, watch};

/// Unix seconds now.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
/// 32 random bytes (an offline draw's randomness).
pub type Randomness = Arc<dyn Fn() -> [u8; 32] + Send + Sync>;

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
    randomness: Randomness,
}

enum Work {
    Open(Manifest),
    Draw { item_id: String, offline: bool },
    Command(Command),
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

    /// Takes offline draws' randomness from `randomness`.
    pub fn randomness(mut self, randomness: impl Fn() -> [u8; 32] + Send + Sync + 'static) -> Self {
        self.io.randomness = Arc::new(randomness);
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
    /// through `notifier`. System clock and randomness; no deadline.
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
                randomness: Arc::new(|| {
                    let mut buf = [0u8; 32];
                    getrandom::fill(&mut buf).expect("OS randomness is available");
                    buf
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
        let chain = self.shared.io.drand.chain_hash();
        Box::pin(async move { Ok(EngineInfo { chain }) })
    }

    fn open<'a>(&'a self, doc: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            let key = format!("{}/{}", doc.raffle_id, doc.open_key());
            let rx = self.submit(&doc.raffle_id, key, Work::Open(doc.clone()));
            self.outcome(rx).await
        })
    }

    fn command<'a>(
        &'a self,
        raffle_id: &'a str,
        cmd: &'a Command,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            let key = format!("{raffle_id}/{command_id}");
            match cmd {
                // Accepted once queued: progress shows in the projection.
                Command::Draw { item_id, offline } => {
                    let work = Work::Draw {
                        item_id: item_id.clone(),
                        offline: *offline,
                    };
                    self.submit(raffle_id, key, work);
                    Ok(())
                }
                _ => {
                    let rx = self.submit(raffle_id, key, Work::Command(cmd.clone()));
                    self.outcome(rx).await
                }
            }
        })
    }
}

/// A raffle's task: applies its jobs one at a time, in order.
async fn run(io: Arc<Io>, raffle_id: String, mut jobs: mpsc::UnboundedReceiver<Job>) {
    let mut state: Option<Drawing> = None;
    while let Some(job) = jobs.recv().await {
        let outcome = io
            .apply(&mut state, job.work)
            .await
            .map_err(|e| (e.kind(), e.to_string()));
        if let Err((_, message)) = &outcome {
            tracing::debug!(raffle_id, message, "refused");
        }
        job.done.send_replace(Some(outcome));
    }
}

impl Io {
    async fn apply(&self, state: &mut Option<Drawing>, work: Work) -> Result<(), DrawingError> {
        let chain = self.drand.chain_hash();
        match work {
            Work::Open(doc) => {
                if let Some(fresh) = Drawing::open_or_check(state.as_ref(), &doc, &chain)? {
                    self.project(&fresh).await;
                    *state = Some(fresh);
                }
                Ok(())
            }
            Work::Draw { item_id, offline } => {
                let s = state.as_mut().ok_or(DrawingError::NotOpen)?;
                s.check_chain(&chain)?;
                let (round, round_time) = self.drand.target_round((self.clock)());
                // A pending draw for this item (e.g. stalled) is re-entered on its round.
                let pending = s.begin(&item_id, round, round_time)?;
                // The commitment is recorded before the beacon is asked for.
                self.project(s).await;
                let outcome = if offline {
                    BeaconOutcome::Offline((self.randomness)())
                } else {
                    match self.drand.wait_for(pending.round).await {
                        Ok(beacon) => BeaconOutcome::Verified(beacon),
                        Err(e) => BeaconOutcome::Unavailable(e.to_string()),
                    }
                };
                s.conclude(outcome)?;
                self.project(s).await;
                Ok(())
            }
            Work::Command(cmd) => {
                let s = state.as_mut().ok_or(DrawingError::NotOpen)?;
                s.apply(&cmd)?;
                self.project(s).await;
                if s.finished {
                    // The app keeps the record; the engine forgets the raffle.
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
                    tracing::warn!(raffle_id = state.id, attempt, error = %e, "projecting failed; retrying");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
                Err(e) => {
                    tracing::error!(raffle_id = state.id, error = %e, "projecting failed; giving up");
                    return;
                }
            }
        }
        self.notifier.notify(&state.id).await;
    }
}

#[cfg(test)]
mod tests {
    use rafi_drand::DrandConfig;
    use rafi_drawing::{drawing::ItemStatus, manifest::ItemRef};
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
            &drand.chain_hash(),
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

        assert_eq!(
            engine.info().await.unwrap().chain,
            doc.drand_chain.parse().unwrap()
        );
        engine.open(&doc).await.unwrap();
        engine.open(&doc).await.unwrap();
        let draw = Command::Draw {
            item_id: "u-1".into(),
            offline: false,
        };
        engine.command("r1", &draw, "d1").await.unwrap();
        projected(&store, |s| {
            s.item("u-1").unwrap().status == ItemStatus::Drawn
        })
        .await;
        engine.command("r1", &Command::Finish, "f1").await.unwrap();
        let finished = projected(&store, |s| s.finished).await;
        assert!(!notifier.notified().is_empty());

        // Forgotten: further commands are refused as not found.
        let err = engine
            .command("r1", &Command::Finish, "f2")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            EngineError::Rejected {
                kind: ErrorKind::NotFound,
                ..
            }
        ));
        assert_eq!(store.latest("r1").unwrap().version, finished.version);
    }
}
