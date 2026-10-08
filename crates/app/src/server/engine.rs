//! The draw engine: the `Raffle` virtual object of the separate
//! `restate-rafi` service, reached through the Restate ingress
//! ([`RestateClient`]).
//!
//! The object owns each raffle's draw state ([`rafi_core::state::DrawState`]).
//! It writes every new state into the database itself and nudges the
//! raffle's live channel; screens pick it up from there.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use anyhow::{Context as _, bail};
pub use rafi_core::engine::Command;
use rafi_core::{commit::CommitDoc, state::DrawState};
use rafi_platform::sendable;
use serde::Serialize;

use super::config::RestateConfig;

/// Calls the ingress over HTTP. reqwest's wasm32 futures aren't `Send`, so
/// each call is wrapped in [`sendable`] for the handlers that make it.
#[derive(Clone)]
pub struct RestateClient {
    http: reqwest::Client,
    cfg: RestateConfig,
}

impl RestateClient {
    pub fn new(cfg: RestateConfig) -> Self {
        // On wasm32 reqwest has no timeouts; the platform bounds requests.
        #[cfg(not(target_arch = "wasm32"))]
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client");
        #[cfg(target_arch = "wasm32")]
        let http = reqwest::Client::new();
        Self { http, cfg }
    }

    async fn invoke(
        &self,
        raffle_id: &str,
        handler: &str,
        send: bool,
        idempotency_key: &str,
        body: &(impl Serialize + Sync),
    ) -> anyhow::Result<()> {
        sendable(async move {
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
        })
        .await
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
        sendable(async move {
            let url = format!("{}/Raffle/{}/get", self.cfg.ingress_url, raffle_id);
            let mut req = self.http.post(&url);
            if !self.cfg.api_key.is_empty() {
                req = req.bearer_auth(&self.cfg.api_key);
            }
            let resp = req.send().await?.error_for_status()?;
            Ok(resp.json().await?)
        })
        .await
    }

    pub async fn healthy(&self) -> bool {
        sendable(async move {
            let url = format!("{}/restate/health", self.cfg.ingress_url);
            let mut req = self.http.get(&url);
            #[cfg(not(target_arch = "wasm32"))]
            {
                req = req.timeout(Duration::from_secs(3));
            }
            if !self.cfg.api_key.is_empty() {
                req = req.bearer_auth(&self.cfg.api_key);
            }
            matches!(req.send().await, Ok(r) if r.status().is_success())
        })
        .await
    }
}
