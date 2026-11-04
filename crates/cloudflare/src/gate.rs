//! The Worker's side of the channel: reaching the raffle's object, nudging it,
//! destroying it, and letting phones connect.
//!
//! Object IDs come from the plain namespace, or from the jurisdiction
//! `RAFI_DO_JURISDICTION` names when it is set (`eu` keeps the objects in the
//! EU): an object's jurisdiction is fixed when its ID is created and can't be
//! changed later. `worker` has no jurisdiction API yet, so the namespace's
//! JavaScript method is called directly.

#[cfg(feature = "connect")]
use partyline_worker::{close, reject};
use wasm_bindgen::{JsCast, JsValue};
#[cfg(feature = "connect")]
use worker::Response;
use worker::{Env, Headers, Method, ObjectNamespace, Request, RequestInit, Result, Stub};

use crate::jurisdiction::{Namespace, choose};

/// The Durable Object binding of the `RaffleRoom` objects.
pub const BINDING: &str = "RAFFLE_ROOM";

/// The header carrying the raffle ID on requests to the object.
pub(crate) const RAFFLE_HEADER: &str = "x-rafi-raffle";

/// The rate-limiting binding for socket connects, keyed by raffle (never by
/// IP: a venue shares one address). Optional.
pub const CONNECT_LIMITER: &str = "CONNECT_LIMITER";

/// The variable naming the objects' jurisdiction (`eu`, `fedramp`), optional:
/// unset or empty (the default), the objects' IDs come from the plain
/// namespace; set, from `ns.jurisdiction(<it>)`. Every Worker creating IDs
/// (the app's, the engine's) must say the same, and it can't change once
/// objects exist. workerd (`wrangler dev`) has no jurisdictions: leave it
/// unset locally.
pub const JURISDICTION_VAR: &str = "RAFI_DO_JURISDICTION";

/// The base URL of requests to the object. The host is never resolved.
const INTERNAL: &str = "https://rafi-raffle-room.internal";

/// The channel's namespace: the plain one, or restricted to the configured
/// jurisdiction ([`JURISDICTION_VAR`]).
pub fn namespace(env: &Env) -> Result<ObjectNamespace> {
    let ns = env.durable_object(BINDING)?;
    let value = crate::lookup(env, JURISDICTION_VAR);
    let jd = match choose(value.as_deref()) {
        Namespace::Plain => return Ok(ns),
        Namespace::Jurisdiction(jd) => jd,
    };
    let raw: &JsValue = ns.as_ref();
    let method = js_sys::Reflect::get(raw, &JsValue::from_str("jurisdiction"))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| worker::Error::RustError("the namespace has no jurisdiction()".into()))?;
    Ok(method.call1(raw, &JsValue::from_str(jd))?.unchecked_into())
}

fn stub(env: &Env, raffle_id: &str) -> Result<Stub> {
    namespace(env)?.id_from_name(raffle_id)?.get_stub()
}

async fn call(env: &Env, raffle_id: &str, method: Method, path: &str) -> Result<String> {
    let headers = Headers::new();
    headers.set(RAFFLE_HEADER, raffle_id)?;
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers);
    let req = Request::new_with_init(&format!("{INTERNAL}{path}"), &init)?;
    let mut res = stub(env, raffle_id)?.fetch_with_request(req).await?;
    let text = res.text().await?;
    if res.status_code() != 200 {
        return Err(worker::Error::RustError(format!(
            "RaffleRoom {path}: {} {text}",
            res.status_code()
        )));
    }
    Ok(text)
}

/// Tells the raffle's channel that something phones see may have changed.
/// Returns the object's decision (`Publish`, `Unchanged`, …).
///
/// Best effort: a lost nudge is repaired by the object's periodic re-read, so
/// callers log failures instead of failing their own write.
/// [`DurableObjects`](crate::DurableObjects) does that for writers.
pub async fn nudge(env: &Env, raffle_id: &str) -> Result<String> {
    call(env, raffle_id, Method::Post, "/nudge").await
}

/// Ends the raffle's live channel: publishes the final view, closes every
/// socket with 4403 and deletes the object's storage. Set `live_ended_at`
/// first, so the object refuses to come back.
pub async fn destroy(env: &Env, raffle_id: &str) -> Result<()> {
    call(env, raffle_id, Method::Post, "/destroy")
        .await
        .map(|_| ())
}

/// The object's diagnostics, as JSON: `reads` (how many times it read the
/// raffle from D1, in this isolate of it) and `sockets` (how many it holds).
/// For the channel's scenario; reachable only through the binding.
pub async fn stats(env: &Env, raffle_id: &str) -> Result<String> {
    call(env, raffle_id, Method::Get, "/stats").await
}

/// Whether `id` has a raffle ID's shape: 32 lowercase hex digits
/// (`rafi_app::ids::id`).
#[cfg(feature = "connect")]
fn is_raffle_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Handles a phone's WebSocket upgrade for a raffle (`GET /partyline/raffle/{id}`).
///
/// Refusals accept the socket and close it with 4403, never an HTTP error:
/// browsers see a failed upgrade as a network error (1006) and keep retrying,
/// while 4403 tells the partyline client to stop (and rafi's page to poll).
#[cfg(feature = "connect")]
pub async fn connect(env: &Env, raffle_id: &str, req: Request) -> Result<Response> {
    // The ID's shape first, and the budget second, both before anything is
    // read: a made-up ID costs no database read, and the budget is keyed by
    // an ID that can at least be a raffle's.
    if !is_raffle_id(raffle_id) {
        return reject(close::FORBIDDEN, "no such raffle");
    }
    if let Ok(limiter) = env.rate_limiter(CONNECT_LIMITER) {
        match limiter.limit(raffle_id.to_owned()).await {
            Ok(outcome) if !outcome.success => {
                return reject(close::FORBIDDEN, "too many connections");
            }
            Ok(_) => {}
            // The limiter is a soft limit over a hard one (the entrant cap);
            // one that errors lets the connect through rather than failing
            // every phone, as the join's limiter does.
            Err(e) => worker::console_error!("rafi: connect limiter: {e}"),
        }
    }
    // Two columns of the raffle's row, not the snapshot: a room reconnecting
    // after an eviction asks this hundreds of times in a second, and the
    // object reads the raffle whole once it accepts (coalescing those too).
    let db = crate::database(env)?;
    let open = match rafi_store::live::accepts_sockets(&db, raffle_id).await {
        Ok(Some(open)) => open,
        Ok(None) => false,
        Err(e) => {
            worker::console_error!("rafi: reading raffle {raffle_id}: {e}");
            return reject(close::INTERNAL_ERROR, "try again");
        }
    };
    if !open {
        return reject(close::FORBIDDEN, "live updates ended");
    }
    // Forward the upgrade, carrying the raffle ID and nothing the client could forge.
    let headers = req.headers().clone();
    headers.delete("x-partyline-tags")?;
    headers.set(RAFFLE_HEADER, raffle_id)?;
    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(headers);
    let forwarded = Request::new_with_init(req.url()?.as_str(), &init)?;
    stub(env, raffle_id)?.fetch_with_request(forwarded).await
}
