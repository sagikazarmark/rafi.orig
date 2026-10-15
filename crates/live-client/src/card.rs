//! The phone's live card, one component for the server (SSR) and the WASM.

#[cfg(feature = "card")]
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "card")]
use crate::{
    screen::{Screen, screen},
    view::{ParticipantView, Phase},
};

/// The card's classes (daisyUI), as in `participant.js`.
pub const CARD: &str = "card bg-base-100 shadow-sm ring-1 ring-base-300 min-h-28 items-center justify-center gap-1 p-6 text-center";

/// The winning card's classes. `participant.js` also adds `animate-pop`; here
/// that comes with [`Card`]'s `announce`.
pub const WON: &str =
    "card bg-success text-success-content shadow-xl items-center gap-2 p-7 text-center";

/// The strings [`Card`] shows. The server embeds them in the page JSON, so
/// the WebAssembly and `fallback.js` show the same text.
///
/// The claimed winner's line reuses `thanks_won`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardStrings {
    pub you_won: String,
    pub go_to_organizer: String,
    pub thanks_won: String,
    pub thanks: String,
    pub drawing: String,
    pub not_this_time: String,
    pub draw_starting: String,
    pub prizes_left: String,
    pub waiting: String,
    pub locking: String,
    pub drand_round: String,
}

impl Default for CardStrings {
    fn default() -> Self {
        Self {
            you_won: "YOU WON!".into(),
            go_to_organizer: "Go to the organizer and show your claim code.".into(),
            thanks_won: "Congratulations again — enjoy your prize!".into(),
            thanks: "Thanks for playing!".into(),
            drawing: "Drawing".into(),
            not_this_time: "Not this time".into(),
            draw_starting: "The draw is starting!".into(),
            prizes_left: "prizes left".into(),
            waiting: "Waiting for the draw to start…".into(),
            locking: "Locking in public randomness".into(),
            drand_round: "drand round".into(),
        }
    }
}

/// The phone's live card: one root element, or nothing for
/// [`Screen::Blank`] and [`Screen::Reload`].
///
/// It reads no clock and calls no browser API while rendering: the caller
/// ticks `now_ms` for the countdown, reloads the page on [`Screen::Reload`],
/// and plays the chime. The mount point (`#live`, with `aria-live`) belongs
/// to the page.
///
/// - `me` and `claim_code` come from the page's entrant, if any;
/// - `page_state` is the state the page was rendered in;
/// - `now_ms` is the server-synced clock, in Unix milliseconds;
/// - `announce` adds `role="alert"` and the pop animation to a win. Set it
///   only when the win happens live, never on page load.
#[cfg(feature = "card")]
#[component]
pub fn Card(
    view: ParticipantView,
    me: Option<String>,
    claim_code: Option<String>,
    page_state: Phase,
    now_ms: u64,
    strings: CardStrings,
    #[props(default)] announce: bool,
) -> Element {
    let s = &strings;
    match screen(&view, me.as_deref(), page_state, now_ms) {
        Screen::Reload | Screen::Blank => rsx! {},
        Screen::Won {
            prize_name,
            claimed,
            done,
        } => {
            let class = if announce {
                format!("{WON} animate-pop")
            } else {
                WON.to_owned()
            };
            let note = if claimed || done {
                &s.thanks_won
            } else {
                &s.go_to_organizer
            };
            rsx! {
                div { class, role: if announce { "alert" },
                    p { class: "text-5xl", "🏆" }
                    p { class: "font-display text-5xl font-black tracking-tight", "{s.you_won}" }
                    p { class: "text-lg font-semibold", "{prize_name}" }
                    if let Some(code) = claim_code {
                        span { class: "claim-code mt-1 rounded-2xl bg-base-100 px-5 py-2 text-4xl text-success shadow-sm",
                            "{code}"
                        }
                    }
                    p { class: "text-sm opacity-90", "{note}" }
                }
            }
        }
        Screen::Thanks => rsx! {
            div { class: CARD,
                p { class: "text-4xl", "🎉" }
                p { class: "font-display text-2xl font-extrabold", "{s.thanks}" }
            }
        },
        Screen::Countdown {
            prize_name,
            round,
            secs_left,
        } => {
            let secs = if secs_left > 0 {
                secs_left.to_string()
            } else {
                "…".to_owned()
            };
            rsx! {
                div { class: CARD,
                    span { class: "badge badge-primary badge-soft font-semibold", "🎲 {s.drawing}" }
                    p { class: "font-display text-2xl font-extrabold", "{prize_name}" }
                    div { class: "font-display text-6xl font-black text-primary", "{secs}" }
                    p { class: "text-xs text-base-content/60", "{s.locking} · {s.drand_round} {round}" }
                }
            }
        }
        Screen::BetweenDraws {
            last,
            entered,
            prizes_left,
        } => rsx! {
            div { class: CARD,
                if let Some(l) = last {
                    p { class: "text-sm text-base-content/60", "🎉 {l.display_name} → {l.prize_name}" }
                    p { class: "text-xl font-bold",
                        if entered {
                            "{s.not_this_time} 🤞"
                        } else {
                            "{s.drawing}"
                        }
                    }
                } else {
                    p { class: "text-3xl", "🥁" }
                    p { class: "text-xl font-bold", "{s.draw_starting}" }
                }
                span { class: "badge badge-secondary badge-soft mt-1", "{prizes_left} {s.prizes_left}" }
            }
        },
        Screen::Waiting => rsx! {
            div { class: CARD,
                p { class: "text-3xl animate-float", "⏳" }
                p { class: "text-lg font-semibold", "{s.waiting}" }
            }
        },
    }
}
