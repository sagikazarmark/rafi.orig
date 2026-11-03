//! Rafi's app Worker on Cloudflare (`worker-build`, `wrangler.toml`), named
//! `rafi`: production's Worker that serves people, beside the raffle
//! engine's (`apps/worker-engine`), which Restate calls, and the rooms
//! Worker (`apps/worker-rooms`), which hosts the raffles' `RaffleRoom`
//! objects.
//!
//! - `fetch`: the raffle engine's paths (`/restate`, `/restate/*`) answer 404,
//!   never the web app's shell, since the engine is another Worker's;
//!   everything else is the app's (`rafi_cloudflare_app::serve`): phones'
//!   sockets to their raffle's `RaffleRoom`, and `rafi-server`'s routes
//!   behind Clerk, with the web app's shell.
//! - `scheduled`: the Cron Trigger runs housekeeping
//!   (`rafi_cloudflare_app::export_scheduled!`).
//! - No `RaffleRoom`: the class is the rooms Worker's, which this one binds
//!   by name (`script_name = "rafi-rooms"`); phones' sockets and nudges go
//!   to the objects through that binding.
//!
//! The app reaches the engine through Restate's ingress
//! (`RAFI_ENGINE_INGRESS_URL`, `RAFI_ENGINE_API_KEY`), never directly.
//! Local development runs all three in one Worker (`apps/worker`).

#![cfg(target_arch = "wasm32")]

use rafi_cloudflare_app::Body;
use worker::{Context, Env, HttpRequest, event};

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> worker::Result<http::Response<Body>> {
    if rafi_cloudflare::is_restate_path(req.uri().path()) {
        return Ok(rafi_cloudflare_app::not_found());
    }
    rafi_cloudflare_app::serve(req, &env).await
}

rafi_cloudflare_app::export_scheduled!();
