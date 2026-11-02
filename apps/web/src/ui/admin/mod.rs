//! Organizer admin: the raffle list, and one raffle's page with its tabs
//! (the draw console, prizes and people, each in its own module).

mod draw;
mod header;
mod list;
mod paragraph;
mod people;
mod prizes;

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ArrowLeft, CalendarClock, Dice5, FlaskConical, Gift, SlidersVertical, TriangleAlert, Trophy,
    UserX, Users,
};
use rafi_lifecycle::{RaffleState, entrant::Source};
use rafi_raffle::{ItemId, ItemStatus, Outcome, Resolution};

use super::{
    Route, command_id, copy_to_clipboard, dates,
    forms::{AddPersonForm, CreateRaffleForm, PrizeForm, SettingsForm, err_text},
    live::{AdminConnection, ServerClock, use_clock, use_live_view},
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

use self::{
    draw::DrawConsole, header::Header, paragraph::ParagraphToSend, people::People, prizes::Prizes,
};

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

/// A state's icon, at `class`'s size.
fn state_icon(s: RaffleState, class: &'static str) -> Element {
    use dioxus_icons::lucide::{
        BrushCleaning, FilePen, Lock, Megaphone, PartyPopper, Pause, Ticket,
    };
    match s {
        RaffleState::Draft => rsx! { FilePen { class } },
        RaffleState::Ready => rsx! { Megaphone { class } },
        RaffleState::Open => rsx! { Ticket { class } },
        RaffleState::Closed => rsx! { Pause { class } },
        RaffleState::Frozen => rsx! { Lock { class } },
        RaffleState::Drawing => rsx! { Dice5 { class } },
        RaffleState::Finished => rsx! { PartyPopper { class } },
        RaffleState::Purged => rsx! { BrushCleaning { class } },
    }
}

#[component]
fn StateBadge(state: RaffleState) -> Element {
    rsx! {
        span { class: "badge badge-soft {state_badge(state)} gap-1 font-semibold", {state_icon(state, "size-3.5")} "{state_label(state)}" }
    }
}

/// Shows errors reported by any action on this page as toasts.
#[component]
fn ErrorToasts(error: Signal<Option<String>>) -> Element {
    let toast = use_toast();
    use_effect(move || {
        if let Some(msg) = error() {
            // Stays until closed: an error with a reference is read out to
            // whoever runs the installation, and five seconds isn't enough.
            toast.error(
                "That didn't work".to_owned(),
                ToastOptions::new().description(msg).permanent(true),
            );
            error.set(None);
        }
    });
    rsx! {}
}

/// A confirmation for an action that cannot be undone. The title and the
/// body are markup, so a name in them is isolated (`bdi`) like everywhere
/// else; `note` comes after the body, in a paragraph of its own.
#[component]
fn Confirm(
    open: Signal<bool>,
    title: Element,
    body: Element,
    #[props(default = VNode::empty())] note: Element,
    action: String,
    #[props(default)] danger: bool,
    on_confirm: EventHandler<()>,
) -> Element {
    rsx! {
        AlertDialog {
            open: Some(open()),
            on_open_change: move |next| open.set(next),
            AlertDialogTitle { {title} }
            AlertDialogDescription { {body} }
            {note}
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
        document::Title { "Your raffles — Rafi" }
        main { class: "mx-auto max-w-6xl space-y-8 px-4 py-8",
            div { class: "space-y-2",
                h1 { class: "text-4xl font-extrabold", "Your raffles " span { class: "inline-block animate-float", "🎟️" } }
                p { class: "text-base-content/70", "Set one up before the meetup, open registration when people arrive, and draw at the end." }
            }
            if retention_stale {
                div { role: "alert", class: "alert alert-warning alert-soft",
                    TriangleAlert { class: "size-5" }
                    span { "Automatic data erasure seems to have stopped. Tell whoever runs this Rafi installation." }
                }
            }
            if access.allows(Access::Manager) {
                div { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                    div { class: "card-body",
                        CreateRaffleForm { on_created: move |id| { nav.push(Route::RaffleAdmin { id, tab: None }); } }
                    }
                }
            } else {
                p { class: "text-sm text-base-content/60", "You run your organization's raffles; an organization admin or manager creates them." }
            }
            match &*raffles.read() {
                None => rsx! { div { class: "grid place-items-center py-16", span { class: "loading loading-dots loading-lg text-primary" } } },
                Some(Err(e)) if e.code == api::codes::UNAUTHORIZED => rsx! { SignInAgain {} },
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
                        if let Some(guide) = crate::config::build().guide_url {
                            a { class: "link link-primary mt-2 inline-block text-sm", href: "{guide}", target: "_blank", rel: "noopener", "How an evening goes, step by step" }
                        }
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
                                to: Route::RaffleAdmin { id: r.id.clone(), tab: None },
                                div { class: "card-body gap-3",
                                    div { class: "flex items-start justify-between gap-2",
                                        span { class: "grid size-11 place-items-center rounded-2xl bg-primary/10 text-primary", {state_icon(r.state, "size-6")} }
                                        if r.test_mode { span { class: "badge badge-warning badge-sm gap-1", FlaskConical { class: "size-3" } "test" } }
                                    }
                                    h2 { class: "card-title text-lg leading-snug", "{r.title}" }
                                    div { class: "flex flex-wrap items-center gap-2 text-sm",
                                        StateBadge { state: r.state }
                                        span { class: "inline-flex items-center gap-1 text-base-content/60", Users { class: "size-4" } "{r.entrant_count}" }
                                        if let Some(day) = rafi_lifecycle::dates::Date::parse(&r.event_date) {
                                            span { class: "inline-flex items-center gap-1 text-base-content/60", CalendarClock { class: "size-4" } "{day.long()}" }
                                        }
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

/// What a page shows when the session ended: signing in again is reloading
/// the page, which shows Clerk's sign-in.
#[component]
fn SignInAgain() -> Element {
    rsx! {
        main { class: "mx-auto max-w-md px-4 py-20",
            div { class: "card bg-base-100 shadow-lg ring-1 ring-base-300",
                div { class: "card-body items-center text-center",
                    h2 { class: "card-title text-2xl", "Please sign in again" }
                    p { class: "text-base-content/70", "Your session ended. Nothing you did was lost." }
                    button { class: "btn btn-primary mt-2", onclick: move |_| { document::eval("location.reload()"); }, "Sign in again" }
                }
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
            title: rsx! { if shared { "Remove your organization's data from this service?" } else { "Remove your data from this service?" } },
            body: rsx! {
                if shared {
                    "Every raffle of the organization, for every co-host: those never frozen are deleted; every other one loses its entrants' personal data and the organizer's name and contacts, for good. The defaults are deleted. This cannot be undone."
                } else {
                    "Raffles of yours that were never frozen are deleted. Every other raffle loses its entrants' personal data and your name and contacts, for good. Your defaults are deleted. This cannot be undone."
                }
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
pub fn RaffleAdmin(id: String, tab: Option<String>) -> Element {
    let id_for_load = id.clone();
    let nav = navigator();
    // With the server's clock as of the reply, for the draw's countdown.
    let mut data = use_resource(move || {
        let id = id_for_load.clone();
        async move {
            api::get_raffle(id).await.map(|admin| {
                let clock = ServerClock::at(admin.view.server_now_ms, super::now_ms());
                (admin, clock)
            })
        }
    });
    let me = use_resource(api::whoami);
    let access = match &*me.read() {
        Some(Ok(me)) => me.access,
        _ => Access::Admin,
    };
    let error = use_signal(|| None::<String>);
    // The tab: from the URL, and written back to it (replacing, so Back
    // leaves the raffle rather than stepping through its tabs).
    let shown_tab = tab;
    let mut tab = use_signal(|| shown_tab.clone().filter(|t| !t.is_empty()));
    let route_id = id.clone();
    use_effect(move || {
        let current = tab();
        if current.is_some() {
            nav.replace(Route::RaffleAdmin {
                id: route_id.clone(),
                tab: current,
            });
        }
    });

    let read = data.read();
    let (admin, clock) = match &*read {
        None => {
            return rsx! { div { class: "grid place-items-center py-24", span { class: "loading loading-dots loading-lg text-primary" } } };
        }
        // The session ended (a long evening, another tab signed out): say
        // so, with the way back, rather than an error.
        Some(Err(e)) if e.code == api::codes::UNAUTHORIZED => {
            return rsx! { SignInAgain {} };
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
        Some(Ok((a, clock))) => (a.clone(), *clock),
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
                LiveRefresh { key: "{admin.view.slug}", slug: admin.view.slug.clone(), view: admin.view.clone(), on_change: move |_| data.restart() }
            }
            // Frozen just now: the draw is what comes next, whatever tab was open.
            Header { admin: admin.clone(), access, error, on_change: move |_| reload(), on_frozen: move |_| tab.set(Some("draw".to_owned())) }
            ErrorToasts { error }
            if matches!(state, RaffleState::Draft | RaffleState::Ready) {
                Checklist { admin: admin.clone(), on_tab: move |t: String| tab.set(Some(t)) }
            }
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
                            "draw" => rsx! { DrawConsole { admin: admin.clone(), clock, error, on_change: move |_| reload() } },
                            "prizes" => rsx! { Prizes { admin: admin.clone(), error, on_change: move |_| reload() } },
                            "people" => rsx! { People { admin: admin.clone(), error, on_change: move |_| reload() } },
                            _ => rsx! {
                                Panel { title: "Settings", icon: rsx! { SlidersVertical { class: "size-5" } },
                                    SettingsForm {
                                        raffle_id: admin.id.clone(),
                                        settings: admin.settings.clone(),
                                        rules_editable: admin.actions.edit_rules,
                                        registration_editable: admin.actions.edit_registration,
                                        retention: admin.retention.clone(),
                                        can_edit: access.allows(Access::Manager),
                                        after_privacy: rsx! { ParagraphToSend { admin: admin.clone() } },
                                        on_saved: move |_| reload(),
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// How long joins alone wait before the admin data is fetched again: a room
/// joining is a version a second, and each fetch is every entrant.
const JOINS_QUIET_MS: u32 = 10_000;

/// What makes the admin fetch its data again at once when the live view
/// moves: the state, the draws and their outcomes, the draw pending, a failure,
/// the channel ended. Anything else (people joining, names, another tab's
/// prize edit) waits for [`JOINS_QUIET_MS`].
#[derive(Clone, Debug, PartialEq)]
struct Moment {
    state: RaffleState,
    draws: Vec<(u64, Outcome)>,
    pending: Option<u64>,
    failure: bool,
    live_ended: bool,
}

impl Moment {
    fn of(v: &RaffleView) -> Self {
        Self {
            state: v.state,
            draws: v.draws.iter().map(|d| (d.k, d.outcome)).collect(),
            pending: v.current.as_ref().map(|c| c.k),
            failure: v.last_failure.is_some(),
            live_ended: v.live_ended,
        }
    }
}

/// Re-fetches admin data when the public live view moves past what the page
/// shows (`view`, the admin data's): at once for a moment of the evening
/// ([`Moment`]), at most every [`JOINS_QUIET_MS`] for joins.
#[component]
fn LiveRefresh(slug: String, view: RaffleView, on_change: EventHandler<()>) -> Element {
    let live = use_live_view(slug);
    // The header's "Offline" goes by this view's fetches too.
    let connection = try_use_context::<AdminConnection>();
    use_effect(move || {
        let trouble = *live.trouble.read();
        if let Some(AdminConnection(mut c)) = connection {
            c.set(trouble);
        }
    });
    use_drop(move || {
        if let Some(AdminConnection(mut c)) = connection {
            c.set(None);
        }
    });
    // What the page shows: its version and moment, kept up with the admin
    // data, however it was fetched (an action of the organizer's reloads it too).
    let mut seen = use_signal(|| (view.v, Moment::of(&view)));
    use_effect(use_reactive((&view,), move |(view,)| {
        if view.v >= seen.peek().0 {
            seen.set((view.v, Moment::of(&view)));
        }
    }));
    let mut waiting = use_signal(|| false);
    use_effect(move || {
        let Some(v) = live.view.read().clone() else {
            return;
        };
        let (shown, moment) = seen.peek().clone();
        if v.v <= shown {
            return;
        }
        if Moment::of(&v) != moment {
            seen.set((v.v, Moment::of(&v)));
            on_change.call(());
        } else if !*waiting.peek() {
            waiting.set(true);
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(JOINS_QUIET_MS).await;
                waiting.set(false);
                let newer = live
                    .view
                    .peek()
                    .as_ref()
                    .is_some_and(|v| v.v > seen.peek().0);
                if newer {
                    on_change.call(());
                }
            });
        }
    });
    live.subscription()
}

/// Before doors open: what is left to do, from what the server says (the
/// prizes, what the settings lack, the state), each a step with where to do
/// it. Shown in Draft and Published.
#[component]
fn Checklist(admin: AdminData, on_tab: EventHandler<String>) -> Element {
    let prizes: u32 = admin.prizes.iter().map(|p| p.qty).sum();
    let missing = admin.actions.missing.clone();
    let published = admin.view.state != RaffleState::Draft;
    let registration = admin.settings.registration;
    let step = |done: bool| if done { "step step-primary" } else { "step" };
    rsx! {
        section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
            div { class: "card-body gap-3",
                h2 { class: "card-title text-lg", "Before doors open" }
                ul { class: "steps steps-vertical",
                    li { class: step(prizes > 0),
                        div { class: "flex flex-wrap items-center gap-2 text-left",
                            if prizes > 0 {
                                span { if prizes == 1 { "1 prize to give away" } else { "{prizes} prizes to give away" } }
                            } else {
                                span { "Add a prize" }
                            }
                            button { class: "btn btn-ghost btn-xs", onclick: move |_| on_tab.call("prizes".into()), "Prizes" }
                        }
                    }
                    li { class: step(missing.is_empty()),
                        div { class: "flex flex-wrap items-center gap-2 text-left",
                            if missing.is_empty() {
                                span { "The organizer and a contact, for the privacy notice" }
                            } else {
                                span { "Fill in {missing.join(\" and \")}: the privacy notice names who runs the raffle and how to reach them" }
                            }
                            button { class: "btn btn-ghost btn-xs", onclick: move |_| on_tab.call("settings".into()), "Settings" }
                        }
                    }
                    li { class: step(admin.settings.test_mode),
                        div { class: "flex flex-wrap items-center gap-2 text-left",
                            span { "Optional: rehearse with Test mode (Settings), fake entrants and a draw, then duplicate the raffle for the evening" }
                        }
                    }
                    li { class: step(published),
                        span { class: "text-left",
                            if !published {
                                "Publish the intro screen (above), and put the projector screen up"
                            } else if registration {
                                "Published: open registration when people arrive"
                            } else {
                                "Published: freeze the list when everyone is in"
                            }
                        }
                    }
                }
            }
        }
    }
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
                div { class: "stat-value", "{left}" }
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
                    p { class: "text-warning", TriangleAlert { class: "mr-1 inline size-4 align-text-bottom" } "The erasure date (end of {erase}) has passed and the list was never frozen. The raffle expires within the hour: its entrants are deleted. If the event was postponed, move the event date (possible until someone joins) or duplicate the raffle." }
                } else if r.expires_unfrozen {
                    p { class: "text-warning", TriangleAlert { class: "mr-1 inline size-4 align-text-bottom" } "The event day has passed and the list was never frozen. The raffle expires at the end of {erase}: its entrants are deleted. Postponed? Move the event date while nobody has joined, or duplicate the raffle." }
                } else if r.past_erase_date {
                    p { "Non-winners' data was erased at the end of {erase}." }
                } else {
                    p { "Entrants' names and emails are erased at the end of " strong { "{erase}" } ", whatever else happens to this raffle. You can erase earlier from the menu once it has finished." }
                }
                if r.held_winners > 0 {
                    p { if r.held_winners == 1 { "1 winner's data is kept" } else { "{r.held_winners} winners' data is kept" } " until you mark their prize delivered, at the latest until the end of " strong { "{winners}" } ". Everything delivered? Use “All prizes delivered” in the menu." }
                }
                if r.unfinished_past_event {
                    p { class: "text-warning", TriangleAlert { class: "mr-1 inline size-4 align-text-bottom" } "Not finished, and the event day has passed. Entrants' data is erased on the date regardless; finish the raffle so its data can be erased for good." }
                }
                if r.erasure_imminent {
                    p { class: "text-warning", TriangleAlert { class: "mr-1 inline size-4 align-text-bottom" } "Erasure is at the end of {erase}, less than a day away: nobody can join or be added after it, and everyone who didn't win is erased on the hour after. An evening that runs past midnight needs the list frozen and the draws done before it." }
                }
            }
        }
    }
}

/// Where the raffle is in its life. Without registration there is no
/// registration step: it goes from Published to the draw.
#[component]
fn Steps(state: RaffleState, registration: bool) -> Element {
    let (steps, at): (&[&str], usize) = if registration {
        (
            &["Prepare", "Publish", "Registration", "Draw", "Done"],
            match state {
                RaffleState::Draft => 0,
                RaffleState::Ready => 1,
                RaffleState::Open | RaffleState::Closed => 2,
                RaffleState::Frozen | RaffleState::Drawing => 3,
                RaffleState::Finished | RaffleState::Purged => 4,
            },
        )
    } else {
        (
            &["Prepare", "Publish", "Draw", "Done"],
            match state {
                RaffleState::Draft => 0,
                // Closed, then switched off: published, the draw next.
                RaffleState::Ready | RaffleState::Open | RaffleState::Closed => 1,
                RaffleState::Frozen | RaffleState::Drawing => 2,
                RaffleState::Finished | RaffleState::Purged => 3,
            },
        )
    };
    rsx! {
        ul { class: "steps steps-horizontal w-full text-xs",
            for (i , label) in steps.iter().enumerate() {
                li { key: "{i}", class: if i <= at { "step step-primary" } else { "step" }, "{label}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(prizes: serde_json::Value, missing: serde_json::Value) -> AdminData {
        serde_json::from_value(serde_json::json!({
            "id": "r1",
            "settings": {
                "title": "Meetup", "description": "", "organizer_name": "", "contact_email": "",
                "rules": "", "event_date": "2026-11-05", "test_mode": false
            },
            "prizes": prizes,
            "entrants": [],
            "view": {
                "v": 1, "follow": { "kind": "poll", "every_ms": 2500 }, "raffle_id": "r1",
                "slug": "meetup", "title": "Meetup", "description": "", "state": "draft",
                "join_code": "ABCD", "join_url": "", "entrant_count": 0,
                "recent_names": [], "prizes": [], "items": [], "current": null, "draws": [],
                "last_failure": null, "manifest_hash": null, "server_now_ms": 0,
                "test_mode": false, "verifiable": true
            },
            "public_url": "https://rafi.example",
            "actions": { "start": null, "draw": false, "export": false, "purge": false, "missing": missing }
        }))
        .unwrap()
    }

    /// A new raffle's path: a prize, the organizer and a contact, an
    /// optional rehearsal, publishing; what's done is marked, what the
    /// settings lack named.
    #[test]
    fn a_draft_has_a_checklist() {
        #[component]
        fn Shown(admin: AdminData) -> Element {
            rsx! { Checklist { admin, on_tab: |_| {} } }
        }
        let html = dioxus_ssr::render_element(rsx! {
            Shown { admin: draft(serde_json::json!([]), serde_json::json!(["the organizer's name"])) }
        });
        for expected in [
            "Before doors open",
            "Add a prize",
            "Fill in the organizer&#39;s name",
            "Publish the intro screen",
        ] {
            assert!(html.contains(expected), "{expected}: {html}");
        }
        assert!(!html.contains("step-primary"), "nothing done yet: {html}");
        let prize = serde_json::json!([{
            "id": "p1", "name": "Book", "sponsor": "", "description": "", "qty": 2, "position": 0,
            "delivery": "later"
        }]);
        let html = dioxus_ssr::render_element(rsx! {
            Shown { admin: draft(prize, serde_json::json!([])) }
        });
        assert!(html.contains("2 prizes to give away"), "{html}");
        assert_eq!(html.matches("step step-primary").count(), 2, "{html}");
    }

    fn view(entrants: usize, draws: serde_json::Value, current: serde_json::Value) -> RaffleView {
        serde_json::from_value(serde_json::json!({
            "v": 1, "follow": { "kind": "poll", "every_ms": 2500 }, "raffle_id": "r1",
            "slug": "meetup", "title": "Meetup", "description": "", "state": "open",
            "join_code": "ABCDEF", "join_url": "", "entrant_count": entrants,
            "recent_names": [], "prizes": [], "items": [], "current": current,
            "draws": draws, "last_failure": null, "manifest_hash": null, "server_now_ms": 0,
            "test_mode": false, "verifiable": true
        }))
        .unwrap()
    }

    /// People joining is no moment: the admin waits before fetching them;
    /// a draw pending, or one decided, is.
    #[test]
    fn joins_are_not_a_moment() {
        let none = serde_json::json!([]);
        let null = serde_json::Value::Null;
        let quiet = Moment::of(&view(2, none.clone(), null.clone()));
        assert_eq!(quiet, Moment::of(&view(40, none.clone(), null.clone())));
        let pending = serde_json::json!({ "k": 1, "item": "i1", "prize_name": "Book", "round": 7, "round_time": 0 });
        assert_ne!(quiet, Moment::of(&view(2, none, pending)));
        let draw = |outcome: &str| {
            serde_json::json!([{
                "k": 1, "item": "i1", "prize_name": "Book", "entrant": "e1", "display_name": "Anna",
                "outcome": outcome, "pass": 1, "round": 7
            }])
        };
        let drawn = Moment::of(&view(2, draw("drawn"), null.clone()));
        assert_ne!(quiet, drawn);
        assert_ne!(drawn, Moment::of(&view(2, draw("claimed"), null)));
    }
}
