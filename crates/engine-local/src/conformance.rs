//! The engine conformance suite: the scenarios every draw engine must pass,
//! through the [`DrawingEngine`] port alone.
//!
//! An engine provides an [`EngineHarness`]: it deploys the engine for a
//! [`Deployment`] (drand, store, notifier) and hands out clients of it. The
//! suite deploys once per [`Profile`] (a drand setup and a store behaviour),
//! runs that profile's scenarios against it side by side (each on its own
//! raffle), and observes results only through the projection (the store) and
//! the port's replies.
//!
//! Besides the contract ([`DrawingEngine`]), the scenarios pin the fairness rules:
//! the commitment (Announce) is recorded before the beacon is asked for; a
//! withheld or invalid beacon stalls the draw on its round and a retry fetches
//! the same round; the only exits without the beacon (offline, abandon) are
//! logged; nothing changes a pending draw's pool; and every log the engine
//! produces passes the public verifier.
//!
//! ```ignore
//! rafi_engine_local::conformance::run(&MyHarness).await;
//! ```

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rafi_drand::{DrandClient, DrandConfig};
use rafi_drawing::{
    BeaconSource, ErrorKind,
    drawing::{DrawMode, Drawing, Event, ItemStatus, Outcome},
    engine::{Command, Decision, DrawRequest},
    manifest::{ItemRef, Manifest},
    verify::{Replaced, verify},
};
use rafi_platform::{
    BoxFuture, DrawStore, DrawingEngine, EngineError, MemoryDrawStore, Notifier, Recording,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
};

/// What an engine is deployed with.
pub struct Deployment {
    pub drand: DrandClient,
    pub store: Arc<dyn DrawStore>,
    pub notifier: Arc<dyn Notifier>,
}

/// An engine under test.
pub trait EngineHarness: Send + Sync {
    /// The engine's name, in failure reports.
    fn name(&self) -> &str;

    /// Deploys an engine drawing with `deployment.drand`, writing to its store
    /// and notifying its notifier.
    fn deploy(&self, deployment: Deployment) -> BoxFuture<'_, Result<Arc<dyn Deployed>, String>>;
}

/// A deployed engine.
pub trait Deployed: Send + Sync {
    /// A client of the engine. With `deadline`, a synchronous call gives up
    /// waiting after it ([`EngineError::Pending`]).
    fn engine(&self, deadline: Option<Duration>) -> Arc<dyn DrawingEngine>;
}

/// A drand setup and a store behaviour; the suite deploys once per profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Mock drand, no lead: every round is already out.
    Instant,
    /// Mock drand, a lead of one period: draws wait for their round.
    Lead,
    /// Quicknet behind a relay that is never reachable: every draw stalls.
    Withheld,
    /// Quicknet behind a relay serving beacons signed by another key: every
    /// draw stalls.
    Forged,
    /// [`Profile::Instant`] with a store that fails each raffle's first writes.
    FlakyStore,
}

impl Profile {
    pub const ALL: [Self; 5] = [
        Self::Instant,
        Self::Lead,
        Self::Withheld,
        Self::Forged,
        Self::FlakyStore,
    ];
}

/// Writes the store fails for each raffle before it accepts any.
const FLAKY_FAILURES: u32 = 2;
/// How long the suite waits for a projection.
const PATIENCE: Duration = Duration::from_secs(45);

type Scenario =
    fn(Arc<Fixture>, String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// Runs every scenario against the engine; panics with every failure.
pub async fn run(harness: &dyn EngineHarness) {
    let mut failures = Vec::new();
    let mut passed = 0;
    for profile in Profile::ALL {
        let fixture = match Fixture::deploy(harness, profile).await {
            Ok(fixture) => Arc::new(fixture),
            Err(e) => {
                failures.push(format!("{profile:?}: deploying: {e}"));
                continue;
            }
        };
        let mut set = JoinSet::new();
        for (name, scenario) in scenarios(profile) {
            let fixture = fixture.clone();
            set.spawn(async move { (name, scenario(fixture, name.replace('_', "-")).await) });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((_, Ok(()))) => passed += 1,
                Ok((name, Err(e))) => failures.push(format!("{profile:?}/{name}: {e}")),
                Err(e) => failures.push(format!("{profile:?}: a scenario panicked: {e}")),
            }
        }
    }
    eprintln!(
        "{passed} conformance scenarios passed on {}",
        harness.name()
    );
    assert!(
        failures.is_empty(),
        "{} of {} conformance scenarios failed on {}:\n- {}",
        failures.len(),
        failures.len() + passed,
        harness.name(),
        failures.join("\n- ")
    );
}

macro_rules! scenario {
    ($f:ident) => {
        (stringify!($f), |fx, id| Box::pin($f(fx, id)) as _)
    };
}

fn scenarios(profile: Profile) -> Vec<(&'static str, Scenario)> {
    match profile {
        Profile::Instant => vec![
            scenario!(open_is_idempotent_per_manifest),
            scenario!(commands_apply_once),
            scenario!(refusals_keep_their_kind),
            scenario!(a_refused_draw_is_silent),
            scenario!(a_finished_raffle_is_forgotten),
        ],
        Profile::Lead => vec![
            scenario!(the_round_is_committed_before_its_beacon),
            scenario!(a_command_waits_for_the_pending_draw),
            scenario!(a_deadline_leaves_the_command_pending),
        ],
        Profile::Withheld => vec![
            scenario!(a_withheld_beacon_stalls_on_the_same_round),
            scenario!(an_offline_draw_replaces_a_stalled_round),
            scenario!(nothing_changes_the_pool_of_a_stalled_draw),
        ],
        Profile::Forged => vec![scenario!(a_forged_beacon_stalls_the_draw)],
        Profile::FlakyStore => vec![scenario!(every_state_reaches_a_flaky_store)],
    }
}

/// One deployed engine and what the suite observes it through.
pub struct Fixture {
    deployed: Arc<dyn Deployed>,
    engine: Arc<dyn DrawingEngine>,
    /// Every state the engine wrote (behind the flaky store's failures).
    store: MemoryDrawStore,
    notifier: Arc<Recording>,
    drand: DrandClient,
}

impl Fixture {
    async fn deploy(harness: &dyn EngineHarness, profile: Profile) -> Result<Self, String> {
        let config = match profile {
            Profile::Instant | Profile::FlakyStore => mock(0),
            Profile::Lead => mock(3),
            Profile::Withheld => quicknet("http://127.0.0.1:9".into()),
            Profile::Forged => quicknet(forging_relay().await?),
        };
        let drand = DrandClient::new(config).map_err(|e| format!("drand: {e:#}"))?;
        let store = MemoryDrawStore::default();
        let notifier = Arc::new(Recording::default());
        let writes: Arc<dyn DrawStore> = match profile {
            Profile::FlakyStore => Arc::new(FlakyStore {
                inner: store.clone(),
                failures: Mutex::default(),
            }),
            _ => Arc::new(store.clone()),
        };
        let deployed = harness
            .deploy(Deployment {
                drand: drand.clone(),
                store: writes,
                notifier: notifier.clone(),
            })
            .await?;
        Ok(Self {
            engine: deployed.engine(None),
            deployed,
            store,
            notifier,
            drand,
        })
    }

    /// A raffle on the engine's chain: `entrants` entrants, prize items
    /// `u-1` … `u-{items}`.
    fn doc(&self, raffle_id: &str, entrants: usize, items: u64) -> Manifest {
        Manifest::new(
            raffle_id,
            BeaconSource::Drand(self.drand.chain_hash()),
            (0..entrants).map(|i| format!("{i:032x}")).collect(),
            (1..=items)
                .map(|n| ItemRef {
                    id: format!("u-{n}"),
                    name: "Prize".into(),
                })
                .collect(),
        )
    }

    /// Waits until `raffle_id`'s latest projection satisfies `f`.
    async fn projected(
        &self,
        raffle_id: &str,
        what: &str,
        f: impl Fn(&Drawing) -> bool,
    ) -> Result<Drawing, String> {
        let start = tokio::time::Instant::now();
        loop {
            if let Some(s) = self.store.latest(raffle_id)
                && f(&s)
            {
                return Ok(s);
            }
            if start.elapsed() > PATIENCE {
                return Err(format!(
                    "never projected: {what}; latest: {:?}",
                    self.store.latest(raffle_id).map(|s| s.events)
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn open(&self, doc: &Manifest) -> Result<Drawing, String> {
        self.engine
            .open(doc)
            .await
            .map_err(|e| format!("open: {e}"))?;
        self.projected(&doc.drawing, "opened", |_| true).await
    }

    async fn send(&self, raffle_id: &str, cmd: Command, id: &str) -> Result<(), EngineError> {
        match &cmd {
            Command::Draw(request) => self.engine.draw(raffle_id, request, id).await,
            Command::Decide(decision) => self.engine.decide(raffle_id, decision, id).await,
        }
    }

    async fn ok(&self, raffle_id: &str, cmd: impl Into<Command>, id: &str) -> Result<(), String> {
        let cmd = cmd.into();
        self.send(raffle_id, cmd.clone(), id)
            .await
            .map_err(|e| format!("{cmd:?}: {e}"))
    }

    async fn refused(
        &self,
        raffle_id: &str,
        cmd: impl Into<Command>,
        id: &str,
        expected: ErrorKind,
    ) -> Result<(), String> {
        let cmd = cmd.into();
        match self.send(raffle_id, cmd.clone(), id).await {
            Err(EngineError::Rejected { kind, .. }) if kind == expected => Ok(()),
            other => Err(format!(
                "{cmd:?}: expected a {expected:?} refusal, got {other:?}"
            )),
        }
    }

    async fn draw(
        &self,
        raffle_id: &str,
        item: &str,
        offline: bool,
        id: &str,
    ) -> Result<(), String> {
        let request = DrawRequest {
            item_id: item.to_owned(),
            offline,
        };
        self.ok(raffle_id, request, id).await
    }

    async fn drawn(&self, raffle_id: &str, item: &str) -> Result<Drawing, String> {
        self.projected(raffle_id, &format!("{item} drawn"), |s| {
            s.item(item).is_some_and(|u| u.status == ItemStatus::Drawn)
        })
        .await
    }

    async fn stalls(&self, raffle_id: &str, n: usize) -> Result<Drawing, String> {
        self.projected(raffle_id, &format!("{n} stall(s)"), |s| {
            count(s, is_stall) >= n
        })
        .await
    }
}

fn mock(lead_secs: u64) -> DrandConfig {
    DrandConfig {
        mock: true,
        lead_secs,
        timeout_secs: 5,
        ..DrandConfig::default()
    }
}

fn quicknet(relay: String) -> DrandConfig {
    DrandConfig {
        relays: vec![relay],
        mock: false,
        lead_secs: 3,
        timeout_secs: 1,
    }
}

/// A drand relay for quicknet that serves every round signed by the mock
/// chain's key instead: well-formed beacons that never verify.
async fn forging_relay() -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("binding the forging relay: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let forger = rafi_drand::mock_chain();
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            // GET /{chain}/public/{round} HTTP/1.1
            let round = request
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.rsplit('/').next())
                .and_then(|r| r.parse::<u64>().ok())
                .filter(|r| *r > 0);
            let (status, body) = match round {
                Some(round) => {
                    let beacon = forger.sign(round);
                    let body = serde_json::json!({
                        "round": round,
                        "signature": hex(&beacon.signature),
                    });
                    ("200 OK", body.to_string())
                }
                None => ("404 Not Found", String::new()),
            };
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    Ok(format!("http://{addr}"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fails the first [`FLAKY_FAILURES`] writes of each raffle.
struct FlakyStore {
    inner: MemoryDrawStore,
    failures: Mutex<HashMap<String, u32>>,
}

impl DrawStore for FlakyStore {
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>> {
        let failed = {
            let mut failures = self.failures.lock().expect("failures lock");
            let n = failures.entry(state.id.clone()).or_default();
            *n += 1;
            *n <= FLAKY_FAILURES
        };
        if failed {
            return Box::pin(async { Err(anyhow::anyhow!("the store is flaky")) });
        }
        self.inner.save_drawing(state)
    }
}

impl fmt::Debug for Fixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fixture").finish_non_exhaustive()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn is_stall(e: &Event) -> bool {
    matches!(e, Event::Stall { .. })
}

fn count(s: &Drawing, f: impl Fn(&Event) -> bool) -> usize {
    s.events.iter().filter(|e| f(e)).count()
}

macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}

/// The public verifier replays the log against the manifest.
fn verified(doc: &Manifest, state: &Drawing) -> Result<rafi_drawing::verify::Report, String> {
    let report = verify(
        &doc.canonical_json(),
        &state.manifest_hash,
        &state.events,
        rafi_drand::resolve_chain,
    );
    ensure!(report.manifest_hash_ok, "the manifest hash doesn't verify");
    ensure!(
        report.errors.is_empty(),
        "the verifier found: {:?}",
        report.errors
    );
    ensure!(
        report.draws.iter().all(|d| d.ok),
        "a draw doesn't verify: {:?}",
        report.draws
    );
    Ok(report)
}

// ---------------------------------------------------------------------------
// Instant
// ---------------------------------------------------------------------------

/// Opening with the same manifest is a no-op; another manifest, another chain or an
/// invalid document is refused.
async fn open_is_idempotent_per_manifest(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 3, 1);
    let opened = fx.open(&doc).await?;
    ensure!(
        opened.events.is_empty() && opened.entrants.len() == 3,
        "{opened:?}"
    );
    ensure!(
        fx.notifier.notified().contains(&id),
        "clients were not notified"
    );
    fx.engine
        .open(&doc)
        .await
        .map_err(|e| format!("reopening with the same manifest: {e}"))?;

    let other = fx.doc(&id, 4, 1);
    match fx.engine.open(&other).await {
        Err(EngineError::Rejected {
            kind: ErrorKind::Conflict,
            ..
        }) => {}
        r => return Err(format!("another list: {r:?}")),
    }
    let mut chain = fx.doc(&format!("{id}-chain"), 3, 1);
    let quicknet = rafi_drawing::drand::chains::QUICKNET_HASH;
    chain.beacon = BeaconSource::Drand(quicknet);
    if fx.drand.chain_hash() == quicknet {
        chain.beacon = BeaconSource::Drand(rafi_drand::mock_chain().chain().hash());
    }
    match fx.engine.open(&chain).await {
        Err(EngineError::Rejected {
            kind: ErrorKind::Conflict,
            ..
        }) => {}
        r => return Err(format!("another chain: {r:?}")),
    }
    let empty = fx.doc(&format!("{id}-empty"), 0, 1);
    match fx.engine.open(&empty).await {
        Err(EngineError::Rejected {
            kind: ErrorKind::BadRequest,
            ..
        }) => {}
        r => return Err(format!("nobody entered: {r:?}")),
    }
    let latest = fx.store.latest(&id).ok_or("no projection")?;
    ensure!(
        latest.version == opened.version,
        "the refusals changed the state"
    );
    Ok(())
}

/// The same command ID applies once.
async fn commands_apply_once(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 3, 2);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    fx.drawn(&id, "u-1").await?;
    let claimed = Decision::Resolve {
        item_id: "u-1".into(),
        outcome: Outcome::Claimed,
    };
    fx.ok(&id, claimed.clone(), "c1").await?;
    // Applied again, it would be refused (no undecided winner).
    fx.ok(&id, claimed, "c1").await?;
    let s = fx.store.latest(&id).ok_or("no projection")?;
    ensure!(
        count(&s, |e| matches!(e, Event::Draw { .. })) == 1,
        "drawn twice"
    );
    ensure!(
        count(&s, |e| matches!(e, Event::Resolve { .. })) == 1,
        "resolved twice"
    );
    verified(&doc, &s)?;
    Ok(())
}

/// Every refusal reaches the caller with its kind.
async fn refusals_keep_their_kind(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    fx.refused(&id, Decision::Finish, "f0", ErrorKind::NotFound)
        .await?;
    let doc = fx.doc(&id, 3, 2);
    fx.open(&doc).await?;
    let resolve = |item: &str, outcome| Decision::Resolve {
        item_id: item.into(),
        outcome,
    };
    fx.refused(
        &id,
        resolve("nope", Outcome::Claimed),
        "r1",
        ErrorKind::NotFound,
    )
    .await?;
    fx.refused(
        &id,
        resolve("u-1", Outcome::Drawn),
        "r2",
        ErrorKind::BadRequest,
    )
    .await?;
    fx.refused(
        &id,
        resolve("u-1", Outcome::Claimed),
        "r3",
        ErrorKind::Conflict,
    )
    .await?;
    let abandon = Decision::Abandon {
        item_id: "u-1".into(),
    };
    fx.refused(&id, abandon, "a1", ErrorKind::Conflict).await?;
    Ok(())
}

/// A draw is accepted before it is checked: a refused one changes nothing.
async fn a_refused_draw_is_silent(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 3, 2);
    let opened = fx.open(&doc).await?;
    fx.draw(&id, "nope", false, "d1").await?;
    // A synchronous command behind it, as a barrier.
    fx.ok(
        &id,
        Decision::Void {
            item_id: "u-2".into(),
        },
        "v1",
    )
    .await?;
    let s = fx
        .projected(&id, "u-2 voided", |s| {
            s.item("u-2")
                .is_some_and(|u| u.status == ItemStatus::Voided)
        })
        .await?;
    ensure!(
        s.version == opened.version + 1,
        "the refused draw changed the state: {s:?}"
    );
    ensure!(s.events.len() == 1, "{:?}", s.events);
    Ok(())
}

/// After finish the engine forgets the raffle; the published record stays.
async fn a_finished_raffle_is_forgotten(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 3, 1);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    fx.drawn(&id, "u-1").await?;
    let claimed = Decision::Resolve {
        item_id: "u-1".into(),
        outcome: Outcome::Claimed,
    };
    fx.ok(&id, claimed, "c1").await?;
    fx.ok(&id, Decision::Finish, "f1").await?;
    let finished = fx.projected(&id, "finished", |s| s.finished).await?;
    fx.refused(&id, Decision::Finish, "f2", ErrorKind::NotFound)
        .await?;
    // The opening replayed: never a change to what was published.
    let _ = fx.engine.open(&doc).await;
    fx.refused(&id, Decision::Finish, "f3", ErrorKind::NotFound)
        .await?;
    let latest = fx.store.latest(&id).ok_or("no projection")?;
    ensure!(
        latest.version == finished.version && latest.finished,
        "a replayed opening changed the record: {latest:?}"
    );
    verified(&doc, &latest)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Lead
// ---------------------------------------------------------------------------

/// The round is announced (recorded) before its beacon is asked for, and is
/// one that is not out yet when the draw starts.
async fn the_round_is_committed_before_its_beacon(
    fx: Arc<Fixture>,
    id: String,
) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 1);
    fx.open(&doc).await?;
    let requested = now();
    fx.draw(&id, "u-1", false, "d1").await?;
    let drawn = fx.drawn(&id, "u-1").await?;
    let announced = fx
        .store
        .states()
        .into_iter()
        .filter(|s| s.id == id)
        .find(|s| s.pending.is_some() && matches!(s.events.last(), Some(Event::Announce { .. })))
        .ok_or("the announcement was never projected on its own")?;
    ensure!(
        announced.version < drawn.version,
        "announced after the draw"
    );
    let Some(Event::Announce {
        round, round_time, ..
    }) = announced.events.last()
    else {
        unreachable!()
    };
    ensure!(
        *round_time >= requested,
        "round {round} was out ({round_time}) before the draw started ({requested})"
    );
    ensure!(
        matches!(drawn.events.last(), Some(Event::Draw { round: r, mode: DrawMode::Beacon, .. }) if r == round),
        "drawn from another round: {:?}",
        drawn.events
    );
    verified(&doc, &drawn)?;
    Ok(())
}

/// A command sent while a draw waits for its beacon applies after the draw,
/// against the settled state.
async fn a_command_waits_for_the_pending_draw(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 1);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    // Refused if it ran before the draw settled (a draw is pending).
    fx.ok(&id, Decision::Finish, "f1").await?;
    let s = fx.projected(&id, "finished", |s| s.finished).await?;
    let order: Vec<_> = s
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Draw { .. } => Some("draw"),
            Event::Finish => Some("finish"),
            _ => None,
        })
        .collect();
    ensure!(order == ["draw", "finish"], "{order:?}");
    verified(&doc, &s)?;
    Ok(())
}

/// Past the caller's deadline a queued command is `Pending`, and still
/// applies, once.
async fn a_deadline_leaves_the_command_pending(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 1);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    let hasty = fx.deployed.engine(Some(Duration::from_millis(300)));
    match hasty.decide(&id, &Decision::Finish, "f1").await {
        Err(EngineError::Pending) => {}
        r => return Err(format!("expected Pending, got {r:?}")),
    }
    let s = fx.projected(&id, "finished", |s| s.finished).await?;
    // Retrying with the same ID applies nothing new.
    fx.ok(&id, Decision::Finish, "f1").await?;
    let latest = fx.store.latest(&id).ok_or("no projection")?;
    ensure!(
        count(&latest, |e| matches!(e, Event::Finish)) == 1,
        "finished twice"
    );
    ensure!(latest.version == s.version, "the retry changed the state");
    Ok(())
}

// ---------------------------------------------------------------------------
// Withheld
// ---------------------------------------------------------------------------

/// A withheld beacon stalls the draw; drawing again (with the clock moved on)
/// fetches the same round. Abandoning it is logged for the verifier.
async fn a_withheld_beacon_stalls_on_the_same_round(
    fx: Arc<Fixture>,
    id: String,
) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 2);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    let first = fx.stalls(&id, 1).await?;
    let pending = first
        .pending
        .clone()
        .ok_or("the stalled draw isn't pending")?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    fx.draw(&id, "u-1", false, "d2").await?;
    let second = fx.stalls(&id, 2).await?;
    ensure!(
        second.pending.as_ref() == Some(&pending),
        "another draw: {second:?}"
    );
    ensure!(
        count(&second, |e| matches!(e, Event::Announce { .. })) == 1,
        "announced twice"
    );
    ensure!(
        second
            .events
            .iter()
            .all(|e| !matches!(e, Event::Stall { round, .. } if *round != pending.round)),
        "stalled on another round"
    );
    // Another item can't start meanwhile; the organizer abandons this one.
    fx.draw(&id, "u-2", false, "d3").await?;
    fx.ok(
        &id,
        Decision::Abandon {
            item_id: "u-1".into(),
        },
        "a1",
    )
    .await?;
    let s = fx
        .projected(&id, "abandoned", |s| s.pending.is_none())
        .await?;
    ensure!(
        matches!(s.events.last(), Some(Event::Abandon { round, .. }) if *round == pending.round),
        "{:?}",
        s.events
    );
    let report = verified(&doc, &s)?;
    ensure!(
        report
            .replaced
            .iter()
            .any(|c| c.round == pending.round && c.replaced == Replaced::Abandoned),
        "the abandoned round isn't reported: {:?}",
        report.replaced
    );
    Ok(())
}

/// An offline draw settles a stalled draw; its round is reported as replaced.
async fn an_offline_draw_replaces_a_stalled_round(
    fx: Arc<Fixture>,
    id: String,
) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 1);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    let stalled = fx.stalls(&id, 1).await?;
    let pending = stalled.pending.clone().ok_or("not pending")?;
    fx.draw(&id, "u-1", true, "o1").await?;
    let s = fx.drawn(&id, "u-1").await?;
    ensure!(
        matches!(s.events.last(), Some(Event::Draw { mode: DrawMode::Offline, k, .. }) if *k == pending.k),
        "{:?}",
        s.events
    );
    let report = verified(&doc, &s)?;
    ensure!(
        report
            .replaced
            .iter()
            .any(|c| c.round == pending.round && c.replaced == Replaced::Offline),
        "the replaced round isn't reported: {:?}",
        report.replaced
    );
    Ok(())
}

/// Nothing that could change a stalled draw's pool applies.
async fn nothing_changes_the_pool_of_a_stalled_draw(
    fx: Arc<Fixture>,
    id: String,
) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 2);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", true, "o1").await?;
    fx.drawn(&id, "u-1").await?;
    fx.draw(&id, "u-2", false, "d2").await?;
    fx.stalls(&id, 1).await?;
    let resolve = |outcome| Decision::Resolve {
        item_id: "u-1".into(),
        outcome,
    };
    fx.refused(&id, resolve(Outcome::NotHere), "r1", ErrorKind::Conflict)
        .await?;
    fx.refused(&id, resolve(Outcome::Declined), "r2", ErrorKind::Conflict)
        .await?;
    fx.refused(
        &id,
        Decision::Void {
            item_id: "u-2".into(),
        },
        "v1",
        ErrorKind::Conflict,
    )
    .await?;
    fx.refused(&id, Decision::Finish, "f1", ErrorKind::Conflict)
        .await?;
    // Handing over can't change the pool.
    fx.ok(&id, resolve(Outcome::Claimed), "c1").await?;
    fx.ok(
        &id,
        Decision::Abandon {
            item_id: "u-2".into(),
        },
        "a1",
    )
    .await?;
    let s = fx
        .projected(&id, "abandoned", |s| s.pending.is_none())
        .await?;
    verified(&doc, &s)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Forged
// ---------------------------------------------------------------------------

/// A beacon that doesn't verify stalls the draw; it never draws from it.
async fn a_forged_beacon_stalls_the_draw(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 5, 1);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    let s = fx.stalls(&id, 1).await?;
    ensure!(s.pending.is_some(), "the draw isn't pending");
    ensure!(
        count(&s, |e| matches!(e, Event::Draw { .. })) == 0,
        "drawn from a forged beacon"
    );
    verified(&doc, &s)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// FlakyStore
// ---------------------------------------------------------------------------

/// Every version reaches the store, however often a write fails first.
async fn every_state_reaches_a_flaky_store(fx: Arc<Fixture>, id: String) -> Result<(), String> {
    let doc = fx.doc(&id, 3, 1);
    fx.open(&doc).await?;
    fx.draw(&id, "u-1", false, "d1").await?;
    fx.drawn(&id, "u-1").await?;
    let claimed = Decision::Resolve {
        item_id: "u-1".into(),
        outcome: Outcome::Claimed,
    };
    fx.ok(&id, claimed, "c1").await?;
    fx.ok(&id, Decision::Finish, "f1").await?;
    let finished = fx.projected(&id, "finished", |s| s.finished).await?;
    let mut versions: Vec<u64> = fx
        .store
        .states()
        .into_iter()
        .filter(|s| s.id == id)
        .map(|s| s.version)
        .collect();
    versions.sort_unstable();
    versions.dedup();
    let expected: Vec<u64> = (1..=finished.version).collect();
    ensure!(versions == expected, "versions written: {versions:?}");
    verified(&doc, &finished)?;
    Ok(())
}
