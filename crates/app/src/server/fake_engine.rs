//! A stand-in for Restate in tests: an in-process HTTP server with the
//! ingress routes [`RestateClient`](super::engine::RestateClient) calls
//! (`/Raffle/{id}/{handler}[/send]`, `/restate/health`). It applies commands
//! with rafi-core's state machine, fetches beacons with the test's drand
//! configuration (mock drand, or an unreachable relay for stalls), and writes
//! every state into the app's database the way `restate-rafi`'s projector
//! does.
//!
//! Like the `Raffle` object, it keeps each raffle's state itself, forgets it
//! when the raffle finishes, answers `draw` asynchronously (`/send`), and
//! returns refusals as `{"message": ...}` with a 4xx status. Unlike it, a draw
//! waiting for its beacon doesn't block other commands (as the in-process
//! engine it replaces didn't), which keeps the tests fast. The real thing is
//! covered by `tests_restate.rs`.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rafi_core::{
    commit::CommitDoc,
    engine::{BeaconOutcome, Command},
    state::{DrawError, DrawState, Pending},
};
use rafi_drand::{DrandClient, DrandConfig};
use rafi_store::{Db, PurgeAfter};

use super::ids;

/// The engine's state, shared by its routes.
struct Engine {
    db: Db,
    drand: DrandClient,
    purge: PurgeAfter,
    states: Mutex<HashMap<String, DrawState>>,
    /// One invocation at a time per raffle, as a virtual object.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Idempotency keys already seen (Restate deduplicates by them).
    seen: Mutex<HashSet<String>>,
}

type Shared = Arc<Engine>;

/// Starts the engine on a free local port; returns its ingress URL.
pub async fn start(db: Db, drand: DrandConfig, purge: PurgeAfter) -> String {
    let engine = Arc::new(Engine {
        db,
        drand: DrandClient::new(drand).expect("drand configuration"),
        purge,
        states: Mutex::default(),
        locks: Mutex::default(),
        seen: Mutex::default(),
    });
    let router = Router::new()
        .route("/restate/health", get(|| async { "ok" }))
        .route("/Raffle/{id}/freeze", post(freeze))
        .route("/Raffle/{id}/draw/send", post(draw))
        .route("/Raffle/{id}/resolve", post(resolve))
        .route("/Raffle/{id}/get", post(get_state))
        .with_state(engine);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local port");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });
    url
}

fn refused(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({ "message": message.to_string() })),
    )
        .into_response()
}

fn draw_error(e: DrawError) -> Response {
    let status = StatusCode::from_u16(e.kind().status()).expect("a status code");
    refused(status, e)
}

impl Engine {
    fn lock(&self, raffle_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(raffle_id.to_owned())
            .or_default()
            .clone()
    }

    fn load(&self, raffle_id: &str) -> Option<DrawState> {
        self.states.lock().unwrap().get(raffle_id).cloned()
    }

    /// Keeps the state and writes it, as the projector does.
    async fn project(&self, state: &DrawState) {
        {
            let mut states = self.states.lock().unwrap();
            if state.finished {
                states.remove(&state.raffle_id);
            } else {
                states.insert(state.raffle_id.clone(), state.clone());
            }
        }
        rafi_store::save_draw_state(&self.db, state, self.purge)
            .await
            .expect("projecting a draw state");
    }

    fn first_time(&self, key: &str) -> bool {
        self.seen.lock().unwrap().insert(key.to_owned())
    }
}

async fn freeze(
    State(engine): State<Shared>,
    Path(id): Path<String>,
    Json(doc): Json<CommitDoc>,
) -> Response {
    if doc.raffle_id != id {
        return refused(
            StatusCode::BAD_REQUEST,
            "raffle ID does not match the object key",
        );
    }
    let lock = engine.lock(&id);
    let _guard = lock.lock().await;
    match DrawState::freeze_or_check(engine.load(&id).as_ref(), &doc, &engine.drand.chain_hash()) {
        Ok(Some(fresh)) => engine.project(&fresh).await,
        Ok(None) => {}
        Err(e) => return draw_error(e),
    }
    StatusCode::OK.into_response()
}

async fn draw(
    State(engine): State<Shared>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(cmd): Json<Command>,
) -> Response {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !engine.first_time(&format!("{id}/draw/{key}")) {
        return StatusCode::ACCEPTED.into_response();
    }
    let Command::Draw { unit_id, offline } = cmd else {
        return refused(StatusCode::BAD_REQUEST, "expected a draw command");
    };
    // `/send`: accepted now, run in the background.
    tokio::spawn(async move {
        let lock = engine.lock(&id);
        let guard = lock.lock().await;
        let Some(mut state) = engine.load(&id) else {
            return;
        };
        if state.check_chain(&engine.drand.chain_hash()).is_err() {
            return;
        }
        let (round, round_time) = engine.drand.target_round(ids::now() as u64);
        // A pending draw for this unit (e.g. stalled) is re-entered on its round.
        let Ok(pending) = state.begin(&unit_id, round, round_time) else {
            return;
        };
        engine.project(&state).await;
        if offline {
            let randomness = BeaconOutcome::Offline(ids::random_bytes::<32>());
            if state.conclude(randomness).is_ok() {
                engine.project(&state).await;
            }
            return;
        }
        drop(guard);
        settle(&engine, &id, pending).await;
    });
    StatusCode::ACCEPTED.into_response()
}

/// Fetches the pending draw's round and settles it; if the beacon can't be
/// had, the draw stalls on its round.
async fn settle(engine: &Engine, id: &str, pending: Pending) {
    let result = engine.drand.wait_for(pending.round).await;
    let lock = engine.lock(id);
    let _guard = lock.lock().await;
    let Some(mut state) = engine.load(id) else {
        return;
    };
    if state.pending.as_ref() != Some(&pending) {
        return; // settled meanwhile (another attempt, offline, abandoned)
    }
    let outcome = match result {
        Ok(beacon) => BeaconOutcome::Verified(beacon),
        Err(e) => BeaconOutcome::Unavailable(e.to_string()),
    };
    if state.conclude(outcome).is_ok() {
        engine.project(&state).await;
    }
}

async fn resolve(
    State(engine): State<Shared>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(cmd): Json<Command>,
) -> Response {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let lock = engine.lock(&id);
    let _guard = lock.lock().await;
    if !engine.first_time(&format!("{id}/resolve/{key}")) {
        return StatusCode::OK.into_response();
    }
    let Some(mut state) = engine.load(&id) else {
        return draw_error(DrawError::NotFrozen);
    };
    if let Err(e) = state.apply(&cmd) {
        return draw_error(e);
    }
    engine.project(&state).await;
    StatusCode::OK.into_response()
}

async fn get_state(State(engine): State<Shared>, Path(id): Path<String>) -> Response {
    Json(engine.load(&id)).into_response()
}
