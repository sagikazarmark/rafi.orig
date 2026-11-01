//! The participant phone page's WebAssembly.
//!
//! The server renders the whole page, including the live card (the same
//! [`rafi_live_client::Card`]), and embeds the page's state as JSON in
//! `<script type="application/json" id="rafi-config">`. This module then:
//!
//! - mounts the card into `#live` and keeps it current the way the server's
//!   `follow` says: from the raffle's participant channel while the
//!   raffle is live, or by polling `participant_url`; and from
//!   `participant_url` when the page wakes, reconnects or the channel stops
//!   (polling then until a later attempt at the channel opens it);
//! - solves the join form's proof-of-work and enables its button;
//! - runs the extras: theme toggle, wake lock, chime and vibration on a win.
//!
//! An inline loader in the page runs a JavaScript fallback if this module
//! doesn't report ready (`window.__rafiReady`) in time, or panics: the panic
//! hook calls `window.rafiFallback()`. A module that arrives after the
//! fallback started leaves the page to it. Nothing here unwraps page or network
//! data.
//!
//! `just build` here has dx build it (`Dioxus.toml`, the `justfile`) into
//! `dist/public/`: the module and its glue under `assets/`, named by their
//! hash, so phones keep them for good; `fallback.js` from `public/`, as it
//! is; and `index.html`, the page's loader (the file here, with the glue's
//! hashed name filled in), which rafi-server compiles in: a server is built
//! against one participant build, and links that one.

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

/// Whether the JavaScript fallback was started (`window.__rafiFallback`):
/// the loader starts it when this module is late, and it then owns the page.
fn fallback_started() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("__rafiFallback")).ok())
        .is_some_and(|v| v.is_truthy())
}

/// Starts the module: `main`, which dx's glue calls once the module is
/// instantiated.
pub fn start() {
    // Arrived after the loader gave up on it (slow venue wifi): the fallback
    // renders the card already, and two renderers would fight over it.
    if fallback_started() {
        return;
    }
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
