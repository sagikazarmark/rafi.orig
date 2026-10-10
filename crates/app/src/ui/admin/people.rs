//! The people tab: entrants, adding and editing them.

use super::*;

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

#[component]
pub(super) fn People(
    admin: AdminData,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
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
    let won = e.deliverable;
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
                            input { class: "input input-sm join-item", value: "{val}", maxlength: "{rafi_lifecycle::entrant::MAX_NAME_CHARS}", oninput: move |ev| renaming.set(Some(ev.value())) }
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
