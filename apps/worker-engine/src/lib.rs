//! Rafi's raffle engine Worker on Cloudflare (`worker-build`,
//! `wrangler.toml`): production's half that Restate calls, the other being
//! the app's Worker (`apps/worker-app`).
//!
//! - `fetch`: `/restate` and `/restate/*` are the raffle engine's Restate
//!   endpoint (`rafi_cloudflare_engine::serve`), whose SDK verifies each
//!   request's signature first; every other path answers 404.
//!
//! Nothing else: no Cron, no `RaffleRoom` (it binds the app Worker's class
//! with `script_name` and only nudges the objects), no app, no secrets. Its
//! bindings are the app's D1 database and the objects' namespace; its vars
//! are the Restate environment's public identity keys and the drand settings
//! (`rafi_cloudflare_engine` lists them). Local development runs the engine
//! inside the combined Worker (`apps/worker`).

#![cfg(target_arch = "wasm32")]

use rafi_cloudflare_engine::Body;
use worker::{Context, Env, HttpRequest, event};

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> worker::Result<http::Response<Body>> {
    if rafi_cloudflare_engine::owns(req.uri().path()) {
        return Ok(rafi_cloudflare_engine::serve(req, &env));
    }
    Ok(rafi_cloudflare_engine::not_found())
}
