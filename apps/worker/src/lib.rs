//! Rafi on Cloudflare Workers (`worker-build`, `wrangler.toml`): the one
//! Worker, named `rafi`, hosting the app and the raffle engine.
//!
//! - `fetch`, in this order:
//!   - `/restate/*`: the raffle engine's Restate endpoint (`engine`). Restate
//!     calls it over HTTPS; the SDK verifies each request's signature before
//!     anything else happens to it. Nothing else sees these requests: not
//!     Workers Assets, not Clerk's layer, not the app's routes;
//!   - `/partyline/raffle/{id}`: phones' live-channel sockets go to the raffle's
//!     [`RaffleRoom`];
//!   - everything else: the same axum routes as the native server
//!     ([`rafi_server::routes`]) behind Clerk. Workers Assets serves the web app
//!     and the participant build's files before the Worker is called
//!     (`run_worker_first` in `wrangler.toml` lists the Worker's paths).
//! - `scheduled`: the Cron Trigger runs housekeeping (`rafi_app::housekeeping::run`).
//! - [`RaffleRoom`] is exported from here, so the class lives in this Worker.
//!
//! Nothing of an installation's is compiled in, so one build of the Worker
//! serves every installation: its values are the Worker's secrets and vars
//! (`wrangler.toml` lists the required ones), and its legal documents (the
//! privacy notice, the terms with the processing agreement) are files among
//! the static files, read through the assets binding (`documents`); their
//! versions are the `RAFI_TERMS_VERSION` and `RAFI_PRIVACY_VERSION` secrets.
//!
//! The Cloudflare pieces come from `rafi-cloudflare`: the notifier over the
//! raffles' Durable Objects (the engine's observer, too) and configuration
//! from the Worker's vars and secrets (see `wrangler.toml`). The join's rate
//! limits, the app's own, are here (`RateLimitBindings`).
//! Missing secrets fail closed: every request is refused rather than run with
//! a default, the engine's included. Dev auth, the mock drand chain and
//! running without Restate identity keys need the `dev` feature.
//!
//! The app and the endpoint are built once per isolate (`cached`): the
//! bindings and vars are the isolate's, Clerk's layer keeps its verified
//! signing keys (JWKS) between requests instead of fetching them for each,
//! the drand client keeps its verified chain, and the documents are read
//! once. The `tracing` subscriber is installed with them (`log`), so the
//! application's warnings and errors reach the console.

#![cfg(target_arch = "wasm32")]

use std::{cell::OnceCell, sync::Arc};

use axum::body::Body;
use dioxus_clerk::server::{ClerkAuthLayer, ClerkAuthLayerConfig};
pub use rafi_cloudflare::RaffleRoom;
use rafi_cloudflare::{DurableObjects, lookup};
use restate_sdk::endpoint::Endpoint;
use tower_service::Service;
use worker::{Context, Env, HttpRequest, ScheduleContext, ScheduledEvent, event};

use rafi_server::{self as server, AppState, config::Config};

mod documents;
mod engine;
mod log;
mod ratelimit;

use ratelimit::RateLimitBindings;

/// The isolate's state: the app, Clerk's layer (none under dev auth) and the
/// raffle engine's endpoint.
#[derive(Clone)]
struct App {
    state: AppState,
    auth: Option<ClerkAuthLayer>,
    endpoint: Endpoint,
}

thread_local! {
    /// The app, built on the isolate's first request (concurrent first
    /// requests may each build one; the first to finish is kept). A failed
    /// build isn't kept: a fixed secret takes effect on the next request.
    static APP: OnceCell<App> = const { OnceCell::new() };
}

/// The isolate's app, built on first use: the configuration from the vars
/// and secrets, the documents from the static files, then everything else.
async fn cached(env: &Env) -> anyhow::Result<App> {
    if let Some(app) = APP.with(|cell| cell.get().cloned()) {
        return Ok(app);
    }
    log::install();
    let mut cfg = Config::from_lookup(|name| lookup(env, name));
    cfg.documents = documents::documents(env, cfg.dev_auth).await?;
    let app = app(env, cfg)?;
    Ok(APP.with(|cell| cell.get_or_init(|| app).clone()))
}

/// The app over the Worker's bindings. Fails closed on missing secrets.
fn app(env: &Env, cfg: Config) -> anyhow::Result<App> {
    let dev = cfg!(feature = "dev");
    if cfg.dev_auth && !dev {
        anyhow::bail!("RAFI_DEV_AUTH is set, but this build has no `dev` feature");
    }
    if lookup(env, "RAFI_ALTCHA_KEY").is_none() {
        anyhow::bail!("RAFI_ALTCHA_KEY is not set");
    }
    if cfg.restate.ingress_url.is_empty() {
        anyhow::bail!("RAFI_ENGINE_INGRESS_URL is not set");
    }
    // On Workers the ingress is Restate Cloud's, which wants a key; without
    // one, draws would fail at the first call with nothing in the log.
    if cfg.restate.api_key.is_empty() && !dev {
        anyhow::bail!("RAFI_ENGINE_API_KEY is not set");
    }
    let auth = if cfg.dev_auth {
        None
    } else {
        let key = lookup(env, "CLERK_SECRET_KEY")
            .ok_or_else(|| anyhow::anyhow!("CLERK_SECRET_KEY is not set"))?;
        Some(
            rafi_server::auth::clerk_layer(&cfg, ClerkAuthLayerConfig::new(key))
                .map_err(|e| anyhow::anyhow!("Clerk configuration: {e:?}"))?,
        )
    };
    let endpoint = engine::endpoint(env)?;
    let remote = server::restate_engine(&cfg);
    let state = AppState::from_parts(
        cfg,
        rafi_cloudflare::database(env)?,
        remote,
        rafi_live_server::LiveUpdates::new(
            Arc::new(DurableObjects::new(env.clone())),
            DurableObjects::follow,
        ),
        Arc::new(RateLimitBindings::new(env.clone())),
    )?;
    Ok(App {
        state,
        auth,
        endpoint,
    })
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
    let path = req.uri().path();
    if engine::owns(path) {
        let App { endpoint, .. } = match cached(&env).await {
            Ok(app) => app,
            Err(e) => return Ok(misconfigured(e)),
        };
        return Ok(engine::handle(&endpoint, req).map(Body::new));
    }
    if let Some(raffle_id) = path.strip_prefix("/partyline/raffle/") {
        let raffle_id = raffle_id.to_owned();
        let req = worker::Request::try_from(req)?;
        return Ok(rafi_cloudflare::connect(&env, &raffle_id, req)
            .await?
            .into());
    }
    let App { state, auth, .. } = match cached(&env).await {
        Ok(app) => app,
        Err(e) => return Ok(misconfigured(e)),
    };
    let router = server::routes(state);
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
    let App { state, .. } = match cached(&env).await {
        Ok(app) => app,
        Err(e) => {
            worker::console_error!("rafi: housekeeping not run: {e:#}");
            return;
        }
    };
    match server::service::housekeeping::run(&state).await {
        Ok(report) => worker::console_log!(
            "rafi: housekeeping: {} raffles expired, {} entrants erased, {} raffles purged, {} live channels ended",
            report.expired,
            report.erased,
            report.purged,
            report.channels_ended
        ),
        Err(e) => worker::console_error!("rafi: housekeeping failed: {e:#}"),
    }
}
