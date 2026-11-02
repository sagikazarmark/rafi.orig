//! The prizes tab.

use dioxus_icons::lucide::{Gift, Lock, Pencil, Trash};
use rafi_lifecycle::prize::Delivery;

use super::*;
use crate::ui::forms::{DeliveryPicker, delivery_hint, delivery_label};

// ---------------------------------------------------------------------------
// Prizes
// ---------------------------------------------------------------------------

#[component]
pub(super) fn Prizes(
    admin: AdminData,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let editable = admin.view.state.prizes_editable();
    let delivery_editable = admin.view.state.prize_delivery_editable();
    let next_position = admin.prizes.iter().map(|i| i.position).max().unwrap_or(0) + 1;
    let total: u32 = admin.prizes.iter().map(|i| i.qty).sum();
    rsx! {
        Panel {
            title: "Prizes",
            icon: rsx! { Gift { class: "size-5" } },
            actions: rsx! {
                span { class: "badge badge-soft badge-secondary", "{total} to give away" }
                if !editable { span { class: "badge badge-ghost gap-1", Lock { class: "size-3.5" } "locked" } }
            },
            if admin.prizes.is_empty() {
                div { class: "rounded-box border-2 border-dashed border-base-300 py-10 text-center",
                    p { class: "text-4xl", "🎁" }
                    p { class: "mt-2 text-base-content/60", "No prizes yet. What are you giving away?" }
                }
            }
            ul { class: "space-y-2",
                for prize in admin.prizes.iter() {
                    PrizeRow { key: "{prize.id}", raffle_id: admin.id.clone(), prize: prize.clone(), editable, delivery_editable, error, on_change }
                }
            }
            if editable {
                div { class: "rounded-box bg-base-200 p-4",
                    p { class: "mb-3 text-sm font-semibold", "Add a prize" }
                    PrizeForm { raffle_id: admin.id.clone(), position: next_position, on_saved: move |_| on_change.call(()) }
                }
                p { class: "text-sm text-base-content/60", "Each copy is drawn on its own, in whatever order you like during the draw." }
            } else if delivery_editable {
                p { class: "text-sm text-base-content/60", "Prizes are locked because the entrant list is frozen. How each one reaches its winner can still change: it applies to the claims from then on." }
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
    delivery_editable: bool,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let mut editing = use_signal(|| false);
    let mut saving = use_signal(|| false);
    let mut ask_remove = use_signal(|| false);
    let (rid, iid) = (raffle_id.clone(), prize.id.clone());
    let remove = move || {
        let (rid, iid) = (rid.clone(), iid.clone());
        async move {
            match api::delete_prize(rid, iid).await {
                Ok(()) => on_change.call(()),
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    let (rid, iid) = (raffle_id.clone(), prize.id.clone());
    let current = prize.delivery;
    let set_delivery = move |delivery: Delivery| {
        let (rid, iid) = (rid.clone(), iid.clone());
        if delivery == current {
            return;
        }
        spawn(async move {
            saving.set(true);
            let result = api::set_prize_delivery(rid, iid, delivery).await;
            saving.set(false);
            match result {
                Ok(()) => {
                    error.set(None);
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        });
    };
    if editing() {
        return rsx! {
            li { class: "rounded-box bg-base-200 p-3",
                PrizeForm {
                    raffle_id,
                    prize_id: Some(prize.id.clone()),
                    name: prize.name.clone(),
                    sponsor: prize.sponsor.clone(),
                    description: prize.description.clone(),
                    qty: prize.qty,
                    position: prize.position,
                    delivery: Some(prize.delivery),
                    on_saved: move |_| { editing.set(false); on_change.call(()); },
                    on_cancel: move |_| editing.set(false),
                }
            }
        };
    }
    rsx! {
        li { class: "flex flex-wrap items-center gap-3 rounded-box border-2 border-base-300 p-3",
            span { class: "grid size-11 shrink-0 place-items-center rounded-xl bg-secondary/15 font-display text-lg font-extrabold text-secondary", "{prize.qty}×" }
            div { class: "min-w-40 flex-1",
                p { class: "truncate font-semibold", "{prize.name}" }
                if !prize.sponsor.is_empty() { p { class: "text-sm text-base-content/60", "by {prize.sponsor}" } }
            }
            if delivery_editable {
                span { title: delivery_hint(Some(prize.delivery)),
                    DeliveryPicker {
                        value: Some(prize.delivery),
                        size: "btn-xs",
                        disabled: saving(),
                        label: "How the winner gets {prize.name}",
                        on_pick: set_delivery,
                    }
                }
            } else {
                span { class: "badge badge-ghost", title: delivery_hint(Some(prize.delivery)), "{delivery_label(prize.delivery)}" }
            }
            if editable {
                button { class: "btn btn-ghost btn-sm btn-square", title: "Edit", aria_label: "Edit {prize.name}", onclick: move |_| editing.set(true), Pencil { class: "size-4" } }
                button { class: "btn btn-ghost btn-sm btn-square text-error", title: "Remove", aria_label: "Remove {prize.name}", onclick: move |_| ask_remove.set(true), Trash { class: "size-4" } }
            }
        }
        Confirm {
            open: ask_remove,
            title: rsx! { "Remove the {prize.name}?" },
            body: rsx! { if prize.qty == 1 { "It won't be drawn." } else { "All {prize.qty} copies go, and none is drawn." } " Add it again if you change your mind." },
            action: "Remove",
            danger: true,
            on_confirm: move |_| { spawn(remove()); },
        }
    }
}
