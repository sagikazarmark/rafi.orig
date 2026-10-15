//! The Rafi raffle engine on Cloudflare Workers.
//!
//! Restate calls this Worker over HTTPS in request-response mode; each request
//! is handed to the SDK's [`Endpoint`]. This crate only wires the raffle engine
//! to Cloudflare: draw states go to D1 (the record) through rafi-store, then
//! the raffle's Durable Object is nudged, best effort, through
//! `rafi-cloudflare`.
//!
//! Configuration (`wrangler.toml` vars and secrets):
//!
//! | Name | Kind | Meaning |
//! | --- | --- | --- |
//! | `RESTATE_IDENTITY_KEYS` | secret | Comma-separated `publickeyv1_…` keys of the Restate environment. Required: without them every request is refused |
//! | `RAFI_DRAND_RELAYS` | var | Comma-separated drand relays (default: the built-in ones) |
//! | `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS` | var | Draw timing (defaults as natively) |
//!
//! With the `dev` feature only: `RAFI_DRAND_MOCK=1` and running without keys.

#![cfg(target_arch = "wasm32")]

use std::{cell::OnceCell, rc::Rc, sync::Arc};

use http_body_util::BodyExt;
use rafi_cloudflare::{DurableObjects, lookup};
use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_store::RaffleWriter;
use restate_sdk::endpoint::{Endpoint, HandleOptions, ProtocolMode, ResponseBody};
use worker::{Context, Env, HttpRequest, Result, event};

fn drand_config(env: &Env) -> DrandConfig {
    let config = DrandConfig::from_lookup(|name| lookup(env, name));
    DrandConfig {
        // Production ignores the mock chain.
        mock: cfg!(feature = "dev") && config.mock,
        ..config
    }
}

/// The SDK endpoint over the Worker's bindings.
fn endpoint(env: &Env) -> anyhow::Result<Endpoint> {
    let store = RaffleWriter::new(rafi_cloudflare::database(env)?);
    let drand = DrandClient::new(drand_config(env))?;
    // A raffle's changes go to its room.
    let observer = Arc::new(DurableObjects::new(env.clone()));
    let (raffle, info) = rafi_engine_restate::services(drand, store, Some(observer));
    let mut builder = Endpoint::builder();
    let keys = lookup(env, "RESTATE_IDENTITY_KEYS").unwrap_or_default();
    let keys: Vec<&str> = keys
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() && !cfg!(feature = "dev") {
        anyhow::bail!("RESTATE_IDENTITY_KEYS is not set: refusing every request");
    }
    for key in keys {
        builder = builder
            .identity_key(key)
            .map_err(|e| anyhow::anyhow!("invalid identity key: {e:?}"))?;
    }
    Ok(builder.bind(raffle).bind(info).build())
}

thread_local! {
    /// The endpoint, built on the isolate's first request: the bindings and
    /// vars are the isolate's, so every request after reuses it (and the
    /// drand client's verified chain). A failed build isn't kept.
    static ENDPOINT: OnceCell<Rc<Endpoint>> = const { OnceCell::new() };
}

fn cached(env: &Env) -> anyhow::Result<Rc<Endpoint>> {
    ENDPOINT.with(|cell| {
        if let Some(endpoint) = cell.get() {
            return Ok(endpoint.clone());
        }
        let endpoint = Rc::new(endpoint(env)?);
        Ok(cell.get_or_init(|| endpoint).clone())
    })
}

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> Result<http::Response<ResponseBody>> {
    let endpoint = cached(&env).map_err(|e| {
        worker::console_error!("rafi-restate-worker: {e:#}");
        worker::Error::RustError("misconfigured".into())
    })?;
    // The SDK wants a body whose error is `Send`; a Workers error holds a JS value.
    let req = req.map(|body| body.map_err(|e| e.to_string()));
    // Identity is verified before any routing, discovery included.
    Ok(endpoint.handle_with_options(
        req,
        HandleOptions {
            protocol_mode: ProtocolMode::RequestResponse,
        },
    ))
}
