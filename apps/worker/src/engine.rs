//! The raffle engine's Restate endpoint, served by this Worker under
//! `/restate/*`. Restate calls it in request-response mode; each request is
//! handed to the SDK's [`Endpoint`], which verifies the request's identity
//! before any routing (discovery included) and routes on the path's last
//! segments, so the prefix is invisible to it.
//!
//! The engine is wired to Cloudflare here and nowhere else: draw states go to
//! D1 (the record) through rafi-store's [`RaffleWriter`], then the raffle's
//! Durable Object is nudged, best effort, through `rafi-cloudflare`.
//!
//! Configuration (`wrangler.toml` vars and secrets):
//!
//! | Name | Kind | Meaning |
//! | --- | --- | --- |
//! | `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` | secret | Comma-separated `publickeyv1_…` keys of the Restate environment. Required: without them the Worker refuses every request |
//! | `RAFI_DRAND_RELAYS` | var | Comma-separated drand relays (default: the built-in ones) |
//! | `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS` | var | Draw timing (defaults as natively) |
//! | `RAFI_DRAND_MOCK` | var | The mock chain, `dev` feature only. The app reads it too, so both sides draw from the same chain |
//!
//! With the `dev` feature only: the mock chain and running without keys.

use std::sync::Arc;

use drand::client::TowerTransport;
use http_body_util::BodyExt;
use rafi_cloudflare::{DurableObjects, lookup};
use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_store::RaffleWriter;
use restate_sdk::endpoint::{Endpoint, HandleOptions, ProtocolMode, ResponseBody};
use tower_worker_fetch::FetchService;
use worker::{Env, HttpRequest};

/// The path prefix Restate's requests arrive under. Registered with Restate
/// as `https://<app host>/restate`; it appends `/discover` and `/invoke/…`.
pub const PREFIX: &str = "/restate";

/// Whether a request path is the engine's: `/restate` or anything under it.
pub fn owns(path: &str) -> bool {
    path.strip_prefix(PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

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
