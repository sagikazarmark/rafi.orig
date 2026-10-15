//! The draw console: drawing each prize and deciding its winner.

use dioxus_icons::lucide::{Check, Dice5, Flag, Lock, TriangleAlert};

use super::*;

// ---------------------------------------------------------------------------
// Draw console
// ---------------------------------------------------------------------------

#[component]
pub(super) fn DrawConsole(
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
    let offset = v.server_now_ms as i64 - crate::ui::now_ms() as i64;
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
                    TriangleAlert { class: "size-5" }
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
                    TriangleAlert { class: "size-5" }
                    p { "The last draw ended without its beacon: {f}" }
                }
            }
            Panel {
                title: "Prizes to draw",
                icon: rsx! { Dice5 { class: "size-5" } },
                actions: rsx! {
                    if let Some(hash) = &v.manifest_hash {
                        span { class: "tooltip tooltip-left", "data-tip": "Manifest hash: {hash}",
                            span { class: "badge badge-ghost gap-1 font-mono", Lock { class: "size-3.5" } "{&hash[..10]}…" }
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
                                                if w.source == Source::Manual { p { class: "text-xs text-base-content/60", "added by you" } }
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
                                                    Dice5 { class: "size-5" } "Draw"
                                                }
                                                button { class: "btn btn-ghost btn-sm", disabled: busy, title: "Withdraw this prize (e.g. it isn't available)", onclick: { let void = void.clone(); let uid = uid.clone(); move |_| void(uid.clone()) }, "Withdraw" }
                                            },
                                            ItemStatus::Drawn if drawable => rsx! {
                                                button { class: "btn btn-success", onclick: { let r = resolve.clone(); let uid = uid.clone(); move |_| r(uid.clone(), Resolution::Claimed) }, Check { class: "size-5" } "Handed over" }
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
                            Flag { class: "size-5" }
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
