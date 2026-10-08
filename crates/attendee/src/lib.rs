//! The attendee phone page's WebAssembly.
//!
//! The server renders the whole page, including the live card (the same
//! [`rafi_live::Card`]), and embeds the page's state as JSON in
//! `<script type="application/json" id="rafi-config">`. This module then:
//!
//! - mounts the card into `#live` and keeps it current the way the server's
//!   `live` transport says: from the raffle's partyline channel while the
//!   raffle is live, or by polling `participant_url`; and from
//!   `participant_url` when the page wakes, reconnects or the channel stops;
//! - solves the join form's proof-of-work and enables its button;
//! - runs the extras: theme toggle, wake lock, chime and vibration on a win.
//!
//! An inline loader in the page runs a JavaScript fallback if this module
//! doesn't report ready (`window.__rafiReady`) in time, or panics: the panic
//! hook calls `window.rafiFallback()`. Nothing here unwraps page or network
//! data.

#![cfg(target_arch = "wasm32")]

mod app;
mod clock;
mod config;
mod extras;
mod net;

use wasm_bindgen::prelude::*;

fn window_call(name: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Ok(f) = js_sys::Reflect::get(&window, &JsValue::from_str(name))
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}

/// Hands the page over to the JavaScript fallback.
pub(crate) fn fallback() {
    window_call("rafiFallback");
}

#[wasm_bindgen(start)]
pub fn start() {
    // A fixed message: formatting the panic info costs kilobytes on the phone.
    std::panic::set_hook(Box::new(|_| {
        web_sys::console::error_1(&"rafi: the live card stopped; falling back".into());
        fallback();
    }));
    let Some(cfg) = config::load() else {
        web_sys::console::error_1(&"rafi: no usable page config".into());
        fallback();
        return;
    };
    clock::init(cfg.server_now_ms);
    extras::init(&cfg);
    app::mount(cfg);
    if let Some(window) = web_sys::window() {
        let _ = js_sys::Reflect::set(&window, &JsValue::from_str("__rafiReady"), &JsValue::TRUE);
    }
}
