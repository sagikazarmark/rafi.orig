//! The people tab: entrants, adding and editing them.

use dioxus_icons::lucide::{
    Check, ClipboardList, Eye, EyeOff, Pencil, Trash, Undo2, UserMinus, UserPlus, Users,
};
use rafi_api::UndoBatch;
use rafi_lifecycle::entrant::{MAX_ADDED_NAME_CHARS, Removal};

use super::{
    list::{AddList, list_refusal_text},
    *,
};

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
    // The server says whether entrants can be removed or erased now: it knows
    // the erasure date.
    let removal = admin.actions.removal;
    let mut filter = use_signal(String::new);
    let mut adding = use_signal(|| false);
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
                || e.claim_code
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase() == q)
        })
        .cloned()
        .collect();

    rsx! {
        Panel {
            title: "People",
            icon: rsx! { Users { class: "size-5" } },
            actions: rsx! {
                span { class: "badge badge-soft badge-primary", "{admin.entrants.len()} entrants" }
                if admin.settings.test_mode && editable {
                    button { class: "btn btn-sm btn-warning btn-soft", onclick: fakes, "🧪 Add 25 fake entrants" }
                }
                if editable {
                    button { class: "btn btn-sm btn-secondary", onclick: move |_| adding.set(true), UserPlus { class: "size-4" } "Add people" }
                }
            },
            if editable {
                AddPeople { open: adding, admin: admin.clone(), on_added: move |_| on_change.call(()) }
            }
            if let Some(undo) = admin.actions.undo.clone() {
                UndoLastAdd { raffle_id: admin.id.clone(), undo, error, on_change }
            }
            if admin.entrants.is_empty() {
                div { class: "rounded-box border-2 border-dashed border-base-300 py-10 text-center",
                    p { class: "text-4xl", "🙋" }
                    p { class: "mt-2 text-base-content/60",
                        if admin.settings.registration {
                            "Nobody yet. Open registration and show the projector screen."
                        } else {
                            "Nobody yet. Add the people taking part, then freeze the list."
                        }
                    }
                }
            } else {
                label { class: "input w-full",
                    Eye { class: "size-4 opacity-50" }
                    input { class: "grow", r#type: "search", placeholder: "Search by name, email or claim code", value: "{filter}", oninput: move |e| filter.set(e.value()) }
                }
                div { class: "overflow-x-auto",
                    table { class: "table",
                        thead { tr { th { "Name" } th { "Email" } th { "Claim code" } th { "Status" } th {} } }
                        tbody {
                            for e in shown.iter() {
                                EntrantRow { key: "{e.id}", raffle_id: admin.id.clone(), entrant: e.clone(), removal, winners_erase_date: admin.retention.winners_erase_date.clone(), registration: admin.settings.registration, error, on_change }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The Add people dialog: one person, by name and optional email, or a
/// list, pasted or uploaded. Everyone added here is in the draw like
/// everyone else, with no phone of theirs. The list tab is there when the
/// server says the raffle takes a list ([`AdminActions::add_list`](rafi_api::AdminActions::add_list)),
/// and otherwise says why not.
#[component]
fn AddPeople(open: Signal<bool>, admin: AdminData, on_added: EventHandler<()>) -> Element {
    let raffle_id = admin.id.clone();
    let lists = admin.actions.add_list;
    let refusal = admin.actions.add_list_refusal;
    rsx! {
        AlertDialog {
            open: Some(open()),
            on_open_change: move |next| open.set(next),
            class: "max-w-4xl",
            AlertDialogTitle { "Add people" }
            AlertDialogDescription { "For people who don't join with the QR code: someone without a smartphone, or your guest list. Their names are shown on the screen like everyone's; with no phone to show a claim code, check it's them at the stage." }
            Tabs { appearance: TabsAppearance::Border, default_value: "one", class: "w-full",
                TabTrigger { value: "one", index: 0usize, class: "gap-2", UserPlus { class: "size-4" } "One person" }
                TabContent { value: "one", index: 0usize, appearance: TabContentAppearance::None, class: "w-full pt-4",
                    AddPersonForm { raffle_id, removed: admin.removed.clone(), on_added }
                }
                TabTrigger { value: "list", index: 1usize, class: "gap-2", ClipboardList { class: "size-4" } "Paste or upload a list" }
                TabContent { value: "list", index: 1usize, appearance: TabContentAppearance::None, class: "w-full pt-4",
                    if lists {
                        AddList { admin: admin.clone(), on_added, on_done: move |_| open.set(false) }
                    } else {
                        div { role: "status", class: "alert alert-info alert-soft text-sm", "{list_refusal_text(refusal)}" }
                    }
                }
            }
            AlertDialogActions {
                AlertDialogCancel { "Close" }
            }
        }
    }
}

/// "Undo last add": takes the latest add's people off the list again, as
/// the server offers it (before the freeze).
#[component]
fn UndoLastAdd(
    raffle_id: String,
    undo: UndoBatch,
    error: Signal<Option<String>>,
    on_change: EventHandler<()>,
) -> Element {
    let mut confirm = use_signal(|| false);
    let toast = use_toast();
    let people = match undo.count {
        1 => "1 person".to_owned(),
        n => format!("{n} people"),
    };
    let run = move || {
        let (raffle_id, batch_id) = (raffle_id.clone(), undo.batch_id.clone());
        async move {
            match api::undo_add(raffle_id, batch_id).await {
                Ok(n) => {
                    let who = if n == 1 {
                        "1 person".to_owned()
                    } else {
                        format!("{n} people")
                    };
                    toast.success(format!("↩️ {who} taken off the list"), ToastOptions::new());
                    on_change.call(());
                }
                Err(e) => error.set(Some(err_text(e))),
            }
        }
    };
    rsx! {
        div { class: "flex justify-end",
            button { class: "btn btn-sm btn-ghost", onclick: move |_| confirm.set(true), Undo2 { class: "size-4" } "Undo last add ({people})" }
        }
        Confirm {
            open: confirm,
            title: rsx! { "Undo the last add?" },
            body: rsx! { "This takes the {people} of your last add off the list. You can add them again before the freeze." },
            action: "Undo add",
            danger: true,
            on_confirm: move |_| { spawn(run()); },
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
    /// How the entrant can be removed now, if at all.
    removal: Option<Removal>,
    /// The day held winners are erased at the latest, in words.
    winners_erase_date: String,
    /// Whether people join with the QR code (the remove dialog says they
    /// can join again).
    #[props(default = true)]
    registration: bool,
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
    let mut confirm_object = use_signal(|| false);
    let mut confirm_unobject = use_signal(|| false);
    let remove = {
        let (rid, eid) = (raffle_id.clone(), e.id.clone());
        move |at_request: bool| {
            let (rid, eid) = (rid.clone(), eid.clone());
            async move {
                match api::remove_entrant(rid, eid, at_request).await {
                    Ok(()) => on_change.call(()),
                    Err(e) => error.set(Some(err_text(e))),
                }
            }
        }
    };
    // "Took themselves off", and its undo.
    let objection = {
        let (rid, eid) = (raffle_id.clone(), e.id.clone());
        move |object: bool| {
            let (rid, eid) = (rid.clone(), eid.clone());
            async move {
                let done = if object {
                    api::object_entrant(rid, eid).await
                } else {
                    api::unobject_entrant(rid, eid).await
                };
                match done {
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
        _ if e.erased => ("erased".to_owned(), "badge-neutral"),
        _ => (String::new(), ""),
    };
    let won = e.deliverable;
    let avatar = match e.source {
        Source::Manual => "bg-secondary text-secondary-content",
        Source::Fake => "bg-warning text-warning-content",
        Source::SelfService => "bg-primary text-primary-content",
    };

    rsx! {
        tr { class: if e.hidden { "opacity-60" } else { "" },
            td {
                match renaming() {
                    Some(val) => rsx! {
                        // The organizer's limit for anyone they rename, a joiner
                        // too: the server checks the same.
                        form { class: "flex flex-col gap-1", onsubmit: { let patch = patch.clone(); move |ev: FormEvent| { ev.prevent_default(); let name = renaming().unwrap_or_default(); renaming.set(None); patch(EntrantPatch { display_name: Some(name), ..Default::default() }) } },
                            div { class: "join",
                                input { class: "input input-sm join-item", value: "{val}", maxlength: "{MAX_ADDED_NAME_CHARS}", aria_label: "Name", aria_describedby: "rename-hint-{e.id}", oninput: move |ev| renaming.set(Some(ev.value())) }
                                button { class: "btn btn-sm btn-primary join-item", r#type: "submit", aria_label: "Save name", Check { class: "size-4" } }
                            }
                            p { id: "rename-hint-{e.id}", class: "text-xs text-base-content/60", "Up to {MAX_ADDED_NAME_CHARS} characters." }
                        }
                    },
                    None => rsx! {
                        div { class: "flex items-center gap-3",
                            div { class: "avatar avatar-placeholder",
                                div { class: "w-9 rounded-full {avatar}", span { class: "text-xs font-bold", "{initials(&e.display_name)}" } }
                            }
                            div {
                                // In full, however long (the organizer gives up to 64
                                // characters), isolated from the text around it.
                                p { class: "max-w-64 font-semibold wrap-break-word", bdi { "{e.display_name}" } }
                                div { class: "flex gap-1",
                                    if e.objected { span { class: "badge badge-warning badge-soft badge-xs", "took themselves off" } }
                                    if e.hidden { span { class: "badge badge-ghost badge-xs", "hidden on screen" } }
                                    if e.source == Source::Manual { span { class: "badge badge-secondary badge-soft badge-xs", "no phone" } }
                                    if e.source == Source::Fake { span { class: "badge badge-warning badge-soft badge-xs", "fake" } }
                                    if e.held_until.is_some() { span { class: "badge badge-info badge-soft badge-xs", "data held until {winners_erase_date}" } }
                                }
                            }
                        }
                    },
                }
            }
            td { class: "text-sm text-base-content/70", "{e.email.clone().unwrap_or_default()}" }
            td { span { class: "claim-code text-sm", "{e.claim_code.clone().unwrap_or_default()}" } }
            td { if !status.is_empty() { span { class: "badge badge-soft {status_class} whitespace-nowrap", "{status}" } } }
            td {
                if !e.erased {
                    div { class: "flex justify-end gap-1",
                        // Someone who took themselves off stays hidden: the
                        // server refuses to show them.
                        if !e.objected {
                            button {
                                class: "btn btn-ghost btn-xs btn-square tooltip",
                                "data-tip": if e.hidden { "Show name on screen" } else { "Hide name on screen" },
                                onclick: { let patch = patch.clone(); let hidden = !e.hidden; move |_| patch(EntrantPatch { hidden: Some(hidden), ..Default::default() }) },
                                if e.hidden { Eye { class: "size-4" } } else { EyeOff { class: "size-4" } }
                            }
                        }
                        button { class: "btn btn-ghost btn-xs btn-square tooltip", "data-tip": "Rename", onclick: { let n = e.display_name.clone(); move |_| renaming.set(Some(n.clone())) }, Pencil { class: "size-4" } }
                        if won {
                            button {
                                class: if e.delivered { "btn btn-success btn-xs" } else { "btn btn-ghost btn-xs tooltip" },
                                "data-tip": if e.held_until.is_some() { format!("Their name and email are kept until {}, or until you mark the prize delivered", winners_erase_date) } else { String::new() },
                                onclick: { let patch = patch.clone(); let d = !e.delivered; move |_| patch(EntrantPatch { delivered: Some(d), ..Default::default() }) },
                                if e.delivered { "✓ delivered" } else { "Mark delivered" }
                            }
                        }
                        if e.actions.object {
                            button {
                                class: "btn btn-ghost btn-xs tooltip",
                                "data-tip": "They asked not to take part",
                                onclick: move |_| confirm_object.set(true),
                                UserMinus { class: "size-4" }
                                "Took themselves off"
                            }
                        }
                        if e.actions.unobject {
                            button {
                                class: "btn btn-ghost btn-xs tooltip",
                                "data-tip": "Undo “Took themselves off”",
                                onclick: move |_| confirm_unobject.set(true),
                                Undo2 { class: "size-4" }
                                "Undo"
                            }
                        }
                        if let Some(removal) = removal {
                            button {
                                class: "btn btn-ghost btn-xs btn-square tooltip text-error",
                                "data-tip": match removal { Removal::Delete => "Remove", Removal::Erase => "Erase personal data" },
                                onclick: move |_| confirm.set(true),
                                Trash { class: "size-4" }
                            }
                        }
                    }
                    match removal {
                        Some(Removal::Delete) => rsx! {
                            RemoveDialog { open: confirm, entrant: e.clone(), registration, on_confirm: { let remove = remove.clone(); move |at_request| { spawn(remove(at_request)); } } }
                        },
                        Some(Removal::Erase) => rsx! {
                            Confirm {
                                open: confirm,
                                title: rsx! { "Erase " bdi { "{e.display_name}" } "'s personal data?" },
                                body: rsx! { "The anonymous entry stays (the draw must remain verifiable), but the name, email and claim code are erased for good." },
                                action: "Erase",
                                danger: true,
                                on_confirm: { let remove = remove.clone(); move |_| { spawn(remove(false)); } },
                            }
                        },
                        None => rsx! {},
                    }
                    if e.actions.object {
                        Confirm {
                            open: confirm_object,
                            title: rsx! { bdi { "{e.display_name}" } " took themselves off?" },
                            body: rsx! { "{OBJECT_BODY}" },
                            action: "Took themselves off",
                            danger: true,
                            on_confirm: { let objection = objection.clone(); move |_| { spawn(objection(true)); } },
                        }
                    }
                    if e.actions.unobject {
                        Confirm {
                            open: confirm_unobject,
                            title: rsx! { "Put " bdi { "{e.display_name}" } " back?" },
                            body: rsx! { "For a mistake: they are in the draw as before and erased on the usual dates. Their name stays hidden until you show it." },
                            action: "Put back",
                            on_confirm: { let objection = objection.clone(); move |_| { spawn(objection(false)); } },
                        }
                    }
                }
            }
        }
    }
}

/// What "Took themselves off" does, for the organizer to confirm.
const OBJECT_BODY: &str = "For someone you added who asks, after the freeze, not to take part. Their name is hidden on the screen and the verification page from now on. If they are drawn, the prize can only be declined, and it is drawn again. Their name and email are erased when the raffle is over, not on the erasure date. Their anonymous ID stays in the draw, so it can still be verified.";

/// Removing an entrant before the freeze: plainly (a duplicate, a troll),
/// or at their request, which a later add remembers. Offered for anyone
/// real; it says when a later add can't recognize them.
#[component]
fn RemoveDialog(
    open: Signal<bool>,
    entrant: AdminEntrant,
    /// Whether people join with the QR code: then they can join again.
    #[props(default = true)]
    registration: bool,
    on_confirm: EventHandler<bool>,
) -> Element {
    let mut at_request = use_signal(|| false);
    let real = entrant.source != Source::Fake;
    let keyed = entrant.email.is_some() || entrant.list_ref.is_some();
    rsx! {
        AlertDialog {
            open: Some(open()),
            on_open_change: move |next: bool| {
                if !next {
                    at_request.set(false);
                }
                open.set(next)
            },
            AlertDialogTitle { "Remove " bdi { "{entrant.display_name}" } "?" }
            AlertDialogDescription {
                if registration {
                    "They are taken off the entrant list. They can join again while registration is open."
                } else {
                    "They are taken off the entrant list."
                }
            }
            if real {
                label { class: "flex items-start gap-3 text-sm",
                    Switch { value: Some(at_request()), on_change: move |on| at_request.set(on) }
                    span { class: "space-y-1",
                        span { class: "block font-medium", "They asked to be taken off" }
                        span { class: "block text-xs text-base-content/60",
                            if keyed {
                                "Adding your list again leaves them out; you can still add them if they change their mind. Their email or registration ID is kept for this until the freeze."
                            } else {
                                "They have no email or registration ID, so adding your list again can't recognize them: leave them off the list."
                            }
                        }
                    }
                }
            }
            AlertDialogActions {
                AlertDialogCancel { "Cancel" }
                AlertDialogAction {
                    color: AlertDialogButtonColor::Error,
                    on_click: move |_| on_confirm.call(at_request()),
                    "Remove"
                }
            }
        }
    }
}

/// A person's row, server-rendered natively: what it offers someone the
/// organizer added after the freeze, and what it shows once they took
/// themselves off.
#[cfg(test)]
mod tests {
    use super::*;

    fn entrant(objected: bool) -> AdminEntrant {
        serde_json::from_value(serde_json::json!({
            "id": "e1", "display_name": "Anna Kovács", "email": "anna@example.com",
            "source": "manual", "hidden": objected, "claim_code": "K2P9", "created_at": 0,
            "delivered": false, "erased": false, "outcome": null, "prize": null,
            "objected": objected,
            "actions": { "object": !objected, "unobject": objected }
        }))
        .unwrap()
    }

    #[component]
    fn Row(entrant: AdminEntrant) -> Element {
        let error = use_signal(|| None::<String>);
        rsx! {
            table { tbody {
                EntrantRow { raffle_id: "r1", entrant, removal: None, winners_erase_date: String::new(), error, on_change: |_| {} }
            } }
        }
    }

    #[test]
    fn taking_oneself_off() {
        let html = dioxus_ssr::render_element(rsx! { Row { entrant: entrant(false) } });
        assert!(html.contains("Took themselves off"), "{html}");
        assert!(html.contains("Hide name on screen"), "{html}");
        assert!(
            !html.contains("took themselves off"),
            "no badge yet: {html}"
        );

        let html = dioxus_ssr::render_element(rsx! { Row { entrant: entrant(true) } });
        for expected in ["took themselves off", "hidden on screen", "Undo"] {
            assert!(html.contains(expected), "{expected}: {html}");
        }
        assert!(
            !html.contains("Show name on screen"),
            "their name stays hidden: {html}"
        );
        assert!(!html.contains(">Took themselves off<"), "{html}");
    }
}
