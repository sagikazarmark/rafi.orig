//! The Dioxus app: organizer admin, projector screen and verification page.

mod admin;
pub mod dates;
mod forms;
mod live;
mod screen;
mod verify;

use dioxus::prelude::*;
use dioxus_icons::lucide::{
    BookOpen, Dice5, Monitor, Moon, QrCode, ShieldCheck, Sparkles, Sun, Ticket, Trophy, WifiOff,
};

use admin::{AdminHome, RaffleAdmin};
use screen::Screen;
use verify::Verify;

use crate::components::toast::{ToastBlock, ToastProvider};

/// Bundled without a hash, at rafi-server's `pages::style::STYLESHEET_URL`, so the participant pages and
/// `index.html` can link it too. Generic options: dx (0.7.9) ignores the hash flag for CSS options.
const TAILWIND_CSS: Asset = asset!(
    "/assets/tailwind.css",
    AssetOptions::builder().with_hash_suffix(false)
);
/// The display font's Latin subset, preloaded by `index.html` and the
/// server's pages. Both subsets are served at a stable path because the
/// stylesheet references them by URL.
const _: Asset = asset!(
    "/assets/fonts/bricolage-latin.woff2",
    AssetOptions::builder().with_hash_suffix(false)
);
/// Its Latin Extended subset, which the stylesheet loads only for those letters.
const _: Asset = asset!(
    "/assets/fonts/bricolage-latin-ext.woff2",
    AssetOptions::builder().with_hash_suffix(false)
);
/// The icon (the tilted ticket), the home-screen icon and the link preview's
/// image, at stable paths: the server's pages and `index.html` link them.
const _: Asset = asset!(
    "/assets/favicon.svg",
    AssetOptions::builder().with_hash_suffix(false)
);
const _: Asset = asset!(
    "/assets/apple-touch-icon.png",
    AssetOptions::builder().with_hash_suffix(false)
);
const _: Asset = asset!(
    "/assets/og.png",
    AssetOptions::builder().with_hash_suffix(false)
);
/// The display font's licence (SIL OFL 1.1), which goes wherever the font
/// files go: served beside them.
const _: Asset = asset!(
    "/assets/fonts/OFL.txt",
    AssetOptions::builder().with_hash_suffix(false)
);

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    Home {},
    #[nest("/admin")]
        #[layout(AdminLayout)]
            #[route("/")]
            AdminHome {},
            // The tab in the URL (`?tab=people`): a reload mid-evening comes
            // back to it.
            #[route("/r/:id?:tab")]
            RaffleAdmin { id: String, tab: Option<String> },
        #[end_layout]
    #[end_nest]
    #[route("/s/:slug")]
    Screen { slug: String },
    #[route("/v/:slug")]
    Verify { slug: String },
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[component]
pub fn App() -> Element {
    use_context_provider(|| Framed(in_a_frame()));
    rsx! {
        document::Stylesheet { href: TAILWIND_CSS }
        document::Title { "Rafi" }
        Router::<Route> {}
    }
}

/// The logo: a tilted ticket and the wordmark.
#[component]
pub fn Brand(#[props(default = "text-xl".to_string())] size: String) -> Element {
    rsx! {
        span { class: "brand {size}",
            span { class: "grid place-items-center rounded-xl bg-primary p-1.5 text-primary-content -rotate-6 shadow-sm",
                Ticket { class: "size-5" }
            }
            "rafi"
        }
    }
}

/// Which Rafi this is and where its source is (AGPL §13: the people using
/// it over the network are offered the source of the version they use),
/// from the server's `/config.js`.
#[component]
pub fn Colophon(info: crate::config::Build) -> Element {
    rsx! {
        footer { class: "mx-auto flex max-w-6xl flex-wrap items-center justify-center gap-x-2 gap-y-1 px-4 py-6 text-xs text-base-content/60",
            span { "Rafi" if let Some(v) = &info.version { " {v}" } }
            if let Some(url) = &info.source_url {
                span { aria_hidden: "true", "·" }
                a { class: "link link-hover", href: "{url}", target: "_blank", rel: "noopener", "Source code (AGPL-3.0)" }
            }
            span { aria_hidden: "true", "·" }
            a { class: "link link-hover", href: "https://drand.love", target: "_blank", rel: "noopener", "Randomness by drand" }
        }
    }
}

/// Cycles the colour scheme: system → light → dark. The choice is stored in
/// `localStorage` and applied before paint by `/config.js`, which
/// `index.html` loads in its head.
#[component]
pub fn ThemeToggle(
    #[props(default)] class: String,
    /// Told the scheme picked (`system`, `rafi`, `rafi-dark`).
    #[props(default)]
    on_change: Option<EventHandler<String>>,
) -> Element {
    let mut mode = use_signal(|| "system".to_owned());
    use_effect(move || {
        spawn(async move {
            if let Ok(v) =
                document::eval("return localStorage.getItem('rafi-theme') || 'system'").await
                && let Some(s) = v.as_str()
            {
                mode.set(s.to_owned());
            }
        });
    });
    let (icon, label, next) = match mode().as_str() {
        "rafi" => (rsx! { Sun { class: "size-5" } }, "Light theme", "rafi-dark"),
        "rafi-dark" => (rsx! { Moon { class: "size-5" } }, "Dark theme", "system"),
        _ => (rsx! { Monitor { class: "size-5" } }, "System theme", "rafi"),
    };
    rsx! {
        button {
            class: "btn btn-ghost btn-circle btn-sm {class}",
            title: "{label} (click to change)",
            aria_label: "{label} (click to change)",
            onclick: move |_| {
                let js = format!(
                    "try {{ const t = {next:?}; if (t === 'system') {{ localStorage.removeItem('rafi-theme'); document.documentElement.removeAttribute('data-theme'); }} else {{ localStorage.setItem('rafi-theme', t); document.documentElement.setAttribute('data-theme', t); }} }} catch (e) {{}}"
                );
                document::eval(&js);
                mode.set(next.to_owned());
                if let Some(on_change) = on_change {
                    on_change.call(next.to_owned());
                }
            },
            {icon}
        }
    }
}

#[component]
fn Home() -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-100 bg-dots",
            nav { class: "mx-auto flex max-w-6xl items-center gap-3 px-5 py-4",
                Brand {}
                span { class: "flex-1" }
                ThemeToggle {}
                Link { class: "btn btn-sm btn-ghost", to: Route::AdminHome {}, "Organizer login" }
            }
            main { class: "mx-auto grid max-w-6xl items-center gap-12 px-5 pb-20 pt-10 lg:grid-cols-[1.2fr_1fr]",
                div { class: "space-y-6",
                    span { class: "badge badge-soft badge-secondary gap-1.5 font-semibold",
                        Sparkles { class: "size-3.5" }
                        "For meetups, conferences & community events"
                    }
                    h1 { class: "text-5xl font-extrabold leading-[1.05] sm:text-6xl",
                        "Raffles that are "
                        span { class: "gradient-text", "fun" }
                        " and "
                        span { class: "gradient-text", "provably fair." }
                    }
                    p { class: "max-w-xl text-lg text-base-content/70",
                        "Show a QR code, let people join from their phones, and draw every prize live with public randomness nobody can predict, not even you."
                    }
                    div { class: "flex flex-wrap gap-3",
                        Link { class: "btn btn-primary btn-lg hover-wiggle", to: Route::AdminHome {},
                            Ticket { class: "size-5" }
                            "Start a raffle"
                        }
                        a { class: "btn btn-ghost btn-lg", href: "https://drand.love", target: "_blank",
                            ShieldCheck { class: "size-5" }
                            "How fairness works"
                        }
                    }
                }
                div { class: "relative mx-auto w-full max-w-sm",
                    div { class: "absolute -left-6 -top-6 size-24 rounded-full bg-accent/40 blur-2xl" }
                    div { class: "absolute -bottom-8 -right-4 size-32 rounded-full bg-secondary/30 blur-2xl" }
                    div { class: "ticket relative animate-float bg-primary p-8 text-primary-content shadow-xl",
                        p { class: "text-sm font-semibold uppercase tracking-widest opacity-80", "You won!" }
                        p { class: "font-display text-4xl font-extrabold", "KCD Budapest ticket" }
                        div { class: "my-5 border-t-2 border-dashed border-primary-content/40" }
                        div { class: "flex items-end justify-between",
                            div {
                                p { class: "text-xs uppercase tracking-widest opacity-70", "Claim code" }
                                p { class: "claim-code text-3xl", "K7Q2" }
                            }
                            Trophy { class: "size-12 opacity-90" }
                        }
                    }
                }
            }
            section { class: "mx-auto grid max-w-6xl gap-4 px-5 pb-16 md:grid-cols-3",
                Feature { icon: rsx! { QrCode { class: "size-6" } }, title: "Scan & join", body: "No app, no account. A name and an email on any phone, even on shaky venue wifi." }
                Feature { icon: rsx! { Dice5 { class: "size-6" } }, title: "Drawn in public", body: "Every prize commits to a future drand round first. The countdown is the suspense." }
                Feature { icon: rsx! { ShieldCheck { class: "size-6" } }, title: "Check it yourself", body: "Anyone can re-run every draw in the browser afterwards. Personal data is erased on a date entrants see before they join." }
            }
            nav { class: "flex justify-center gap-4 text-sm text-base-content/70",
                a { class: "link link-hover", href: "/terms", "Terms" }
                a { class: "link link-hover", href: "/privacy", "Privacy" }
            }
            Colophon { info: use_hook(crate::config::build) }
        }
    }
}

#[component]
fn Feature(icon: Element, title: String, body: String) -> Element {
    rsx! {
        div { class: "card bg-base-100 shadow-sm ring-1 ring-base-300 transition hover:-translate-y-1 hover:shadow-md",
            div { class: "card-body",
                span { class: "grid size-11 place-items-center rounded-2xl bg-primary/10 text-primary", {icon} }
                h3 { class: "card-title text-xl", "{title}" }
                p { class: "text-base-content/70", "{body}" }
            }
        }
    }
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! {
        main { class: "grid min-h-screen place-items-center bg-base-100 bg-dots p-6",
            div { class: "space-y-4 text-center",
                p { class: "text-7xl", "🎟️" }
                h1 { class: "text-4xl font-extrabold", "Nothing to win here" }
                p { class: "text-base-content/70", "There is nothing at /{segments.join(\"/\")}." }
                Link { class: "btn btn-primary", to: Route::Home {}, "Back home" }
            }
        }
    }
}

#[component]
fn AdminShell(children: Element, #[props(default = VNode::empty())] trailing: Element) -> Element {
    let connection = use_context_provider(|| live::AdminConnection(Signal::new(None)));
    let guide = use_hook(|| crate::config::build().guide_url);
    let online = live::use_online();
    rsx! {
        ToastProvider { block: ToastBlock::Top, class: "z-50 pt-16",
        div { class: "min-h-screen bg-base-200",
            header { class: "sticky top-0 z-30 border-b border-base-300 bg-base-100/85 backdrop-blur",
                div { class: "mx-auto flex max-w-6xl items-center gap-2 px-4 py-2.5",
                    Link { to: Route::AdminHome {}, Brand {} }
                    span { class: "badge badge-ghost badge-sm ml-1 hidden sm:inline-flex", "organizer" }
                    span { class: "flex-1" }
                    OfflineBadge { online: online(), trouble: connection.0() }
                    if let Some(guide) = guide.clone() {
                        a { class: "btn btn-ghost btn-sm", href: "{guide}", target: "_blank", rel: "noopener", BookOpen { class: "size-4" } span { class: "hidden sm:inline", "Guide" } }
                    }
                    ThemeToggle {}
                    {trailing}
                }
            }
            {children}
            Colophon { info: use_hook(crate::config::build) }
        }
        }
    }
}

/// "Offline" in the admin's header, while the browser says it is or the
/// raffle shown can't be fetched for a while: what the page shows may be
/// out of date, and what is pressed may not arrive.
#[component]
fn OfflineBadge(online: bool, trouble: Option<live::Trouble>) -> Element {
    if online && !trouble.is_some_and(|t| t.offline()) {
        return rsx! {};
    }
    rsx! {
        span { class: "badge badge-warning gap-1 font-semibold", role: "status",
            title: "Can't reach Rafi: what this page shows may be out of date. It catches up by itself once the connection is back.",
            WifiOff { class: "size-3.5" }
            "Offline"
        }
    }
}

/// Whether the web app is shown inside another page's frame, which [`App`]
/// reads once: a document stays in the frame it was loaded into.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Framed(bool);

/// Whether this window isn't the top one (`window.top !== window.self`). A
/// window this can't tell of counts as framed: the admin fails closed.
fn in_a_frame() -> bool {
    let Some(window) = web_sys::window() else {
        return true;
    };
    !matches!(window.top(), Ok(Some(top)) if top == window)
}

/// The admin's layout, which doesn't render inside another page's frame.
///
/// The server sends `frame-ancestors 'none'` with the admin, but a browser
/// checks that only when a document loads, and the router changes pages
/// without loading one: a projector or a verification page framed elsewhere
/// (which the server allows) that navigated to the admin would show it in the
/// framer's page, under the framer's clicks. So the admin checks for itself,
/// before Clerk loads, and offers a link that opens it at the top instead.
#[component]
fn AdminLayout() -> Element {
    let Framed(framed) = use_context();
    if framed {
        rsx! { OpenAtTheTop {} }
    } else {
        rsx! { AdminBehindSignIn {} }
    }
}

/// What a framed admin shows instead: a link that opens the page in the
/// whole tab, where the server's `frame-ancestors 'none'` applies.
#[component]
fn OpenAtTheTop() -> Element {
    let here = use_route::<Route>().to_string();
    rsx! {
        main { class: "grid min-h-screen place-items-center bg-base-100 bg-dots p-6",
            div { class: "max-w-md space-y-4 text-center",
                Brand {}
                h1 { class: "text-3xl font-extrabold", "The organizer pages don't open inside another site" }
                p { class: "text-base-content/70", "Open Rafi in its own tab to sign in and run raffles." }
                a { class: "btn btn-primary", href: "{here}", target: "_top", "Open Rafi in its own tab" }
            }
        }
    }
}

/// The admin behind sign-in, as the server configured it (`/config.js`,
/// [`crate::config`]): Clerk with the server's publishable key, or dev auth,
/// which the badge says out loud.
#[component]
fn AdminBehindSignIn() -> Element {
    let key = use_hook(crate::config::clerk_publishable_key);
    match key {
        Some(key) => rsx! {
            dioxus_clerk::ClerkProvider { publishable_key: key,
                AdminShell {
                    trailing: rsx! {
                        dioxus_clerk::OrganizationSwitcher {}
                        dioxus_clerk::UserButton {}
                    },
                    dioxus_clerk::SignedOutWhenLoaded {
                        fallback: rsx! {
                            div { class: "grid place-items-center py-24", span { class: "loading loading-dots loading-lg text-primary" } }
                        },
                        div { class: "mx-auto max-w-md px-4 py-20",
                            div { class: "card bg-base-100 shadow-lg ring-1 ring-base-300",
                                div { class: "card-body items-center text-center",
                                    span { class: "text-5xl", "🎟️" }
                                    h2 { class: "card-title text-2xl", "Organizer sign in" }
                                    p { class: "text-base-content/70", "Sign in to set up raffles and run the draw." }
                                    dioxus_clerk::SignInButton { class: "btn btn-primary btn-wide mt-2", "Sign in" }
                                }
                            }
                        }
                    }
                    dioxus_clerk::SignedIn { Outlet::<Route> {} }
                }
            }
        },
        None => rsx! {
            AdminShell {
                trailing: rsx! {
                    span { class: "badge badge-warning badge-sm", title: "RAFI_DEV_AUTH: admin pages are not protected", "dev auth" }
                },
                Outlet::<Route> {}
            }
        },
    }
}

/// A join code in groups of three (`K7Q-2X3`), easier to read out and to
/// type; the server drops the dash.
pub fn grouped(code: &str) -> String {
    code.chars()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// The colour scheme picked with the toggle (`localStorage rafi-theme`), if
/// one was.
pub fn stored_theme() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item("rafi-theme")
        .ok()?
        .filter(|t| !t.is_empty())
}

/// Copies text to the clipboard (best effort).
pub fn copy_to_clipboard(text: String) {
    let js = format!(
        "navigator.clipboard && navigator.clipboard.writeText({})",
        serde_json::to_string(&text).unwrap_or_default()
    );
    document::eval(&js);
}

/// A fresh idempotency key for one button press.
pub fn command_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The browser's clock, in unix milliseconds.
pub fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

/// The themes' colours read: WCAG contrast of 4.5:1 or more for every colour
/// and its `-content`, base-content on each base, and the light theme's
/// gradient text on the bases.
#[cfg(test)]
mod theme_contrast {
    const CSS: &str = include_str!("../../tailwind.css");

    fn luminance(hex: &str) -> f64 {
        let hex = hex.trim_start_matches('#');
        let channel = |i: usize| {
            let c = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Each theme's name and its `--color-*` hex values.
    fn themes() -> Vec<(String, Vec<(String, String)>)> {
        CSS.split("@plugin \"daisyui/theme\" {")
            .skip(1)
            .map(|block| {
                let block = &block[..block.find('}').unwrap()];
                let mut name = String::new();
                let mut colours = Vec::new();
                for line in block.lines().map(str::trim) {
                    if let Some(n) = line.strip_prefix("name: ") {
                        name = n.trim_matches(|c| c == '"' || c == ';').to_owned();
                    } else if let Some(rest) = line.strip_prefix("--color-")
                        && let Some((key, value)) = rest.split_once(':')
                    {
                        colours.push((
                            key.to_owned(),
                            value.trim().trim_end_matches(';').to_owned(),
                        ));
                    }
                }
                (name, colours)
            })
            .collect()
    }

    #[test]
    fn every_pair_reads() {
        let themes = themes();
        assert_eq!(themes.len(), 2, "{themes:?}");
        let mut failures = Vec::new();
        for (name, colours) in &themes {
            let get = |k: &str| {
                colours
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.as_str())
            };
            let mut pairs: Vec<(String, String)> = colours
                .iter()
                .filter(|(k, _)| get(&format!("{k}-content")).is_some())
                .map(|(k, _)| (k.clone(), format!("{k}-content")))
                .collect();
            assert!(pairs.len() >= 8, "{name}: {pairs:?}");
            for base in ["base-100", "base-200", "base-300"] {
                pairs.push((base.to_owned(), "base-content".to_owned()));
            }
            for (bg, fg) in pairs {
                let ratio = contrast(get(&bg).unwrap(), get(&fg).unwrap());
                if ratio < 4.5 {
                    failures.push(format!("{name}: {fg} on {bg}: {ratio:.2}"));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The light theme's `.gradient-text` (the winner's name, the thank-you):
    /// each of its stops reads on the bases.
    #[test]
    fn the_gradient_text_reads() {
        let themes = themes();
        let (_, light) = themes.iter().find(|(n, _)| n == "rafi").unwrap();
        let get = |k: &str| light.iter().find(|(key, _)| key == k).unwrap().1.clone();
        let rule = CSS.split(".gradient-text {").nth(1).unwrap();
        let rule = &rule[..rule.find('}').unwrap()];
        let gradient = rule.split("linear-gradient(").nth(1).unwrap();
        let gradient = &gradient[..gradient.find(");").unwrap()];
        let stops: Vec<String> = gradient
            .split(',')
            .skip(1)
            .map(|stop| {
                let colour = stop.split_whitespace().next().unwrap();
                match colour.strip_prefix("var(--color-") {
                    Some(var) => get(var.trim_end_matches(')')),
                    None => colour.to_owned(),
                }
            })
            .collect();
        assert_eq!(stops.len(), 3, "{gradient}");
        for stop in &stops {
            for base in ["base-100", "base-200"] {
                let ratio = contrast(stop, &get(base));
                assert!(ratio >= 4.5, "{stop} on {base}: {ratio:.2}");
            }
        }
    }
}

/// The router, server-rendered natively at a path, framed or not as a test
/// says ([`App`] would ask the browser). Only pages that touch no browser API
/// on their first render can be rendered so: the framed admin, the home page,
/// the 404.
#[cfg(test)]
mod tests {
    use std::{rc::Rc, str::FromStr};

    use dioxus::history::{MemoryHistory, provide_history_context};

    use super::*;

    #[component]
    fn At(path: String, framed: bool) -> Element {
        use_context_provider(|| Framed(framed));
        use_hook(|| provide_history_context(Rc::new(MemoryHistory::with_initial_path(&path))));
        rsx! { Router::<Route> {} }
    }

    fn render(path: &str, framed: bool) -> String {
        dioxus_ssr::render_element(rsx! { At { path, framed } })
    }

    const OPEN_AT_THE_TOP: &str = "Open Rafi in its own tab";

    /// The footer names the version and links its source; a server before
    /// them leaves just the name.
    #[test]
    fn the_colophon_links_the_source() {
        let build = crate::config::Build {
            version: Some("0.1.0".into()),
            source_url: Some("https://github.com/example/rafi/tree/v0.1.0".into()),
            verifier_url: None,
            guide_url: None,
        };
        let html = dioxus_ssr::render_element(rsx! { Colophon { info: build } });
        assert!(
            html.contains("Rafi 0.1.0")
                && html.contains(r#"href="https://github.com/example/rafi/tree/v0.1.0""#),
            "{html}"
        );
        let html =
            dioxus_ssr::render_element(rsx! { Colophon { info: crate::config::Build::default() } });
        assert!(
            html.contains("Rafi") && !html.contains("Source code"),
            "{html}"
        );
    }

    /// "Offline" when the browser says so, or when the raffle's view has
    /// failed for a while; not for a poll or two lost.
    #[test]
    fn the_admin_says_when_it_is_offline() {
        use live::{Trouble, TroubleKind};
        let render =
            |online, trouble| dioxus_ssr::render_element(rsx! { OfflineBadge { online, trouble } });
        assert!(!render(true, None).contains("Offline"));
        assert!(render(false, None).contains("Offline"));
        let first = Trouble::after(None, TroubleKind::Unreachable, 0);
        assert!(!render(true, Some(first)).contains("Offline"));
        let later = Trouble::after(Some(first), TroubleKind::Unreachable, 10_000);
        assert!(render(true, Some(later)).contains("Offline"));
    }

    #[test]
    fn the_admin_does_not_render_in_a_frame() {
        for path in ["/admin", "/admin/", "/admin/r/some-id"] {
            let route = Route::from_str(path).unwrap();
            assert!(
                matches!(route, Route::AdminHome {} | Route::RaffleAdmin { .. }),
                "{path}: {route:?}"
            );
            let html = render(path, true);
            assert!(html.contains(OPEN_AT_THE_TOP), "{path}: {html}");
            assert!(
                html.contains(&format!(r#"href="{route}" target="_top""#)),
                "{path}: {html}"
            );
            assert!(
                !html.contains("<header"),
                "{path}: the admin's shell: {html}"
            );
        }
    }

    /// A way into the admin from a frame the server allows: `/s/` with one
    /// segment too many is framed (the server goes by the prefix) but is the
    /// router's 404, which links home, which links to the admin. Those pages
    /// render in a frame; the admin is the one that won't.
    #[test]
    fn the_pages_on_the_way_to_the_admin_render_in_a_frame() {
        let not_found = render("/s/some-slug/more", true);
        assert!(not_found.contains("Nothing to win here"), "{not_found}");
        assert!(not_found.contains(r#"href="/""#), "{not_found}");
        assert!(!not_found.contains(OPEN_AT_THE_TOP), "{not_found}");

        let home = render("/", true);
        assert!(home.contains(r#"href="/admin/""#), "{home}");
        assert!(!home.contains(OPEN_AT_THE_TOP), "{home}");
    }
}
