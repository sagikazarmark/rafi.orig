//! The browser-API parts of the page: theme toggle, proof-of-work, wake lock,
//! chime and vibration.
//!
//! They attach to the server-rendered page by element ID, outside Dioxus. The
//! inline theme script in `<head>` stays JavaScript: it must run before the
//! first paint.

use std::cell::{Cell, RefCell};

use dioxus_altcha::SolveOptions;
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioContext, Element, HtmlButtonElement, HtmlFormElement, HtmlInputElement, OscillatorType,
    PageTransitionEvent,
};

use crate::config::PageConfig;

thread_local! {
    static AUDIO: RefCell<Option<AudioContext>> = const { RefCell::new(None) };
    /// When the solved proof-of-work stops being usable (`performance.now()`).
    static POW_DEADLINE: Cell<Option<f64>> = const { Cell::new(None) };
    /// Bumped by each solve; a stale one doesn't touch the form.
    static POW_RUN: Cell<u32> = const { Cell::new(0) };
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn by_id<T: JsCast>(id: &str) -> Option<T> {
    document()?.get_element_by_id(id)?.dyn_into().ok()
}

fn listen(target: &web_sys::EventTarget, event: &str, f: impl FnMut() + 'static) {
    let f = Closure::<dyn FnMut()>::new(f);
    let _ = target.add_event_listener_with_callback(event, f.as_ref().unchecked_ref());
    f.forget();
}

fn listen_event(
    target: &web_sys::EventTarget,
    event: &str,
    f: impl FnMut(web_sys::Event) + 'static,
) {
    let f = Closure::<dyn FnMut(web_sys::Event)>::new(f);
    let _ = target.add_event_listener_with_callback(event, f.as_ref().unchecked_ref());
    f.forget();
}

pub fn init(cfg: &PageConfig) {
    theme_toggle();
    if let Some(wake) = by_id::<HtmlButtonElement>("wake") {
        let (button, on, off) = (
            wake.clone(),
            cfg.screen_on.clone(),
            cfg.keep_screen_on.clone(),
        );
        listen(&wake, "click", move || {
            unlock_audio();
            keep_awake(button.clone(), on.clone(), off.clone());
        });
    }
    // Audio needs a user gesture; joining is one.
    if let Some(form) = by_id::<Element>("join") {
        listen(&form, "submit", unlock_audio);
    }
    if cfg.needs_altcha {
        proof_of_work(cfg.altcha_url.clone());
    }
}

// ---- Theme: system -> light -> dark ----

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn current_theme() -> String {
    storage()
        .and_then(|s| s.get_item("rafi-theme").ok().flatten())
        .unwrap_or_else(|| "system".into())
}

fn show_theme(button: &Element, theme: &str) {
    for (t, class) in [
        ("system", "theme-icon-system"),
        ("rafi", "theme-icon-light"),
        ("rafi-dark", "theme-icon-dark"),
    ] {
        if let Ok(Some(icon)) = button.query_selector(&format!(".{class}")) {
            let _ = icon.class_list().toggle_with_force("hidden", t != theme);
        }
    }
}

fn theme_toggle() {
    let Some(button) = by_id::<Element>("theme") else {
        return;
    };
    show_theme(&button, &current_theme());
    let b = button.clone();
    listen(&button, "click", move || {
        let next = match current_theme().as_str() {
            "system" => "rafi",
            "rafi" => "rafi-dark",
            _ => "system",
        };
        let Some(root) = document().and_then(|d| d.document_element()) else {
            return;
        };
        if next == "system" {
            if let Some(s) = storage() {
                let _ = s.remove_item("rafi-theme");
            }
            let _ = root.remove_attribute("data-theme");
        } else {
            if let Some(s) = storage() {
                let _ = s.set_item("rafi-theme", next);
            }
            let _ = root.set_attribute("data-theme", next);
        }
        show_theme(&b, next);
    });
}

// ---- Proof of work (join form) ----

/// dioxus-altcha's solver, served by rafi-server (the version it was built
/// with) for fallback.js too: one file, cached, instead of a copy in the wasm.
const ALTCHA_WORKER: &str = "/participant/altcha-worker.js";

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.now())
}

/// Solves on load. A payload past its deadline (a phone left on the form
/// for 20 minutes) is solved again before the form posts, and so is one in a
/// page restored from the back/forward cache: it's spent.
fn proof_of_work(url: String) {
    let (Some(form), Some(input), Some(submit)) = (
        by_id::<HtmlFormElement>("join"),
        by_id::<HtmlInputElement>("altcha"),
        by_id::<HtmlButtonElement>("submit"),
    ) else {
        return;
    };
    let solve = {
        let (url, input, submit) = (url, input.clone(), submit);
        move |then: Option<HtmlFormElement>| {
            spawn_local(solve_altcha(
                url.clone(),
                input.clone(),
                submit.clone(),
                then,
            ));
        }
    };
    solve(None);

    let (resolve, f) = (solve.clone(), form.clone());
    listen_event(&form, "submit", move |e| {
        let stale = POW_DEADLINE.get().is_some_and(|d| now_ms() >= d);
        if stale && !input.value().is_empty() {
            e.prevent_default();
            resolve(Some(f.clone()));
        }
    });
    if let Some(window) = web_sys::window() {
        listen_event(&window, "pageshow", move |e| {
            if e.dyn_ref::<PageTransitionEvent>()
                .is_some_and(PageTransitionEvent::persisted)
            {
                solve(None);
            }
        });
    }
}

/// The Join button is rendered disabled; it's enabled once a solution is in
/// the form (or the attempt failed: the server then asks to retry). With
/// `then`, the form is posted once solved.
async fn solve_altcha(
    url: String,
    input: HtmlInputElement,
    submit: HtmlButtonElement,
    then: Option<HtmlFormElement>,
) {
    let run = POW_RUN.get().wrapping_add(1);
    POW_RUN.set(run);
    input.set_value("");
    POW_DEADLINE.set(None);
    submit.set_disabled(true);
    let solved = match dioxus_altcha::fetch_challenge(url).await {
        Ok(challenge) => {
            dioxus_altcha::solve(&challenge, &SolveOptions::new().worker_url(ALTCHA_WORKER))
                .await
                .ok()
                .map(|solution| (dioxus_altcha::payload(&challenge, &solution), challenge))
        }
        Err(_) => None,
    };
    if POW_RUN.get() != run {
        return;
    }
    if let Some((payload, challenge)) = solved {
        input.set_value(&payload);
        let deadline = challenge
            .usable_for()
            .map(|left| now_ms() + left.as_secs_f64() * 1000.0);
        POW_DEADLINE.set(deadline);
    }
    if let Some(label) = submit.get_attribute("data-label") {
        submit.set_inner_html(&label);
    }
    submit.set_disabled(false);
    if let Some(form) = then.filter(|_| !input.value().is_empty()) {
        // requestSubmit runs the submit handlers again (Safari 16+).
        if form.request_submit().is_err() {
            let _ = form.submit();
        }
    }
}

// ---- Wake lock ----
//
// web-sys only has `WakeLock` behind `--cfg=web_sys_unstable_apis`; these
// are the two calls needed.

#[wasm_bindgen]
extern "C" {
    type WakeLockApi;
    #[wasm_bindgen(method, js_name = request)]
    fn request(this: &WakeLockApi, kind: &str) -> js_sys::Promise;
}

fn keep_awake(button: HtmlButtonElement, screen_on: String, keep_screen_on: String) {
    spawn_local(async move {
        let Some(window) = web_sys::window() else {
            return;
        };
        let navigator = window.navigator();
        let Ok(api) = js_sys::Reflect::get(&navigator, &JsValue::from_str("wakeLock")) else {
            return;
        };
        if api.is_undefined() {
            return;
        }
        let api: WakeLockApi = api.unchecked_into();
        // iOS may refuse after backgrounding; the button stays.
        let Ok(sentinel) = JsFuture::from(api.request("screen")).await else {
            return;
        };
        button.set_text_content(Some(&format!("✓ {screen_on}")));
        button.set_disabled(true);
        let b = button.clone();
        let target: web_sys::EventTarget = sentinel.unchecked_into();
        listen(&target, "release", move || {
            b.set_text_content(Some(&keep_screen_on));
            b.set_disabled(false);
        });
    });
}

// ---- Sound and vibration (need a user gesture first) ----

fn unlock_audio() {
    AUDIO.with(|a| {
        let mut a = a.borrow_mut();
        if a.is_none() {
            *a = AudioContext::new().ok();
        }
        let Some(ctx) = a.as_ref() else { return };
        if ctx.state() == web_sys::AudioContextState::Suspended {
            let _ = ctx.resume();
        }
        if let (Ok(buf), Ok(src)) = (ctx.create_buffer(1, 1, 22050.0), ctx.create_buffer_source()) {
            src.set_buffer(Some(&buf));
            let _ = src.connect_with_audio_node(&ctx.destination());
            let _ = src.start();
        }
        // Safari only, not in web-sys.
        if let Some(navigator) = web_sys::window().map(|w| w.navigator())
            && let Ok(session) =
                js_sys::Reflect::get(&navigator, &JsValue::from_str("audioSession"))
            && !session.is_undefined()
        {
            let _ = js_sys::Reflect::set(
                &session,
                &JsValue::from_str("type"),
                &JsValue::from_str("playback"),
            );
        }
    });
}

fn chime() -> Option<()> {
    AUDIO.with(|a| {
        let a = a.borrow();
        let ctx = a.as_ref()?;
        let t = ctx.current_time();
        for (i, f) in [523.25, 659.25, 783.99, 1046.5].into_iter().enumerate() {
            let at = t + i as f64 * 0.12;
            let o = ctx.create_oscillator().ok()?;
            let g = ctx.create_gain().ok()?;
            o.frequency().set_value(f);
            o.set_type(OscillatorType::Triangle);
            g.gain().set_value_at_time(0.0001, at).ok()?;
            g.gain()
                .exponential_ramp_to_value_at_time(0.3, at + 0.02)
                .ok()?;
            g.gain()
                .exponential_ramp_to_value_at_time(0.0001, at + 0.4)
                .ok()?;
            o.connect_with_audio_node(&g).ok()?;
            g.connect_with_audio_node(&ctx.destination()).ok()?;
            o.start_with_when(at).ok()?;
            o.stop_with_when(at + 0.45).ok()?;
        }
        Some(())
    })
}

/// Once, when the phone's own win arrives live.
pub fn announce_win() {
    let _ = chime();
    if let Some(navigator) = web_sys::window().map(|w| w.navigator()) {
        let pattern = js_sys::Array::of5(
            &200.into(),
            &100.into(),
            &200.into(),
            &100.into(),
            &400.into(),
        );
        let _ = navigator.vibrate_with_pattern(&pattern);
    }
}
