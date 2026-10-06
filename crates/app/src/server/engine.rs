//! Draw engines.
//!
//! The draw state ([`DrawState`]) is owned by exactly one engine per raffle:
//!
//! - **Restate** (production): the `Raffle` virtual object of the separate
//!   `restate-rafi` service, reached through the Restate ingress
//!   ([`RestateClient`]). It writes every new state into the same SQLite
//!   database, where [`super::watch`] picks it up.
//! - **Local** (development, tests, emergency fallback): the same state machine
//!   run in-process behind a per-raffle mutex.
//!
//! Either way, every step lands in SQLite and is pushed to the screens.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context as _, bail};
pub use rafi_core::engine::Command;
use rafi_core::{
    commit::CommitDoc,
    drand::{round_time, target_round},
    state::DrawState,
};
use serde::Serialize;
use tokio::sync::Mutex;

use super::{
    AppState,
    config::{EngineKind, RestateConfig},
    ids,
};

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Restate => "restate",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "restate" {
            Self::Restate
        } else {
            Self::Local
        }
    }
}

// ---------------------------------------------------------------------------
// Local engine
// ---------------------------------------------------------------------------

/// In-process engine: per-raffle locks around the core state machine.
#[derive(Clone, Default)]
pub struct LocalEngine {
    locks: Arc<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl LocalEngine {
    fn lock_for(&self, raffle_id: &str) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .expect("lock map")
            .entry(raffle_id.to_owned())
            .or_default()
            .clone()
    }

    async fn load(app: &AppState, raffle_id: &str) -> anyhow::Result<DrawState> {
        let raffle = super::db::raffle(&app.db, raffle_id)
            .await?
            .context("raffle not found")?;
        raffle.draw_state().context("raffle is not frozen")
    }

    pub async fn freeze(&self, app: &AppState, doc: &CommitDoc) -> anyhow::Result<()> {
        let lock = self.lock_for(&doc.raffle_id);
        let _guard = lock.lock().await;
        if let Ok(existing) = Self::load(app, &doc.raffle_id).await {
            let expected = hex::encode(rafi_core::commit::list_hash(&doc.canonical_json()));
            if existing.list_hash == expected {
                return Ok(());
            }
            bail!("raffle is already frozen with a different entrant list");
        }
        app.project(&DrawState::freeze(doc)).await
    }

    pub async fn command(
        &self,
        app: &AppState,
        raffle_id: &str,
        cmd: Command,
    ) -> anyhow::Result<()> {
        let lock = self.lock_for(raffle_id);
        let guard = lock.lock().await;
        let mut state = Self::load(app, raffle_id).await?;
        match cmd {
            Command::Draw { unit_id, offline } => {
                let now = ids::now() as u64;
                let round = target_round(now, app.drand.config().lead_secs);
                let pending = state.begin(&unit_id, round, round_time(round))?;
                app.project(&state).await?;
                if offline {
                    state.complete_offline(ids::random_bytes::<32>())?;
                    app.project(&state).await?;
                    return Ok(());
                }
                drop(guard);
                let (app, this, raffle_id) = (app.clone(), self.clone(), raffle_id.to_owned());
                tokio::spawn(async move {
                    let result = app.drand.fetch_until(pending.round).await;
                    let lock = this.lock_for(&raffle_id);
                    let _guard = lock.lock().await;
                    let outcome = async {
                        let mut state = Self::load(&app, &raffle_id).await?;
                        if state.pending.as_ref() != Some(&pending) {
                            return Ok(()); // superseded
                        }
                        match result {
                            Ok(beacon) => {
                                if let Err(e) = state.complete(&beacon) {
                                    state.fail(&e.to_string())?;
                                }
                            }
                            Err(e) => state.fail(&e.to_string())?,
                        }
                        app.project(&state).await
                    };
                    if let Err(e) = outcome.await {
                        tracing::error!(raffle_id, error = %e, "local draw failed");
                    }
                });
            }
            Command::Resolve { unit_id, outcome } => {
                state.resolve(&unit_id, outcome)?;
                app.project(&state).await?;
            }
            Command::Void { unit_id } => {
                state.void(&unit_id)?;
                app.project(&state).await?;
            }
            Command::Finish => {
                state.finish()?;
                app.project(&state).await?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Restate ingress client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RestateClient {
    http: reqwest::Client,
    cfg: RestateConfig,
}

impl RestateClient {
    pub fn new(cfg: RestateConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client");
        Self { http, cfg }
    }

    async fn invoke(
        &self,
        raffle_id: &str,
        handler: &str,
        send: bool,
        idempotency_key: &str,
        body: &impl Serialize,
    ) -> anyhow::Result<()> {
        let mut url = format!("{}/Raffle/{}/{handler}", self.cfg.ingress_url, raffle_id);
        if send {
            url.push_str("/send");
        }
        let mut req = self
            .http
            .post(&url)
            .json(body)
            .header("idempotency-key", idempotency_key);
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("calling Restate {handler}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // Restate wraps terminal errors as JSON {"message": ...}.
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
                .unwrap_or(text);
            bail!("{message} (Restate {handler}: HTTP {status})");
        }
        Ok(())
    }

    pub async fn freeze(&self, doc: &CommitDoc) -> anyhow::Result<()> {
        let key = format!(
            "freeze-{}",
            rafi_core::commit::list_hash(&doc.canonical_json())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        self.invoke(&doc.raffle_id, "freeze", false, &key, doc)
            .await
    }

    pub async fn command(
        &self,
        raffle_id: &str,
        cmd: &Command,
        command_id: &str,
    ) -> anyhow::Result<()> {
        match cmd {
            // Draws take a few seconds (the committed round is in the future):
            // send asynchronously, progress arrives through the projection.
            Command::Draw { .. } => self.invoke(raffle_id, "draw", true, command_id, cmd).await,
            _ => {
                self.invoke(raffle_id, "resolve", false, command_id, cmd)
                    .await
            }
        }
    }

    /// Reads the object's state through the shared handler.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn get(&self, raffle_id: &str) -> anyhow::Result<Option<DrawState>> {
        let url = format!("{}/Raffle/{}/get", self.cfg.ingress_url, raffle_id);
        let mut req = self.http.post(&url);
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn healthy(&self) -> bool {
        let url = format!("{}/restate/health", self.cfg.ingress_url);
        let mut req = self.http.get(&url).timeout(Duration::from_secs(3));
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }
        matches!(req.send().await, Ok(r) if r.status().is_success())
    }
}
