//! dioxus-altcha in a browser: `dx serve` here, then open the page.
//!
//! The page issues the challenges and verifies the solutions itself, with a
//! key in its source: a demo of the client side, not a way to protect a form.
//! `?bench` measures instead (see bench.rs).

mod bench;

use dioxus::prelude::*;
use dioxus_altcha::{
    AltchaField, AltchaState, RawChallenge, UseAltcha, altcha, decode_payload, use_altcha,
};

const KEY: &str = "demo-only-key";

fn main() {
    let bench = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|s| s.contains("bench"));
    dioxus::launch(if bench { bench::Bench } else { App });
}

#[component]
fn App() -> Element {
    let mut mounted = use_signal(|| true);
    let mut cost = use_signal(|| 5_000u32);
    rsx! {
        main { style: "font-family: system-ui; max-width: 40rem; margin: 2rem auto; padding: 0 1rem",
            h1 { "dioxus-altcha" }
            p {
                "The page issues and verifies the challenges itself, with a key in its source: "
                "a demo of the client side only."
            }
            p {
                label {
                    "PBKDF2 cost (iterations per attempt, about 256 attempts) "
                    input {
                        r#type: "number",
                        min: "1",
                        value: "{cost}",
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse::<u32>() {
                                cost.set(v.max(1));
                            }
                        },
                    }
                }
            }
            p {
                button { onclick: move |_| mounted.toggle(),
                    if mounted() { "Unmount the form (cancels a solve)" } else { "Mount the form" }
                }
            }
            if mounted() {
                Form { cost: cost() }
            }
        }
    }
}

#[component]
fn Form(cost: u32) -> Element {
    // The hook keeps the latest closure: a new cost applies to the next fetch.
    let altcha = use_altcha(move || async move { issue(cost) });
    let mut verdict = use_signal(|| None::<String>);
    rsx! {
        form {
            onsubmit: move |e: FormEvent| async move {
                e.prevent_default();
                let result = match altcha.solved().await {
                    Ok(payload) => verify(&payload),
                    Err(e) => format!("no solution: {e}"),
                };
                verdict.set(Some(result));
                // The challenge is spent.
                altcha.refresh();
            },
            AltchaField { altcha }
            p { "State: " strong { {label(altcha)} } }
            button { r#type: "submit", "Submit" }
            " "
            button { r#type: "button", onclick: move |_| altcha.refresh(), "New challenge" }
        }
        if let Some(v) = verdict() {
            p { "{v}" }
        }
    }
}

fn label(altcha: UseAltcha) -> String {
    match altcha.state() {
        AltchaState::Idle => "idle".into(),
        AltchaState::Fetching => "fetching".into(),
        AltchaState::Solving => "solving…".into(),
        AltchaState::Solved { payload } => format!("solved ({} bytes)", payload.len()),
        AltchaState::Expired => "expired".into(),
        AltchaState::Failed(e) => format!("failed: {e}"),
    }
}

/// What a server would do on `GET /api/altcha`.
fn issue(cost: u32) -> Result<RawChallenge, String> {
    let now = (js_sys::Date::now() / 1000.0) as i64;
    altcha::create_challenge(altcha::CreateChallengeOptions {
        algorithm: "PBKDF2/SHA-256".into(),
        cost,
        key_prefix: "00".into(),
        expires_at: Some(now + 120),
        hmac_signature_secret: Some(KEY.into()),
        ..Default::default()
    })
    .map(|c| RawChallenge::from_challenge(&c))
    .map_err(|e| e.to_string())
}

/// What a server would do with the posted form value.
fn verify(payload: &str) -> String {
    let payload = match decode_payload(payload) {
        Ok(p) => p,
        Err(e) => return format!("refused: {e}"),
    };
    let solved_ms = payload.solution.time.unwrap_or_default();
    match altcha::verify_solution(altcha::VerifySolutionOptions::new(
        &payload.challenge,
        &payload.solution,
        KEY,
    )) {
        Ok(r) if r.verified => format!(
            "verified: counter {}, solved in {solved_ms:.0} ms",
            payload.solution.counter
        ),
        Ok(r) => format!("refused: {r:?}"),
        Err(e) => format!("refused: {e}"),
    }
}
