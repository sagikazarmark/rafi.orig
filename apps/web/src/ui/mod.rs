//! The Dioxus app: organizer admin, projector screen and verification page.

mod admin;
mod forms;
mod live;
mod screen;
mod verify;

use dioxus::prelude::*;
use dioxus_icons::lucide::{
    Dice5, Monitor, Moon, QrCode, ShieldCheck, Sparkles, Sun, Ticket, Trophy,
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
/// The display font's Latin subset, preloaded. Both subsets are served at a
/// stable path because the stylesheet references them by URL.
const DISPLAY_FONT: Asset = asset!(
    "/assets/fonts/bricolage-latin.woff2",
    AssetOptions::builder().with_hash_suffix(false)
);
/// Its Latin Extended subset, which the stylesheet loads only for those letters.
const _: Asset = asset!(
    "/assets/fonts/bricolage-latin-ext.woff2",
    AssetOptions::builder().with_hash_suffix(false)
);

/// Clerk publishable key, baked in at build time. Without it the admin runs
/// in development mode (the server must then run with `RAFI_DEV_AUTH=1`).
pub const CLERK_PUBLISHABLE_KEY: Option<&str> = option_env!("CLERK_PUBLISHABLE_KEY");

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    Home {},
    #[nest("/admin")]
        #[layout(AdminLayout)]
            #[route("/")]
            AdminHome {},
            #[route("/r/:id")]
            RaffleAdmin { id: String },
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
    rsx! {
        document::Stylesheet { href: TAILWIND_CSS }
        document::Link {
            rel: "preload",
            href: DISPLAY_FONT,
            r#as: "font",
            r#type: "font/woff2",
            crossorigin: "anonymous",
        }
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

/// Cycles the colour scheme: system → light → dark. The choice is stored in
/// `localStorage` and applied before paint by `index.html`.
#[component]
pub fn ThemeToggle(#[props(default)] class: String) -> Element {
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
            section { class: "mx-auto grid max-w-6xl gap-4 px-5 pb-24 md:grid-cols-3",
                Feature { icon: rsx! { QrCode { class: "size-6" } }, title: "Scan & join", body: "No app, no account. A name and an email on any phone, even on shaky venue wifi." }
                Feature { icon: rsx! { Dice5 { class: "size-6" } }, title: "Drawn in public", body: "Every prize commits to a future drand round first. The countdown is the suspense." }
                Feature { icon: rsx! { ShieldCheck { class: "size-6" } }, title: "Check it yourself", body: "Anyone can re-run every draw in the browser afterwards. Personal data is erased in one click." }
            }
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
    rsx! {
        ToastProvider { block: ToastBlock::Top, class: "z-50 pt-16",
        div { class: "min-h-screen bg-base-200",
            header { class: "sticky top-0 z-30 border-b border-base-300 bg-base-100/85 backdrop-blur",
                div { class: "mx-auto flex max-w-6xl items-center gap-2 px-4 py-2.5",
                    Link { to: Route::AdminHome {}, Brand {} }
                    span { class: "badge badge-ghost badge-sm ml-1 hidden sm:inline-flex", "organizer" }
                    span { class: "flex-1" }
                    ThemeToggle {}
                    {trailing}
                }
            }
            {children}
        }
        }
    }
}

#[component]
fn AdminLayout() -> Element {
    match CLERK_PUBLISHABLE_KEY {
        Some(key) => rsx! {
            dioxus_clerk::ClerkProvider { publishable_key: key.to_owned(),
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
