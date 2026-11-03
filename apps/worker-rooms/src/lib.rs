//! Rafi's rooms Worker on Cloudflare (`worker-build`, `wrangler.toml`),
//! named `rafi-rooms`: production's Worker hosting the raffles' live
//! channels, one [`RaffleRoom`] Durable Object per raffle, beside the app's
//! (`apps/worker-app`) and the raffle engine's (`apps/worker-engine`), which
//! both bind the class by this Worker's name (`script_name = "rafi-rooms"`).
//!
//! - [`RaffleRoom`] is exported from here, so the class lives in this Worker,
//!   with its Durable Object migrations; it reads raffles from the D1
//!   database (`DB`). Its `RAFI_DO_JURISDICTION` names the jurisdiction the
//!   other Workers create the objects' IDs in (the objects don't read it;
//!   the deploy checks that all three Workers say the same).
//! - `fetch`: 404 for everything. The Worker has no address (no route, no
//!   workers.dev, no preview URLs), and the objects are reached through the
//!   other Workers' bindings, never through it. The handler is there so the
//!   Worker has one: `worker-build`'s module always exports a default
//!   entrypoint, which without it would have no handler at all, and
//!   Cloudflare refuses to upload a Worker with no event handlers (10068);
//!   a 404 costs nothing.
//!
//! The app and the engine of the previous release call these objects while a
//! new one goes out (the app until it is replaced, the engine until it
//! drains), so a new rooms Worker accepts their requests: a change to them
//! takes two releases. Local development hosts the class in the combined
//! Worker (`apps/worker`).

#![cfg(target_arch = "wasm32")]

pub use rafi_cloudflare::RaffleRoom;
use worker::{Context, Env, Request, Response, Result, event};

#[event(fetch)]
async fn fetch(_req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    Response::error("Not Found", 404)
}
