//! The Rafi draw engine on Cloudflare Workers.
//!
//! Restate calls this Worker over HTTPS in request-response mode; each request
//! is handed to the SDK's [`Endpoint`]. This crate only wires the draw engine
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
//! | `RAFI_RETENTION_DAYS`, `RAFI_RETENTION_WINNERS_DAYS` | var | Purge dates set when a raffle finishes (30, 90) |
//!
//! With the `dev` feature only: `RAFI_DRAND_MOCK=1` and running without keys.

#![cfg(target_arch = "wasm32")]

use http_body_util::BodyExt;
use rafi_cloudflare::{DurableObjects, lookup};
use rafi_drand::{DrandClient, DrandConfig};
use rafi_store::{DrawWriter, PurgeAfter};
use restate_rafi::Raffle;
use restate_sdk::endpoint::{Endpoint, HandleOptions, ProtocolMode, ResponseBody};
use worker::{Context, Env, HttpRequest, Result, event};

fn number<T: std::str::FromStr>(env: &Env, name: &str, default: T) -> T {
    lookup(env, name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn drand_config(env: &Env) -> DrandConfig {
    let defaults = DrandConfig::default();
    DrandConfig {
        relays: lookup(env, "RAFI_DRAND_RELAYS")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or(defaults.relays),
        mock: cfg!(feature = "dev") && lookup(env, "RAFI_DRAND_MOCK").as_deref() == Some("1"),
        lead_secs: number(env, "RAFI_DRAND_LEAD_SECS", defaults.lead_secs),
        timeout_secs: number(env, "RAFI_DRAND_TIMEOUT_SECS", defaults.timeout_secs),
    }
}

/// The SDK endpoint for this request's bindings.
fn endpoint(env: &Env) -> anyhow::Result<Endpoint> {
    let store = DrawWriter::new(
        rafi_cloudflare::database(env)?,
        PurgeAfter::days(
            number(env, "RAFI_RETENTION_DAYS", 30),
            number(env, "RAFI_RETENTION_WINNERS_DAYS", 90),
        ),
    );
    let raffle = Raffle::new(
        DrandClient::new(drand_config(env))?,
        store,
        DurableObjects::new(env.clone()),
    );
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
    Ok(builder.bind(raffle).build())
}

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> Result<http::Response<ResponseBody>> {
    let endpoint = endpoint(&env).map_err(|e| {
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
