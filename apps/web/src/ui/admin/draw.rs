//! The draw console: drawing each prize and deciding its winner.

use dioxus_icons::lucide::{Check, Dice5, Flag, Lock, TriangleAlert};
use rafi_lifecycle::prize::Delivery;

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
    let deliveries: HashMap<&str, Delivery> = admin
        .prizes
        .iter()
        .map(|p| (p.id.as_str(), p.delivery))
        .collect();
    // What the console sent and the page doesn't show yet: the buttons stay
    // off meanwhile, or a second click would go out against a stale view.
    let drawing = use_signal(|| None::<Sent>);
    let claiming = use_signal(|| None::<Sent>);
    let held = |sent: Signal<Option<Sent>>| {
        sent.read()
            .as_ref()
            .filter(|s| s.holds(&v, now()))
            .map(|s| s.item.clone())
    };
    let draw_held = held(drawing);
    let claim_held = held(claiming);
    let shown = |sent: Signal<Option<Sent>>| sent.read().as_ref().is_some_and(|s| s.shown(&v));
    use_settle(drawing, shown(drawing), now, error);
    use_settle(claiming, shown(claiming), now, error);
    let pending = v.current.clone();
    let busy = pending.is_some() || draw_held.is_some();
    let drawable = v.state.drawable();
    let mut ask_finish = use_signal(|| false);
    let mut ask_abandon = use_signal(|| false);
    let mut ask_irreversible = use_signal(|| false);
    let mut irreversible = use_signal(|| None::<Irreversible>);
    let mut confirm_first = move |step: Irreversible| {
        irreversible.set(Some(step));
        ask_irreversible.set(true);
    };
    let toast = use_toast();

    let id = admin.id.clone();
    let draw = move |item: ItemId| {
        let id = id.clone();
        // Held from the click, and a click while a draw is out ignored: a
        // double click can come before the button is disabled.
        let mut drawing = drawing;
        let ignored = drawing.peek().as_ref().is_some_and(Sent::out);
        if !ignored {
            drawing.set(Some(Sent::new(Awaiting::Draw, item.clone())));
        }
        async move {
            if ignored {
                return;
            }
            match api::draw_item(id, item, command_id()).await {
                Ok(()) => {
                    if let Some(s) = drawing.write().as_mut() {
                        s.answer();
                    }
                    error.set(None);
                    on_change.call(());
                }
                Err(e) => {
                    drawing.set(None);
                    error.set(Some(err_text(e)));
                }
            }
        }
    };
    let id = admin.id.clone();
    // Whether the engine took it.
    let resolved = move |item: ItemId, resolution: Resolution| {
        let id = id.clone();
        async move {
            match api::resolve_item(id, item, resolution, command_id()).await {
                Ok(()) => {
                    error.set(None);
                    on_change.call(());
                    true
                }
                Err(e) => {
                    error.set(Some(err_text(e)));
                    false
                }
            }
        }
    };
    let claim = {
        let resolved = resolved.clone();
        move |item: ItemId| {
            // As a draw: held from the click, a click while a claim is out
            // ignored.
            let mut claiming = claiming;
            let ignored = claiming.peek().as_ref().is_some_and(Sent::out);
            if !ignored {
                claiming.set(Some(Sent::new(Awaiting::Claim, item.clone())));
            }
            let sent = resolved(item, Resolution::Claimed);
            async move {
                if ignored {
                    return;
                }
                let ok = sent.await;
                let mut held = claiming.write();
                match held.as_mut() {
                    Some(s) if ok => s.answer(),
                    _ => *held = None,
                }
            }
        }
    };
    let resolve = move |item: ItemId, resolution: Resolution| {
        let sent = resolved(item, resolution);
        async move {
            sent.await;
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
                            button { class: "btn btn-sm btn-warning", disabled: draw_held.is_some(), onclick: { let draw = draw.clone(); let u = u.clone(); move |_| draw(u.clone()) },
                                if draw_held.is_some() { span { class: "loading loading-spinner loading-xs" } }
                                "Retry round {cur.round}"
                            }
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
                            // Took themselves off: declined only (the server
                            // refuses the rest).
                            let objected = winner.is_some_and(|w| w.objected);
                            // No phone tells them, and no code to show: the
                            // organizer checks it is them.
                            let no_phone = winner.is_some_and(|w| w.no_phone);
                            let uid = u.item.clone();
                            let winner_name = winner.map_or_else(|| "The winner".to_owned(), |w| w.display_name.clone());
                            let prize = u.prize_name.clone();
                            let on_the_spot = deliveries.get(u.prize_id.as_str()).is_some_and(|d| d.delivered_on_claim());
                            let row = match u.status {
                                ItemStatus::Drawn => "border-success/50 bg-success/10",
                                ItemStatus::Claimed => "border-base-300 bg-base-200/60",
                                ItemStatus::Voided => "border-base-300 opacity-50",
                                ItemStatus::Available => "border-base-300",
                            };
                            let is_current = pending.as_ref().is_some_and(|p| p.item == u.item);
                            let claiming_this = claim_held.as_ref() == Some(&u.item);
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
                                        WinnerCard { key: "{w.id}", winner: w.clone(), to_check: u.status == ItemStatus::Drawn }
                                    }
                                    div { class: "flex flex-wrap gap-2",
                                        match u.status {
                                            ItemStatus::Available if drawable => rsx! {
                                                button { class: "btn btn-primary hover-wiggle", disabled: busy, onclick: { let draw = draw.clone(); let uid = uid.clone(); move |_| draw(uid.clone()) },
                                                    if draw_held.as_ref() == Some(&u.item) { span { class: "loading loading-spinner loading-sm" } } else { Dice5 { class: "size-5" } }
                                                    "Draw"
                                                }
                                                button { class: "btn btn-ghost btn-sm", disabled: busy, title: "Withdraw this prize (e.g. it isn't available)", onclick: { let item = uid.clone(); let prize = prize.clone(); move |_| confirm_first(Irreversible::Withdraw { item: item.clone(), prize: prize.clone() }) }, "Withdraw" }
                                            },
                                            ItemStatus::Drawn if drawable && objected => rsx! {
                                                button { class: "btn btn-warning", disabled: busy, title: if busy { "Wait for the current draw to finish: this could change who it can pick" } else { "They took themselves off the raffle: the prize is drawn again" }, onclick: { let item = uid.clone(); let winner = winner_name.clone(); let prize = prize.clone(); move |_| confirm_first(Irreversible::Declined { item: item.clone(), winner: winner.clone(), prize: prize.clone() }) }, "Declined" }
                                            },
                                            ItemStatus::Drawn if drawable => rsx! {
                                                // The resolution is "claimed" either way: with no
                                                // phone, the organizer's check stands for the code.
                                                ClaimButton { no_phone, held: claiming_this, onclick: { let claim = claim.clone(); let uid = uid.clone(); move |_| { spawn(claim(uid.clone())); } } }
                                                if on_the_spot {
                                                    span { class: "text-xs text-base-content/60", title: "{claimed_means(no_phone)} This prize is handed over on the spot (Prizes tab), so claiming it also marks it delivered.", "Handed over now: claiming marks it delivered." }
                                                } else {
                                                    span { class: "text-xs text-base-content/60", title: "{claimed_means(no_phone)} This prize is sent later (Prizes tab): mark it delivered in the People tab once it reached them.", "Sent later? Mark it delivered in People." }
                                                }
                                                button { class: "btn btn-ghost", disabled: busy || claiming_this, title: if busy { "Wait for the current draw to finish: this could change who it can pick" } else { "" }, onclick: { let item = uid.clone(); let winner = winner_name.clone(); let prize = prize.clone(); move |_| confirm_first(Irreversible::NotHere { item: item.clone(), winner: winner.clone(), prize: prize.clone() }) }, "Not here" }
                                                button { class: "btn btn-ghost", disabled: busy || claiming_this, title: if busy { "Wait for the current draw to finish: this could change who it can pick" } else { "" }, onclick: { let item = uid.clone(); let winner = winner_name.clone(); let prize = prize.clone(); move |_| confirm_first(Irreversible::Declined { item: item.clone(), winner: winner.clone(), prize: prize.clone() }) }, "Declined" }
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
                title: rsx! { if items_left == 0 { "Finish the raffle?" } else { "Finish with {items_left} prize(s) left?" } },
                body: rsx! { "The thank-you screen goes up and nothing more can be drawn. Personal data is erased on the dates entrants were shown; mark prizes delivered as they reach their winners, or erase earlier from the raffle's menu." },
                action: "🎉 Finish",
                on_confirm: move |_| { spawn(finish()); },
            }
            if let Some(cur) = &pending {
                Confirm {
                    open: ask_abandon,
                    title: rsx! { "Abandon the draw on round {cur.round}?" },
                    body: rsx! { "The prize goes back to “to be drawn” and a new draw commits to a new round. The verification page lists the abandoned round so anyone can see whom it would have picked." },
                    action: "Abandon",
                    danger: true,
                    on_confirm: { let abandon = abandon.clone(); let u = cur.item.clone(); move |_| { spawn(abandon(u.clone())); } },
                }
            }
            if let Some(step) = irreversible() {
                Confirm {
                    open: ask_irreversible,
                    title: step.title(),
                    body: step.body(),
                    action: step.action(),
                    danger: true,
                    on_confirm: {
                        let resolve = resolve.clone();
                        let void = void.clone();
                        move |_| {
                            match step.clone() {
                                Irreversible::NotHere { item, .. } => { spawn(resolve(item, Resolution::NotHere)); }
                                Irreversible::Declined { item, .. } => { spawn(resolve(item, Resolution::Declined)); }
                                Irreversible::Withdraw { item, .. } => { spawn(void(item)); }
                            }
                        }
                    },
                }
            }
        }
    }
}

/// A step on a prize that can't be undone, held while the organizer confirms it.
#[derive(Clone, PartialEq)]
enum Irreversible {
    /// The winner isn't here to collect the prize: out of the raffle, the prize is drawn again.
    NotHere {
        item: ItemId,
        winner: String,
        prize: String,
    },
    /// The winner declined the prize: out of the raffle, the prize is drawn again.
    Declined {
        item: ItemId,
        winner: String,
        prize: String,
    },
    /// The prize is withdrawn and never drawn.
    Withdraw { item: ItemId, prize: String },
}

impl Irreversible {
    /// The confirmation's title, the winner's name isolated (`bdi`).
    fn title(&self) -> Element {
        match self {
            Self::NotHere { winner, .. } => rsx! { bdi { "{winner}" } " isn't here?" },
            Self::Declined { winner, .. } => rsx! { bdi { "{winner}" } " declined?" },
            Self::Withdraw { prize, .. } => rsx! { "Withdraw the {prize}?" },
        }
    }

    fn body(&self) -> Element {
        match self {
            Self::NotHere { winner, prize, .. } | Self::Declined { winner, prize, .. } => rsx! {
                bdi { "{winner}" }
                " is out of the raffle, and the {prize} goes back to be drawn again."
            },
            Self::Withdraw { .. } => rsx! { "It won't be drawn." },
        }
    }

    fn action(&self) -> String {
        match self {
            Self::NotHere { .. } => "Not here",
            Self::Declined { .. } => "Declined",
            Self::Withdraw { .. } => "Withdraw",
        }
        .to_owned()
    }
}

/// A drawn winner as the console shows them: the name in full, wrapped
/// however long; the email masked until tapped; "added by you"; and what to
/// check. A joiner shows the claim code on their phone, matched against the
/// one here; someone the organizer added has no phone and never saw a code,
/// so the organizer checks it is them instead (`to_check`: while the item
/// waits for a decision).
#[component]
fn WinnerCard(winner: AdminEntrant, to_check: bool) -> Element {
    let w = winner;
    rsx! {
        div { class: "flex min-w-0 max-w-full items-center gap-3",
            div { class: "min-w-0 text-right",
                p { class: "font-display text-lg font-bold wrap-break-word", bdi { "{w.display_name}" } }
                if let Some(email) = &w.email { MaskedEmail { email: email.clone() } }
                if w.source == Source::Manual { p { class: "text-xs text-base-content/60", "added by you" } }
                if w.objected {
                    p { class: "text-xs font-semibold text-warning", "Took themselves off: decline" }
                } else if w.no_phone && to_check {
                    p { class: "max-w-72 text-xs font-semibold text-info", "Check it's them: ask for the email they registered with, or their badge." }
                }
            }
            if let (Some(code), false) = (&w.claim_code, w.no_phone) {
                span { class: "ticket bg-accent px-5 py-2 text-accent-content",
                    span { class: "claim-code text-lg", "{code}" }
                }
            }
        }
    }
}

/// The button that resolves a winner as claimed: "Claimed" for a joiner,
/// who showed their code, "Confirmed: it's them" for someone with no phone,
/// whom the organizer checked. The resolution is `claimed` either way.
/// `held` while a claim sent from it isn't on the page yet.
#[component]
fn ClaimButton(no_phone: bool, #[props(default)] held: bool, onclick: EventHandler<()>) -> Element {
    rsx! {
        button { class: "btn btn-success", disabled: held, onclick: move |_| onclick.call(()),
            if held { span { class: "loading loading-spinner loading-sm" } } else { Check { class: "size-5" } }
            if no_phone { "Confirmed: it's them" } else { "Claimed" }
        }
    }
}

/// What "Claimed" stands for, for the tooltip by it: a code checked, or,
/// for someone with no phone, the organizer's own check.
fn claimed_means(no_phone: bool) -> &'static str {
    if no_phone {
        "Confirmed means present, and you checked it's them: they have no claim code."
    } else {
        "Claimed means present with a valid code."
    }
}

/// An email as the console shows it until tapped: the first character and
/// the domain ("a•••@example.com"). Consoles get mirrored onto projectors,
/// and the email is how the organizer checks a winner they added, by asking
/// for it: shown in full, it would give the answer away.
fn masked_email(email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    let first: String = local.chars().take(1).collect();
    if domain.is_empty() {
        format!("{first}•••")
    } else {
        format!("{first}•••@{domain}")
    }
}

/// A winner's email, masked until "Show" is tapped.
#[component]
fn MaskedEmail(email: String) -> Element {
    let mut shown = use_signal(|| false);
    rsx! {
        p { class: "flex items-center justify-end gap-1 text-xs text-base-content/60",
            span { class: "break-all", if shown() { "{email}" } else { "{masked_email(&email)}" } }
            button {
                class: "btn btn-ghost btn-xs",
                aria_pressed: if shown() { "true" } else { "false" },
                onclick: move |_| shown.toggle(),
                if shown() { "Hide" } else { "Show" }
            }
        }
    }
}

/// How long a command the engine took holds its buttons at most, waiting
/// for the page to show it: past that, the page missed it or the engine
/// refused it after taking it, and the buttons come back.
const HOLD_MS: u64 = 10_000;

/// What the organizer is told when a draw the engine took never showed up
/// within [`HOLD_MS`]: it didn't start, and drawing again is safe (if it did
/// start after all, drawing that prize again retries its round).
const DRAW_DIDNT_START: &str = "The draw didn't start. Try again.";

/// What a command sent from the console waits to see on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Awaiting {
    /// A draw: the engine takes it before recording it, so the page shows
    /// it only once a reload finds it pending (or already drawn).
    Draw,
    /// A claim: recorded when answered, shown by the reload after it.
    Claim,
}

/// How a command the engine took stopped holding its buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Over {
    /// The page shows it.
    Shown,
    /// [`HOLD_MS`] passed without the page showing it.
    GaveUp,
}

/// A command sent from the console, holding its buttons from the click until
/// the page shows it: a second click before that goes out against a view
/// that doesn't know about the first, and the engine refuses it unseen (or,
/// for another prize's draw, runs it straight after).
#[derive(Clone, Debug, PartialEq)]
struct Sent {
    awaiting: Awaiting,
    item: ItemId,
    /// When the engine took it (ms since the epoch); `None` while sent.
    answered_at: Option<u64>,
}

impl Sent {
    fn new(awaiting: Awaiting, item: ItemId) -> Self {
        Self {
            awaiting,
            item,
            answered_at: None,
        }
    }

    fn answer(&mut self) {
        self.answered_at = Some(crate::ui::now_ms());
    }

    /// Whether the request is still out: a click meanwhile is ignored, as
    /// the button it came from may not be disabled yet.
    fn out(&self) -> bool {
        self.answered_at.is_none()
    }

    /// Whether `v` shows it: a draw pending (any: only one can be) or its
    /// prize no longer to be drawn; a claim's prize no longer waiting for
    /// its winner.
    fn shown(&self, v: &RaffleView) -> bool {
        let status = v
            .items
            .iter()
            .find(|u| u.item == self.item)
            .map(|u| u.status);
        match self.awaiting {
            Awaiting::Draw => v.current.is_some() || status != Some(ItemStatus::Available),
            Awaiting::Claim => status != Some(ItemStatus::Drawn),
        }
    }

    /// Whether it stopped holding at `now_ms`, the page showing it or not
    /// (`shown`): never while the request is out.
    fn over(&self, shown: bool, now_ms: u64) -> Option<Over> {
        let at = self.answered_at?;
        if shown {
            Some(Over::Shown)
        } else if now_ms.saturating_sub(at) >= HOLD_MS {
            Some(Over::GaveUp)
        } else {
            None
        }
    }

    /// Whether its buttons are held at `now_ms` with the page showing `v`.
    fn holds(&self, v: &RaffleView, now_ms: u64) -> bool {
        self.over(self.shown(v), now_ms).is_none()
    }

    /// What to tell the organizer when it stopped holding as `over`.
    fn notice(&self, over: Over) -> Option<&'static str> {
        (self.awaiting == Awaiting::Draw && over == Over::GaveUp).then_some(DRAW_DIDNT_START)
    }
}

/// Forgets `sent` once it stopped holding, saying so on `error` if it never
/// showed up: kept, a draw shown and later put back to be drawn (declined)
/// would hold its prize again. `shown` is [`Sent::shown`] for the page now.
fn use_settle(
    mut sent: Signal<Option<Sent>>,
    shown: bool,
    now: Signal<u64>,
    mut error: Signal<Option<String>>,
) {
    use_effect(use_reactive((&shown,), move |(shown,)| {
        let Some(s) = sent.read().clone() else {
            return;
        };
        if let Some(over) = s.over(shown, now()) {
            sent.set(None);
            if let Some(notice) = s.notice(over) {
                error.set(Some(notice.to_owned()));
            }
        }
    }));
}

fn status_label(s: ItemStatus) -> &'static str {
    match s {
        ItemStatus::Available => "to be drawn",
        ItemStatus::Drawn => "drawn: waiting for the winner",
        ItemStatus::Claimed => "claimed",
        ItemStatus::Voided => "withdrawn",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn winner(source: &str, no_phone: bool) -> AdminEntrant {
        serde_json::from_value(serde_json::json!({
            "id": "e1", "display_name": "Anna Kovács", "email": "anna@example.com",
            "source": source, "hidden": false, "claim_code": "K2P9", "created_at": 0,
            "delivered": false, "erased": false, "outcome": "drawn", "prize": "Book",
            "no_phone": no_phone
        }))
        .unwrap()
    }

    #[test]
    fn emails_are_masked() {
        assert_eq!(masked_email("anna@example.com"), "a•••@example.com");
        assert_eq!(masked_email("ágnes@példa.hu"), "á•••@példa.hu");
        assert_eq!(masked_email("a@b"), "a•••@b");
        assert_eq!(masked_email("nobody"), "n•••");
    }

    /// Someone the organizer added: masked email, "added by you", the
    /// check to make, no code to compare. A joiner: their code.
    #[test]
    fn a_winner_to_check() {
        let html = dioxus_ssr::render_element(
            rsx! { WinnerCard { winner: winner("manual", true), to_check: true } },
        );
        for present in [
            "<bdi>Anna Kovács</bdi>",
            "a•••@example.com",
            "Show",
            "added by you",
            "Check it&#39;s them: ask for the email they registered with, or their badge.",
        ] {
            assert!(html.contains(present), "{present}: {html}");
        }
        for absent in ["anna@example.com", "K2P9"] {
            assert!(!html.contains(absent), "{absent}: {html}");
        }

        let html = dioxus_ssr::render_element(
            rsx! { WinnerCard { winner: winner("self", false), to_check: true } },
        );
        assert!(
            html.contains("K2P9") && html.contains("a•••@example.com"),
            "{html}"
        );
        for absent in ["anna@example.com", "added by you", "Check it"] {
            assert!(!html.contains(absent), "{absent}: {html}");
        }
    }

    /// The button, its handler made inside the runtime.
    #[component]
    fn Claim(no_phone: bool, held: bool) -> Element {
        rsx! { ClaimButton { no_phone, held, onclick: |_| {} } }
    }

    #[test]
    fn the_claim_button_says_what_was_checked() {
        let html = dioxus_ssr::render_element(rsx! { Claim { no_phone: true, held: false } });
        assert!(html.contains("Confirmed: it&#39;s them"), "{html}");
        let html = dioxus_ssr::render_element(rsx! { Claim { no_phone: false, held: false } });
        assert!(
            html.contains("Claimed") && !html.contains("Confirmed"),
            "{html}"
        );
        assert!(!html.contains("disabled"), "{html}");
        let html = dioxus_ssr::render_element(rsx! { Claim { no_phone: false, held: true } });
        assert!(html.contains("disabled"), "{html}");
    }

    /// A view with `item-1` in `status`, and a draw pending if `pending`.
    fn view(status: &str, pending: bool) -> RaffleView {
        let current = pending.then(|| {
            serde_json::json!({
                "k": 0, "item": "item-1", "prize_name": "Book", "round": 7, "round_time": 0
            })
        });
        serde_json::from_value(serde_json::json!({
            "v": 1, "follow": { "kind": "poll", "every_ms": 2500 }, "raffle_id": "r1",
            "slug": "meetup", "title": "Meetup", "description": "", "state": "drawing",
            "join_code": "ABCDEF", "join_url": "https://rafi.example/j/ABCDEF", "entrant_count": 2,
            "recent_names": [], "prizes": [],
            "items": [{
                "item": "item-1", "prize_id": "p1", "prize_name": "Book", "status": status,
                "entrant": null, "display_name": null
            }],
            "current": current, "draws": [], "last_failure": null, "manifest_hash": null,
            "server_now_ms": 0, "test_mode": false, "verifiable": true, "registration": true,
            "organizer": "Cloud Native Budapest", "privacy_url": "https://rafi.example/privacy/meetup"
        }))
        .unwrap()
    }

    fn sent(awaiting: Awaiting, answered_at: Option<u64>) -> Sent {
        Sent {
            awaiting,
            item: "item-1".into(),
            answered_at,
        }
    }

    /// A draw holds the buttons while it is sent, and after the engine took
    /// it until the page shows it pending or drawn, or for HOLD_MS at most.
    #[test]
    fn a_draw_holds_until_the_page_shows_it() {
        let available = view("available", false);
        let draw = |at| sent(Awaiting::Draw, at);
        assert!(draw(None).holds(&available, u64::MAX), "sent");
        assert!(
            draw(Some(1_000)).holds(&available, 1_000),
            "taken, not shown"
        );
        assert!(draw(Some(1_000)).holds(&available, 1_000 + HOLD_MS - 1));
        assert!(
            !draw(Some(1_000)).holds(&available, 1_000 + HOLD_MS),
            "given up"
        );
        assert!(
            !draw(Some(1_000)).holds(&view("available", true), 1_000),
            "pending"
        );
        assert!(
            !draw(Some(1_000)).holds(&view("drawn", false), 1_000),
            "drawn"
        );
        assert!(!draw(Some(1_000)).holds(&view("voided", false), 1_000));
    }

    /// A claim holds its row while it is sent, and after the engine took it
    /// until the page shows the prize no longer waiting for its winner.
    #[test]
    fn a_claim_holds_until_the_page_shows_it() {
        let drawn = view("drawn", false);
        let claim = |at| sent(Awaiting::Claim, at);
        assert!(claim(None).holds(&drawn, u64::MAX), "sent");
        assert!(claim(Some(1_000)).holds(&drawn, 1_000), "taken, not shown");
        assert!(
            !claim(Some(1_000)).holds(&drawn, 1_000 + HOLD_MS),
            "given up"
        );
        assert!(
            !claim(Some(1_000)).holds(&view("claimed", false), 1_000),
            "claimed"
        );
    }

    /// Only a request still out ignores another click: once answered, the
    /// page's hold disables the buttons instead.
    #[test]
    fn a_click_is_ignored_while_the_request_is_out() {
        for awaiting in [Awaiting::Draw, Awaiting::Claim] {
            assert!(sent(awaiting, None).out());
            assert!(!sent(awaiting, Some(1_000)).out());
        }
    }

    /// A command stops holding once shown, or after HOLD_MS; a draw that
    /// never showed up says so, a claim (recorded when answered) doesn't.
    #[test]
    fn what_ends_a_hold_and_what_the_organizer_is_told() {
        let draw = sent(Awaiting::Draw, Some(1_000));
        assert_eq!(sent(Awaiting::Draw, None).over(true, u64::MAX), None);
        assert_eq!(draw.over(false, 1_000 + HOLD_MS - 1), None);
        assert_eq!(draw.over(true, 1_000), Some(Over::Shown));
        assert_eq!(draw.over(true, 1_000 + HOLD_MS), Some(Over::Shown));
        assert_eq!(draw.over(false, 1_000 + HOLD_MS), Some(Over::GaveUp));
        assert_eq!(draw.notice(Over::GaveUp), Some(DRAW_DIDNT_START));
        assert_eq!(draw.notice(Over::Shown), None);
        let claim = sent(Awaiting::Claim, Some(1_000));
        assert_eq!(claim.notice(Over::GaveUp), None);
        assert_eq!(claim.notice(Over::Shown), None);
    }

    /// A draw shows as any draw pending or its prize moved on; a claim as
    /// its prize no longer waiting for the winner.
    #[test]
    fn what_the_page_shows() {
        let draw = sent(Awaiting::Draw, Some(1_000));
        assert!(!draw.shown(&view("available", false)));
        assert!(draw.shown(&view("available", true)));
        assert!(draw.shown(&view("drawn", false)));
        let claim = sent(Awaiting::Claim, Some(1_000));
        assert!(!claim.shown(&view("drawn", false)));
        assert!(claim.shown(&view("claimed", false)));
        // Put back to be drawn (declined): no longer the claim's to wait for.
        assert!(claim.shown(&view("available", false)));
    }

    #[test]
    fn a_held_claim_button_spins() {
        let html = dioxus_ssr::render_element(rsx! { Claim { no_phone: false, held: true } });
        assert!(html.contains("loading-spinner"), "{html}");
        let html = dioxus_ssr::render_element(rsx! { Claim { no_phone: false, held: false } });
        assert!(!html.contains("loading-spinner"), "{html}");
    }
}
