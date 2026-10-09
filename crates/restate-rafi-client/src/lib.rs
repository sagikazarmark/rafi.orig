//! The Rafi draw engine through the Restate ingress: [`RestateClient`]
//! implements [`DrawingEngine`] by calling the `restate-rafi` service's handlers
//! over HTTP.
//!
//! Kept apart from `restate-rafi` so the app links only this (reqwest, no
//! Restate SDK). The handler names are here; `restate-rafi`'s tests check them
//! against the service.
//!
//! - Opening and commands other than draws are synchronous calls; a draw is sent
//!   (`/send`) and settles in the background, as the [`DrawingEngine`] contract
//!   says.
//! - Each call carries an `idempotency-key` (the open key, or the command
//!   ID), so a retry applies once.
//! - A refusal is a handler's terminal error: the ingress marks it
//!   `x-restate-error-source: invocation` and answers `{"message": ...}` with
//!   the kind's status code ([`ErrorKind::from_status`]). Anything else (the
//!   ingress's own errors included, such as an unregistered service) means the
//!   engine is unavailable.
//! - Natively a call that outlasts the configured timeout (queued behind a draw
//!   waiting for its beacon) returns [`EngineError::Pending`]. On wasm32
//!   reqwest has no timeouts: the platform bounds the request instead.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use anyhow::anyhow;
use rafi_drawing::{ErrorKind, drand::ChainHash, engine::Command, manifest::Manifest};
use rafi_platform::{BoxFuture, DrawingEngine, EngineError, EngineInfo, sendable};
use serde::{Deserialize, Serialize};

/// The `Raffle` virtual object (keyed by raffle ID).
pub const RAFFLE_SERVICE: &str = "Raffle";
/// The keyless `RaffleEngine` service.
pub const INFO_SERVICE: &str = "RaffleEngine";
/// `Raffle`'s handlers.
pub const OPEN: &str = "open";
pub const DRAW: &str = "draw";
pub const RESOLVE: &str = "resolve";
/// `RaffleEngine`'s handler.
pub const INFO: &str = "info";

/// How to reach the ingress.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Ingress base URL: `http://restate-tunnel:8080` (the Cloud tunnel client's
    /// authenticating proxy), `https://<env>.env.eu.restate.cloud:8080`, or a
    /// self-hosted `http://localhost:8080`.
    pub ingress_url: String,
    /// API key for the ingress (Restate Cloud direct). Empty behind the tunnel
    /// proxy and for unauthenticated local servers.
    pub api_key: String,
    /// How long a synchronous call may take before it counts as
    /// [`EngineError::Pending`] (natively). Set it above the engine's drand
    /// lead plus fetch timeout, the longest a command waits behind a draw.
    pub timeout_secs: u64,
}

impl ClientConfig {
    /// The default timeout: above the engine's default drand lead (6 s) plus
    /// fetch timeout (60 s).
    pub const DEFAULT_TIMEOUT_SECS: u64 = 90;
}

/// The draw engine through the Restate ingress.
#[derive(Clone)]
pub struct RestateClient {
    http: reqwest::Client,
    cfg: ClientConfig,
}

#[derive(Deserialize)]
struct InfoReply {
    chain: String,
}

impl RestateClient {
    pub fn new(mut cfg: ClientConfig) -> Self {
        cfg.ingress_url = cfg.ingress_url.trim_end_matches('/').to_owned();
        // On wasm32 reqwest has no timeouts; the platform bounds requests.
        #[cfg(not(target_arch = "wasm32"))]
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .expect("http client");
        #[cfg(target_arch = "wasm32")]
        let http = reqwest::Client::new();
        Self { http, cfg }
    }

    /// POSTs to `path` under the ingress. A timeout counts as
    /// [`EngineError::Pending`] when `pending_on_timeout` (the call was
    /// accepted and queued), as unavailable otherwise.
    async fn post(
        &self,
        path: &str,
        idempotency_key: Option<&str>,
        body: Option<&(impl Serialize + Sync)>,
        pending_on_timeout: bool,
    ) -> Result<reqwest::Response, EngineError> {
        let mut req = self.http.post(format!("{}/{path}", self.cfg.ingress_url));
        if let Some(body) = body {
            req = req.json(body);
        }
        if let Some(key) = idempotency_key {
            req = req.header("idempotency-key", key);
        }
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }
        let resp = match req.send().await {
            Ok(resp) => resp,
            Err(e) if pending_on_timeout && e.is_timeout() => return Err(EngineError::Pending),
            Err(e) => {
                return Err(EngineError::Unavailable(
                    anyhow::Error::from(e).context(format!("calling Restate {path}")),
                ));
            }
        };
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status().as_u16();
        let from_handler = resp
            .headers()
            .get(ERROR_SOURCE)
            .is_some_and(|v| v.as_bytes() == b"invocation");
        let text = resp.text().await.unwrap_or_default();
        Err(refusal(path, status, from_handler, &text))
    }

    async fn call(
        &self,
        raffle_id: &str,
        handler: &str,
        send: bool,
        idempotency_key: &str,
        body: &(impl Serialize + Sync),
    ) -> Result<(), EngineError> {
        let mut path = format!("{RAFFLE_SERVICE}/{raffle_id}/{handler}");
        if send {
            path.push_str("/send");
        }
        self.post(&path, Some(idempotency_key), Some(body), !send)
            .await
            .map(|_| ())
    }
}

/// The header the ingress marks error replies with: `invocation` for a
/// handler's terminal error, `ingress` for the ingress's own.
pub const ERROR_SOURCE: &str = "x-restate-error-source";

/// An error reply: a refusal by the engine (a handler's terminal error,
/// `{"message": ...}` with the kind's status code), or the engine failing.
fn refusal(path: &str, status: u16, from_handler: bool, text: &str) -> EngineError {
    let message = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_else(|| text.to_owned());
    match ErrorKind::from_status(status).filter(|_| from_handler) {
        Some(kind) => EngineError::Rejected { kind, message },
        None => EngineError::Unavailable(anyhow!("{message} (Restate {path}: HTTP {status})")),
    }
}

impl DrawingEngine for RestateClient {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        Box::pin(sendable(async move {
            let path = format!("{INFO_SERVICE}/{INFO}");
            let resp = self.post(&path, None, None::<&()>, false).await?;
            let reply: InfoReply = resp
                .json()
                .await
                .map_err(|e| EngineError::Unavailable(anyhow!("reading {path}: {e}")))?;
            let chain: ChainHash = reply.chain.parse().map_err(|e| {
                EngineError::Unavailable(anyhow!("the engine reports chain {:?}: {e}", reply.chain))
            })?;
            Ok(EngineInfo { chain })
        }))
    }

    fn open<'a>(&'a self, doc: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(sendable(async move {
            self.call(&doc.raffle_id, OPEN, false, &doc.open_key(), doc)
                .await
        }))
    }

    fn command<'a>(
        &'a self,
        raffle_id: &'a str,
        cmd: &'a Command,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(sendable(async move {
            match cmd {
                // Draws take a few seconds (the committed round is in the
                // future): sent, progress arrives through the projection.
                Command::Draw { .. } => self.call(raffle_id, DRAW, true, command_id, cmd).await,
                _ => self.call(raffle_id, RESOLVE, false, command_id, cmd).await,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_keep_their_kind_and_message() {
        let e = refusal(
            "Raffle/r/resolve",
            409,
            true,
            r#"{"message":"another draw is in progress"}"#,
        );
        assert!(matches!(
            e,
            EngineError::Rejected { kind: ErrorKind::Conflict, ref message }
                if message == "another draw is in progress"
        ));
        let e = refusal(
            "Raffle/r/resolve",
            500,
            true,
            r#"{"message":"invalid manifest hash"}"#,
        );
        assert!(matches!(
            e,
            EngineError::Rejected {
                kind: ErrorKind::Invariant,
                ..
            }
        ));
        assert!(matches!(
            refusal("Raffle/r/resolve", 502, true, "bad gateway"),
            EngineError::Unavailable(_)
        ));
        // The ingress's own 404 (say, an unregistered service) is no refusal.
        assert!(matches!(
            refusal(
                "RaffleEngine/info",
                404,
                false,
                r#"{"message":"service not found"}"#
            ),
            EngineError::Unavailable(_)
        ));
    }
}
