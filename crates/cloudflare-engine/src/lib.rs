//! Rafi's raffle engine on Cloudflare Workers: the Restate endpoint, served
//! under `/restate/*` by the engine Worker (`apps/worker-engine`, production)
//! and the combined one (`apps/worker`, local development). Restate calls it
//! in request-response mode; each request is handed to the SDK's
//! [`Endpoint`], which verifies the request's identity before any routing
//! (discovery included) and routes on the path's last segments, so the
//! prefix is invisible to it.
//!
//! The engine is wired to Cloudflare here and nowhere else: draw states go to
//! D1 (the record) through rafi-store's [`RaffleWriter`], then the raffle's
//! Durable Object is nudged, best effort, through `rafi-cloudflare`'s
//! [`DurableObjects`] over the `RAFFLE_ROOM` binding (in the engine Worker,
//! a binding to the live Worker's class, `script_name`: the class isn't
//! exported here). Nothing of the app's is linked: no routes, no Clerk, no
//! secrets.
//!
//! Configuration (`wrangler.toml` bindings and vars; none of it secret):
//!
//! | Name | Kind | Meaning |
//! | --- | --- | --- |
//! | `DB` | D1 binding | Rafi's database, the app's |
//! | `RAFFLE_ROOM` | Durable Object binding | The raffles' `RaffleRoom` objects |
//! | `RAFI_DO_JURISDICTION` | var | Optional: the objects' jurisdiction (`eu`), as the live Worker's and the app's; unset, the plain namespace |
//! | `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` | var | Comma-separated `publickeyv1_…` keys of the Restate environment. Required: without them the endpoint refuses every request |
//! | `RAFI_DRAND_RELAYS` | var | Comma-separated drand relays (default: the built-in ones) |
//! | `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS` | var | Draw timing (defaults as natively) |
//! | `RAFI_DRAND_MOCK` | var | The mock chain, `dev` feature only. The app reads it too, so both sides draw from the same chain |
//!
//! With the `dev` feature only: the mock chain and running without keys.
//!
//! The endpoint is built once per isolate ([`cached`]), with the `tracing`
//! subscriber (`rafi_cloudflare::log`): the drand client keeps its verified
//! chain between requests. A failed build isn't kept, so a fixed var takes
//! effect on the next request.

#![cfg(target_arch = "wasm32")]

use std::{cell::OnceCell, sync::Arc};

use bytes::Bytes;
use drand::client::TowerTransport;
use http_body_util::{BodyExt, Either, Full};
use rafi_cloudflare::{DurableObjects, lookup};
use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_store::RaffleWriter;
use restate_sdk::endpoint::{Endpoint, HandleOptions, ProtocolMode, ResponseBody};
use tower_worker_fetch::FetchService;
use worker::{Env, HttpRequest};

pub use rafi_cloudflare::{RESTATE_PREFIX as PREFIX, is_restate_path as owns};

/// A response of the engine's: the SDK's, or a refusal or 404 of its own.
pub type Body = Either<ResponseBody, Full<Bytes>>;

fn drand_config(env: &Env) -> anyhow::Result<DrandConfig> {
    let config = DrandConfig::from_lookup(|name| lookup(env, name))?;
    Ok(DrandConfig {
        // Production ignores the mock chain.
        mock: cfg!(feature = "dev") && config.mock,
        ..config
    })
}

/// The SDK endpoint over the Worker's bindings. Fails closed on missing keys.
pub fn endpoint(env: &Env) -> anyhow::Result<Endpoint> {
    let store = RaffleWriter::new(rafi_cloudflare::database(env)?);
    // The relays are asked through the Workers `fetch` API. The client's own
    // per-request timeout bounds each one; dropping the request aborts it.
    let drand = DrandClient::builder(drand_config(env)?)
        .http_transport(TowerTransport::new(FetchService::new()))
        .build()?;
    // A raffle's changes go to its room.
    let observer = Arc::new(DurableObjects::new(env.clone()));
    let (raffle, info) = rafi_engine_restate::services(drand, store, Some(observer));
    let mut builder = Endpoint::builder();
    let keys = lookup(env, "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS").unwrap_or_default();
    let keys: Vec<&str> = keys
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() && !cfg!(feature = "dev") {
        anyhow::bail!("RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is not set: refusing every request");
    }
    for key in keys {
        builder = builder
            .identity_key(key)
            .map_err(|e| anyhow::anyhow!("invalid identity key: {e:?}"))?;
    }
    Ok(builder.bind(raffle).bind(info).build())
}

thread_local! {
    /// The endpoint, built on the isolate's first request to it (concurrent
    /// first requests may each build one; the first to finish is kept).
    static ENDPOINT: OnceCell<Endpoint> = const { OnceCell::new() };
}

/// The isolate's endpoint, built on first use (and the `tracing` subscriber
/// installed with it). A failed build isn't kept.
pub fn cached(env: &Env) -> anyhow::Result<Endpoint> {
    if let Some(endpoint) = ENDPOINT.with(|cell| cell.get().cloned()) {
        return Ok(endpoint);
    }
    rafi_cloudflare::log::install();
    let endpoint = endpoint(env)?;
    Ok(ENDPOINT.with(|cell| cell.get_or_init(|| endpoint).clone()))
}

/// Answers one of Restate's requests. The path is passed through unchanged:
/// the SDK checks the signature's audience against it.
pub fn handle(endpoint: &Endpoint, req: HttpRequest) -> http::Response<ResponseBody> {
    // The SDK wants a body whose error is `Send`; a Workers error holds a JS value.
    let req = req.map(|body| body.map_err(|e| e.to_string()));
    endpoint.handle_with_options(
        req,
        HandleOptions {
            protocol_mode: ProtocolMode::RequestResponse,
        },
    )
}

/// Serves one of Restate's requests (a path [`owns`] says is the engine's)
/// with the isolate's endpoint; a Worker that can't build one refuses with
/// 500, why in the log.
pub fn serve(req: HttpRequest, env: &Env) -> http::Response<Body> {
    match cached(env) {
        Ok(endpoint) => handle(&endpoint, req).map(Either::Left),
        Err(e) => misconfigured(e),
    }
}

/// A 404, for every path that isn't the engine's.
pub fn not_found() -> http::Response<Body> {
    text(http::StatusCode::NOT_FOUND, "not found\n".into())
}

/// Every request refused: why in the log, and in the body when the reason
/// names only a setting (never a value).
fn misconfigured(e: anyhow::Error) -> http::Response<Body> {
    worker::console_error!("rafi: engine: {e:#}");
    let reason = e.root_cause().to_string();
    let body = if reason.ends_with(" is not set") || reason.contains(" is not set:") {
        format!("misconfigured: {reason}\n\nThe Worker's log has the details (`wrangler tail`).\n")
    } else {
        "misconfigured\n\nThe reason is in the Worker's log (`wrangler tail`).\n".to_owned()
    };
    text(http::StatusCode::INTERNAL_SERVER_ERROR, body)
}

fn text(status: http::StatusCode, body: String) -> http::Response<Body> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Either::Right(Full::new(Bytes::from(body))))
        .expect("a static response")
}
