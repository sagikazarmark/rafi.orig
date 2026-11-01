//! The projector screen: public and read-only.

use dioxus::prelude::*;
use rafi_lifecycle::RaffleState;
use rafi_raffle::Outcome;

use dioxus_icons::lucide::{Dice5, Maximize, ShieldCheck, Trophy};
use rafi_live_client::ParticipantView;

use super::{
    Brand, ThemeToggle,
    live::{ServerClock, Trouble, TroubleKind, reload_once, use_clock, use_live_view},
};
use rafi_api::{DrawView, RaffleView};

#[component]
pub fn Screen(slug: String) -> Element {
    rsx! {
        ScreenInner { key: "{slug}", slug: slug.clone() }
    }
}

#[component]
fn ScreenInner(slug: String) -> Element {
    let live = use_live_view(slug.clone());
    let trouble = *live.trouble.read();
    // A view this page can't read: the server is newer, and so is its page.
    use_effect(move || {
        if live
            .trouble
            .read()
            .is_some_and(|t| t.kind == TroubleKind::Newer)
        {
            reload_once();
        }
    });
    let guard = live.view.read();
    let Some(v) = guard.as_ref() else {
        return rsx! { Waiting { trouble } };
    };
    let v = v.clone();
    drop(guard);
    let ahead = live.ahead_of(&v);
    let clock = *live.clock.read();

    let body = match v.state {
        RaffleState::Draft => {
            rsx! { p { class: "text-3xl font-bold text-base-content/60", "{NOT_FOUND}" } }
        }
        RaffleState::Ready | RaffleState::Open | RaffleState::Closed => {
            rsx! { Intro { view: v.clone() } }
        }
        RaffleState::Frozen | RaffleState::Drawing => {
            rsx! { Drawing { view: v.clone(), ahead, clock } }
        }
        RaffleState::Finished | RaffleState::Purged => rsx! { Finished { view: v.clone() } },
    };

    rsx! {
        document::Title { "{v.title} — Rafi" }
        main { class: "flex min-h-screen flex-col gap-[2vh] bg-base-100 bg-dots px-[3vw] py-[2.5vh] text-[clamp(16px,1.5vw,34px)]",
            header { class: "flex items-center gap-4",
                h1 { class: "flex-1 text-[2.2em] font-extrabold leading-tight", "{v.title}" }
                Connection { trouble }
                if v.test_mode {
                    span { class: "badge badge-warning badge-lg font-bold", "🧪 TEST MODE" }
                }
                div { class: "flex items-center gap-1 opacity-40 transition hover:opacity-100",
                    ThemeToggle {}
                    FullscreenButton {}
                }
            }
            {body}
            {live.subscription()}
            footer { class: "flex items-center justify-between text-[0.7em] text-base-content/50",
                Brand { size: "text-[1.1em]" }
                span { class: "inline-flex items-center gap-1", ShieldCheck { class: "size-[1.2em]" } "drand · provably fair" }
            }
        }
    }
}

/// What the screen says at a slug with no raffle to show (none, or not
/// published yet).
const NOT_FOUND: &str = "No raffle at this address, or it isn't published yet.";

/// What the screen says when its page is older than the server.
const RELOADING: &str = "This screen is older than the server: reloading…";

/// Before the first view: loading, or why there is none yet. Fetching goes
/// on, so a raffle published meanwhile comes up by itself.
#[component]
fn Waiting(trouble: Option<Trouble>) -> Element {
    let kind = trouble.map(|t| t.kind);
    rsx! {
        main { class: "grid min-h-screen place-items-center bg-base-100 bg-dots p-6 text-[clamp(16px,1.5vw,34px)]",
            div { class: "flex flex-col items-center gap-[1em] text-center", role: "status",
                Brand { size: "text-4xl" }
                match kind {
                    Some(TroubleKind::NotFound) => rsx! {
                        p { class: "max-w-[24em] text-[1.4em] font-bold", "{NOT_FOUND}" }
                    },
                    Some(TroubleKind::Newer) => rsx! {
                        p { class: "text-[1.4em] font-bold", "{RELOADING}" }
                    },
                    _ => rsx! {
                        span { class: "loading loading-dots loading-lg text-primary" }
                        if trouble.is_some_and(|t| t.offline()) {
                            p { class: "text-[1.2em] font-semibold text-base-content/70", "Can't reach the server. Retrying…" }
                        }
                    },
                }
            }
        }
    }
}

/// A badge in the header while the view can't be refreshed: the screen goes
/// on showing the last one, and the room shouldn't trust it blindly.
#[component]
fn Connection(trouble: Option<Trouble>) -> Element {
    let label = match trouble {
        Some(t) if t.kind == TroubleKind::Newer => RELOADING,
        Some(t) if t.offline() => "Reconnecting…",
        _ => return rsx! {},
    };
    rsx! {
        span { class: "inline-flex items-center gap-[0.4em] rounded-full bg-warning px-[0.8em] py-[0.25em] text-[0.8em] font-bold text-warning-content",
            role: "status",
            span { class: "loading loading-spinner size-[1em]" }
            "{label}"
        }
    }
}

#[component]
fn FullscreenButton() -> Element {
    rsx! {
        button {
            class: "btn btn-ghost btn-circle btn-sm",
            title: "Fullscreen (keeps the screen awake)",
            onclick: move |_| {
                document::eval(
                    "(async () => { try { if (!document.fullscreenElement) await document.documentElement.requestFullscreen(); \
                     if ('wakeLock' in navigator) await navigator.wakeLock.request('screen'); } catch (e) {} })()",
                );
            },
            Maximize { class: "size-5" }
        }
    }
}

/// `url` as the screen shows it, without its scheme: shorter to read off
/// and type.
fn without_scheme(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, rest)| rest)
}

fn qr_svg(url: &str) -> String {
    use qrcode::{EcLevel, QrCode, render::svg};
    QrCode::with_error_correction_level(url.as_bytes(), EcLevel::M)
        .map(|code| {
            code.render::<svg::Color>()
                .min_dimensions(320, 320)
                .quiet_zone(false)
                .dark_color(svg::Color("#211b33"))
                .light_color(svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default()
}

#[component]
fn PrizeList(view: RaffleView) -> Element {
    rsx! {
        section { class: "space-y-[0.6em]",
            h2 { class: "text-[0.8em] font-bold uppercase tracking-[0.2em] text-base-content/50", "Prizes" }
            ul { class: "space-y-[0.5em]",
                for p in view.prizes.iter() {
                    li {
                        key: "{p.prize_id}",
                        class: if p.left == 0 { "flex items-center gap-[0.6em] rounded-box bg-base-200/70 px-[0.8em] py-[0.5em] opacity-40 line-through" } else { "flex items-center gap-[0.6em] rounded-box bg-base-200/70 px-[0.8em] py-[0.5em]" },
                        span { class: "grid min-w-[2.4em] place-items-center rounded-xl bg-secondary px-[0.4em] py-[0.15em] font-display font-extrabold text-secondary-content", "{p.left}×" }
                        span { class: "flex-1 font-semibold", "{p.name}" }
                        if !p.sponsor.is_empty() {
                            span { class: "text-[0.75em] text-base-content/50", "{p.sponsor}" }
                        }
                    }
                }
            }
        }
    }
}

/// Before the freeze: the prizes, and how to get in. With registration,
/// the QR code while it is open, and, once people were added from a list,
/// that they needn't scan; without, no code, link or "No smartphone?": the
/// organizer's list is who takes part, and the screen says so, how to stay
/// out, and where the privacy notice is.
#[component]
fn Intro(view: RaffleView) -> Element {
    let qr = if view.registration {
        qr_svg(&view.join_url)
    } else {
        String::new()
    };
    let short = without_scheme(&view.join_url).to_owned();
    let privacy = without_scheme(&view.privacy_url).to_owned();
    rsx! {
        div { class: "grid flex-1 items-center gap-[3vw] lg:grid-cols-[1.15fr_1fr]",
            div { class: "min-w-0 space-y-[1.4em]",
                if !view.description.is_empty() {
                    p { class: "max-w-[34em] text-[1.25em] text-base-content/75", "{view.description}" }
                }
                PrizeList { view: view.clone() }
                if view.registration {
                    div { class: "inline-flex items-center gap-[0.6em] rounded-box bg-accent px-[1em] py-[0.6em] font-bold text-accent-content -rotate-1 shadow-sm",
                        span { class: "text-[1.4em]", "📵" }
                        "No smartphone? Come to the organizer!"
                    }
                }
            }
            div { class: "flex min-w-0 flex-col items-center gap-[0.8em] text-center",
                match view.state {
                    // Without registration: no QR code, and nothing said of
                    // registration opening or closing. Who is in goes here.
                    // Who is in, and how to stay out: the people on the
                    // list saw no join form, and the room is where they
                    // learn the draw is theirs too.
                    _ if !view.registration => rsx! {
                        span { class: "animate-float text-[5em]", "🎟️" }
                        p { class: "font-display text-[2em] font-extrabold", "The draw is coming up" }
                        p { class: "max-w-[24em] text-[1.15em] font-semibold",
                            "Everyone on "
                            if view.organizer.is_empty() {
                                "the organizer's"
                            } else {
                                bdi { "{view.organizer}" }
                                "'s"
                            }
                            " list is in the draw. Rather not? Tell the organizer before the draw."
                        }
                        if !privacy.is_empty() {
                            p { class: "text-[0.95em] text-base-content/70",
                                "Privacy: "
                                span { class: "rounded-lg bg-base-200 px-[0.4em] py-[0.1em] font-mono font-bold", "{privacy}" }
                            }
                        }
                    },
                    RaffleState::Open => rsx! {
                        div { class: "relative",
                            div { class: "absolute -inset-3 rounded-[2rem] bg-gradient-to-br from-primary via-secondary to-accent opacity-80 blur-xl" }
                            div {
                                class: "relative w-[min(26vw,38vh)] rotate-2 rounded-[1.6rem] bg-white p-[1.1vw] shadow-2xl transition hover:rotate-0",
                                div { class: "[&>svg]:h-auto [&>svg]:w-full", dangerous_inner_html: "{qr}" }
                            }
                        }
                        // People on a registration list the organizer added
                        // needn't scan: they are in.
                        if view.list_added {
                            p { class: "font-display text-[1.4em] font-extrabold", "Registered? You're already in." }
                            p { class: "font-display text-[1.6em] font-extrabold", "Not registered? Scan to join." }
                        } else {
                            p { class: "font-display text-[1.6em] font-extrabold", "Scan to join" }
                        }
                        p { class: "text-[1.05em]",
                            "or visit "
                            span { class: "rounded-lg bg-base-200 px-[0.4em] py-[0.1em] font-mono font-bold", "{short}" }
                        }
                    },
                    RaffleState::Closed => rsx! {
                        span { class: "text-[4em]", "🔒" }
                        p { class: "font-display text-[2em] font-extrabold", "Registration is closed." }
                    },
                    _ => rsx! {
                        span { class: "animate-float text-[5em]", "🎟️" }
                        p { class: "font-display text-[2em] font-extrabold", "Registration opens soon" }
                    },
                }
                div { class: "flex items-baseline gap-[0.4em]",
                    span { key: "{view.entrant_count}", class: "inline-block animate-pop font-display text-[2.6em] font-black leading-none text-primary", "{view.entrant_count}" }
                    span { class: "text-[1.2em] font-semibold text-base-content/60", if view.entrant_count == 1 { "entrant" } else { "entrants" } }
                }
                // The latest names, a long one cut with "…" (the organizer
                // gives up to 64 characters) so the others still fit.
                ul { class: "flex max-h-[11vh] max-w-full flex-wrap justify-center gap-[0.4em] overflow-hidden", aria_hidden: "true",
                    for (i , name) in view.recent_names.iter().enumerate() {
                        li { key: "{name}-{i}", class: "badge badge-lg animate-pop max-w-[min(100%,18em)] border-base-300 bg-base-200 font-semibold",
                            bdi { class: "truncate", "{name}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Drawing(view: RaffleView, ahead: Option<ParticipantView>, clock: ServerClock) -> Element {
    let now = use_clock(200);
    let left: u32 = view.prizes.iter().map(|p| p.left).sum();

    // The draw in progress and the latest winner: from the live channel's
    // participant view while it's ahead (the reveal comes first), otherwise
    // from the full view.
    let (current, winner) = match &ahead {
        Some(p) => (
            p.current
                .as_ref()
                .map(|c| (c.prize_name.clone(), c.round, c.round_time)),
            p.last.as_ref().map(|l| Reveal {
                display_name: l.display_name.clone(),
                prize_name: l.prize_name.clone(),
                round: None,
                no_phone: l.no_phone,
            }),
        ),
        None => (
            view.current
                .as_ref()
                .map(|c| (c.prize_name.clone(), c.round, c.round_time)),
            view.draws
                .last()
                .filter(|d| d.outcome == Outcome::Drawn || d.outcome == Outcome::Claimed)
                .map(|d| Reveal {
                    display_name: d.display_name.clone(),
                    prize_name: d.prize_name.clone(),
                    round: Some(d.round),
                    no_phone: d.no_phone,
                }),
        ),
    };

    let stage = if let Some((prize_name, round, round_time)) = current {
        let remaining_ms = (round_time as i64 * 1000) - clock.now(now());
        let secs = ((remaining_ms.max(0) + 999) / 1000).min(99);
        rsx! {
            div { class: "flex flex-col items-center gap-[0.8em] text-center", role: "status",
                p { class: "badge badge-primary badge-lg gap-2 font-bold uppercase tracking-widest", Dice5 { class: "size-[1.1em]" } "Drawing" }
                h2 { class: "text-[2.8em] font-extrabold leading-tight", "{prize_name}" }
                div { class: "bounce-dots flex gap-[0.5em]", aria_hidden: "true",
                    for i in 0..7 {
                        span { key: "{i}", style: "--i: {i}" }
                    }
                }
                if secs > 0 {
                    span { class: "countdown font-display text-[9em] font-black leading-none text-primary",
                        span { style: "--value:{secs};", aria_live: "polite", aria_label: "{secs}", "{secs}" }
                    }
                } else {
                    span { class: "loading loading-dots loading-lg text-primary" }
                }
                if view.last_failure.is_some() && view.current.is_some() {
                    p { class: "alert alert-warning alert-soft text-[0.8em]", "⏳ Still fetching drand round {round}" }
                }
                p { class: "text-[0.9em] text-base-content/60", "Locking in public randomness · drand round {round}" }
            }
        }
    } else if let Some(reveal) = winner {
        rsx! { Winner { reveal } }
    } else {
        rsx! {
            div { class: "flex flex-col items-center gap-[0.6em] text-center",
                span { class: "animate-float text-[5em]", "🎁" }
                p { class: "font-display text-[3em] font-black", "{left} " span { class: "text-base-content/60", if left == 1 { "prize left" } else { "prizes left" } } }
                p { class: "text-[1.2em] text-base-content/60", if view.entrant_count == 1 { "1 entrant" } else { "{view.entrant_count} entrants" } }
                if let Some(f) = &view.last_failure {
                    p { class: "alert alert-warning alert-soft text-[0.8em]", "⚠ {f}" }
                }
            }
        }
    };

    rsx! {
        // `min-w-0`: a long name wraps or is cut inside its column rather
        // than widening it past the screen.
        div { class: "grid flex-1 gap-[3vw] lg:grid-cols-[2fr_1fr]",
            div { class: "card grid min-w-0 place-items-center bg-base-200/60 p-[3vh] ring-1 ring-base-300",
                div { class: "w-full min-w-0", aria_live: "polite", {stage} }
            }
            aside { class: "min-w-0 space-y-[2em]",
                PrizeList { view: view.clone() }
                History { draws: view.draws.clone() }
            }
        }
    }
}

/// A winner on the big screen.
#[derive(Debug, Clone, PartialEq)]
struct Reveal {
    display_name: String,
    prize_name: String,
    /// The drand round, once the full view has it.
    round: Option<u64>,
    /// No phone tells them they won (the organizer added them): the screen
    /// calls them to the front.
    no_phone: bool,
}

#[component]
fn Winner(reveal: Reveal) -> Element {
    // Keyed by who and what, so the full view catching up with the live
    // channel doesn't replay the animation.
    let key = format!("{}→{}", reveal.display_name, reveal.prize_name);
    rsx! {
        div { class: "relative flex flex-col items-center gap-[0.4em] text-center", key: "{key}",
            div { class: "confetti", aria_hidden: "true",
                for i in 0..28 {
                    span { key: "{i}", style: "--i: {i}" }
                }
            }
            span { class: "badge badge-secondary badge-lg gap-2 font-bold", Trophy { class: "size-[1.1em]" } "{reveal.prize_name}" }
            h2 { class: "gradient-text animate-reveal max-w-full py-[0.1em] font-black leading-[1.05] text-balance wrap-anywhere {reveal_size(&reveal.display_name)}",
                bdi { "{reveal.display_name}" }
            }
            // Only under a name they can recognize: not under a hidden one
            // (someone who took themselves off, whose prize is declined).
            if reveal.no_phone && reveal.display_name != rafi_live_client::HIDDEN_NAME {
                p { class: "animate-pop font-display text-[2.2em] font-extrabold text-primary", "Come to the front!" }
            }
            if let Some(round) = reveal.round {
                p { class: "text-[0.9em] text-base-content/60",
                    "drand round {round}"
                }
            }
        }
    }
}

/// The winner reveal's font size for `name`: smaller the longer it is, so
/// a name the organizer added (up to 64 characters) takes about three lines
/// on the projector, while a short one fills the screen as before. Sized by
/// the viewport's width, as the screen is; the tiers keep roughly 10, 16, 22
/// and 28 characters a line in the stage's column. What doesn't fit wraps,
/// between words first. The classes are written out whole: Tailwind finds
/// them only so.
fn reveal_size(name: &str) -> &'static str {
    match name.chars().count() {
        0..=10 => "text-[clamp(3rem,9vw,11rem)]",
        11..=18 => "text-[clamp(2.5rem,6vw,7.5rem)]",
        19..=32 => "text-[clamp(2rem,4.5vw,5.5rem)]",
        _ => "text-[clamp(1.75rem,3.5vw,4.25rem)]",
    }
}

#[component]
fn History(draws: Vec<DrawView>) -> Element {
    if draws.is_empty() {
        return rsx! {};
    }
    rsx! {
        section { class: "space-y-[0.6em]",
            h2 { class: "text-[0.8em] font-bold uppercase tracking-[0.2em] text-base-content/50", "Winners" }
            ul { class: "space-y-[0.4em]",
                for d in draws.iter().rev() {
                    li { key: "{d.k}", class: "flex items-center gap-[0.5em]",
                        span { class: "text-[1.1em]",
                            match d.outcome {
                                Outcome::Claimed => "🏆",
                                Outcome::Drawn => "🎉",
                                _ => "👻",
                            }
                        }
                        bdi { class: if matches!(d.outcome, Outcome::NotHere | Outcome::Declined) { "min-w-0 truncate font-bold line-through opacity-50" } else { "min-w-0 truncate font-bold" }, "{d.display_name}" }
                        span { class: "truncate text-[0.85em] text-base-content/60", "{d.prize_name}" }
                        match d.outcome {
                            Outcome::NotHere => rsx! { span { class: "badge badge-ghost badge-sm", "not here" } },
                            Outcome::Declined => rsx! { span { class: "badge badge-ghost badge-sm", "declined" } },
                            _ => rsx! {},
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Finished(view: RaffleView) -> Element {
    let winners: Vec<DrawView> = view
        .draws
        .iter()
        .filter(|d| d.outcome == Outcome::Claimed)
        .cloned()
        .collect();
    // From a server before `verify_url`: the path alone.
    let (verify, qr) = if view.verify_url.is_empty() {
        (format!("/v/{}", view.slug), String::new())
    } else {
        (
            without_scheme(&view.verify_url).to_owned(),
            qr_svg(&view.verify_url),
        )
    };
    rsx! {
        div { class: "relative flex flex-1 flex-col items-center justify-center gap-[1.2em] text-center",
            div { class: "confetti", aria_hidden: "true",
                for i in 0..28 {
                    span { key: "{i}", style: "--i: {i}" }
                }
            }
            p { class: "text-[4em]", "🎉" }
            h2 { class: "gradient-text text-[3.5em] font-black leading-none", "Thanks for playing!" }
            if !winners.is_empty() {
                div { class: "flex max-w-[70vw] flex-wrap justify-center gap-[0.6em]",
                    for d in winners {
                        // A long name is cut with "…", so one winner doesn't
                        // push the others off the screen.
                        div { key: "{d.k}", class: "card max-w-[min(100%,24em)] bg-base-200 px-[1em] py-[0.6em] ring-1 ring-base-300",
                            p { class: "truncate font-display text-[1.2em] font-extrabold", "🏆 " bdi { "{d.display_name}" } }
                            p { class: "truncate text-[0.8em] text-base-content/60", "{d.prize_name}" }
                        }
                    }
                }
            }
            div { class: "flex items-center gap-[0.8em]",
                if !qr.is_empty() {
                    div { class: "w-[5em] shrink-0 rounded-xl bg-white p-[0.3em] [&>svg]:h-auto [&>svg]:w-full", aria_hidden: "true", dangerous_inner_html: "{qr}" }
                }
                p { class: "inline-flex flex-wrap items-center justify-center gap-[0.4em] text-[1em] text-base-content/70",
                    ShieldCheck { class: "size-[1.2em]" }
                    "Check the draw yourself:"
                    span { class: "rounded-lg bg-base-200 px-[0.4em] py-[0.1em] font-mono font-bold text-base-content", "{verify}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(state: &str, registration: bool) -> RaffleView {
        serde_json::from_value(serde_json::json!({
            "v": 1, "follow": { "kind": "poll", "every_ms": 2500 }, "raffle_id": "r1",
            "slug": "meetup", "title": "Meetup", "description": "", "state": state,
            "join_code": "ABCDEF", "join_url": "https://rafi.example/j/ABCDEF", "entrant_count": 2,
            "recent_names": ["Anna Kovács", "Bea"], "prizes": [], "items": [], "current": null,
            "draws": [], "last_failure": null, "manifest_hash": null, "server_now_ms": 0,
            "test_mode": false, "verifiable": true, "registration": registration,
            "organizer": "Cloud Native Budapest", "privacy_url": "https://rafi.example/privacy/meetup",
            "verify_url": "https://rafi.example/v/meetup"
        }))
        .unwrap()
    }

    /// The thank-you screen names the verification page with its host, as
    /// the intro does the join link, and shows its QR code.
    #[test]
    fn the_thanks_screen_says_where_to_check() {
        let html = dioxus_ssr::render_element(rsx! { Finished { view: view("finished", true) } });
        assert!(html.contains("rafi.example/v/meetup"), "{html}");
        assert!(
            !html.contains("https://rafi.example/v/"),
            "shown short: {html}"
        );
        let qr = "rounded-xl bg-white";
        assert!(html.contains(qr), "{html}");
        // A view from a server before `verify_url`: the path, no code.
        let mut old = view("finished", true);
        old.verify_url = String::new();
        let html = dioxus_ssr::render_element(rsx! { Finished { view: old } });
        assert!(html.contains("/v/meetup") && !html.contains(qr), "{html}");
    }

    fn trouble(kind: TroubleKind, for_ms: u64) -> Option<Trouble> {
        let first = Trouble::after(None, kind, 1_000);
        Some(Trouble::after(Some(first), kind, 1_000 + for_ms))
    }

    /// Before any view: loading; nothing at the slug; a page older than
    /// the server; the server out of reach for a while.
    #[test]
    fn what_the_screen_says_before_a_view() {
        let html = dioxus_ssr::render_element(rsx! { Waiting { trouble: None } });
        assert!(
            html.contains("loading-dots") && !html.contains(NOT_FOUND),
            "{html}"
        );
        let html = dioxus_ssr::render_element(
            rsx! { Waiting { trouble: trouble(TroubleKind::NotFound, 0) } },
        );
        assert!(
            html.contains("No raffle at this address, or it isn&#39;t published yet."),
            "{html}"
        );
        let html = dioxus_ssr::render_element(
            rsx! { Waiting { trouble: trouble(TroubleKind::Newer, 0) } },
        );
        assert!(html.contains("older than the server: reloading"), "{html}");
        let html = dioxus_ssr::render_element(
            rsx! { Waiting { trouble: trouble(TroubleKind::Unreachable, 0) } },
        );
        assert!(!html.contains("Retrying"), "not yet: {html}");
        let html = dioxus_ssr::render_element(
            rsx! { Waiting { trouble: trouble(TroubleKind::Unreachable, 10_000) } },
        );
        assert!(
            html.contains("Can&#39;t reach the server. Retrying…"),
            "{html}"
        );
    }

    /// Over a view: a badge once the view can't be refreshed for a while,
    /// or right away when the server is newer; none while all is well.
    #[test]
    fn the_screen_says_when_it_is_stale() {
        let render = |trouble| dioxus_ssr::render_element(rsx! { Connection { trouble } });
        assert!(!render(None).contains("<span"));
        assert!(!render(trouble(TroubleKind::Unreachable, 2_500)).contains("Reconnecting"));
        assert!(render(trouble(TroubleKind::Unreachable, 10_000)).contains("Reconnecting…"));
        assert!(render(trouble(TroubleKind::NotFound, 10_000)).contains("Reconnecting…"));
        assert!(render(trouble(TroubleKind::Newer, 0)).contains("reloading"));
    }

    /// Without registration the intro says whose list is in, how to stay
    /// out and where the notice is; with it, once people were added from a
    /// list, that they needn't scan.
    #[test]
    fn the_intro_tells_the_people_on_the_list() {
        let html = dioxus_ssr::render_element(rsx! { Intro { view: view("ready", false) } });
        for present in [
            "Everyone on <bdi>Cloud Native Budapest</bdi>&#39;s list is in the draw. Rather not? Tell the organizer before the draw.",
            "rafi.example/privacy/meetup",
        ] {
            assert!(html.contains(present), "{present}: {html}");
        }
        assert!(
            !html.contains("https://rafi.example/privacy"),
            "shown short: {html}"
        );
        let mut nameless = view("closed", false);
        nameless.organizer = String::new();
        let html = dioxus_ssr::render_element(rsx! { Intro { view: nameless } });
        assert!(
            html.contains("Everyone on the organizer&#39;s list is in the draw."),
            "{html}"
        );

        // With registration: as before until a list is added.
        let html = dioxus_ssr::render_element(rsx! { Intro { view: view("open", true) } });
        assert!(html.contains("Scan to join"), "{html}");
        for absent in ["Registered?", "Everyone on", "privacy/meetup"] {
            assert!(!html.contains(absent), "{absent}: {html}");
        }
        let mut listed = view("open", true);
        listed.list_added = true;
        let html = dioxus_ssr::render_element(rsx! { Intro { view: listed } });
        assert!(
            html.contains("Registered? You&#39;re already in.")
                && html.contains("Not registered? Scan to join."),
            "{html}"
        );
    }

    /// A winner with no phone to tell them is called to the front; one
    /// whose phone says so isn't.
    #[test]
    fn a_winner_without_a_phone_comes_to_the_front() {
        let reveal = |no_phone| Reveal {
            display_name: "Anna Kovács".into(),
            prize_name: "Book".into(),
            round: Some(7),
            no_phone,
        };
        let html = dioxus_ssr::render_element(rsx! { Winner { reveal: reveal(true) } });
        assert!(
            html.contains("Anna Kovács") && html.contains("Come to the front!"),
            "{html}"
        );
        let html = dioxus_ssr::render_element(rsx! { Winner { reveal: reveal(false) } });
        assert!(!html.contains("Come to the front!"), "{html}");
        // A hidden name (someone who took themselves off): nobody can tell
        // it's them.
        let hidden = Reveal {
            display_name: rafi_live_client::HIDDEN_NAME.into(),
            ..reveal(true)
        };
        let html = dioxus_ssr::render_element(rsx! { Winner { reveal: hidden } });
        assert!(!html.contains("Come to the front!"), "{html}");
    }

    /// Without registration the intro has the count and the names, and no
    /// way to join: no QR code, link, "No smartphone?" or word of
    /// registration opening.
    #[test]
    fn the_intro_without_registration() {
        for state in ["ready", "closed"] {
            let html = dioxus_ssr::render_element(rsx! { Intro { view: view(state, false) } });
            for absent in [
                "<svg",
                "rafi.example/j/ABCDEF",
                "No smartphone?",
                "Registration opens soon",
                "Registration is closed.",
                "Scan to join",
            ] {
                assert!(!html.contains(absent), "{state}, {absent}: {html}");
            }
            for present in ["The draw is coming up", "Anna Kovács", "entrants"] {
                assert!(html.contains(present), "{state}, {present}: {html}");
            }
        }
        // With it, as before.
        let html = dioxus_ssr::render_element(rsx! { Intro { view: view("ready", true) } });
        assert!(html.contains("Registration opens soon") && html.contains("No smartphone?"));
        let html = dioxus_ssr::render_element(rsx! { Intro { view: view("open", true) } });
        assert!(
            html.contains("<svg") && html.contains("rafi.example/j/ABCDEF"),
            "{html}"
        );
        // A view from before the setting: registration on.
        let mut old = serde_json::to_value(view("open", true)).unwrap();
        old.as_object_mut().unwrap().remove("registration");
        assert!(
            serde_json::from_value::<RaffleView>(old)
                .unwrap()
                .registration
        );
    }

    /// Longer names get smaller type, never larger; a joiner's short name
    /// keeps the full size, the organizer's longest the smallest.
    #[test]
    fn long_names_are_revealed_smaller() {
        let sizes: Vec<&str> = (1..=rafi_lifecycle::entrant::MAX_ADDED_NAME_CHARS)
            .map(|n| reveal_size(&"é".repeat(n)))
            .collect();
        let mut tiers = sizes.clone();
        tiers.dedup();
        assert_eq!(tiers.len(), 4, "{tiers:?}");
        assert_eq!(reveal_size("Anna"), "text-[clamp(3rem,9vw,11rem)]");
        assert_eq!(
            sizes.last().copied(),
            Some("text-[clamp(1.75rem,3.5vw,4.25rem)]")
        );
        assert_eq!(
            reveal_size(&"é".repeat(10)),
            reveal_size("x"),
            "counted in characters, not bytes"
        );
    }
}
