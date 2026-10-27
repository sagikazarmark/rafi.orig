//! Organizer admin: the raffle list, and one raffle's page with its tabs
//! (the draw console, prizes and people, each in its own module).

mod draw;
mod header;
mod people;
mod prizes;

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ArrowLeft, CalendarClock, Dice5, Gift, SlidersVertical, TriangleAlert, Trophy, UserX, Users,
};
use rafi_lifecycle::{RaffleState, entrant::Source};
use rafi_raffle::{ItemId, ItemStatus, Outcome, Resolution};

use super::{
    Route, command_id, copy_to_clipboard, dates,
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
        switch::Switch,
        tabs::{TabContent, TabContentAppearance, TabTrigger, Tabs, TabsAppearance},
        toast::{ToastOptions, use_toast},
    },
};
use rafi_api::{
    Access, AdminEntrant, EntrantPatch, LifecycleCommand, Prize, RaffleAdmin as AdminData,
    RaffleView, StartStep,
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
    // Raffles whose personal data is erased stay (their draws are
    // verifiable) and would otherwise fill the list for good.
    let mut show_erased = use_signal(|| false);
    let (retention_stale, access, shared) = match &*me.read() {
        Some(Ok(me)) => (me.retention_stale, me.access, me.shared),
        // Until we know, assume the best: the server refuses anyway.
        _ => (false, Access::Admin, false),
    };

    rsx! {
        main { class: "mx-auto max-w-6xl space-y-8 px-4 py-8",
            div { class: "space-y-2",
                h1 { class: "text-4xl font-extrabold", "Your raffles " span { class: "inline-block animate-float", "🎟️" } }
                p { class: "text-base-content/70", "Set one up before the meetup, open registration when people arrive, and draw at the end." }
            }
            if retention_stale {
                div { role: "alert", class: "alert alert-warning alert-soft",
                    TriangleAlert { class: "size-5" }
                    span { "Automatic erasure of personal data hasn't run in the last two hours. Entrants were promised erasure dates; check the scheduled job (the Cron Trigger on Workers)." }
                }
            }
            if access.allows(Access::Manager) {
                div { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                    div { class: "card-body",
                        CreateRaffleForm { on_created: move |id| { nav.push(Route::RaffleAdmin { id }); } }
                    }
                }
            } else {
                p { class: "text-sm text-base-content/60", "You run your organization's raffles; an organization admin or manager creates them." }
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
                Some(Ok(list)) => {
                    let erased = list.iter().filter(|r| r.state == RaffleState::Purged).count();
                    let shown: Vec<_> = list
                        .iter()
                        .filter(|r| show_erased() || r.state != RaffleState::Purged)
                        .collect();
                    rsx! {
                    if erased > 0 {
                        label { class: "flex items-center gap-2 text-sm text-base-content/60",
                            Switch { value: Some(show_erased()), on_change: move |on| show_erased.set(on) }
                            if erased == 1 { "Show 1 raffle whose data is erased" } else { "Show {erased} raffles whose data is erased" }
                        }
                    }
                    div { class: "grid gap-4 sm:grid-cols-2 lg:grid-cols-3",
                        for r in shown.into_iter() {
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
                    }
                }
            }
            if access.allows(Access::Admin) {
                YourData { shared, on_removed: move |_| { raffles.restart(); } }
            }
        }
    }
}

/// Removing the organizer's data from the service: a person's own, or an
/// organization's (every co-host's raffles), by one of its admins.
#[component]
fn YourData(shared: bool, on_removed: EventHandler<()>) -> Element {
    let mut ask = use_signal(|| false);
    let toast = use_toast();
    let (whose, title) = if shared {
        ("your organization's", "Your organization's data")
    } else {
        ("your", "Your data")
    };
    rsx! {
        details { class: "collapse collapse-arrow bg-base-100 ring-1 ring-base-300",
            summary { class: "collapse-title text-sm font-semibold", "{title}" }
            div { class: "collapse-content space-y-3 text-sm text-base-content/70",
                p { "This service keeps the organizer name and contacts on {whose} raffles, {whose} defaults, and the audit log of what was done. Entrants' data is erased on the dates their raffles show; it can always be erased earlier from a raffle's menu. By using the service you accept its " a { class: "link", href: "/terms", target: "_blank", "terms" } ", which include the data processing agreement." }
                if shared {
                    p { "Leaving? Remove the organization's data here, for every co-host: raffles that were never frozen are deleted, finished ones lose their entrants' data and the organizer's name, and the defaults go. The anonymous draws stay verifiable. Then delete the organization in your account settings." }
                } else {
                    p { "Leaving? Remove your data here: raffles that were never frozen are deleted, finished ones lose their entrants' data and your name, and your defaults go. The anonymous draws stay verifiable. Then delete your account in your account settings." }
                }
                Button { class: "btn-outline btn-error btn-sm", onclick: move |_| ask.set(true), UserX { class: "size-4" } if shared { "Remove the organization's data" } else { "Remove my data" } }
            }
        }
        Confirm {
            open: ask,
            title: if shared { "Remove your organization's data from this service?" } else { "Remove your data from this service?" },
            body: if shared {
                "Every raffle of the organization, for every co-host: those never frozen are deleted; every other one loses its entrants' personal data and the organizer's name and contacts, for good. The defaults are deleted. This cannot be undone."
            } else {
                "Raffles of yours that were never frozen are deleted. Every other raffle loses its entrants' personal data and your name and contacts, for good. Your defaults are deleted. This cannot be undone."
            },
            action: if shared { "Remove the organization's data" } else { "Remove my data" },
            danger: true,
            on_confirm: move |_| {
                spawn(async move {
                    match api::remove_my_data().await {
                        Ok(()) => {
                            toast.success("Your data was removed".to_owned(), ToastOptions::new().description("Delete your account in your account settings to finish."));
                            on_removed.call(());
                        }
                        Err(e) => toast.error(err_text(e), ToastOptions::new()),
                    }
                });
            },
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
    let me = use_resource(api::whoami);
    let access = match &*me.read() {
        Some(Ok(me)) => me.access,
        _ => Access::Admin,
    };
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
            Header { admin: admin.clone(), access, error, on_change: move |_| reload() }
            ErrorToasts { error }
            Stats { admin: admin.clone() }
            RetentionCard { admin: admin.clone() }
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
                                    SettingsForm { raffle_id: admin.id.clone(), settings: admin.settings.clone(), rules_editable: admin.actions.edit_rules, retention: admin.retention.clone(), can_edit: access.allows(Access::Manager), on_saved: move |_| reload() }
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

/// When the raffle's personal data is erased, and what is still held: the
/// server's dates and warnings, the page derives none.
#[component]
fn RetentionCard(admin: AdminData) -> Element {
    let r = &admin.retention;
    let (erase, winners) = (&r.erase_date, &r.winners_erase_date);
    rsx! {
        div { class: "flex flex-wrap items-start gap-3 rounded-box bg-base-100 p-4 text-sm shadow-sm ring-1 ring-base-300",
            CalendarClock { class: "size-5 shrink-0 text-primary" }
            div { class: "space-y-1",
                if admin.view.state == RaffleState::Purged && admin.view.manifest_hash.is_none() {
                    p { "Expired: the list was never frozen, so nothing is kept. Duplicate the raffle to run it again." }
                } else if admin.view.state == RaffleState::Purged {
                    p { "Personal data erased. The anonymous draw stays verifiable." }
                } else if r.expires_unfrozen && r.past_erase_date {
                    p { class: "text-warning", "⚠ The erasure date (end of {erase}) has passed and the list was never frozen. The raffle expires within the hour: its entrants are deleted. If the event was postponed, move the event date (possible until someone joins) or duplicate the raffle." }
                } else if r.expires_unfrozen {
                    p { class: "text-warning", "⚠ The event day has passed and the list was never frozen. The raffle expires at the end of {erase}: its entrants are deleted. Postponed? Move the event date while nobody has joined, or duplicate the raffle." }
                } else if r.past_erase_date {
                    p { "Non-winners' data was erased at the end of {erase}." }
                } else {
                    p { "Entrants' names and emails are erased at the end of " strong { "{erase}" } ", whatever else happens to this raffle. You can erase earlier from the menu once it has finished." }
                }
                if r.held_winners > 0 {
                    p { "{r.held_winners} winner(s)' data is kept until you mark their prize delivered, at the latest until the end of " strong { "{winners}" } ". Everything handed over? Use “All prizes delivered” in the menu." }
                }
                if r.unfinished_past_event {
                    p { class: "text-warning", "⚠ Not finished, and the event day has passed. Entrants' data is erased on the date regardless; finish the raffle so its data can be erased for good." }
                }
                if r.erasure_imminent {
                    p { class: "text-warning", "⚠ Erasure is at the end of {erase}, less than a day away: registration closes then, and everyone who didn't win is erased on the hour after. An evening that runs past midnight needs the list frozen and the draws done before it." }
                }
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
