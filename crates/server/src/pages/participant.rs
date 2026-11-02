//! The participant phone page.
//!
//! The whole page is rendered here, including the live card
//! ([`rafi_live_client::Card`], the same component the phone's WebAssembly
//! mounts), with the page's state embedded as JSON in `#rafi-config` (see
//! [`PageConfig`]). The participant build's loader ([`LOADER`]) then starts
//! the WebAssembly from `/participant/assets/`, or the JavaScript fallback
//! when it can't run. They keep the card current, solve the join form's
//! proof-of-work and enable its button.

use dioxus::prelude::*;
use rafi_live_client::{Card, CardStrings, Follow, PageConfig, ParticipantView, Phase};
use serde::Serialize;

use dioxus_icons::lucide::{Monitor, Moon, Sun, Ticket};

use super::{
    style::STYLESHEET_URL,
    text::{ErasureDates, erasure_sentence},
};

/// The raffle, as the public pages show it.
#[derive(Debug, Clone, Copy)]
pub struct Raffle<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub slug: &'a str,
    pub join_code: &'a str,
    pub test_mode: bool,
    /// Whether someone may join now: registration open, and the erasure
    /// date not yet passed ([`rafi_lifecycle::entrant::may_join`]). The
    /// form is shown only then.
    pub joinable: bool,
    /// Whether people join with the QR code at all. Without registration
    /// the page has no form, whatever the state: the organizer adds the
    /// people taking part.
    pub registration: bool,
    /// Whether the frozen list is published: the verification page has it.
    pub verifiable: bool,
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
    /// Generated names the join form's 🎲 button suggests.
    pub name_ideas: &'a [String],
    /// The server's clock (unix ms); the phone measures its offset from it.
    pub now_ms: u64,
    /// When personal data is erased: the join form says so, in one sentence.
    pub retention: ErasureDates,
    /// The youngest an entrant may be: the join form says so too.
    pub min_age: u32,
    /// The installation's public origin, for the link preview's image.
    pub public_url: &'a str,
}

/// [`APPLY_THEME`]'s source, a macro so that [`HEAD_SCRIPT`] can `concat!` it.
macro_rules! apply_theme {
    () => {
        r#"try{var t=localStorage.getItem("rafi-theme");if(t)document.documentElement.setAttribute("data-theme",t)}catch(e){}"#
    };
}

/// Applies the saved colour scheme (`rafi-theme` in `localStorage`, which the
/// pages and the web app share) before first paint: in [`HEAD_SCRIPT`] here,
/// and in `/config.js` for the web app, whose shell runs nothing inline.
pub(crate) const APPLY_THEME: &str = apply_theme!();

/// Runs before first paint: records when the page started parsing (the
/// phone's clock offset is measured from it) and applies the saved colour
/// scheme ([`APPLY_THEME`]).
const HEAD_SCRIPT: &str = concat!("window.__rafiT0=Date.now();", apply_theme!());

/// The page's loader, a `<script>`: imports the participant glue, which
/// fetches the module by its hashed name and calls `main`; or starts the
/// JavaScript fallback when there is no WebAssembly, the glue fails to
/// load, the module fails to load or compile (dx's glue starts that itself,
/// not chained to the `import()`, so it surfaces as an unhandled rejection:
/// before the module reported ready, that means it isn't coming), or it
/// isn't ready within 8 s (the module's panic hook also calls
/// `rafiFallback`). Its source is `apps/participant/index.html`; this is the
/// copy the build left (`just build` in `apps/participant`), with the glue's hashed
/// name filled in. Compiled in, so a server is built against one participant
/// build and links that one; natively, `Dirs::check_participant` checks the
/// directory served is that build.
pub const LOADER: &str = include_str!("../../../../apps/participant/dist/public/index.html");

/// The scripts the pages embed: [`HEAD_SCRIPT`] in every page, and the
/// [`LOADER`]'s in the participant page. The pages' policy allows these by
/// their hashes (`crate::csp`), and no others.
pub(crate) fn embedded_scripts() -> Vec<&'static str> {
    let mut scripts = vec![HEAD_SCRIPT];
    scripts.extend(crate::csp::inline_scripts(LOADER));
    scripts
}

/// What a link to a page shows when it's pasted into a chat (Open Graph):
/// the raffle's title, the line under it, and the image, absolute.
#[derive(Debug, Clone)]
pub(crate) struct Preview<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// The installation's public origin (`RAFI_PUBLIC_URL`).
    pub public_url: &'a str,
}

impl Preview<'_> {
    fn meta(&self) -> String {
        let origin = self.public_url.trim_end_matches('/');
        format!(
            "<meta property=\"og:type\" content=\"website\"><meta property=\"og:title\" content=\"{}\">\
             <meta property=\"og:description\" content=\"{}\">\
             <meta property=\"og:image\" content=\"{}/assets/og.png\">\
             <meta property=\"og:image:width\" content=\"1200\"><meta property=\"og:image:height\" content=\"630\">\
             <meta name=\"twitter:card\" content=\"summary_large_image\">",
            escape(self.title),
            escape(self.description),
            escape(origin),
        )
    }
}

/// A whole document: `tail` is raw HTML appended to the body (scripts);
/// `preview`, what a link to it shows in a chat.
pub(crate) fn document(
    title: &str,
    body: String,
    tail: Option<String>,
    preview: Option<Preview<'_>>,
) -> String {
    let title = escape(title);
    let css = escape(STYLESHEET_URL);
    let tail = tail.unwrap_or_default();
    let preview = preview.map(|p| p.meta()).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
         <meta name=\"robots\" content=\"noindex\"><meta name=\"color-scheme\" content=\"light dark\">\
         <meta name=\"theme-color\" content=\"#fffdf8\" media=\"(prefers-color-scheme: light)\">\
         <meta name=\"theme-color\" content=\"#17131f\" media=\"(prefers-color-scheme: dark)\">\
         <link rel=\"icon\" href=\"/assets/favicon.svg\" type=\"image/svg+xml\">\
         <link rel=\"apple-touch-icon\" href=\"/assets/apple-touch-icon.png\">{preview}\
         <script>{HEAD_SCRIPT}</script>\
         <link rel=\"stylesheet\" href=\"{css}\">\
         <link rel=\"preload\" href=\"/assets/bricolage-latin.woff2\" as=\"font\" type=\"font/woff2\" crossorigin>\
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
        name_ideas,
        now_ms,
        retention,
        min_age,
        public_url,
    } = page;
    let erasure = erasure_sentence(&retention);
    let age = super::text::age_line(min_age);
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
    // The verification page finds this entrant in the frozen list. After the
    // `#`, their ID stays out of requests and server logs.
    let verify_href = match &me {
        Some(id) => format!("/v/{}#me={id}", raffle.slug),
        None => format!("/v/{}", raffle.slug),
    };
    let claim_code = entrant.map(|e| e.claim_code.to_owned());
    let joining = entrant.is_none() && raffle.joinable && notice != Notice::AlreadyIn;

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
                    // Up to 64 characters once the organizer renames
                    // them: wrapped, and isolated from the page's direction.
                    h2 { class: "text-3xl font-extrabold wrap-anywhere", bdi { "{e.display_name}" } }
                    p { class: "text-sm opacity-80", "Your name on the screen." }
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
                        div { class: "fieldset",
                            label { class: "fieldset-legend text-sm", r#for: "name", "Name shown on the screen (optional)" }
                            div { class: "join w-full",
                                input { class: "input input-lg join-item w-full", id: "name", name: "name", maxlength: "{rafi_lifecycle::entrant::MAX_NAME_CHARS}", autocomplete: "nickname", aria_describedby: "name-hint", value: "{name}" }
                                // Enabled by the WebAssembly (or the fallback):
                                // fills in the next of `name_ideas`.
                                button { class: "btn btn-lg join-item", r#type: "button", id: "name-dice", disabled: true, title: "Suggest a name", aria_label: "Suggest a name", "🎲" }
                            }
                            p { class: "label whitespace-normal", id: "name-hint", "A first name or nickname is enough. Leave it blank and we'll pick one for you, like Brave Otter." }
                        }
                        div { class: "fieldset",
                            label { class: "fieldset-legend text-sm", r#for: "email", "Email" }
                            input { class: "input input-lg w-full", id: "email", name: "email", r#type: "email", required: true, autocomplete: "email", inputmode: "email", aria_describedby: "email-hint", value: "{email}" }
                            p { class: "label whitespace-normal", id: "email-hint", "Only to tell entries apart and to deliver prizes. Never shown." }
                        }
                        label { class: "mt-3 flex items-start gap-3",
                            input { class: "checkbox checkbox-primary mt-0.5", r#type: "checkbox", name: "rules", value: "on", required: true }
                            span { class: "text-sm",
                                "I accept the "
                                a { class: "link link-primary", href: "{rules_href}", target: "_blank", "raffle rules" }
                                "."
                            }
                        }
                        // The notice is read, not accepted: it isn't consent.
                        p { class: "mt-2 text-xs text-base-content/70",
                            "{erasure} {age} "
                            a { class: "link link-primary", href: "{privacy_href}", target: "_blank", "Privacy notice" }
                        }
                        input { r#type: "hidden", name: "proof", id: "proof" }
                        // Enabled by the WebAssembly (or the fallback) once the
                        // proof-of-work is solved; it then shows `data-label`.
                        button { class: "btn btn-primary btn-lg btn-block mt-4 rounded-full", r#type: "submit", id: "submit", disabled: true, "data-label": "Join 🎟️", aria_describedby: "pow-status",
                            span { class: "loading loading-spinner loading-md" }
                            "Getting ready…"
                        }
                        // What the wait is, said aloud too: the button above
                        // is disabled meanwhile, and a disabled button says
                        // nothing. The scripts clear it once solved, or say
                        // `data-failed` when the check couldn't run (Join
                        // then tries again).
                        p { id: "pow-status", class: "min-h-5 text-center text-sm text-base-content/70", role: "status", aria_live: "polite",
                            "data-failed": "Couldn't reach the server for the bot check. Tap Join to try again.",
                            "Checking this isn't a bot… (a few seconds)"
                        }
                    }
                }
            }
        }
        None => {
            let (emoji, msg) = match (notice, phase) {
                // One wording whoever is in under that email: someone who
                // joined on another device, or someone the organizer added,
                // who has no device with a claim code. Typing an email never
                // attaches this device to the entry (no cookie is set), or
                // anyone could take over an entry with someone else's email.
                (Notice::AlreadyIn, _) => (
                    "👋",
                    "This email is already entered. Your entry is safe. If you win, go to the organizer.",
                ),
                // Nothing to join, in any state, and a posted form was
                // refused for it: no "not open yet" to wait for.
                _ if !raffle.registration => (
                    "📋",
                    "This raffle has no registration: the organizer adds the people taking part.",
                ),
                (Notice::Error(why), _) => ("⚠️", why),
                (_, Phase::Ready) => (
                    "⏳",
                    "Registration is not open yet. Keep an eye on the screen!",
                ),
                _ => ("🔒", "Joining has closed."),
            };
            rsx! {
                section { class: "{card} items-center gap-2 p-6 text-center",
                    span { class: "text-4xl", "{emoji}" }
                    p { class: "text-lg font-semibold", role: "status", "{msg}" }
                    // Someone who mistyped their email (or shares a phone)
                    // gets the form back.
                    if notice == Notice::AlreadyIn && raffle.joinable {
                        a { class: "link link-primary text-sm", href: "/j/{code}", "Not you? Join with another email" }
                    }
                    // What someone on the organizer's list reads: the rules
                    // they are in under, the notice, and the frozen list.
                    if !raffle.registration {
                        p { class: "flex flex-wrap justify-center gap-x-3 text-sm",
                            a { class: "link link-primary", href: "{rules_href}", "Rules" }
                            a { class: "link link-primary", href: "{privacy_href}", "Privacy notice" }
                            if raffle.verifiable {
                                a { class: "link link-primary", href: "{verify_href}", "Verification page" }
                            }
                        }
                    }
                }
                {live}
            }
        }
    };

    let title = raffle.title;
    let header = header_bar(raffle.test_mode);
    let source_href = crate::build_info::source_url();
    let body = dioxus::ssr::render_element(rsx! {
        // Clear of a notch or rounded corner in landscape (viewport-fit=cover).
        main { class: "mx-auto flex max-w-md flex-col gap-4 pt-[max(1rem,env(safe-area-inset-top))] pr-[max(1rem,env(safe-area-inset-right))] pb-[max(2.5rem,env(safe-area-inset-bottom))] pl-[max(1rem,env(safe-area-inset-left))]",
            {header}
            h1 { class: "text-2xl font-extrabold leading-tight", "{title}" }
            {main}
            footer { class: "mt-2 flex flex-wrap justify-center gap-x-3 gap-y-1 text-sm text-base-content/60",
                a { class: "link link-hover", href: "{rules_href}", "Rules" }
                a { class: "link link-hover", href: "{privacy_href}", "Privacy notice" }
                a { class: "link link-hover", href: "{verify_href}", "Check the draw yourself" }
                // AGPL §13: the source of the version running.
                a { class: "link link-hover", href: "{source_href}", rel: "noopener", "Source code" }
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
        needs_pow: joining,
        challenge_url: PageConfig::CHALLENGE_URL.into(),
        screen_on: "Screen will stay on".to_owned(),
        keep_screen_on: "Keep my screen on".to_owned(),
        name_ideas: if joining {
            name_ideas.to_vec()
        } else {
            Vec::new()
        },
    };
    let tail = format!(
        "<script type=\"application/json\" id=\"rafi-config\">{}</script>{LOADER}",
        script_json(&config)
    );
    let preview = Preview {
        title: raffle.title,
        description: "Join the raffle from your phone: no app, no account.",
        public_url,
    };
    document(raffle.title, body, Some(tail), Some(preview))
}

/// The page for a raffle that isn't there (or isn't public), as HTML.
pub fn not_found() -> String {
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex min-h-dvh max-w-md flex-col items-center justify-center gap-3 px-4 text-center",
            span { class: "text-6xl", "🎟️" }
            p { class: "text-xl font-bold", "This raffle does not exist." }
        }
    });
    document("Rafi", body, None, None)
}
