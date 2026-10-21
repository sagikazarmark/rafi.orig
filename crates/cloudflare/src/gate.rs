//! The Worker's side of the channel: reaching the raffle's object, nudging it,
//! destroying it, and letting phones connect.
//!
//! Every object ID comes from the EU-jurisdiction namespace: an object's
//! jurisdiction is fixed when its ID is created and can't be changed later.
//! `worker` has no jurisdiction API yet, so the namespace's JavaScript method
//! is called directly.

use partyline_worker::{close, reject};
use wasm_bindgen::{JsCast, JsValue};
use worker::{Env, Headers, Method, ObjectNamespace, Request, RequestInit, Response, Result, Stub};

/// The Durable Object binding of the `RaffleRoom` objects.
pub const BINDING: &str = "RAFFLE_ROOM";

/// The header carrying the raffle ID on requests to the object.
pub(crate) const RAFFLE_HEADER: &str = "x-rafi-raffle";

/// The rate-limiting binding for socket connects, keyed by raffle (never by
/// IP: a venue shares one address). Optional.
pub const CONNECT_LIMITER: &str = "CONNECT_LIMITER";

/// The variable naming the objects' jurisdiction. Defaults to `eu`; set it to
/// an empty string to create objects without one (local experiments only).
pub const JURISDICTION_VAR: &str = "RAFI_DO_JURISDICTION";

/// The base URL of requests to the object. The host is never resolved.
const INTERNAL: &str = "https://rafi-raffle-room.internal";

fn jurisdiction(env: &Env) -> String {
    env.var(JURISDICTION_VAR)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "eu".into())
}

/// The channel's namespace, restricted to the configured jurisdiction.
pub fn namespace(env: &Env) -> Result<ObjectNamespace> {
    let ns = env.durable_object(BINDING)?;
    let jd = jurisdiction(env);
    if jd.is_empty() {
        return Ok(ns);
    }
    let raw: &JsValue = ns.as_ref();
    let method = js_sys::Reflect::get(raw, &JsValue::from_str("jurisdiction"))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| worker::Error::RustError("the namespace has no jurisdiction()".into()))?;
    Ok(method.call1(raw, &JsValue::from_str(&jd))?.unchecked_into())
}

fn stub(env: &Env, raffle_id: &str) -> Result<Stub> {
    namespace(env)?.id_from_name(raffle_id)?.get_stub()
}

async fn call(env: &Env, raffle_id: &str, path: &str) -> Result<String> {
    let headers = Headers::new();
    headers.set(RAFFLE_HEADER, raffle_id)?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_headers(headers);
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
    call(env, raffle_id, "/nudge").await
}

/// Ends the raffle's live channel: publishes the final view, closes every
/// socket with 4403 and deletes the object's storage. Set `live_ended_at`
/// first, so the object refuses to come back.
pub async fn destroy(env: &Env, raffle_id: &str) -> Result<()> {
    call(env, raffle_id, "/destroy").await.map(|_| ())
}

/// Handles a phone's WebSocket upgrade for a raffle (`GET /partyline/raffle/{id}`).
///
/// Refusals accept the socket and close it with 4403, never an HTTP error:
/// browsers see a failed upgrade as a network error (1006) and keep retrying,
/// while 4403 tells the partyline client to stop (and rafi's page to poll).
pub async fn connect(env: &Env, raffle_id: &str, req: Request) -> Result<Response> {
    let db = crate::database(env)?;
    let open = match rafi_store::live::snapshot(&db, raffle_id).await {
        Ok(Some(s)) => s.accepts_sockets(),
        Ok(None) => false,
        Err(e) => {
            worker::console_error!("rafi: reading raffle {raffle_id}: {e}");
            return reject(close::INTERNAL_ERROR, "try again");
        }
    };
    if !open {
        return reject(close::FORBIDDEN, "live updates ended");
    }
    if let Ok(limiter) = env.rate_limiter(CONNECT_LIMITER)
        && !limiter.limit(raffle_id.to_owned()).await?.success
    {
        return reject(close::FORBIDDEN, "too many connections");
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
