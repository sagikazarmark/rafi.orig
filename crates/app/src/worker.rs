//! The app on Cloudflare Workers.
//!
//! - `fetch`: phones' live-channel sockets (`/partyline/raffle/{id}`) go to the
//!   raffle's [`RaffleChannel`]; everything else is the same axum routes as the
//!   native server ([`server::routes`]) behind Clerk. Workers Assets serves the
//!   web app and the attendee scripts before the Worker is called
//!   (`run_worker_first` in `wrangler.toml` lists the Worker's paths).
//! - `scheduled`: the Cron Trigger runs [`server::retention::run`].
//! - [`RaffleChannel`] is exported from here, so the class lives in this Worker.
//!
//! Configuration comes from the Worker's vars and secrets (see `wrangler.toml`).
//! Missing secrets fail closed: every request is refused rather than run with
//! a default. Dev auth and mock drand need the `worker-dev` feature.

use std::sync::Arc;

use axum::body::Body;
use dioxus_clerk::server::{ClerkAuthLayer, ClerkAuthLayerConfig};
pub use rafi_channel::RaffleChannel;
use rafi_store::Db;
use tower_service::Service;
use worker::{
    Context, Env, HttpRequest, ScheduleContext, ScheduledEvent, event,
    send::{SendFuture, SendWrapper},
};

use crate::server::{
    self, AppState,
    config::Config,
    live::{BoxFuture, Live},
    ratelimit::{Limit, RateLimit},
};

/// The live channel: one Durable Object per raffle.
struct Channels(SendWrapper<Env>);

impl Live for Channels {
    fn has_channel(&self) -> bool {
        true
    }

    fn nudge<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(SendFuture::new(async move {
            if let Err(e) = rafi_channel::nudge(&self.0, raffle_id).await {
                worker::console_error!("rafi: nudging {raffle_id}: {e}");
            }
        }))
    }

    fn destroy<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(SendFuture::new(async move {
            rafi_channel::destroy(&self.0, raffle_id)
                .await
                .map_err(|e| anyhow::anyhow!("ending the live channel of {raffle_id}: {e}"))
        }))
    }
}

/// Cloudflare's rate-limiting bindings: approximate and per location, so a
/// soft limit (the hard one is the entrant cap in the database). A missing
/// binding allows.
struct Bindings(SendWrapper<Env>);

impl Bindings {
    fn binding(limit: Limit) -> &'static str {
        match limit {
            Limit::JoinPerRaffle => "JOIN_PER_RAFFLE_LIMITER",
            Limit::JoinPerEmail => "JOIN_PER_EMAIL_LIMITER",
        }
    }
}

impl RateLimit for Bindings {
    fn allow<'a>(&'a self, limit: Limit, key: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(SendFuture::new(async move {
            let Ok(limiter) = self.0.rate_limiter(Self::binding(limit)) else {
                return true;
            };
            match limiter.limit(key.to_owned()).await {
                Ok(outcome) => outcome.success,
                Err(e) => {
                    worker::console_error!("rafi: rate limiter {}: {e}", Self::binding(limit));
                    true
                }
            }
        }))
    }
}

/// A secret, else a var; empty counts as unset.
fn lookup(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|s| s.to_string())
        .ok()
        .or_else(|| env.var(name).map(|v| v.to_string()).ok())
        .filter(|v| !v.trim().is_empty())
}

/// The app for this request's bindings. Fails closed on missing secrets.
fn app(env: &Env) -> anyhow::Result<(AppState, Option<ClerkAuthLayer>)> {
    let cfg = Config::from_lookup(|name| match name {
        // Erasure is on unless switched off explicitly (docs/workers-plan.md §2.10).
        "RAFI_RETENTION" => lookup(env, name).or_else(|| Some("1".into())),
        _ => lookup(env, name),
    });
    let dev = cfg!(feature = "worker-dev");
    if cfg.dev_auth && !dev {
        anyhow::bail!("RAFI_DEV_AUTH is set, but this build has no `worker-dev` feature");
    }
    if cfg.drand.mock && !dev {
        anyhow::bail!("RAFI_DRAND_MOCK is set, but this build has no `worker-dev` feature");
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
    let db = Db::d1(env.d1(rafi_channel::D1_BINDING)?);
    let app = AppState::from_parts(
        cfg,
        db,
        Arc::new(Channels(SendWrapper(env.clone()))),
        Arc::new(Bindings(SendWrapper(env.clone()))),
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
        let db = Db::d1(env.d1(rafi_channel::D1_BINDING)?);
        let req = worker::Request::try_from(req)?;
        return Ok(rafi_channel::connect(&env, &db, &raffle_id, req)
            .await?
            .into());
    }
    let (app, auth) = match app(&env) {
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
    let app = match app(&env) {
        Ok((app, _)) => app,
        Err(e) => {
            worker::console_error!("rafi: retention not run: {e:#}");
            return;
        }
    };
    match server::retention::run(&app).await {
        Ok(report) => worker::console_log!(
            "rafi: retention: {} erased, {} live channels ended",
            report.erased,
            report.channels_ended
        ),
        Err(e) => worker::console_error!("rafi: retention failed: {e:#}"),
    }
}
