//! Rafi's app on Cloudflare Workers: everything a Worker serves but the
//! raffle engine, for the app Worker (`apps/worker-app`, production) and the
//! combined one (`apps/worker`, local development). The entry crates only
//! dispatch to it and export its handlers.
//!
//! - [`serve`]:
//!   - `/partyline/raffle/{id}`: phones' live-channel sockets go to the
//!     raffle's `RaffleRoom` (`rafi_cloudflare::connect`);
//!   - everything else: the same axum routes as the native server
//!     ([`rafi_server::routes`]), with the web app's shell (`SHELL`) at
//!     every path they don't answer, behind Clerk, with Cloudflare's Ray ID
//!     as each request's ID (`rafi_server::trace`). Workers Assets serves the
//!     web app's and the participant build's files before the Worker is
//!     called; a path that isn't a file comes here (no single-page fallback
//!     in `wrangler.toml`: the shell carries a Content-Security-Policy naming
//!     the installation's Clerk host, which the static files' `_headers`
//!     can't know).
//!
//!   The raffle engine's paths (`rafi_cloudflare::is_restate_path`) are the
//!   entry crate's to route: the engine (the combined Worker) or a 404 (the
//!   app Worker).
//! - [`housekeeping`]: one run, as the Cron Trigger fires it
//!   (`rafi_app::housekeeping::run`), exported by the entry crate with
//!   [`export_scheduled!`]; a run that fails, or can't start, fails the
//!   invocation.
//!
//! The `RaffleRoom` class isn't here: phones' sockets reach the objects
//! through the `RAFFLE_ROOM` binding (`rafi-cloudflare`'s `connect`
//! feature), whose class is the live Worker's in production
//! (`apps/worker-live`) and the combined Worker's own locally.
//!
//! Nothing of an installation's is compiled in, so one build of the Worker
//! serves every installation: its values are the Worker's secrets and vars
//! (`wrangler.toml` lists the required ones), and its legal documents (the
//! privacy notice, the terms with the processing agreement) are files among
//! the static files, read through the assets binding (`documents`), each
//! naming its version in its front matter.
//!
//! The Cloudflare pieces come from `rafi-cloudflare`: the notifier over the
//! raffles' Durable Objects and configuration from the Worker's vars and
//! secrets (see `wrangler.toml`). The join's rate limits, the app's own, are
//! here (`RateLimitBindings`). Missing secrets fail closed: every request is
//! refused rather than run with a default. Dev auth and the mock drand chain
//! need the `dev` feature.
//!
//! The app and its router are built once per isolate (`cached`): the
//! bindings and vars are the isolate's, Clerk's layer keeps its verified
//! signing keys (JWKS) between requests instead of fetching them for each,
//! the documents are read once, and each request clones the router (an
//! `Arc`) instead of building its routes and layers again. The `tracing`
//! subscriber is installed with them (`rafi_cloudflare::log`), so the
//! application's warnings and errors reach the console.

#![cfg(target_arch = "wasm32")]

use std::{cell::OnceCell, sync::Arc};

use axum::Router;
pub use axum::body::Body;
use dioxus_clerk::server::ClerkAuthLayerConfig;
use rafi_cloudflare::{DurableObjects, lookup};
use tower_service::Service;
use worker::{Env, HttpRequest};

use rafi_server::{self as server, AppState, config::Config};

mod documents;
mod ratelimit;

use ratelimit::RateLimitBindings;

/// The web app's `index.html`, as the build left it: compiled in, as it
/// ships in one artifact with the files it links (`just artifact` in
/// `apps/worker-app` builds the web app first), and isn't among the static
/// files, so Workers Assets can't serve it without its policy.
const SHELL: &str = include_str!("../../../target/dx/rafi-web/release/web/public/index.html");

/// The isolate's state: the app (for the Cron) and its routes behind Clerk's
/// layer (no layer under dev auth).
#[derive(Clone)]
struct App {
    state: AppState,
    router: Router,
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
    rafi_cloudflare::log::install();
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
    if lookup(env, "RAFI_POW_KEY").is_none() {
        anyhow::bail!("RAFI_POW_KEY is not set");
    }
    if cfg.restate.ingress_url.is_empty() {
        anyhow::bail!("RESTATE_INGRESS_URL is not set");
    }
    // On Workers the ingress is Restate Cloud's, which wants a key; without
    // one, draws would fail at the first call with nothing in the log.
    if cfg.restate.api_key.is_empty() && !dev {
        anyhow::bail!("RESTATE_INGRESS_API_KEY is not set");
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
        rafi_app::Clock::system(),
    )?;
    let shell = server::Shell::new(SHELL)?;
    let router =
        server::routes(state.clone()).fallback_service(server::shell_routes(&state, Some(shell)));
    let router = match auth {
        Some(layer) => router.layer(layer),
        None => router,
    };
    // Around Clerk's layer, so its refusals have an ID too.
    let router = server::trace::layer(router, server::trace::RequestIds::cloudflare());
    Ok(App { state, router })
}

/// Every request refused: why in the log, and in the body when the reason
/// names only a setting or the documents (`rafi_server::config::refusal`).
fn misconfigured(e: anyhow::Error) -> http::Response<Body> {
    worker::console_error!("rafi: {e:#}");
    http::Response::builder()
        .status(500)
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(server::config::refusal(&e, "`wrangler tail`")))
        .expect("a static response")
}

/// Serves one request of the app's: a phone's socket to its raffle's room,
/// or the routes (the shell at every path they don't answer). Every request
/// is refused (500) while the app can't be built.
pub async fn serve(req: HttpRequest, env: &Env) -> worker::Result<http::Response<Body>> {
    if let Some(raffle_id) = req.uri().path().strip_prefix("/partyline/raffle/") {
        let raffle_id = raffle_id.to_owned();
        let req = worker::Request::try_from(req)?;
        return Ok(rafi_cloudflare::connect(env, &raffle_id, req).await?.into());
    }
    let App { mut router, .. } = match cached(env).await {
        Ok(app) => app,
        Err(e) => return Ok(misconfigured(e)),
    };
    Ok(router
        .call(req.map(Body::new))
        .await
        .unwrap_or_else(|never| match never {}))
}

/// A 404, for the raffle engine's paths in a Worker without the engine.
pub fn not_found() -> http::Response<Body> {
    http::Response::builder()
        .status(http::StatusCode::NOT_FOUND)
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("not found\n"))
        .expect("a static response")
}

/// One housekeeping run, as the Cron Trigger fires it. An error is a run that
/// didn't happen (the app didn't build) or didn't finish.
pub async fn housekeeping(env: &Env) -> anyhow::Result<()> {
    let App { state, .. } = cached(env)
        .await
        .map_err(|e| e.context("housekeeping not run"))?;
    let report = server::service::housekeeping::run(&state)
        .await
        .map_err(|e| e.context("housekeeping failed"))?;
    worker::console_log!(
        "rafi: housekeeping: {} raffles expired, {} entrants erased ({} who took themselves off), {} removals at their request forgotten, {} raffles purged, {} live channels ended",
        report.expired,
        report.erased,
        report.objected,
        report.removals_forgotten,
        report.purged,
        report.channels_ended
    );
    Ok(())
}

/// Exports the Cron Trigger's handler, `scheduled`, running [`housekeeping`],
/// from the Worker crate it is invoked in (which depends on `worker`).
///
/// By hand, not `#[event(scheduled)]`: that export always resolves, so a run
/// that failed would show as a success in the Cron history. This one rejects
/// instead, and the invocation is an exception there. The export is the
/// macro's otherwise (worker 0.8: same name, arguments and `wasm_bindgen`
/// paths; no `experimental_tokio` here). A macro, so the export is the entry
/// crate's: a library's `#[wasm_bindgen]` export would be in every Worker
/// linking it.
// rustfmt indents the attribute's lines further on every run inside a macro.
#[rustfmt::skip]
#[macro_export]
macro_rules! export_scheduled {
    () => {
        mod scheduled {
            use ::worker::{
                Env, js_sys,
                wasm_bindgen::{JsValue, prelude::wasm_bindgen},
                worker_sys::{ScheduleContext, ScheduledEvent},
            };

            #[wasm_bindgen(
                wasm_bindgen = ::worker::wasm_bindgen,
                wasm_bindgen_futures = ::worker::wasm_bindgen_futures,
                js_sys = ::worker::js_sys
            )]
            pub async fn scheduled(
                _event: ScheduledEvent,
                env: Env,
                _ctx: ScheduleContext,
            ) -> Result<JsValue, JsValue> {
                $crate::housekeeping(&env).await.map_err(|e| {
                    let message = format!("rafi: {e:#}");
                    ::worker::console_error!("{message}");
                    JsValue::from(js_sys::Error::new(&message))
                })?;
                Ok(JsValue::UNDEFINED)
            }
        }
    };
}
