//! The raffle's header: its state, the next lifecycle step and the menu.

use super::*;

#[component]
pub(super) fn Header(
    admin: AdminData,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
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
    // What the organizer can do next is the server's to say (the raffle's
    // rules); the page derives none of it.
    let unconfirmed = admin.actions.start == Some(StartStep::Retry);
    let join_url = v.join_url.clone();
    let can_export = admin.actions.export;
    let can_purge = admin.actions.purge;

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
