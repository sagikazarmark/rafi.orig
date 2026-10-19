//! The app on Cloudflare Workers (`worker-build`, `wrangler.toml`): the
//! Worker named `rafi`.
//!
//! - `fetch`: phones' live-channel sockets (`/partyline/raffle/{id}`) go to the
//!   raffle's [`RaffleRoom`]; everything else is the same axum routes as the
//!   native server ([`rafi_server::routes`]) behind Clerk. Workers Assets serves the
//!   web app and the participant scripts before the Worker is called
//!   (`run_worker_first` in `wrangler.toml` lists the Worker's paths).
//! - `scheduled`: the Cron Trigger runs [`rafi_app::housekeeping::run`].
//! - [`RaffleRoom`] is exported from here, so the class lives in this Worker.
//!
//! The Cloudflare pieces come from `rafi-cloudflare`: the notifier over the
//! raffles' Durable Objects and configuration from the Worker's vars and
//! secrets (see `wrangler.toml`). The join's rate limits, the app's own, are
//! here ([`RateLimitBindings`]).
//! Missing secrets fail closed: every request is refused rather than run with
//! a default. Dev auth and mock drand need the `dev` feature.
//!
//! The app is built once per isolate ([`cached`]): the bindings and vars are
//! the isolate's, and Clerk's layer keeps its verified signing keys (JWKS)
//! between requests instead of fetching them for each.

#![cfg(target_arch = "wasm32")]

use std::{cell::OnceCell, sync::Arc};

use axum::body::Body;
use dioxus_clerk::server::{ClerkAuthLayer, ClerkAuthLayerConfig};
pub use rafi_cloudflare::RaffleRoom;
use rafi_cloudflare::{DurableObjects, lookup};
use tower_service::Service;
use worker::{Context, Env, HttpRequest, ScheduleContext, ScheduledEvent, event};

use rafi_server::{self as server, AppState, config::Config};

mod ratelimit;

use ratelimit::RateLimitBindings;

type App = (AppState, Option<ClerkAuthLayer>);

thread_local! {
    /// The app, built on the isolate's first request. A failed build isn't
    /// kept: a fixed secret takes effect on the next request.
    static APP: OnceCell<App> = const { OnceCell::new() };
}

/// The isolate's app, built on first use.
fn cached(env: &Env) -> anyhow::Result<App> {
    APP.with(|cell| {
        if let Some(app) = cell.get() {
            return Ok(app.clone());
        }
        let app = app(env)?;
        Ok(cell.get_or_init(|| app).clone())
    })
}

/// The app over the Worker's bindings. Fails closed on missing secrets.
fn app(env: &Env) -> anyhow::Result<App> {
    let cfg = Config::from_lookup(|name| lookup(env, name));
    let dev = cfg!(feature = "dev");
    if cfg.dev_auth && !dev {
        anyhow::bail!("RAFI_DEV_AUTH is set, but this build has no `dev` feature");
    }
    if lookup(env, "RAFI_ALTCHA_KEY").is_none() {
        anyhow::bail!("RAFI_ALTCHA_KEY is not set");
    }
    if cfg.restate.ingress_url.is_empty() {
        anyhow::bail!("RESTATE_INGRESS_URL is not set");
    }
    let auth = if cfg.dev_auth {
        None
    } else {
        let key = lookup(env, "CLERK_SECRET_KEY")
            .ok_or_else(|| anyhow::anyhow!("CLERK_SECRET_KEY is not set"))?;
        Some(
            ClerkAuthLayer::from_config(ClerkAuthLayerConfig::new(key))
                .map_err(|e| anyhow::anyhow!("Clerk configuration: {e:?}"))?,
        )
    };
    let engine = server::restate_engine(&cfg);
    let app = AppState::from_parts(
        cfg,
        rafi_cloudflare::database(env)?,
        engine,
        rafi_live_server::LiveUpdates::new(
            Arc::new(DurableObjects::new(env.clone())),
            DurableObjects::follow,
        ),
        Arc::new(RateLimitBindings::new(env.clone())),
    )?;
    Ok((app, auth))
}

fn misconfigured(e: anyhow::Error) -> http::Response<Body> {
    worker::console_error!("rafi: {e:#}");
    http::Response::builder()
        .status(500)
        .body(Body::from("misconfigured"))
        .expect("a static response")
}

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> worker::Result<http::Response<Body>> {
    if let Some(raffle_id) = req.uri().path().strip_prefix("/partyline/raffle/") {
        let raffle_id = raffle_id.to_owned();
        let req = worker::Request::try_from(req)?;
        return Ok(rafi_cloudflare::connect(&env, &raffle_id, req)
            .await?
            .into());
    }
    let (app, auth) = match cached(&env) {
        Ok(parts) => parts,
        Err(e) => return Ok(misconfigured(e)),
    };
    let router = server::routes(app);
    let mut router = match auth {
        Some(layer) => router.layer(layer),
        None => router,
    };
    Ok(router
        .call(req.map(Body::new))
        .await
        .unwrap_or_else(|never| match never {}))
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let app = match cached(&env) {
        Ok((app, _)) => app,
        Err(e) => {
            worker::console_error!("rafi: housekeeping not run: {e:#}");
            return;
        }
    };
    match server::service::housekeeping::run(&app).await {
        Ok(report) => worker::console_log!(
            "rafi: housekeeping: {} live channels ended",
            report.channels_ended
        ),
        Err(e) => worker::console_error!("rafi: housekeeping failed: {e:#}"),
    }
}
