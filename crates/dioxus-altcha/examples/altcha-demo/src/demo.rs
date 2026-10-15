//! The demo page: a form protected by the proof-of-work, its settings, and
//! attacks the server refuses.

use dioxus::prelude::*;
use dioxus_altcha::{
    AltchaField, AltchaOptions, AltchaState, Auto, RawChallenge, SolveOptions, UseAltcha, altcha,
    decode_payload, use_altcha_with_options,
};

use crate::server::{MAX_COST, Outcome, get_challenge, join};

/// When the form solves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// On load, so Join is ready at once (`Auto::OnMount`).
    OnLoad,
    /// On submit (`Auto::Off` and `solved().await`): nothing runs until then.
    OnSubmit,
}

/// The settings and the form. `strict`: the page's CSP forbids `blob:`
/// workers (`worker-src 'self'`).
#[component]
pub fn Demo(strict: bool) -> Element {
    let mut cost = use_signal(|| 5_000u32);
    let mut draft = use_signal(|| 5_000u32);
    let mut mode = use_signal(|| Mode::OnLoad);
    let mut embedded = use_signal(|| false);
    // A change remounts the form: the hook reads its options once.
    let key = format!("{}-{:?}-{}", cost(), mode(), embedded());
    rsx! {
        section { class: "card",
            h2 { "Settings" }
            label { class: "field",
                span { "Difficulty: " strong { "{draft}" } " PBKDF2 iterations per attempt, about 256 attempts" }
                input {
                    r#type: "range",
                    min: "1000",
                    max: "{MAX_COST}",
                    step: "1000",
                    value: "{draft}",
                    oninput: move |e| draft.set(e.value().parse().unwrap_or(5_000)),
                    onchange: move |e| cost.set(e.value().parse().unwrap_or(5_000)),
                }
            }
            fieldset { class: "choices",
                legend { "Solve" }
                label {
                    input {
                        r#type: "radio",
                        name: "mode",
                        checked: mode() == Mode::OnLoad,
                        onchange: move |_| mode.set(Mode::OnLoad),
                    }
                    " on load (Join is ready at once)"
                }
                label {
                    input {
                        r#type: "radio",
                        name: "mode",
                        checked: mode() == Mode::OnSubmit,
                        onchange: move |_| mode.set(Mode::OnSubmit),
                    }
                    " on submit (nothing runs until then)"
                }
            }
            if strict {
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: embedded(),
                        onchange: move |e| embedded.set(e.checked()),
                    }
                    " Use the embedded worker anyway: the CSP blocks it, so it falls back to solving on the page"
                }
            }
        }
        {rsx! {
            Form { key: "{key}", cost: cost(), mode: mode(), worker_url: strict && !embedded() }
        }}
    }
}

#[component]
fn Form(cost: u32, mode: Mode, worker_url: bool) -> Element {
    let altcha = use_altcha_with_options(
        move || get_challenge(cost),
        AltchaOptions::new()
            .auto(match mode {
                Mode::OnLoad => Auto::OnMount,
                Mode::OnSubmit => Auto::Off,
            })
            .solve(if worker_url {
                // The server serves dioxus_altcha::WORKER_JS here.
                SolveOptions::new().worker_url("/altcha-worker.js")
            } else {
                SolveOptions::new()
            }),
    );
    let mut name = use_signal(String::new);
    let mut result = use_signal(|| None::<Result<Outcome, String>>);
    let mut accepted = use_signal(|| None::<String>);
    let took = use_stopwatch(altcha);

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        let payload = match altcha.solved().await {
            Ok(p) => p,
            Err(e) => return result.set(Some(Err(e.to_string()))),
        };
        let outcome = join(name(), payload.clone()).await;
        if let Ok(Outcome::Joined { .. }) = outcome {
            accepted.set(Some(payload));
        }
        result.set(Some(outcome.map_err(|e| e.to_string())));
        // That challenge is spent.
        altcha.refresh();
    };

    rsx! {
        section { class: "card",
            h2 { "Join" }
            form { onsubmit: submit,
                label { class: "field",
                    span { "Name" }
                    input {
                        required: true,
                        placeholder: "Ada",
                        value: "{name}",
                        oninput: move |e| name.set(e.value()),
                    }
                }
                AltchaField { altcha }
                div { class: "row",
                    button { r#type: "submit", disabled: busy(altcha), "Join" }
                    Status { altcha, took: took() }
                }
            }
            if let Some(r) = result() {
                Verdict { result: r }
            }
        }
        if let Some(payload) = accepted() {
            Attacks { name: name(), payload }
        }
    }
}

fn busy(altcha: UseAltcha) -> bool {
    matches!(altcha.state(), AltchaState::Fetching | AltchaState::Solving)
}

/// Milliseconds from fetching to solved, measured on the page.
fn use_stopwatch(altcha: UseAltcha) -> Signal<Option<f64>> {
    let mut started = use_signal(|| None::<f64>);
    let mut took = use_signal(|| None::<f64>);
    use_effect(move || match altcha.state() {
        AltchaState::Fetching => {
            started.set(Some(now_ms()));
            took.set(None);
        }
        AltchaState::Solved { .. } => {
            if let Some(start) = *started.peek() {
                took.set(Some(now_ms() - start));
            }
        }
        _ => {}
    });
    took
}

fn now_ms() -> f64 {
    #[cfg(feature = "web")]
    return js_sys::Date::now();
    #[cfg(not(feature = "web"))]
    return 0.0;
}

#[component]
fn Status(altcha: UseAltcha, took: Option<f64>) -> Element {
    let (class, text) = match altcha.state() {
        AltchaState::Idle => ("idle", "Not solved yet".to_owned()),
        AltchaState::Fetching => ("busy", "Fetching a challenge…".to_owned()),
        AltchaState::Solving => ("busy", "Solving…".to_owned()),
        AltchaState::Solved { payload } => {
            let counter = decode_payload(&payload).map_or(0, |p| p.solution.counter);
            let time = took.map(|ms| format!(" in {ms:.0} ms")).unwrap_or_default();
            ("ok", format!("Solved{time} (counter {counter})"))
        }
        AltchaState::Expired => ("busy", "Expired: solving a new one…".to_owned()),
        AltchaState::Failed(e) => ("bad", format!("Failed: {e}")),
    };
    rsx! {
        span { class: "status {class}", "{text}" }
    }
}

#[component]
fn Verdict(result: Result<Outcome, String>) -> Element {
    match result {
        Ok(Outcome::Joined { name, counter }) => rsx! {
            p { class: "verdict ok", "✓ The server let {name} in (counter {counter})." }
        },
        Ok(Outcome::Refused { reason }) => rsx! {
            p { class: "verdict bad", "✗ Refused: {reason}." }
        },
        Err(e) => rsx! {
            p { class: "verdict bad", "✗ {e}" }
        },
    }
}

/// What a bot might try with the accepted payload, and what the server says.
#[component]
fn Attacks(name: String, payload: String) -> Element {
    let mut results = use_signal(Vec::<(&'static str, Result<Outcome, String>)>::new);
    let attack = use_callback(move |(what, forged): (&'static str, String)| {
        let name = name.clone();
        spawn(async move {
            let r = join(name, forged).await.map_err(|e| e.to_string());
            results.write().insert(0, (what, r));
        });
    });
    let (p1, p2, p3) = (payload.clone(), payload.clone(), payload);
    rsx! {
        section { class: "card",
            h2 { "Attacks" }
            p { class: "muted",
                "The last accepted payload, posted again or changed. The server refuses each, and says why."
            }
            div { class: "row wrap",
                button { onclick: move |_| attack(("Replay it", p1.clone())), "Replay it" }
                button {
                    onclick: move |_| {
                        attack(("Change the counter", tamper(&p2, |p| p.solution.counter += 1)))
                    },
                    "Change the counter"
                }
                button {
                    onclick: move |_| attack(("Lower the cost", tamper(&p3, |p| p.challenge.parameters.cost = 1))),
                    "Lower the cost"
                }
                button { onclick: move |_| attack(("Post none", String::new())), "Post none" }
            }
            ul { class: "log",
                for (what, r) in results() {
                    li {
                        strong { "{what}: " }
                        Verdict { result: r }
                    }
                }
            }
        }
    }
}

/// The payload re-encoded after `change`.
fn tamper(payload: &str, change: impl FnOnce(&mut altcha::Payload)) -> String {
    let Ok(mut p) = decode_payload(payload) else {
        return payload.to_owned();
    };
    change(&mut p);
    dioxus_altcha::payload(&RawChallenge::from_challenge(&p.challenge), &p.solution)
}
