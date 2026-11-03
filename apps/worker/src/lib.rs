//! Rafi on Cloudflare Workers for local development (`worker-build`,
//! `wrangler.toml`): the one Worker, named `rafi`, hosting the app and the
//! raffle engine, as `wrangler dev --env dev`, `just workers` and the devenv
//! run it. Production runs them as three Workers, the app's
//! (`apps/worker-app`), the engine's (`apps/worker-engine`) and the rooms
//! Worker (`apps/worker-rooms`, the `RaffleRoom` class), over the same
//! libraries; this one combines their dispatch and exports:
//!
//! - `fetch`, in this order:
//!   - `/restate/*`: the raffle engine's Restate endpoint
//!     (`rafi_cloudflare_engine`). Restate calls it over HTTP; the SDK
//!     verifies each request's signature before anything else happens to it
//!     (with the `dev` feature, keys are optional). Nothing else sees these
//!     requests: not Workers Assets, not Clerk's layer, not the app's routes.
//!     The engine builds without the app's secrets, as in its own Worker;
//!   - everything else: the app (`rafi_cloudflare_app::serve`): phones'
//!     sockets to their raffle's `RaffleRoom`, and `rafi-server`'s routes
//!     behind Clerk, with the web app's shell.
//! - `scheduled`: the Cron Trigger runs housekeeping
//!   (`rafi_cloudflare_app::export_scheduled!`); a run that fails, or can't
//!   start, fails the invocation.
//! - [`RaffleRoom`] is exported from here, so the class lives in this Worker.
//!
//! Missing secrets fail closed: every request of the app's is refused rather
//! than run with a default, and the engine refuses without identity keys.
//! Dev auth, the mock drand chain and running without Restate identity keys
//! need the `dev` feature. The top level of `wrangler.toml` is a one-Worker
//! production shape, which `scripts/workers-check.sh` builds and checks; a
//! release ships the three production Workers (`apps/worker-app`,
//! `apps/worker-engine`, `apps/worker-rooms`) instead.

#![cfg(target_arch = "wasm32")]

pub use rafi_cloudflare::RaffleRoom;
use rafi_cloudflare_app::Body;
use worker::{Context, Env, HttpRequest, event};

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> worker::Result<http::Response<Body>> {
    if rafi_cloudflare_engine::owns(req.uri().path()) {
        return Ok(rafi_cloudflare_engine::serve(req, &env).map(Body::new));
    }
    rafi_cloudflare_app::serve(req, &env).await
}

rafi_cloudflare_app::export_scheduled!();
