//! Organizer admin: the raffle list, and one raffle's page with its tabs
//! (the draw console, prizes and people, each in its own module).

mod draw;
mod header;
mod people;
mod prizes;

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_icons::lucide::{ArrowLeft, Dice5, Gift, SlidersVertical, TriangleAlert, Trophy, Users};
use rafi_lifecycle::{RaffleState, entrant::Source};
use rafi_raffle::{ItemId, ItemStatus, Outcome, Resolution};

use super::{
    Route, command_id, copy_to_clipboard,
    forms::{AddEntrantForm, CreateRaffleForm, PrizeForm, SettingsForm, err_text},
    live::{use_clock, use_live_view},
};
use crate::{
    api,
    components::{
        alert_dialog::{
            AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogButtonColor,
            AlertDialogCancel, AlertDialogDescription, AlertDialogTitle,
        },
        button::{Button, ButtonColor, ButtonSize},
        dropdown_menu::{
            DropdownMenu, DropdownMenuAlign, DropdownMenuContent, DropdownMenuItem,
            DropdownMenuTrigger,
        },
        tabs::{TabContent, TabContentAppearance, TabTrigger, Tabs, TabsAppearance},
        toast::{ToastOptions, use_toast},
    },
};
use rafi_api::{
    AdminEntrant, EntrantPatch, LifecycleCommand, Prize, RaffleAdmin as AdminData, RaffleView,
    StartStep,
};

use self::{draw::DrawConsole, header::Header, people::People, prizes::Prizes};

fn state_label(s: RaffleState) -> &'static str {
    match s {
        RaffleState::Draft => "Draft",
        RaffleState::Ready => "Published",
        RaffleState::Open => "Registration open",
        RaffleState::Closed => "Registration closed",
        RaffleState::Frozen => "Ready to draw",
        RaffleState::Drawing => "Drawing",
        RaffleState::Finished => "Finished",
        RaffleState::Purged => "Data erased",
    }
}

fn state_badge(s: RaffleState) -> &'static str {
    match s {
        RaffleState::Draft => "badge-ghost",
        RaffleState::Ready => "badge-info",
        RaffleState::Open => "badge-success",
        RaffleState::Closed => "badge-warning",
        RaffleState::Frozen | RaffleState::Drawing => "badge-primary",
        RaffleState::Finished => "badge-secondary",
        RaffleState::Purged => "badge-neutral",
    }
}

fn state_emoji(s: RaffleState) -> &'static str {
    match s {
        RaffleState::Draft => "📝",
        RaffleState::Ready => "📣",
        RaffleState::Open => "🎟️",
        RaffleState::Closed => "⏸️",
        RaffleState::Frozen => "🔒",
        RaffleState::Drawing => "🎲",
        RaffleState::Finished => "🎉",
        RaffleState::Purged => "🧹",
    }
}

#[component]
fn StateBadge(state: RaffleState) -> Element {
    rsx! {
        span { class: "badge badge-soft {state_badge(state)} gap-1 font-semibold", "{state_emoji(state)} {state_label(state)}" }
    }
}

/// Shows errors reported by any action on this page as toasts.
#[component]
fn ErrorToasts(error: Signal<Option<String>>) -> Element {
    let toast = use_toast();
    use_effect(move || {
        if let Some(msg) = error() {
            toast.error(
                "That didn't work".to_owned(),
                ToastOptions::new().description(msg),
            );
            error.set(None);
        }
    });
    rsx! {}
}

/// A confirmation for an action that cannot be undone.
#[component]
fn Confirm(
    open: Signal<bool>,
    title: String,
    body: String,
    action: String,
    #[props(default)] danger: bool,
    on_confirm: EventHandler<()>,
) -> Element {
    rsx! {
        AlertDialog {
            open: Some(open()),
            on_open_change: move |next| open.set(next),
            AlertDialogTitle { "{title}" }
            AlertDialogDescription { "{body}" }
            AlertDialogActions {
                AlertDialogCancel { "Cancel" }
                AlertDialogAction {
                    color: if danger { AlertDialogButtonColor::Error } else { AlertDialogButtonColor::Primary },
                    on_click: move |_| on_confirm.call(()),
                    "{action}"
                }
            }
        }
    }
}

#[component]
fn Panel(
    title: String,
    icon: Element,
    #[props(default = VNode::empty())] actions: Element,
    children: Element,
) -> Element {
    rsx! {
        section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
            div { class: "card-body gap-4",
                div { class: "flex flex-wrap items-center gap-3",
                    span { class: "grid size-9 place-items-center rounded-xl bg-primary/10 text-primary", {icon} }
                    h2 { class: "card-title flex-1 text-xl", "{title}" }
                    {actions}
                }
                {children}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Raffle list
// ---------------------------------------------------------------------------

#[component]
pub fn AdminHome() -> Element {
    let mut raffles = use_resource(api::list_raffles);
    let me = use_resource(api::whoami);
    let nav = navigator();
    let retention_stale = matches!(&*me.read(), Some(Ok(me)) if me.retention_stale);

    rsx! {
        main { class: "mx-auto max-w-6xl space-y-8 px-4 py-8",
            div { class: "space-y-2",
                h1 { class: "text-4xl font-extrabold", "Your raffles " span { class: "inline-block animate-float", "🎟️" } }
                p { class: "text-base-content/70", "Set one up before the meetup, open registration when people arrive, and draw at the end." }
            }
            if retention_stale {
                div { role: "alert", class: "alert alert-warning alert-soft",
                    TriangleAlert { class: "size-5" }
                    span { "Automatic erasure of personal data hasn't run in the last two hours. Check the scheduled job (the Cron Trigger on Workers)." }
                }
            }
            div { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                div { class: "card-body",
                    CreateRaffleForm { on_created: move |id| { nav.push(Route::RaffleAdmin { id }); } }
                }
            }
            match &*raffles.read() {
                None => rsx! { div { class: "grid place-items-center py-16", span { class: "loading loading-dots loading-lg text-primary" } } },
                Some(Err(e)) => rsx! {
                    div { role: "alert", class: "alert alert-error alert-soft",
                        TriangleAlert { class: "size-5" }
                        span { "{err_text(e.clone())}" }
                        button { class: "btn btn-sm", onclick: move |_| raffles.restart(), "Retry" }
                    }
                },
                Some(Ok(list)) if list.is_empty() => rsx! {
                    div { class: "rounded-box border-2 border-dashed border-base-300 py-16 text-center",
                        p { class: "text-5xl", "🎁" }
                        p { class: "mt-3 font-display text-xl font-bold", "No raffles yet" }
                        p { class: "text-base-content/60", "Create your first one above." }
                    }
                },
                Some(Ok(list)) => rsx! {
                    div { class: "grid gap-4 sm:grid-cols-2 lg:grid-cols-3",
                        for r in list.iter() {
                            Link {
                                key: "{r.id}",
                                class: "card bg-base-100 shadow-sm ring-1 ring-base-300 transition hover:-translate-y-1 hover:shadow-lg hover:ring-primary/40",
                                to: Route::RaffleAdmin { id: r.id.clone() },
                                div { class: "card-body gap-3",
                                    div { class: "flex items-start justify-between gap-2",
                                        span { class: "text-3xl", "{state_emoji(r.state)}" }
                                        if r.test_mode { span { class: "badge badge-warning badge-sm", "test" } }
                                    }
                                    h2 { class: "card-title text-lg leading-snug", "{r.title}" }
                                    div { class: "flex flex-wrap items-center gap-2 text-sm",
                                        StateBadge { state: r.state }
                                        span { class: "inline-flex items-center gap-1 text-base-content/60", Users { class: "size-4" } "{r.entrant_count}" }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// One raffle
// ---------------------------------------------------------------------------

#[component]
pub fn RaffleAdmin(id: String) -> Element {
    let id_for_load = id.clone();
    let mut data = use_resource(move || {
        let id = id_for_load.clone();
        async move { api::get_raffle(id).await }
    });
    let error = use_signal(|| None::<String>);
    let mut tab = use_signal(|| None::<String>);

    let read = data.read();
    let admin = match &*read {
        None => {
            return rsx! { div { class: "grid place-items-center py-24", span { class: "loading loading-dots loading-lg text-primary" } } };
        }
        Some(Err(e)) => {
            let msg = err_text(e.clone());
            return rsx! {
                main { class: "mx-auto max-w-6xl space-y-4 px-4 py-8",
                    div { role: "alert", class: "alert alert-error alert-soft", TriangleAlert { class: "size-5" } span { "{msg}" } }
                    Link { class: "btn btn-ghost", to: Route::AdminHome {}, ArrowLeft { class: "size-5" } "All raffles" }
                }
            };
        }
        Some(Ok(a)) => a.clone(),
    };
    drop(read);
    let mut reload = move || data.restart();

    let state = admin.view.state;
    // The draw console, once the list is frozen (the server says).
    let drawable = admin.actions.draw;
    let default_tab = if drawable { "draw" } else { "prizes" };
    let current = tab().unwrap_or_else(|| default_tab.to_owned());
    let mut tabs: Vec<(&'static str, &'static str, Element)> = Vec::new();
    if drawable {
        tabs.push(("draw", "Draw", rsx! { Dice5 { class: "size-4" } }));
    }
    tabs.extend([
        ("prizes", "Prizes", rsx! { Gift { class: "size-4" } }),
        ("people", "People", rsx! { Users { class: "size-4" } }),
        (
            "settings",
            "Settings",
            rsx! { SlidersVertical { class: "size-4" } },
        ),
    ]);

    rsx! {
        document::Title { "{admin.settings.title} — Rafi" }
        main { class: "mx-auto max-w-6xl space-y-6 px-4 py-6",
            Link { class: "btn btn-ghost btn-sm -ml-2", to: Route::AdminHome {}, ArrowLeft { class: "size-4" } "All raffles" }
            if state.is_public() {
                LiveRefresh { key: "{admin.view.slug}", slug: admin.view.slug.clone(), version: admin.view.v, on_change: move |_| data.restart() }
            }
            Header { admin: admin.clone(), error, on_change: move |_| reload() }
            ErrorToasts { error }
            Stats { admin: admin.clone() }
            Tabs {
                key: "{drawable}",
                appearance: TabsAppearance::Box,
                class: "w-full gap-y-4",
                value: Some(current.clone()),
                on_value_change: move |value| tab.set(Some(value)),
                for (index , (value , label , icon)) in tabs.into_iter().enumerate() {
                    TabTrigger { key: "t-{value}", value: value.to_owned(), index, class: "gap-2",
                        {icon}
                        "{label}"
                    }
                    TabContent { value: value.to_owned(), index, appearance: TabContentAppearance::None, class: "w-full",
                        match value {
                            "draw" => rsx! { DrawConsole { admin: admin.clone(), error, on_change: move |_| reload() } },
                            "prizes" => rsx! { Prizes { admin: admin.clone(), error, on_change: move |_| reload() } },
                            "people" => rsx! { People { admin: admin.clone(), error, on_change: move |_| reload() } },
                            _ => rsx! {
                                Panel { title: "Settings", icon: rsx! { SlidersVertical { class: "size-5" } },
                                    SettingsForm { raffle_id: admin.id.clone(), settings: admin.settings.clone(), rules_editable: admin.actions.edit_rules, on_saved: move |_| reload() }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// Re-fetches admin data whenever the public live view moves past `version`.
#[component]
fn LiveRefresh(slug: String, version: u64, on_change: EventHandler<()>) -> Element {
    let live = use_live_view(slug);
    use_effect(move || {
        if let Some(v) = live.view.read().as_ref()
            && v.v > version
        {
            on_change.call(());
        }
    });
    live.subscription()
}

#[component]
fn Stats(admin: AdminData) -> Element {
    let v = &admin.view;
    let total: u32 = v.prizes.iter().map(|p| p.qty).sum();
    let left: u32 = v.prizes.iter().map(|p| p.left).sum();
    let manual = admin
        .entrants
        .iter()
        .filter(|e| e.source == Source::Manual)
        .count();
    let winners = v
        .draws
        .iter()
        .filter(|d| d.outcome == Outcome::Claimed)
        .count();
    rsx! {
        div { class: "stats stats-vertical w-full bg-base-100 shadow-sm ring-1 ring-base-300 sm:stats-horizontal",
            div { class: "stat",
                div { class: "stat-figure text-primary", Users { class: "size-8" } }
                div { class: "stat-title", "Entrants" }
                div { class: "stat-value text-primary", "{v.entrant_count}" }
                div { class: "stat-desc", "{manual} added by you" }
            }
            div { class: "stat",
                div { class: "stat-figure text-secondary", Gift { class: "size-8" } }
                div { class: "stat-title", "Prizes left" }
                div { class: "stat-value text-secondary", "{left}" }
                div { class: "stat-desc", "of {total}" }
            }
            div { class: "stat",
                div { class: "stat-figure text-accent", Trophy { class: "size-8" } }
                div { class: "stat-title", "Winners" }
                div { class: "stat-value", "{winners}" }
                div { class: "stat-desc", "{v.draws.len()} draws so far" }
            }
        }
    }
}

#[component]
fn Steps(state: RaffleState) -> Element {
    let at = match state {
        RaffleState::Draft => 0,
        RaffleState::Ready => 1,
        RaffleState::Open | RaffleState::Closed => 2,
        RaffleState::Frozen | RaffleState::Drawing => 3,
        RaffleState::Finished | RaffleState::Purged => 4,
    };
    let steps = ["Prepare", "Publish", "Registration", "Draw", "Done"];
    rsx! {
        ul { class: "steps steps-horizontal w-full text-xs",
            for (i , label) in steps.iter().enumerate() {
                li { key: "{i}", class: if i <= at { "step step-primary" } else { "step" }, "{label}" }
            }
        }
    }
}
