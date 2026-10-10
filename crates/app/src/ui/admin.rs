//! Organizer admin.

use std::collections::HashMap;

use dioxus::prelude::*;
use rafi_drawing::{ItemId, ItemStatus, Outcome, Resolution};
use rafi_raffle::RaffleState;

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
    i18n::Lang,
    icons::{I, Icon},
    model::{
        AdminEntrant, EntrantPatch, EntrantSource, Lifecycle, Prize, RaffleAdmin as AdminData,
        RaffleView,
    },
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
    icon: I,
    #[props(default = VNode::empty())] actions: Element,
    children: Element,
) -> Element {
    rsx! {
        section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
            div { class: "card-body gap-4",
                div { class: "flex flex-wrap items-center gap-3",
                    span { class: "grid size-9 place-items-center rounded-xl bg-primary/10 text-primary", Icon { i: icon, class: "size-5" } }
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
    let retention_stale = matches!(&*me.read(), Some(Ok(me)) if me.retention_stale((crate::model::now_unix_ms() / 1000) as i64));

    rsx! {
        main { class: "mx-auto max-w-6xl space-y-8 px-4 py-8",
            div { class: "space-y-2",
                h1 { class: "text-4xl font-extrabold", "Your raffles " span { class: "inline-block animate-float", "🎟️" } }
                p { class: "text-base-content/70", "Set one up before the meetup, open registration when people arrive, and draw at the end." }
            }
            if retention_stale {
                div { role: "alert", class: "alert alert-warning alert-soft",
                    Icon { i: I::Alert }
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
                        Icon { i: I::Alert }
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
                                        span { class: "inline-flex items-center gap-1 text-base-content/60", Icon { i: I::Users, class: "size-4" } "{r.entrant_count}" }
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
                    div { role: "alert", class: "alert alert-error alert-soft", Icon { i: I::Alert } span { "{msg}" } }
                    Link { class: "btn btn-ghost", to: Route::AdminHome {}, Icon { i: I::ArrowLeft } "All raffles" }
                }
            };
        }
        Some(Ok(a)) => a.clone(),
    };
    drop(read);
    let mut reload = move || data.restart();

    let state = admin.view.state;
    let drawable = matches!(
        state,
        RaffleState::Frozen | RaffleState::Drawing | RaffleState::Finished | RaffleState::Purged
    );
    let default_tab = if drawable { "draw" } else { "prizes" };
    let current = tab().unwrap_or_else(|| default_tab.to_owned());
    let mut tabs: Vec<(&'static str, &'static str, I)> = Vec::new();
    if drawable {
        tabs.push(("draw", "Draw", I::Dice));
    }
    tabs.extend([
        ("prizes", "Prizes", I::Gift),
        ("people", "People", I::Users),
        ("settings", "Settings", I::Sliders),
    ]);

    rsx! {
        document::Title { "{admin.settings.title} — Rafi" }
        main { class: "mx-auto max-w-6xl space-y-6 px-4 py-6",
            Link { class: "btn btn-ghost btn-sm -ml-2", to: Route::AdminHome {}, Icon { i: I::ArrowLeft, class: "size-4" } "All raffles" }
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
                        Icon { i: icon, class: "size-4" }
                        "{label}"
                    }
                    TabContent { value: value.to_owned(), index, appearance: TabContentAppearance::None, class: "w-full",
                        match value {
                            "draw" => rsx! { DrawConsole { admin: admin.clone(), error, on_change: move |_| reload() } },
                            "prizes" => rsx! { Prizes { admin: admin.clone(), error, on_change: move |_| reload() } },
                            "people" => rsx! { People { admin: admin.clone(), error, on_change: move |_| reload() } },
                            _ => rsx! {
                                Panel { title: "Settings", icon: I::Sliders,
                                    SettingsForm { raffle_id: admin.id.clone(), settings: admin.settings.clone(), on_saved: move |_| reload() }
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
    live.feed()
}

#[component]
fn Stats(admin: AdminData) -> Element {
    let v = &admin.view;
    let total: u32 = v.prizes.iter().map(|p| p.qty).sum();
    let left: u32 = v.prizes.iter().map(|p| p.left).sum();
    let manual = admin
        .entrants
        .iter()
        .filter(|e| e.source == EntrantSource::Manual)
        .count();
    let winners = v
        .draws
        .iter()
        .filter(|d| d.outcome == Outcome::Claimed)
        .count();
    rsx! {
        div { class: "stats stats-vertical w-full bg-base-100 shadow-sm ring-1 ring-base-300 sm:stats-horizontal",
            div { class: "stat",
                div { class: "stat-figure text-primary", Icon { i: I::Users, class: "size-8" } }
                div { class: "stat-title", "Entrants" }
                div { class: "stat-value text-primary", "{v.entrant_count}" }
                div { class: "stat-desc", "{manual} added by you" }
            }
            div { class: "stat",
                div { class: "stat-figure text-secondary", Icon { i: I::Gift, class: "size-8" } }
                div { class: "stat-title", "Prizes left" }
                div { class: "stat-value text-secondary", "{left}" }
                div { class: "stat-desc", "of {total}" }
            }
            div { class: "stat",
                div { class: "stat-figure text-accent", Icon { i: I::Trophy, class: "size-8" } }
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

#[component]
fn Header(admin: AdminData, error: Signal<Option<String>>, on_change: EventHandler<()>) -> Element {
    let v = admin.view.clone();
    let mut ask_delete = use_signal(|| false);
    let mut ask_purge = use_signal(|| false);
    let mut ask_freeze = use_signal(|| false);
    let mut ask_end = use_signal(|| false);
    let mut busy = use_signal(|| false);
    // The admin is in English; these strings also have their Hungarian
    // versions in i18n.
    let t = |k: &str| Lang::En.t(k);
    let nav = navigator();
    let toast = use_toast();
    let screen_url = format!("{}/s/{}", admin.public_url, v.slug);
    let verify_url = format!("{}/v/{}", admin.public_url, v.slug);

    let id = admin.id.clone();
    let act = move |action: Lifecycle| {
        let id = id.clone();
        async move {
            busy.set(true);
            match api::lifecycle(id, action).await {
                Ok(()) => on_change.call(()),
                Err(e) => error.set(Some(err_text(e))),
            }
            busy.set(false);
        }
    };
    let id = admin.id.clone();
    let start = move || {
        let id = id.clone();
        async move {
            busy.set(true);
            match api::start_draw(id).await {
                Ok(()) => {
                    toast.success(
                        "🔒 List frozen — let's draw!".to_owned(),
                        ToastOptions::new(),
                    );
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
            busy.set(false);
        }
    };
    let id = admin.id.clone();
    let duplicate = move || {
        let id = id.clone();
        async move {
            match api::duplicate_raffle(id).await {
                Ok(new_id) => {
                    toast.success(
                        "Duplicated".to_owned(),
                        ToastOptions::new()
                            .description("Prizes and settings were copied; entrants were not."),
                    );
                    nav.push(Route::RaffleAdmin { id: new_id });
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let delete = move || {
        let id = id.clone();
        async move {
            match api::delete_raffle(id).await {
                Ok(()) => {
                    toast.info("Raffle deleted".to_owned(), ToastOptions::new());
                    nav.push(Route::AdminHome {});
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let export = move || {
        let id = id.clone();
        async move {
            match api::export_winners(id).await {
                Ok(csv) => download("winners.csv", &csv),
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let purge = move || {
        let id = id.clone();
        async move {
            match api::purge_raffle(id).await {
                Ok(()) => {
                    toast.success("🧹 Personal data erased".to_owned(), ToastOptions::new());
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let end_live = move || {
        let id = id.clone();
        async move {
            busy.set(true);
            match api::end_live_updates(id).await {
                Ok(()) => {
                    toast.success(t("live_ended").to_owned(), ToastOptions::new());
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
            busy.set(false);
        }
    };
    let unconfirmed = v.state == RaffleState::Frozen
        && v.manifest_hash.is_some()
        && v.draws.is_empty()
        && v.current.is_none();
    let join_url = v.join_url.clone();
    let can_export = !matches!(
        v.state,
        RaffleState::Draft | RaffleState::Ready | RaffleState::Open | RaffleState::Closed
    );
    let can_purge = v.state == RaffleState::Finished;

    let (dup, exp) = (duplicate.clone(), export.clone());
    let on_menu = use_callback(move |value: String| match value.as_str() {
        "duplicate" => {
            spawn(dup());
        }
        "export" => {
            spawn(exp());
        }
        "purge" => ask_purge.set(true),
        "delete" => ask_delete.set(true),
        _ => {}
    });
    let mut menu: Vec<(&'static str, &'static str, I)> = vec![("duplicate", "Duplicate", I::Copy)];
    if can_export {
        menu.push(("export", "Export winners (CSV)", I::Download));
    }
    if can_purge {
        menu.push(("purge", "Erase personal data now", I::Trash));
    }
    menu.push(("delete", "Delete raffle", I::Trash));

    rsx! {
        section { class: "card overflow-hidden bg-base-100 shadow-sm ring-1 ring-base-300",
            div { class: "h-1.5 bg-gradient-to-r from-primary via-secondary to-accent" }
            div { class: "card-body gap-5",
                div { class: "flex flex-wrap items-start gap-3",
                    div { class: "min-w-0 flex-1 space-y-2",
                        h1 { class: "text-3xl font-extrabold leading-tight", "{v.title}" }
                        div { class: "flex flex-wrap items-center gap-2",
                            StateBadge { state: v.state }
                            if v.test_mode { span { class: "badge badge-warning gap-1 font-semibold", "🧪 test mode" } }
                            if !v.verifiable { span { class: "badge badge-warning gap-1", title: "Draws use the mock drand chain: nobody can verify them", Icon { i: I::Zap, class: "size-3.5" } "mock drand" } }
                        }
                    }
                    DropdownMenu { align: DropdownMenuAlign::End,
                        DropdownMenuTrigger { class: "btn-ghost btn-circle", aria_label: "More actions", Icon { i: I::More } }
                        DropdownMenuContent { class: "z-20 w-60",
                            for (index , (value , label , icon)) in menu.into_iter().enumerate() {
                                DropdownMenuItem::<String> {
                                    key: "{value}",
                                    value: value.to_owned(),
                                    index,
                                    on_select: on_menu,
                                    class: if matches!(value, "purge" | "delete") { "text-error" } else { "" },
                                    Icon { i: icon, class: "size-4" }
                                    "{label}"
                                }
                            }
                        }
                    }
                }
                Steps { state: v.state }
                if v.state.is_public() {
                    div { class: "flex flex-wrap items-center gap-2",
                        a { class: "btn btn-neutral btn-sm", href: "{screen_url}", target: "_blank", Icon { i: I::Monitor, class: "size-4" } "Projector screen" }
                        div { class: "join",
                            span { class: "join-item inline-flex h-8 items-center gap-1.5 border border-base-300 bg-base-200 px-3 font-mono text-sm",
                                Icon { i: I::QrCode, class: "size-4 opacity-60" }
                                "{v.join_url}"
                            }
                            Button {
                                size: ButtonSize::Sm,
                                class: "join-item",
                                title: "Copy the join link",
                                onclick: move |_| {
                                    copy_to_clipboard(join_url.clone());
                                    toast.info("📋 Join link copied".to_owned(), ToastOptions::new());
                                },
                                Icon { i: I::Copy, class: "size-4" }
                            }
                        }
                        span { class: "badge badge-lg badge-accent font-mono font-bold tracking-widest", "{v.join_code}" }
                        if v.manifest_hash.is_some() {
                            a { class: "btn btn-ghost btn-sm", href: "{verify_url}", target: "_blank", Icon { i: I::ShieldCheck, class: "size-4" } "Verification page" }
                        }
                    }
                }
                div { class: "flex flex-wrap items-center gap-2",
                    match v.state {
                        RaffleState::Draft => rsx! {
                            Button { color: ButtonColor::Primary, class: "hover-wiggle", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(Lifecycle::Publish)); } },
                                Icon { i: I::Play } "Publish the intro screen"
                            }
                        },
                        RaffleState::Ready => rsx! {
                            Button { color: ButtonColor::Success, class: "hover-wiggle", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(Lifecycle::Open)); } }, Icon { i: I::Ticket } "Open registration" }
                            Button { class: "btn-ghost", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(Lifecycle::Unpublish)); } }, Icon { i: I::Undo } "Unpublish" }
                        },
                        RaffleState::Open => rsx! {
                            Button { color: ButtonColor::Primary, class: "hover-wiggle", disabled: busy(), onclick: move |_| ask_freeze.set(true), Icon { i: I::Lock } "Freeze list & start the draw" }
                            Button { class: "btn-ghost", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(Lifecycle::Close)); } }, "Close registration" }
                        },
                        RaffleState::Closed => rsx! {
                            Button { color: ButtonColor::Primary, class: "hover-wiggle", disabled: busy(), onclick: move |_| ask_freeze.set(true), Icon { i: I::Lock } "Freeze list & start the draw" }
                            Button { class: "btn-ghost", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(Lifecycle::Open)); } }, "Reopen registration" }
                        },
                        RaffleState::Frozen if unconfirmed => rsx! {
                            Button { color: ButtonColor::Warning, disabled: busy(), onclick: { let start = start.clone(); move |_| { spawn(start()); } }, Icon { i: I::Undo } "Retry handing the list to the drawing engine" }
                        },
                        RaffleState::Finished | RaffleState::Purged if v.live_ended => rsx! {
                            span { class: "badge badge-ghost gap-1", Icon { i: I::Check, class: "size-3.5" } "{t(\"live_ended\")}" }
                        },
                        RaffleState::Finished | RaffleState::Purged => rsx! {
                            Button { class: "btn-outline", disabled: busy(), onclick: move |_| ask_end.set(true), Icon { i: I::Flag } "{t(\"end_live\")}" }
                        },
                        _ => rsx! {},
                    }
                    if busy() { span { class: "loading loading-dots loading-md text-primary" } }
                }
            }
        }
        Confirm {
            open: ask_freeze,
            title: "Freeze the list and start the draw?",
            body: format!(
                "Registration closes for good and the list of {} entrants is published. Latecomers can't be added afterwards.",
                v.entrant_count
            ),
            action: "🔒 Freeze & start",
            on_confirm: move |_| { spawn(start()); },
        }
        Confirm {
            open: ask_end,
            title: t("end_live_title"),
            body: t("end_live_body"),
            action: t("end_live"),
            on_confirm: move |_| { spawn(end_live()); },
        }
        Confirm {
            open: ask_purge,
            title: "Erase all personal data now?",
            body: "Names, emails and claim codes of every entrant are erased. The anonymous draw log stays verifiable. This cannot be undone.",
            action: "Erase personal data",
            danger: true,
            on_confirm: move |_| { spawn(purge()); },
        }
        Confirm {
            open: ask_delete,
            title: "Delete this raffle?",
            body: format!("“{}”, its prizes, entrants and draw log are deleted for good.", v.title),
            action: "Delete raffle",
            danger: true,
            on_confirm: move |_| { spawn(delete()); },
        }
    }
}

fn download(name: &str, content: &str) {
    let js = format!(
        "(() => {{ const b = new Blob([{}], {{type: 'text/csv;charset=utf-8'}}); const a = document.createElement('a'); \
         a.href = URL.createObjectURL(b); a.download = {}; document.body.appendChild(a); a.click(); a.remove(); }})()",
        serde_json::to_string(content).unwrap_or_default(),
        serde_json::to_string(name).unwrap_or_default()
    );
    document::eval(&js);
}

// ---------------------------------------------------------------------------
// Draw console
// ---------------------------------------------------------------------------

#[component]
fn DrawConsole(
    admin: AdminData,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let v: RaffleView = admin.view.clone();
    let now = use_clock(250);
    let by_id: HashMap<String, AdminEntrant> = admin
        .entrants
        .iter()
        .map(|e| (e.id.clone(), e.clone()))
        .collect();
    let pending = v.current.clone();
    let busy = pending.is_some();
    let drawable = v.state.drawable();
    let mut ask_finish = use_signal(|| false);
    let mut ask_abandon = use_signal(|| false);
    let toast = use_toast();

    let id = admin.id.clone();
    let draw = move |item: ItemId| {
        let id = id.clone();
        async move {
            match api::draw_item(id, item, command_id()).await {
                Ok(()) => {
                    error.set(None);
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let resolve = move |item: ItemId, resolution: Resolution| {
        let id = id.clone();
        async move {
            match api::resolve_item(id, item, resolution, command_id()).await {
                Ok(()) => {
                    error.set(None);
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let void = move |item: ItemId| {
        let id = id.clone();
        async move {
            match api::void_item(id, item, command_id()).await {
                Ok(()) => on_change.call(()),
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let abandon = move |item: ItemId| {
        let id = id.clone();
        async move {
            match api::abandon_draw(id, item, command_id()).await {
                Ok(()) => on_change.call(()),
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let id = admin.id.clone();
    let finish = move || {
        let id = id.clone();
        async move {
            match api::finish_raffle(id, command_id()).await {
                Ok(()) => {
                    toast.success(
                        "🎉 That's a wrap!".to_owned(),
                        ToastOptions::new().description("Thanks for running a fair raffle."),
                    );
                    on_change.call(())
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let items_left = v
        .items
        .iter()
        .filter(|u| matches!(u.status, ItemStatus::Available | ItemStatus::Drawn))
        .count();
    let pending_item = pending.as_ref().map(|p| p.item.clone());
    let offset = v.server_now_ms as i64 - crate::model::now_unix_ms() as i64;
    // A pending draw whose beacon could not be fetched (or whose fetch was lost,
    // e.g. a restart): it stays on its round; the organizer can retry or
    // abandon it.
    let overdue = pending
        .as_ref()
        .is_some_and(|c| now() as i64 + offset > c.round_time as i64 * 1000 + 90_000);
    let stalled = pending.is_some() && (v.last_failure.is_some() || overdue);

    rsx! {
        div { class: "space-y-4",
            if let Some(cur) = &pending {
                {
                    let remaining_ms = (cur.round_time as i64 * 1000 - now() as i64 - offset).max(0);
                    let secs = (remaining_ms + 999) / 1000;
                    let pct = (100 - (remaining_ms / 60).min(100)).max(5);
                    rsx! {
                        div { class: "card bg-primary text-primary-content shadow-lg animate-pop",
                            div { class: "card-body flex-row items-center gap-5",
                                div { class: "radial-progress bg-primary-content/15 font-mono text-2xl font-black", style: "--value:{pct}; --size:5rem; --thickness:6px;", role: "progressbar",
                                    if secs > 0 { "{secs}" } else { span { class: "loading loading-dots loading-sm" } }
                                }
                                div { class: "space-y-1",
                                    p { class: "text-sm font-semibold uppercase tracking-widest opacity-80", if stalled { "Retrying round {cur.round}" } else { "Drawing now" } }
                                    p { class: "font-display text-2xl font-extrabold", "{cur.prize_name}" }
                                    p { class: "text-sm opacity-80", "Waiting for public drand round {cur.round}, committed before it exists." }
                                }
                            }
                        }
                    }
                }
            }
            if let (true, Some(cur), Some(u)) = (stalled, &pending, pending_item.clone()) {
                div { role: "alert", class: "alert alert-warning alert-soft items-start",
                    Icon { i: I::Alert }
                    div { class: "space-y-2",
                        p { class: "font-semibold",
                            "Round {cur.round} could not be fetched yet"
                            if let Some(f) = &v.last_failure { ": {f}" } else { "." }
                        }
                        p { class: "text-sm",
                            "The draw stays on round {cur.round}, so retrying can't change the result. "
                            "A retry queues behind the current attempt and may take up to a minute. "
                            "If drand stays unreachable, abandon this draw; the public verification page shows that it was abandoned and whom its round would have picked."
                        }
                        div { class: "flex flex-wrap gap-2",
                            button { class: "btn btn-sm btn-warning", onclick: { let draw = draw.clone(); let u = u.clone(); move |_| draw(u.clone()) }, "Retry round {cur.round}" }
                            button { class: "btn btn-sm btn-ghost", onclick: move |_| ask_abandon.set(true), "Abandon this draw" }
                        }
                    }
                }
            } else if let (Some(f), None) = (&v.last_failure, &pending) {
                div { role: "alert", class: "alert alert-soft items-start",
                    Icon { i: I::Alert }
                    p { "The last draw ended without its beacon: {f}" }
                }
            }
            Panel {
                title: "Prizes to draw",
                icon: I::Dice,
                actions: rsx! {
                    if let Some(hash) = &v.manifest_hash {
                        span { class: "tooltip tooltip-left", "data-tip": "Manifest hash: {hash}",
                            span { class: "badge badge-ghost gap-1 font-mono", Icon { i: I::Lock, class: "size-3.5" } "{&hash[..10]}…" }
                        }
                    }
                },
                ul { class: "space-y-2",
                    for u in v.items.iter() {
                        {
                            let winner = u.entrant.as_ref().and_then(|id| by_id.get(id.as_str()));
                            let uid = u.item.clone();
                            let row = match u.status {
                                ItemStatus::Drawn => "border-success/50 bg-success/10",
                                ItemStatus::Claimed => "border-base-300 bg-base-200/60",
                                ItemStatus::Voided => "border-base-300 opacity-50",
                                ItemStatus::Available => "border-base-300",
                            };
                            let is_current = pending.as_ref().is_some_and(|p| p.item == u.item);
                            let emoji = match u.status {
                                ItemStatus::Available if is_current => "🎲",
                                ItemStatus::Available => "🎁",
                                ItemStatus::Drawn => "🎉",
                                ItemStatus::Claimed => "✅",
                                ItemStatus::Voided => "🚫",
                            };
                            rsx! {
                                li { key: "{u.item}", class: "flex flex-wrap items-center gap-3 rounded-box border-2 p-3 transition {row}",
                                    span { class: "text-2xl", "{emoji}" }
                                    div { class: "min-w-40 flex-1",
                                        p { class: "font-semibold", "{u.prize_name}" }
                                        p { class: "text-sm text-base-content/60", "{status_label(u.status)}" }
                                    }
                                    if let Some(w) = winner {
                                        div { class: "flex items-center gap-3",
                                            div { class: "text-right",
                                                p { class: "font-display text-lg font-bold", "{w.display_name}" }
                                                if let Some(email) = &w.email { p { class: "text-xs text-base-content/60", "{email}" } }
                                                if w.source == EntrantSource::Manual { p { class: "text-xs text-base-content/60", "added by you" } }
                                            }
                                            span { class: "ticket bg-accent px-5 py-2 text-accent-content",
                                                span { class: "claim-code text-lg", "{w.claim_code}" }
                                            }
                                        }
                                    }
                                    div { class: "flex flex-wrap gap-2",
                                        match u.status {
                                            ItemStatus::Available if drawable => rsx! {
                                                button { class: "btn btn-primary hover-wiggle", disabled: busy, onclick: { let draw = draw.clone(); let uid = uid.clone(); move |_| draw(uid.clone()) },
                                                    Icon { i: I::Dice } "Draw"
                                                }
                                                button { class: "btn btn-ghost btn-sm", disabled: busy, title: "Withdraw this prize (e.g. it isn't available)", onclick: { let void = void.clone(); let uid = uid.clone(); move |_| void(uid.clone()) }, "Withdraw" }
                                            },
                                            ItemStatus::Drawn if drawable => rsx! {
                                                button { class: "btn btn-success", onclick: { let r = resolve.clone(); let uid = uid.clone(); move |_| r(uid.clone(), Resolution::Claimed) }, Icon { i: I::Check } "Handed over" }
                                                button { class: "btn btn-ghost", disabled: busy, title: if busy { "Wait for the current draw to finish: this could change who it can pick" } else { "" }, onclick: { let r = resolve.clone(); let uid = uid.clone(); move |_| r(uid.clone(), Resolution::NotHere) }, "Not here" }
                                                button { class: "btn btn-ghost", disabled: busy, title: if busy { "Wait for the current draw to finish: this could change who it can pick" } else { "" }, onclick: { let r = resolve.clone(); let uid = uid.clone(); move |_| r(uid.clone(), Resolution::Declined) }, "Declined" }
                                            },
                                            _ => rsx! {},
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if drawable {
                    div { class: "flex flex-wrap items-center gap-3 border-t border-base-300 pt-4",
                        p { class: "flex-1 text-sm text-base-content/60",
                            "{items_left} prize(s) left. “Not here” and “Declined” take that person out and put the prize back for a new draw."
                        }
                        Button {
                            color: if items_left == 0 { ButtonColor::Secondary } else { ButtonColor::Default },
                            class: if items_left == 0 { "hover-wiggle" } else { "btn-ghost" },
                            disabled: busy,
                            onclick: move |_| ask_finish.set(true),
                            Icon { i: I::Flag }
                            "Finish the raffle"
                        }
                    }
                }
            }
            Confirm {
                open: ask_finish,
                title: if items_left == 0 { "Finish the raffle?".to_owned() } else { format!("Finish with {items_left} prize(s) left?") },
                body: "The thank-you screen goes up and nothing more can be drawn. Personal data is erased automatically after the retention period.",
                action: "🎉 Finish",
                on_confirm: move |_| { spawn(finish()); },
            }
            if let Some(cur) = &pending {
                Confirm {
                    open: ask_abandon,
                    title: format!("Abandon the draw on round {}?", cur.round),
                    body: "The prize goes back to “to be drawn” and a new draw commits to a new round. The verification page lists the abandoned round so anyone can see whom it would have picked.",
                    action: "Abandon",
                    danger: true,
                    on_confirm: { let abandon = abandon.clone(); let u = cur.item.clone(); move |_| { spawn(abandon(u.clone())); } },
                }
            }
        }
    }
}

fn status_label(s: ItemStatus) -> &'static str {
    match s {
        ItemStatus::Available => "to be drawn",
        ItemStatus::Drawn => "drawn: waiting for the winner",
        ItemStatus::Claimed => "handed over",
        ItemStatus::Voided => "withdrawn",
    }
}

// ---------------------------------------------------------------------------
// Prizes
// ---------------------------------------------------------------------------

#[component]
fn Prizes(admin: AdminData, error: Signal<Option<String>>, on_change: EventHandler<()>) -> Element {
    let editable = admin.view.state.prizes_editable();
    let next_position = admin.prizes.iter().map(|i| i.position).max().unwrap_or(0) + 1;
    let total: u32 = admin.prizes.iter().map(|i| i.qty).sum();
    rsx! {
        Panel {
            title: "Prizes",
            icon: I::Gift,
            actions: rsx! {
                span { class: "badge badge-soft badge-secondary", "{total} to give away" }
                if !editable { span { class: "badge badge-ghost gap-1", Icon { i: I::Lock, class: "size-3.5" } "locked" } }
            },
            if admin.prizes.is_empty() {
                div { class: "rounded-box border-2 border-dashed border-base-300 py-10 text-center",
                    p { class: "text-4xl", "🎁" }
                    p { class: "mt-2 text-base-content/60", "No prizes yet. What are you giving away?" }
                }
            }
            ul { class: "space-y-2",
                for prize in admin.prizes.iter() {
                    PrizeRow { key: "{prize.id}", raffle_id: admin.id.clone(), prize: prize.clone(), editable, error, on_change }
                }
            }
            if editable {
                div { class: "rounded-box bg-base-200 p-4",
                    p { class: "mb-3 text-sm font-semibold", "Add a prize" }
                    PrizeForm { raffle_id: admin.id.clone(), position: next_position, on_saved: move |_| on_change.call(()) }
                }
                p { class: "text-sm text-base-content/60", "Each copy is drawn on its own, in whatever order you like during the draw." }
            } else {
                p { class: "text-sm text-base-content/60", "Prizes are locked because the entrant list is frozen." }
            }
        }
    }
}

#[component]
fn PrizeRow(
    raffle_id: String,
    prize: Prize,
    editable: bool,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let mut editing = use_signal(|| false);
    let (rid, iid) = (raffle_id.clone(), prize.id.clone());
    let remove = move |_| {
        let (rid, iid) = (rid.clone(), iid.clone());
        async move {
            match api::delete_prize(rid, iid).await {
                Ok(()) => on_change.call(()),
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    if editing() {
        return rsx! {
            li { class: "rounded-box bg-base-200 p-3",
                PrizeForm {
                    raffle_id,
                    prize_id: Some(prize.id.clone()),
                    name: prize.name.clone(),
                    sponsor: prize.sponsor.clone(),
                    qty: prize.qty,
                    position: prize.position,
                    on_saved: move |_| { editing.set(false); on_change.call(()); },
                    on_cancel: move |_| editing.set(false),
                }
            }
        };
    }
    rsx! {
        li { class: "flex items-center gap-3 rounded-box border-2 border-base-300 p-3",
            span { class: "grid size-11 shrink-0 place-items-center rounded-xl bg-secondary/15 font-display text-lg font-extrabold text-secondary", "{prize.qty}×" }
            div { class: "min-w-0 flex-1",
                p { class: "truncate font-semibold", "{prize.name}" }
                if !prize.sponsor.is_empty() { p { class: "text-sm text-base-content/60", "by {prize.sponsor}" } }
            }
            if editable {
                button { class: "btn btn-ghost btn-sm btn-square", title: "Edit", aria_label: "Edit {prize.name}", onclick: move |_| editing.set(true), Icon { i: I::Pencil, class: "size-4" } }
                button { class: "btn btn-ghost btn-sm btn-square text-error", title: "Remove", aria_label: "Remove {prize.name}", onclick: remove, Icon { i: I::Trash, class: "size-4" } }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

#[component]
fn People(admin: AdminData, error: Signal<Option<String>>, on_change: EventHandler<()>) -> Element {
    let editable = admin.view.state.entrants_editable();
    let mut filter = use_signal(String::new);
    let id = admin.id.clone();
    let fakes = move |_| {
        let id = id.clone();
        async move {
            match api::add_fake_entrants(id, 25).await {
                Ok(()) => on_change.call(()),
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let q = filter().to_lowercase();
    let shown: Vec<AdminEntrant> = admin
        .entrants
        .iter()
        .filter(|e| {
            q.is_empty()
                || e.display_name.to_lowercase().contains(&q)
                || e.email.as_deref().unwrap_or("").to_lowercase().contains(&q)
                || e.claim_code.to_lowercase() == q
        })
        .cloned()
        .collect();

    rsx! {
        Panel {
            title: "People",
            icon: I::Users,
            actions: rsx! {
                span { class: "badge badge-soft badge-primary", "{admin.entrants.len()} entrants" }
                if admin.settings.test_mode && editable {
                    button { class: "btn btn-sm btn-warning btn-soft", onclick: fakes, "🧪 Add 25 fake entrants" }
                }
            },
            if editable {
                div { class: "rounded-box bg-base-200 p-4",
                    p { class: "mb-3 text-sm font-semibold", "Someone without a smartphone? Add them here." }
                    AddEntrantForm { raffle_id: admin.id.clone(), on_added: move |_| on_change.call(()) }
                }
            }
            if admin.entrants.is_empty() {
                div { class: "rounded-box border-2 border-dashed border-base-300 py-10 text-center",
                    p { class: "text-4xl", "🙋" }
                    p { class: "mt-2 text-base-content/60", "Nobody yet. Open registration and show the projector screen." }
                }
            } else {
                label { class: "input w-full",
                    Icon { i: I::Eye, class: "size-4 opacity-50" }
                    input { class: "grow", r#type: "search", placeholder: "Search by name, email or claim code", value: "{filter}", oninput: move |e| filter.set(e.value()) }
                }
                div { class: "overflow-x-auto",
                    table { class: "table",
                        thead { tr { th { "Name" } th { "Email" } th { "Claim code" } th { "Status" } th {} } }
                        tbody {
                            for e in shown.iter() {
                                EntrantRow { key: "{e.id}", raffle_id: admin.id.clone(), entrant: e.clone(), editable, error, on_change }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[component]
fn EntrantRow(
    raffle_id: String,
    entrant: AdminEntrant,
    editable: bool,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let mut renaming = use_signal(|| None::<String>);
    let mut confirm = use_signal(|| false);
    let e = entrant.clone();

    let patch = {
        let (rid, eid) = (raffle_id.clone(), e.id.clone());
        move |p: EntrantPatch| {
            let (rid, eid) = (rid.clone(), eid.clone());
            async move {
                match api::patch_entrant(rid, eid, p).await {
                    Ok(()) => {
                        error.set(None);
                        on_change.call(());
                    }
                    Err(e) => error.set(Some(err_text(e))),
                }
            }
        }
    };
    let remove = {
        let (rid, eid) = (raffle_id.clone(), e.id.clone());
        move || {
            let (rid, eid) = (rid.clone(), eid.clone());
            async move {
                match api::remove_entrant(rid, eid).await {
                    Ok(()) => on_change.call(()),
                    Err(e) => error.set(Some(err_text(e))),
                }
            }
        }
    };
    let (status, status_class) = match (e.outcome, &e.prize) {
        (Some(Outcome::Claimed), Some(p)) => (format!("🏆 won {p}"), "badge-success"),
        (Some(Outcome::Drawn), Some(p)) => (format!("🎉 drawn for {p}"), "badge-primary"),
        (Some(Outcome::NotHere), _) => ("not here".to_owned(), "badge-ghost"),
        (Some(Outcome::Declined), _) => ("declined".to_owned(), "badge-ghost"),
        _ if e.purged => ("erased".to_owned(), "badge-neutral"),
        _ => (String::new(), ""),
    };
    let won = e.outcome == Some(Outcome::Claimed);
    let avatar = match e.source {
        EntrantSource::Manual => "bg-secondary text-secondary-content",
        EntrantSource::Fake => "bg-warning text-warning-content",
        EntrantSource::SelfService => "bg-primary text-primary-content",
    };

    rsx! {
        tr { class: if e.hidden { "opacity-60" } else { "" },
            td {
                match renaming() {
                    Some(val) => rsx! {
                        form { class: "join", onsubmit: { let patch = patch.clone(); move |ev: FormEvent| { ev.prevent_default(); let name = renaming().unwrap_or_default(); renaming.set(None); patch(EntrantPatch { display_name: Some(name), ..Default::default() }) } },
                            input { class: "input input-sm join-item", value: "{val}", maxlength: "24", oninput: move |ev| renaming.set(Some(ev.value())) }
                            button { class: "btn btn-sm btn-primary join-item", r#type: "submit", Icon { i: I::Check, class: "size-4" } }
                        }
                    },
                    None => rsx! {
                        div { class: "flex items-center gap-3",
                            div { class: "avatar avatar-placeholder",
                                div { class: "w-9 rounded-full {avatar}", span { class: "text-xs font-bold", "{initials(&e.display_name)}" } }
                            }
                            div {
                                p { class: "font-semibold", "{e.display_name}" }
                                div { class: "flex gap-1",
                                    if e.hidden { span { class: "badge badge-ghost badge-xs", "hidden on screen" } }
                                    if e.source == EntrantSource::Manual { span { class: "badge badge-secondary badge-soft badge-xs", "no phone" } }
                                    if e.source == EntrantSource::Fake { span { class: "badge badge-warning badge-soft badge-xs", "fake" } }
                                }
                            }
                        }
                    },
                }
            }
            td { class: "text-sm text-base-content/70", "{e.email.clone().unwrap_or_default()}" }
            td { span { class: "claim-code text-sm", "{e.claim_code}" } }
            td { if !status.is_empty() { span { class: "badge badge-soft {status_class} whitespace-nowrap", "{status}" } } }
            td {
                if !e.purged {
                    div { class: "flex justify-end gap-1",
                        button {
                            class: "btn btn-ghost btn-xs btn-square tooltip",
                            "data-tip": if e.hidden { "Show name on screen" } else { "Hide name on screen" },
                            onclick: { let patch = patch.clone(); let hidden = !e.hidden; move |_| patch(EntrantPatch { hidden: Some(hidden), ..Default::default() }) },
                            if e.hidden { Icon { i: I::Eye, class: "size-4" } } else { Icon { i: I::EyeOff, class: "size-4" } }
                        }
                        button { class: "btn btn-ghost btn-xs btn-square tooltip", "data-tip": "Rename", onclick: { let n = e.display_name.clone(); move |_| renaming.set(Some(n.clone())) }, Icon { i: I::Pencil, class: "size-4" } }
                        if won {
                            button {
                                class: if e.delivered { "btn btn-success btn-xs" } else { "btn btn-ghost btn-xs" },
                                onclick: { let patch = patch.clone(); let d = !e.delivered; move |_| patch(EntrantPatch { delivered: Some(d), ..Default::default() }) },
                                if e.delivered { "✓ delivered" } else { "Mark delivered" }
                            }
                        }
                        button {
                            class: "btn btn-ghost btn-xs btn-square tooltip text-error",
                            "data-tip": if editable { "Remove" } else { "Erase personal data" },
                            onclick: move |_| confirm.set(true),
                            Icon { i: I::Trash, class: "size-4" }
                        }
                    }
                    Confirm {
                        open: confirm,
                        title: if editable { format!("Remove {}?", e.display_name) } else { format!("Erase {}'s personal data?", e.display_name) },
                        body: if editable {
                            "They are taken off the entrant list. They can join again while registration is open.".to_owned()
                        } else {
                            "The list is frozen, so the anonymous entry stays (the draw must remain verifiable), but the name, email and claim code are erased for good.".to_owned()
                        },
                        action: if editable { "Remove" } else { "Erase" },
                        danger: true,
                        on_confirm: { let remove = remove.clone(); move |_| { spawn(remove()); } },
                    }
                }
            }
        }
    }
}
