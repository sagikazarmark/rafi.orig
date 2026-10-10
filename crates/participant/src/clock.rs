//! The server-synced clock for the countdown.
//!
//! The page's inline script records `Date.now()` as `window.__rafiT0` while it
//! parses, next to the server time the page was rendered at, so the offset
//! doesn't include the WebAssembly's download. Fetches of the participant view
//! re-sync it from their `Date` header.

use std::cell::Cell;

use wasm_bindgen::JsValue;

thread_local! {
    static OFFSET_MS: Cell<f64> = const { Cell::new(0.0) };
}

fn local_now() -> f64 {
    js_sys::Date::now()
}

pub fn init(server_now_ms: u64) {
    let t0 = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("__rafiT0")).ok())
        .and_then(|v| v.as_f64())
        .unwrap_or_else(local_now);
    OFFSET_MS.set(server_now_ms as f64 - t0);
}

/// The server's time now, Unix milliseconds.
pub fn now_ms() -> u64 {
    (local_now() + OFFSET_MS.get()).max(0.0) as u64
}

/// Re-syncs from a response's `Date` header (one-second resolution), if it
/// disagrees by more than its resolution allows. `sent_at` is the local time
/// the request was sent.
pub fn resync(date_header: &str, sent_at: f64) {
    let server = js_sys::Date::parse(date_header);
    if !server.is_finite() {
        return;
    }
    let midpoint = (sent_at + local_now()) / 2.0;
    let offset = server + 500.0 - midpoint;
    if (offset - OFFSET_MS.get()).abs() > 1_500.0 {
        OFFSET_MS.set(offset);
    }
}

pub fn local_ms() -> f64 {
    local_now()
}
