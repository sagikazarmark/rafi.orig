//! The raffle engine through the Restate ingress, as the app sees it:
//! [`RestateClient`] implements [`Engine`] over the clients the SDK
//! generates from the services ([`RaffleIngressClient`],
//! [`EngineInfoIngressClient`]), so the handlers' names and types are the
//! services' own.
//!
//! - Opening and decisions are calls; a draw is sent and settles in the
//!   background, as the [`Engine`] contract says.
//! - Each call carries an idempotency key (the open key, or the command ID),
//!   so a retry applies once.
//! - A refusal is a handler's terminal error: the ingress marks it
//!   `x-restate-error-source: invocation` and answers `{"message": ...}` with
//!   the kind's status code ([`refusal_kind`]). Anything else (the ingress's
//!   own errors included, such as an unregistered service) means the engine is
//!   unavailable.
//! - A call that outlasts the configured timeout (queued behind a draw waiting
//!   for its beacon) returns [`EngineError::Pending`]: natively through
//!   reqwest's timeout; on wasm32, where reqwest has none, by racing a timer.

use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use anyhow::anyhow;
use bytes::Bytes;
use rafi_engine::{Engine, EngineError, EngineInfo};
use rafi_platform::{BoxFuture, sendable};
use rafi_raffle::{Decision, DrawRequest, Manifest, RaffleId};
use restate_sdk::{
    ingress::{CallResponse, Client, ClientError, RequestExecutor},
    serde::Json,
};

use crate::{EngineInfoIngressClient, RaffleIngressClient, status::refusal_kind};

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

/// The raffle engine through the Restate ingress.
#[derive(Clone)]
pub struct RestateClient {
    /// The ingress, or why the configuration doesn't name one (every call is
    /// then unavailable).
    ingress: Result<Client<Http>, String>,
    /// How long a synchronous call waits ([`ClientConfig::timeout_secs`]).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    timeout_secs: u64,
}

impl RestateClient {
    pub fn new(cfg: ClientConfig) -> Self {
        // On wasm32 reqwest has no timeouts; the platform bounds requests.
        #[cfg(not(target_arch = "wasm32"))]
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .expect("http client");
        #[cfg(target_arch = "wasm32")]
        let http = reqwest::Client::new();
        let ingress = cfg
            .ingress_url
            .trim_end_matches('/')
            .parse()
            .map_err(|e| format!("invalid ingress URL {:?}: {e}", cfg.ingress_url))
            .and_then(|uri| Client::new(uri, Http(http)).map_err(|e| e.to_string()))
            .and_then(|client| {
                if cfg.api_key.is_empty() {
                    Ok(client)
                } else {
                    client
                        .with_restate_auth_token(&cfg.api_key)
                        .map_err(|e| e.to_string())
                }
            });
        Self {
            ingress,
            timeout_secs: cfg.timeout_secs.max(1),
        }
    }

    fn ingress(&self) -> Result<Client<Http>, EngineError> {
        self.ingress
            .clone()
            .map_err(|e| EngineError::Unavailable(anyhow!(e)))
    }

    fn raffle(&self, raffle_id: &RaffleId) -> Result<RaffleIngressClient<Http>, EngineError> {
        Ok(RaffleIngressClient::from_client(
            self.ingress()?,
            raffle_id.as_str(),
        ))
    }

    /// A synchronous call, bounded by the timeout: past it, the call was
    /// accepted and is queued, so it counts as [`EngineError::Pending`].
    /// Natively reqwest's own timeout bounds it ([`engine_error`]).
    #[cfg(not(target_arch = "wasm32"))]
    async fn bounded(
        &self,
        call: impl Future<Output = Result<(), EngineError>>,
    ) -> Result<(), EngineError> {
        call.await
    }

    /// A synchronous call, bounded by the timeout: past it, the call was
    /// accepted and is queued, so it counts as [`EngineError::Pending`].
    /// reqwest has no timeouts on wasm32, so the call races a timer.
    #[cfg(target_arch = "wasm32")]
    async fn bounded(
        &self,
        call: impl Future<Output = Result<(), EngineError>>,
    ) -> Result<(), EngineError> {
        use futures_util::future::{Either, select};
        let ms = u32::try_from(self.timeout_secs.saturating_mul(1000)).unwrap_or(u32::MAX);
        let timer = gloo_timers::future::TimeoutFuture::new(ms);
        let call = std::pin::pin!(call);
        match select(call, timer).await {
            Either::Left((outcome, _)) => outcome,
            Either::Right(_) => Err(EngineError::Pending),
        }
    }
}

/// reqwest as the generated clients' transport. Its futures are `Send` only
/// natively; [`sendable`] covers wasm32 (single-threaded).
#[derive(Clone)]
struct Http(reqwest::Client);

impl RequestExecutor for Http {
    type Error = reqwest::Error;

    fn execute(
        &self,
        request: http::Request<Bytes>,
    ) -> impl Future<Output = Result<http::Response<Bytes>, Self::Error>> + Send {
        let http = self.0.clone();
        sendable(async move {
            let response = http.execute(request.try_into()?).await?;
            let mut reply = http::Response::new(Bytes::new());
            *reply.status_mut() = response.status();
            *reply.headers_mut() = response.headers().clone();
            *reply.body_mut() = response.bytes().await?;
            Ok(reply)
        })
    }
}

/// What a failed ingress call means for the engine's caller. A timeout counts
/// as [`EngineError::Pending`] when `pending_on_timeout` (the call was accepted
/// and queued), as unavailable otherwise.
fn engine_error(e: ClientError, path: &str, pending_on_timeout: bool) -> EngineError {
    if let ClientError::Transport { source } = &e
        && pending_on_timeout
        && source
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)
    {
        return EngineError::Pending;
    }
    match e.response() {
        Some(reply) if !reply.status().is_success() => {
            let from_handler = reply
                .headers()
                .get(ERROR_SOURCE)
                .is_some_and(|v| v.as_bytes() == b"invocation");
            refusal(
                path,
                reply.status().as_u16(),
                from_handler,
                &String::from_utf8_lossy(reply.body()),
            )
        }
        _ => EngineError::Unavailable(
            anyhow::Error::from(e).context(format!("calling Restate {path}")),
        ),
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
    match refusal_kind(status).filter(|_| from_handler) {
        Some(kind) => EngineError::Rejected { kind, message },
        None => EngineError::Unavailable(anyhow!("{message} (Restate {path}: HTTP {status})")),
    }
}

impl Engine for RestateClient {
    fn info(&self) -> BoxFuture<'_, Result<EngineInfo, EngineError>> {
        Box::pin(sendable(async move {
            let path = "EngineInfo/info";
            let info = EngineInfoIngressClient::from_client(self.ingress()?)
                .info()
                .call()
                .await
                .and_then(CallResponse::into_body)
                .map_err(|e| engine_error(e, path, false))?
                .into_inner();
            Ok(info)
        }))
    }

    fn open<'a>(&'a self, doc: &'a Manifest) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(sendable(async move {
            let raffle = &doc.raffle;
            let call = self.raffle(raffle)?.open(Json(doc.clone()));
            self.bounded(async {
                call.idempotency_key(doc.open_key())
                    .call()
                    .await
                    .and_then(CallResponse::into_body)
                    .map_err(|e| engine_error(e, &format!("Raffle/{raffle}/open"), true))
            })
            .await
        }))
    }

    fn draw<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        request: &'a DrawRequest,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        // Draws take a few seconds (the committed round is in the future):
        // sent, progress arrives through the projection.
        Box::pin(sendable(async move {
            self.raffle(raffle_id)?
                .draw(Json(request.clone()))
                .idempotency_key(command_id)
                .send()
                .await
                .map(|_| ())
                .map_err(|e| engine_error(e, &format!("Raffle/{raffle_id}/draw/send"), false))
        }))
    }

    fn decide<'a>(
        &'a self,
        raffle_id: &'a RaffleId,
        decision: &'a Decision,
        command_id: &'a str,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(sendable(async move {
            let call = self.raffle(raffle_id)?.decide(Json(decision.clone()));
            self.bounded(async {
                call.idempotency_key(command_id)
                    .call()
                    .await
                    .and_then(CallResponse::into_body)
                    .map_err(|e| engine_error(e, &format!("Raffle/{raffle_id}/decide"), true))
            })
            .await
        }))
    }
}

#[cfg(test)]
mod tests {
    use rafi_raffle::ErrorKind;

    use super::*;

    #[test]
    fn refusals_keep_their_kind_and_message() {
        let e = refusal(
            "Raffle/r/decide",
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
            "Raffle/r/decide",
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
            refusal("Raffle/r/decide", 502, true, "bad gateway"),
            EngineError::Unavailable(_)
        ));
        // The ingress's own 404 (say, an unregistered service) is no refusal.
        assert!(matches!(
            refusal(
                "EngineInfo/info",
                404,
                false,
                r#"{"message":"service not found"}"#
            ),
            EngineError::Unavailable(_)
        ));
    }
}
