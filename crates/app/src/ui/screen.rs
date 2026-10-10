//! The projector screen: public and read-only.

use dioxus::prelude::*;
use rafi_lifecycle::RaffleState;
use rafi_raffle::Outcome;

use rafi_live::ParticipantView;

use super::{
    Brand, ThemeToggle,
    live::{use_clock, use_live_view},
};
use crate::{
    i18n::Lang,
    icons::{I, Icon},
    model::{DrawView, RaffleView},
};

#[component]
pub fn Screen(slug: String) -> Element {
    rsx! {
        ScreenInner { key: "{slug}", slug: slug.clone() }
    }
}

#[component]
fn ScreenInner(slug: String) -> Element {
    let live = use_live_view(slug.clone());
    let guard = live.view.read();
    let Some(v) = guard.as_ref() else {
        return rsx! {
            main { class: "grid min-h-screen place-items-center bg-base-100 bg-dots",
                div { class: "flex flex-col items-center gap-4",
                    Brand { size: "text-4xl" }
                    span { class: "loading loading-dots loading-lg text-primary" }
                }
            }
        };
    };
    let lang = Lang::parse(&v.lang);
    let v = v.clone();
    drop(guard);
    let ahead = live.ahead_of(&v);

    let body = match v.state {
        RaffleState::Draft => {
            rsx! { p { class: "text-3xl font-bold text-base-content/60", "{lang.t(\"not_found\")}" } }
        }
        RaffleState::Ready | RaffleState::Open | RaffleState::Closed => {
            rsx! { Intro { view: v.clone() } }
        }
        RaffleState::Frozen | RaffleState::Drawing => rsx! { Drawing { view: v.clone(), ahead } },
        RaffleState::Finished | RaffleState::Purged => rsx! { Finished { view: v.clone() } },
    };

    rsx! {
        document::Title { "{v.title} — Rafi" }
        main { class: "flex min-h-screen flex-col gap-[2vh] bg-base-100 bg-dots px-[3vw] py-[2.5vh] text-[clamp(16px,1.5vw,34px)]",
            header { class: "flex items-center gap-4",
                h1 { class: "flex-1 text-[2.2em] font-extrabold leading-tight", "{v.title}" }
                if v.test_mode {
                    span { class: "badge badge-warning badge-lg font-bold", "🧪 {lang.t(\"test_mode\")}" }
                }
                div { class: "flex items-center gap-1 opacity-40 transition hover:opacity-100",
                    ThemeToggle {}
                    FullscreenButton {}
                }
            }
            {body}
            {live.feed()}
            footer { class: "flex items-center justify-between text-[0.7em] text-base-content/50",
                Brand { size: "text-[1.1em]" }
                span { class: "inline-flex items-center gap-1", Icon { i: I::ShieldCheck, class: "size-[1.2em]" } "drand · provably fair" }
            }
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
            Icon { i: I::Maximize, class: "size-5" }
        }
    }
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
    let lang = Lang::parse(&view.lang);
    rsx! {
        section { class: "space-y-[0.6em]",
            h2 { class: "text-[0.8em] font-bold uppercase tracking-[0.2em] text-base-content/50", "{lang.t(\"prizes\")}" }
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

#[component]
fn Intro(view: RaffleView) -> Element {
    let lang = Lang::parse(&view.lang);
    let t = |k: &str| lang.t(k);
    let qr = qr_svg(&view.join_url);
    let short = view
        .join_url
        .split("://")
        .nth(1)
        .unwrap_or(&view.join_url)
        .to_owned();
    rsx! {
        div { class: "grid flex-1 items-center gap-[3vw] lg:grid-cols-[1.15fr_1fr]",
            div { class: "space-y-[1.4em]",
                if !view.description.is_empty() {
                    p { class: "max-w-[34em] text-[1.25em] text-base-content/75", "{view.description}" }
                }
                PrizeList { view: view.clone() }
                div { class: "inline-flex items-center gap-[0.6em] rounded-box bg-accent px-[1em] py-[0.6em] font-bold text-accent-content -rotate-1 shadow-sm",
                    span { class: "text-[1.4em]", "📵" }
                    "{t(\"no_phone\")}"
                }
            }
            div { class: "flex flex-col items-center gap-[0.8em] text-center",
                match view.state {
                    RaffleState::Open => rsx! {
                        div { class: "relative",
                            div { class: "absolute -inset-3 rounded-[2rem] bg-gradient-to-br from-primary via-secondary to-accent opacity-80 blur-xl" }
                            div {
                                class: "relative w-[min(26vw,38vh)] rotate-2 rounded-[1.6rem] bg-white p-[1.1vw] shadow-2xl transition hover:rotate-0",
                                div { class: "[&>svg]:h-auto [&>svg]:w-full", dangerous_inner_html: "{qr}" }
                            }
                        }
                        p { class: "font-display text-[1.6em] font-extrabold", "{t(\"scan_to_join\")}" }
                        p { class: "text-[1.05em]",
                            "{t(\"or_visit\")} "
                            span { class: "rounded-lg bg-base-200 px-[0.4em] py-[0.1em] font-mono font-bold", "{short}" }
                        }
                    },
                    RaffleState::Closed => rsx! {
                        span { class: "text-[4em]", "🔒" }
                        p { class: "font-display text-[2em] font-extrabold", "{t(\"reg_closed\")}" }
                    },
                    _ => rsx! {
                        span { class: "animate-float text-[5em]", "🎟️" }
                        p { class: "font-display text-[2em] font-extrabold", "{t(\"registration_soon\")}" }
                    },
                }
                div { class: "flex items-baseline gap-[0.4em]",
                    span { key: "{view.entrant_count}", class: "inline-block animate-pop font-display text-[2.6em] font-black leading-none text-primary", "{view.entrant_count}" }
                    span { class: "text-[1.2em] font-semibold text-base-content/60", "{t(\"entrants\")}" }
                }
                ul { class: "flex max-h-[11vh] flex-wrap justify-center gap-[0.4em] overflow-hidden", aria_hidden: "true",
                    for (i , name) in view.recent_names.iter().enumerate() {
                        li { key: "{name}-{i}", class: "badge badge-lg animate-pop border-base-300 bg-base-200 font-semibold", "{name}" }
                    }
                }
            }
        }
    }
}

#[component]
fn Drawing(view: RaffleView, ahead: Option<ParticipantView>) -> Element {
    let lang = Lang::parse(&view.lang);
    let t = |k: &str| lang.t(k);
    let now = use_clock(200);
    let left: u32 = view.prizes.iter().map(|p| p.left).sum();
    let offset = view.server_now_ms as i64 - crate::model::now_unix_ms() as i64;

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
                }),
        ),
    };

    let stage = if let Some((prize_name, round, round_time)) = current {
        let remaining_ms = (round_time as i64 * 1000) - (now() as i64 + offset);
        let secs = ((remaining_ms.max(0) + 999) / 1000).min(99);
        rsx! {
            div { class: "flex flex-col items-center gap-[0.8em] text-center", role: "status",
                p { class: "badge badge-primary badge-lg gap-2 font-bold uppercase tracking-widest", Icon { i: I::Dice, class: "size-[1.1em]" } "{t(\"drawing\")}" }
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
                    p { class: "alert alert-warning alert-soft text-[0.8em]", "⏳ {t(\"retrying_round\")} {round}" }
                }
                p { class: "text-[0.9em] text-base-content/60", "{t(\"locking\")} · {t(\"drand_round\")} {round}" }
            }
        }
    } else if let Some(reveal) = winner {
        rsx! { Winner { reveal, lang: lang.code().to_owned() } }
    } else {
        rsx! {
            div { class: "flex flex-col items-center gap-[0.6em] text-center",
                span { class: "animate-float text-[5em]", "🎁" }
                p { class: "font-display text-[3em] font-black", "{left} " span { class: "text-base-content/60", "{t(\"prizes_left\")}" } }
                p { class: "text-[1.2em] text-base-content/60", "{view.entrant_count} {t(\"entrants\")}" }
                if let Some(f) = &view.last_failure {
                    p { class: "alert alert-warning alert-soft text-[0.8em]", "⚠ {f}" }
                }
            }
        }
    };

    rsx! {
        div { class: "grid flex-1 gap-[3vw] lg:grid-cols-[2fr_1fr]",
            div { class: "card grid place-items-center bg-base-200/60 p-[3vh] ring-1 ring-base-300",
                div { class: "w-full", aria_live: "polite", {stage} }
            }
            aside { class: "space-y-[2em]",
                PrizeList { view: view.clone() }
                History { draws: view.draws.clone(), lang: lang.code().to_owned() }
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
}

#[component]
fn Winner(reveal: Reveal, lang: String) -> Element {
    let lang = Lang::parse(&lang);
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
            span { class: "badge badge-secondary badge-lg gap-2 font-bold", Icon { i: I::Trophy, class: "size-[1.1em]" } "{reveal.prize_name}" }
            h2 { class: "gradient-text animate-reveal py-[0.1em] text-[clamp(3rem,9vw,11rem)] font-black leading-none", "{reveal.display_name}" }
            if let Some(round) = reveal.round {
                p { class: "text-[0.9em] text-base-content/60",
                    "{lang.t(\"drand_round\")} {round}"
                }
            }
        }
    }
}

#[component]
fn History(draws: Vec<DrawView>, lang: String) -> Element {
    let lang = Lang::parse(&lang);
    if draws.is_empty() {
        return rsx! {};
    }
    rsx! {
        section { class: "space-y-[0.6em]",
            h2 { class: "text-[0.8em] font-bold uppercase tracking-[0.2em] text-base-content/50", "{lang.t(\"winners\")}" }
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
                        span { class: if matches!(d.outcome, Outcome::NotHere | Outcome::Declined) { "font-bold line-through opacity-50" } else { "font-bold" }, "{d.display_name}" }
                        span { class: "truncate text-[0.85em] text-base-content/60", "{d.prize_name}" }
                        match d.outcome {
                            Outcome::NotHere => rsx! { span { class: "badge badge-ghost badge-sm", "{lang.t(\"not_here\")}" } },
                            Outcome::Declined => rsx! { span { class: "badge badge-ghost badge-sm", "{lang.t(\"declined\")}" } },
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
    let lang = Lang::parse(&view.lang);
    let t = |k: &str| lang.t(k);
    let winners: Vec<DrawView> = view
        .draws
        .iter()
        .filter(|d| d.outcome == Outcome::Claimed)
        .cloned()
        .collect();
    rsx! {
        div { class: "relative flex flex-1 flex-col items-center justify-center gap-[1.2em] text-center",
            div { class: "confetti", aria_hidden: "true",
                for i in 0..28 {
                    span { key: "{i}", style: "--i: {i}" }
                }
            }
            p { class: "text-[4em]", "🎉" }
            h2 { class: "gradient-text text-[3.5em] font-black leading-none", "{t(\"thanks\")}" }
            if !winners.is_empty() {
                div { class: "flex max-w-[70vw] flex-wrap justify-center gap-[0.6em]",
                    for d in winners {
                        div { key: "{d.k}", class: "card bg-base-200 px-[1em] py-[0.6em] ring-1 ring-base-300",
                            p { class: "font-display text-[1.2em] font-extrabold", "🏆 {d.display_name}" }
                            p { class: "text-[0.8em] text-base-content/60", "{d.prize_name}" }
                        }
                    }
                }
            }
            p { class: "inline-flex items-center gap-2 text-[0.9em] text-base-content/60",
                Icon { i: I::ShieldCheck, class: "size-[1.2em]" }
                "{t(\"verify_link\")}: /v/{view.slug}"
            }
        }
    }
}
