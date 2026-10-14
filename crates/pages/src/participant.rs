//! The participant phone page.
//!
//! The whole page is rendered here, including the live card
//! ([`rafi_live_client::Card`], the same component the phone's WebAssembly
//! mounts), with the page's state embedded as JSON in `#rafi-config` (see
//! [`PageConfig`]). An inline loader then starts the WebAssembly from
//! `/participant/` (`./scripts/participant.sh`), or the JavaScript fallback
//! when it can't run. They keep the card current, solve the join form's
//! proof-of-work and enable its button.

use dioxus::prelude::*;
use rafi_live_client::{Card, CardStrings, Follow, PageConfig, ParticipantView, Phase};
use serde::Serialize;

use dioxus_icons::lucide::{Monitor, Moon, Sun, Ticket};

use crate::style::STYLESHEET_URL;

/// The raffle, as the public pages show it.
#[derive(Debug, Clone, Copy)]
pub struct Raffle<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub slug: &'a str,
    pub join_code: &'a str,
    pub test_mode: bool,
}

/// The entrant this device joined as.
#[derive(Debug, Clone, Copy)]
pub struct Entrant<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub claim_code: &'a str,
}

/// What the page says about the last join attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    None,
    /// Refused: why.
    Error(&'static str),
    /// That email is already in.
    AlreadyIn,
}

/// A participant page.
#[derive(Debug, Clone)]
pub struct Page<'a> {
    pub raffle: Raffle<'a>,
    /// What phones see now.
    pub view: &'a ParticipantView,
    /// How the page follows the raffle.
    pub follow: Follow,
    /// The entrant this device joined as, if any.
    pub entrant: Option<Entrant<'a>>,
    pub notice: Notice,
    /// The join form's name and email, to fill in again after a refusal.
    pub name: &'a str,
    pub email: &'a str,
    /// The server's clock (unix ms); the phone measures its offset from it.
    pub now_ms: u64,
}

/// Runs before first paint: records when the page started parsing (the
/// phone's clock offset is measured from it) and applies the saved colour
/// scheme (shared with the Dioxus app).
const HEAD_SCRIPT: &str = r#"window.__rafiT0=Date.now();try{var t=localStorage.getItem("rafi-theme");if(t)document.documentElement.setAttribute("data-theme",t)}catch(e){}"#;

/// Starts the participant WebAssembly, or the JavaScript fallback when there is
/// no WebAssembly, the module fails to load, or it isn't ready within 8 s
/// (the module's panic hook also calls `rafiFallback`).
const LOADER: &str = r#"window.rafiFallback=function(){if(window.__rafiFallback)return;window.__rafiFallback=1;var s=document.createElement("script");s.src="/participant/fallback.js";document.body.appendChild(s)};
setTimeout(function(){if(!window.__rafiReady)window.rafiFallback()},8000);
if(typeof WebAssembly!=="object"){window.rafiFallback()}else{import("/participant/rafi_participant.js").then(function(m){return m.default()}).catch(function(e){console.error(e);window.rafiFallback()})}"#;

/// A whole document: `tail` is raw HTML appended to the body (scripts).
pub(crate) fn document(title: &str, body: String, tail: Option<String>) -> String {
    let title = escape(title);
    let css = escape(STYLESHEET_URL);
    let tail = tail.unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
         <meta name=\"robots\" content=\"noindex\"><meta name=\"color-scheme\" content=\"light dark\">\
         <script>{HEAD_SCRIPT}</script>\
         <link rel=\"stylesheet\" href=\"{css}\">\
         <title>{title}</title></head><body class=\"min-h-dvh bg-base-200 bg-dots\">{body}{tail}</body></html>"
    )
}

/// Header shared by the participant pages: logo and theme toggle.
fn header_bar(test_mode: bool) -> Element {
    rsx! {
        header { class: "flex items-center gap-2",
            span { class: "brand text-lg",
                span { class: "grid place-items-center rounded-lg bg-primary p-1 text-primary-content -rotate-6",
                    Ticket { class: "size-4" }
                }
                "rafi"
            }
            if test_mode {
                span { class: "badge badge-warning badge-sm font-bold", "🧪 TEST MODE" }
            }
            span { class: "flex-1" }
            button { id: "theme", class: "btn btn-ghost btn-sm btn-circle", r#type: "button", aria_label: "Theme",
                Monitor { class: "size-4 theme-icon-system" }
                Sun { class: "size-4 theme-icon-light hidden" }
                Moon { class: "size-4 theme-icon-dark hidden" }
            }
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// JSON for embedding inside a `<script>` element.
fn script_json(v: &impl Serialize) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "null".into())
        .replace("</", "<\\/")
        .replace("<!--", "<\\!--")
}

/// The participant page, as HTML.
pub fn page(page: Page<'_>) -> String {
    let Page {
        raffle,
        view,
        follow,
        entrant,
        notice,
        name,
        email,
        now_ms,
    } = page;
    let phase = view.state;
    let strings = CardStrings::default();
    let code = raffle.join_code;
    let rules_href = format!("/rules/{}", raffle.slug);
    let privacy_href = format!("/privacy/{}", raffle.slug);
    let error = match notice {
        Notice::Error(why) => Some(why),
        _ => None,
    };
    let me = entrant.map(|e| e.id.to_owned());
    let claim_code = entrant.map(|e| e.claim_code.to_owned());
    let joining = entrant.is_none() && phase == Phase::Open && notice != Notice::AlreadyIn;

    // The live card's mount point: the page owns it, the card renders inside.
    let live = rsx! {
        section { id: "live", class: "empty:hidden", aria_live: "polite",
            Card {
                view: view.clone(),
                me: me.clone(),
                claim_code: claim_code.clone(),
                page_state: phase,
                now_ms,
                strings: strings.clone(),
            }
        }
    };

    let card = "card bg-base-100 shadow-sm ring-1 ring-base-300";
    let main = match entrant {
        Some(e) => rsx! {
            section { class: "ticket bg-primary text-primary-content shadow-lg animate-pop",
                div { class: "space-y-1 px-7 pt-6",
                    p { class: "text-xs font-bold uppercase tracking-[0.2em] opacity-80", "🎟️ You're in!" }
                    h2 { class: "break-words text-3xl font-extrabold", "{e.display_name}" }
                }
                div { class: "mx-7 my-4 border-t-2 border-dashed border-primary-content/40" }
                div { class: "flex flex-col items-center gap-1 px-7 pb-6 text-center",
                    span { class: "text-xs font-semibold uppercase tracking-widest opacity-75", "Your claim code" }
                    strong { class: "claim-code text-5xl", "{e.claim_code}" }
                    small { class: "text-sm opacity-80", "If you win, show this code to the organizer." }
                }
            }
            {live}
            button { id: "wake", class: "btn btn-outline btn-block rounded-full", r#type: "button", "Keep my screen on" }
            p { class: "text-center text-sm text-base-content/60", "You can close this page. Come back any time — your entry is safe." }
        },
        // The join form: no live card here.
        None if joining => {
            rsx! {
                form { class: "{card}", method: "post", action: "/j/{code}", id: "join",
                    div { class: "card-body gap-1",
                        h2 { class: "card-title text-2xl", "Join the raffle " span { class: "inline-block animate-float", "🎁" } }
                        if let Some(err) = error {
                            div { class: "alert alert-error alert-soft my-2", role: "alert", "{err}" }
                        }
                        fieldset { class: "fieldset",
                            legend { class: "fieldset-legend text-sm", "Name shown on the screen" }
                            input { class: "input input-lg w-full", id: "name", name: "name", required: true, maxlength: "{rafi_lifecycle::entrant::MAX_NAME_CHARS}", autocomplete: "nickname", value: "{name}" }
                            p { class: "label", "A first name or nickname is enough." }
                        }
                        fieldset { class: "fieldset",
                            legend { class: "fieldset-legend text-sm", "Email" }
                            input { class: "input input-lg w-full", id: "email", name: "email", r#type: "email", required: true, autocomplete: "email", inputmode: "email", value: "{email}" }
                            p { class: "label whitespace-normal", "Only to tell entries apart and to deliver prizes. Never shown." }
                        }
                        label { class: "mt-3 flex items-start gap-3",
                            input { class: "checkbox checkbox-primary mt-0.5", r#type: "checkbox", name: "rules", value: "on", required: true }
                            span { class: "text-sm",
                                "I accept the raffle rules and the privacy notice. "
                                a { class: "link link-primary", href: "{rules_href}", target: "_blank", "Rules" }
                                " · "
                                a { class: "link link-primary", href: "{privacy_href}", target: "_blank", "Privacy notice" }
                            }
                        }
                        label { class: "mt-2 flex items-start gap-3",
                            input { class: "checkbox mt-0.5", r#type: "checkbox", name: "marketing", value: "on" }
                            span { class: "text-sm text-base-content/80", "Email me about future meetups (optional)." }
                        }
                        input { r#type: "hidden", name: "altcha", id: "altcha" }
                        // Enabled by the WebAssembly (or the fallback) once the
                        // proof-of-work is solved; it then shows `data-label`.
                        button { class: "btn btn-primary btn-lg btn-block mt-4 rounded-full", r#type: "submit", id: "submit", disabled: true, "data-label": "Join 🎟️",
                            span { class: "loading loading-spinner loading-md" }
                            "Getting ready…"
                        }
                    }
                }
            }
        }
        None => {
            let (emoji, msg) = match (notice, phase) {
                (Notice::AlreadyIn, _) => (
                    "👋",
                    "This email is already entered. Your entry is safe — watch the screen. If you win, the organizer will confirm you by email.",
                ),
                (Notice::Error(why), _) => ("⚠️", why),
                (_, Phase::Ready) => (
                    "⏳",
                    "Registration is not open yet. Keep an eye on the screen!",
                ),
                _ => ("🔒", "Registration is closed."),
            };
            rsx! {
                section { class: "{card} items-center gap-2 p-6 text-center",
                    span { class: "text-4xl", "{emoji}" }
                    p { class: "text-lg font-semibold", role: "status", "{msg}" }
                }
                {live}
            }
        }
    };

    let title = raffle.title;
    let header = header_bar(raffle.test_mode);
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-md flex-col gap-4 px-4 pb-10 pt-4",
            {header}
            h1 { class: "text-2xl font-extrabold leading-tight", "{title}" }
            {main}
            footer { class: "mt-2 flex flex-wrap justify-center gap-x-3 gap-y-1 text-sm text-base-content/60",
                a { class: "link link-hover", href: "{rules_href}", "Rules" }
                a { class: "link link-hover", href: "{privacy_href}", "Privacy notice" }
                a { class: "link link-hover", href: "/v/{raffle.slug}", "Check the draw yourself" }
            }
        }
    });

    let config = PageConfig {
        raffle_id: raffle.id.to_owned(),
        entrant_id: me,
        claim_code,
        page_state: phase,
        view: view.clone(),
        strings,
        participant_url: format!("/api/r/{}/participant", raffle.slug),
        server_now_ms: now_ms,
        follow,
        needs_altcha: joining,
        altcha_url: PageConfig::ALTCHA_URL.into(),
        screen_on: "Screen will stay on".to_owned(),
        keep_screen_on: "Keep my screen on".to_owned(),
    };
    let tail = format!(
        "<script type=\"application/json\" id=\"rafi-config\">{}</script><script>{LOADER}</script>",
        script_json(&config)
    );
    document(raffle.title, body, Some(tail))
}

/// The page for a raffle that isn't there (or isn't public), as HTML.
pub fn not_found() -> String {
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex min-h-dvh max-w-md flex-col items-center justify-center gap-3 px-4 text-center",
            span { class: "text-6xl", "🎟️" }
            p { class: "text-xl font-bold", "This raffle does not exist." }
        }
    });
    document("Rafi", body, None)
}
