//! The raffle's header: its state, the next lifecycle step and the menu.

use dioxus_icons::lucide::{
    Check, Copy, Download, Ellipsis, Flag, Lock, Monitor, PackageCheck, Play, QrCode, ShieldCheck,
    Ticket, Trash, TriangleAlert, Undo2, Zap,
};

use super::*;

#[component]
pub(super) fn Header(
    admin: AdminData,
    /// The signed-in user's tier: duplicating and deleting need a manager.
    access: Access,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let manages = access.allows(Access::Manager);
    let v = admin.view.clone();
    let mut ask_delete = use_signal(|| false);
    let mut ask_purge = use_signal(|| false);
    let mut ask_freeze = use_signal(|| false);
    let mut ask_end = use_signal(|| false);
    let mut ask_deliver = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let nav = navigator();
    let toast = use_toast();
    let screen_url = format!("{}/s/{}", admin.public_url, v.slug);
    let verify_url = format!("{}/v/{}", admin.public_url, v.slug);

    let id = admin.id.clone();
    let act = move |action: LifecycleCommand| {
        let id = id.clone();
        async move {
            busy.set(true);
            match api::set_lifecycle(id, action).await {
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
            // The copy is on the organizer's today.
            let today = dates::today_iso();
            let ends_at = dates::end_of_day_local(&today);
            match api::duplicate_raffle(id, today, ends_at).await {
                Ok(new_id) => {
                    toast.success(
                        "Duplicated".to_owned(),
                        ToastOptions::new().description(
                            "Prizes and settings were copied; entrants were not. The copy is dated today: set its event date.",
                        ),
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
            match api::export_winners_csv(id).await {
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
    let deliver_all = move || {
        let id = id.clone();
        async move {
            match api::deliver_all_prizes(id).await {
                Ok(n) => {
                    toast.success(
                        format!("📦 {n} prize(s) marked delivered"),
                        ToastOptions::new()
                            .description("Those winners' data is erased on the erasure date with everyone else's."),
                    );
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
                    toast.success("Live updates ended".to_owned(), ToastOptions::new());
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
            busy.set(false);
        }
    };
    // What the organizer can do next is the server's to say (the raffle's
    // rules); the page derives none of it.
    let unconfirmed = admin.actions.start == Some(StartStep::Retry);
    let join_url = v.join_url.clone();
    let join_code = grouped(&v.join_code);
    let can_export = admin.actions.export;
    let can_purge = admin.actions.purge;
    let can_deliver = admin.actions.deliver_all;
    // What the settings lack for opening (or, without registration, the
    // freeze from Published), while that step is ahead: the server says.
    let missing = admin.actions.missing.clone();
    let can_open = missing.is_empty();
    // The lifecycle's buttons: the commands the server says the raffle
    // takes now, with its registration setting, and the freeze when the
    // draw can start.
    let offers = {
        let lifecycle = admin.actions.lifecycle.clone();
        move |c: LifecycleCommand| lifecycle.contains(&c)
    };
    let can_freeze = admin.actions.start == Some(StartStep::Freeze);
    // From Published (no registration), the freeze checks what opening
    // would have: the alert below says what is missing. From Open or
    // Closed nothing is.
    let freeze_blocked = !can_open;
    let registration = v.registration;

    let (dup, exp) = (duplicate.clone(), export.clone());
    let on_menu = use_callback(move |value: String| match value.as_str() {
        "duplicate" => {
            spawn(dup());
        }
        "export" => {
            spawn(exp());
        }
        "deliver" => ask_deliver.set(true),
        "purge" => ask_purge.set(true),
        "delete" => ask_delete.set(true),
        _ => {}
    });
    let mut menu: Vec<(&'static str, &'static str, Element)> = Vec::new();
    if manages {
        menu.push(("duplicate", "Duplicate", rsx! { Copy { class: "size-4" } }));
    }
    if can_export {
        menu.push((
            "export",
            "Export winners (CSV)",
            rsx! { Download { class: "size-4" } },
        ));
    }
    if can_deliver {
        menu.push((
            "deliver",
            "All prizes delivered",
            rsx! { PackageCheck { class: "size-4" } },
        ));
    }
    if can_purge {
        menu.push((
            "purge",
            "Erase personal data now",
            rsx! { Trash { class: "size-4" } },
        ));
    }
    if manages && admin.actions.delete {
        menu.push((
            "delete",
            "Delete raffle",
            rsx! { Trash { class: "size-4" } },
        ));
    }

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
                            if !v.verifiable { span { class: "badge badge-warning gap-1", title: "Draws use the mock drand chain: nobody can verify them", Zap { class: "size-3.5" } "mock drand" } }
                        }
                    }
                    DropdownMenu { align: DropdownMenuAlign::End,
                        DropdownMenuTrigger { class: "btn-ghost btn-circle", aria_label: "More actions", Ellipsis { class: "size-5" } }
                        DropdownMenuContent { class: "z-20 w-60",
                            for (index , (value , label , icon)) in menu.into_iter().enumerate() {
                                DropdownMenuItem::<String> {
                                    key: "{value}",
                                    value: value.to_owned(),
                                    index,
                                    on_select: on_menu,
                                    class: if matches!(value, "purge" | "delete") { "text-error" } else { "" },
                                    {icon}
                                    "{label}"
                                }
                            }
                        }
                    }
                }
                Steps { state: v.state, registration }
                if !can_open {
                    div { role: "alert", class: "alert alert-warning alert-soft text-sm",
                        TriangleAlert { class: "size-5" }
                        span {
                            if registration { "Before registration can open" } else { "Before the list can be frozen" }
                            ", fill in {missing.join(\" and \")} in the settings: the privacy notice must name the data controller and how to reach them."
                        }
                    }
                }
                if v.state.is_public() {
                    div { class: "flex flex-wrap items-center gap-2",
                        a { class: "btn btn-neutral btn-sm", href: "{screen_url}", target: "_blank", Monitor { class: "size-4" } "Projector screen" }
                        // Without registration nobody joins: no link or code to share.
                        if registration {
                            div { class: "join",
                                span { class: "join-item inline-flex h-8 items-center gap-1.5 border border-base-300 bg-base-200 px-3 font-mono text-sm",
                                    QrCode { class: "size-4 opacity-60" }
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
                                    Copy { class: "size-4" }
                                }
                            }
                            span { class: "badge badge-lg badge-accent font-mono font-bold tracking-widest", title: "The join code: typed instead of scanning", "{join_code}" }
                        }
                        if v.manifest_hash.is_some() {
                            a { class: "btn btn-ghost btn-sm", href: "{verify_url}", target: "_blank", ShieldCheck { class: "size-4" } "Verification page" }
                        }
                    }
                }
                div { class: "flex flex-wrap items-center gap-2",
                    if offers(LifecycleCommand::Publish) {
                        Button { color: ButtonColor::Primary, class: "hover-wiggle", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(LifecycleCommand::Publish)); } },
                            Play { class: "size-5" } "Publish the intro screen"
                        }
                    }
                    if offers(LifecycleCommand::Open) && v.state == RaffleState::Ready {
                        Button { color: ButtonColor::Success, class: "hover-wiggle", disabled: busy() || !can_open, title: if can_open { "" } else { "Fill in the organizer and a contact first" }, onclick: { let act = act.clone(); move |_| { spawn(act(LifecycleCommand::Open)); } }, Ticket { class: "size-5" } "Open registration" }
                    }
                    if can_freeze {
                        Button { color: ButtonColor::Primary, class: "hover-wiggle", disabled: busy() || freeze_blocked, title: if freeze_blocked { "Fill in the organizer and a contact first" } else { "" }, onclick: move |_| ask_freeze.set(true), Lock { class: "size-5" } "Freeze list & start the draw" }
                    }
                    if offers(LifecycleCommand::Close) {
                        Button { class: "btn-ghost", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(LifecycleCommand::Close)); } }, "Close registration" }
                    }
                    if offers(LifecycleCommand::Open) && v.state != RaffleState::Ready {
                        Button { class: "btn-ghost", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(LifecycleCommand::Open)); } }, "Reopen registration" }
                    }
                    if offers(LifecycleCommand::Unpublish) {
                        Button { class: "btn-ghost", disabled: busy(), onclick: { let act = act.clone(); move |_| { spawn(act(LifecycleCommand::Unpublish)); } }, Undo2 { class: "size-5" } "Unpublish" }
                    }
                    match v.state {
                        RaffleState::Frozen if unconfirmed => rsx! {
                            Button { color: ButtonColor::Warning, disabled: busy(), onclick: { let start = start.clone(); move |_| { spawn(start()); } }, Undo2 { class: "size-5" } "Retry handing the list to the raffle engine" }
                        },
                        RaffleState::Finished | RaffleState::Purged if v.live_ended => rsx! {
                            span { class: "badge badge-ghost gap-1", Check { class: "size-3.5" } "Live updates ended" }
                        },
                        RaffleState::Finished | RaffleState::Purged => rsx! {
                            Button { class: "btn-outline", disabled: busy(), onclick: move |_| ask_end.set(true), Flag { class: "size-5" } "End live updates" }
                        },
                        _ => rsx! {},
                    }
                    if busy() { span { class: "loading loading-dots loading-md text-primary" } }
                }
            }
        }
        FreezeDialog { open: ask_freeze, view: v.clone(), on_confirm: move |_| { spawn(start()); } }
        Confirm {
            open: ask_end,
            title: rsx! { "End live updates for this raffle?" },
            body: rsx! { "Phones and the projector stop following the raffle live. The results stay on the pages and the verification page." },
            action: "End live updates",
            on_confirm: move |_| { spawn(end_live()); },
        }
        Confirm {
            open: ask_deliver,
            title: rsx! { "Mark every prize delivered?" },
            body: rsx! { "{admin.retention.held_winners} winner(s) still have their name and email kept so you can deliver their prize. Once marked delivered, their data is erased with everyone else's. Only do this when no prize is still on its way." },
            action: "All prizes delivered",
            on_confirm: move |_| { spawn(deliver_all()); },
        }
        Confirm {
            open: ask_purge,
            title: rsx! { "Erase all personal data now?" },
            body: rsx! { "Names, emails and claim codes of every entrant are erased. The anonymous draw log stays verifiable. This cannot be undone." },
            action: "Erase personal data",
            danger: true,
            on_confirm: move |_| { spawn(purge()); },
        }
        Confirm {
            open: ask_delete,
            title: rsx! { "Delete this raffle?" },
            body: rsx! { "“{v.title}”, its prizes, entrants and draw log are deleted for good." },
            action: "Delete raffle",
            danger: true,
            on_confirm: move |_| { spawn(delete()); },
        }
    }
}

/// Freezing the list: what it publishes, and, when people were added from
/// a list, the reminder that they must have been told (no checkbox, nothing
/// recorded: the organizer owes them the notice, and the terms say so).
#[component]
fn FreezeDialog(open: Signal<bool>, view: RaffleView, on_confirm: EventHandler<()>) -> Element {
    rsx! {
        Confirm {
            open,
            title: rsx! { "Freeze the list and start the draw?" },
            body: rsx! { FreezeBody { registration: view.registration, count: view.entrant_count } },
            note: rsx! { FreezeReminder { list_added: view.list_added } },
            action: "🔒 Freeze & start",
            on_confirm: move |_| on_confirm.call(()),
        }
    }
}

/// What freezing does.
#[component]
fn FreezeBody(registration: bool, count: usize) -> Element {
    rsx! {
        if registration {
            "Registration closes for good and the list of {count} entrants is published. Latecomers can't be added afterwards."
        } else {
            "The list of {count} entrants is published. Nobody can be added afterwards."
        }
    }
}

/// The reminder, when people were added from a list
/// ([`RaffleView::list_added`]).
#[component]
fn FreezeReminder(list_added: bool) -> Element {
    rsx! {
        if list_added {
            p { class: "alert alert-info alert-soft text-sm", role: "note",
                "People added from a list will be shown on the screen and the verification page. Make sure they were told about the draw."
            }
        }
    }
}

/// A join code in groups of three (`K7Q-2X3`), easier to read out and to
/// type; the server drops the dash.
fn grouped(code: &str) -> String {
    code.chars()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// With people added from a list, the freeze reminds the organizer
    /// they must have been told: one line, nothing to tick. (The dialog
    /// itself opens only in a browser.)
    #[test]
    fn the_freeze_reminds_of_the_people_on_a_list() {
        let html = dioxus_ssr::render_element(rsx! { FreezeReminder { list_added: true } });
        assert_eq!(
            html,
            "<p class=\"alert alert-info alert-soft text-sm\" role=\"note\">People added from a list will be shown on the screen and the verification page. Make sure they were told about the draw.</p>"
        );
        let html = dioxus_ssr::render_element(rsx! { FreezeReminder { list_added: false } });
        assert!(html.is_empty(), "{html}");
        let html =
            dioxus_ssr::render_element(rsx! { FreezeBody { registration: false, count: 12 } });
        assert!(
            html.contains("The list of 12 entrants is published."),
            "{html}"
        );
    }
}
