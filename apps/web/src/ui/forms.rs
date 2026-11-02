//! Organizer forms: dioform for typed state, validation and managed async
//! submit; the dioxus-daisyui-components registry for the fields, which read
//! dioform's field metadata (errors, required, touched) through `dioxus-field`.

use dioform::prelude::*;
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    BookmarkCheck, BookmarkPlus, Check, Gift, Plus, Smartphone, Sparkles, TriangleAlert, Undo2,
};
use rafi_lifecycle::{
    entrant::{self, MAX_ADDED_NAME_CHARS, Origin},
    prize::Delivery,
    settings::{self, MAX_PRIZE_QTY, MAX_TITLE_CHARS},
};

use crate::{
    api::{self, ApiError},
    components::{
        button::{Button, ButtonColor, ButtonSize},
        input::{InputField, InputSize},
        switch::{SwitchColor, SwitchField},
        textarea::TextareaField,
        toast::{ToastOptions, use_toast},
    },
    ui::{
        command_id, dates,
        problem::{Problem, Problems},
    },
};
use rafi_api::{
    AddRequest, AddRow, PrizeInput, RaffleDefaults, RaffleSettings, RemovedAtRequest, Retention,
    SkipReason,
};

/// What the organizer reads for a failed call: its message, and for a
/// server error the request's ID, which the operator finds in the log.
pub fn err_text(e: ApiError) -> String {
    match e.request_id {
        Some(id) => format!("{} (reference: {id})", e.message),
        None => e.message,
    }
}

/// Form-level errors (e.g. the server rejected the submission).
#[component]
fn FormErrors(errors: Vec<String>) -> Element {
    rsx! {
        for e in errors {
            div { role: "alert", class: "alert alert-error alert-soft py-2 text-sm", TriangleAlert { class: "size-4" } "{e}" }
        }
    }
}

fn messages<E: std::fmt::Display + Clone>(errors: Vec<ValidationErrorSnapshot<E>>) -> Vec<String> {
    errors.into_iter().map(|e| e.error().to_string()).collect()
}

fn required<M: 'static>(
    message: &'static str,
) -> impl for<'a> Fn(&'a String, ValidatorContext<'a, M>) -> Vec<String> + 'static {
    move |value: &String, _ctx: ValidatorContext<'_, M>| {
        if value.trim().is_empty() {
            vec![message.to_owned()]
        } else {
            Vec::new()
        }
    }
}

/// Blank, or an email address by the same rule the server applies
/// ([`settings::email`]).
fn optional_email<M: 'static>()
-> impl for<'a> Fn(&'a String, ValidatorContext<'a, M>) -> Vec<String> + 'static {
    |value: &String, _ctx: ValidatorContext<'_, M>| {
        if settings::email(value).is_some() {
            Vec::new()
        } else {
            vec!["That does not look like an email.".to_owned()]
        }
    }
}

fn optional_https_url<M: 'static>()
-> impl for<'a> Fn(&'a String, ValidatorContext<'a, M>) -> Vec<String> + 'static {
    |value: &String, _ctx: ValidatorContext<'_, M>| {
        if settings::url(value).is_some() {
            Vec::new()
        } else {
            vec!["An absolute https:// link.".to_owned()]
        }
    }
}

// ---------------------------------------------------------------------------
// New raffle
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Form)]
struct NewRaffle {
    title: String,
    /// The event day, `YYYY-MM-DD`; today unless changed.
    event_date: String,
}

#[component]
pub fn CreateRaffleForm(on_created: EventHandler<String>) -> Element {
    let form = use_form_handle(|| {
        let handle = FormHandle::<NewRaffle>::from_config(
            FormConfig::new(NewRaffle {
                title: String::new(),
                event_date: dates::today_iso(),
            })
            .id_namespace("new-raffle")
            .validation_mode(ValidationMode::on_submit()),
        );
        handle.write_advanced(|core| {
            core.register_sync_field_validator(
                NewRaffle::fields().title(),
                "required",
                required("Give your raffle a name."),
            );
            core.register_sync_field_validator(
                NewRaffle::fields().event_date(),
                "required",
                required("When is the event?"),
            );
        });
        handle
    });
    let fields = NewRaffle::fields();
    let submit = form.managed_submit();
    let form_errors = messages(form.visible_form_validation_errors());
    let busy = form.is_submitting();
    let reset = form.clone();

    rsx! {
        form {
            class: "grid gap-x-3 sm:grid-cols-[1fr_auto_auto] sm:items-start",
            novalidate: true,
            onsubmit: move |event| {
                let reset = reset.clone();
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<NewRaffle>| async move {
                    let NewRaffle { title, event_date } = s.value().clone();
                    let ends_at = dates::end_of_day_local(&event_date);
                    match api::create_raffle(title, event_date, ends_at).await {
                        Ok(id) => {
                            reset.reinitialize(NewRaffle {
                                title: String::new(),
                                event_date: dates::today_iso(),
                            });
                            on_created.call(id);
                            SubmitErrors::none()
                        }
                        Err(e) => SubmitError::form(err_text(e)).into(),
                    }
                });
            },
            InputField {
                context: form.text(fields.title()),
                label: "New raffle",
                size: InputSize::Lg,
                required: true,
                maxlength: "{MAX_TITLE_CHARS}",
                placeholder: "e.g. “Cloud Native Budapest — October”",
                prefix: rsx! { Sparkles { class: "size-5 text-secondary" } },
                wrapper_attributes: dioxus_primitives::dioxus_attributes::attributes!(span { class: "w-full" }),
            }
            InputField {
                context: form.text(fields.event_date()),
                label: "Event date",
                size: InputSize::Lg,
                r#type: "date",
                required: true,
                title: "The day of the draw. Entrants' data is erased a set number of days after it.",
            }
            // The label (1.5rem line) and the Field's gap-2 sit above the input.
            div { class: "sm:pt-8",
                Button { color: ButtonColor::Primary, size: ButtonSize::Lg, class: "hover-wiggle", r#type: "submit", disabled: busy,
                    if busy { span { class: "loading loading-spinner loading-sm" } } else { Plus { class: "size-5" } }
                    "Create"
                }
            }
            div { class: "sm:col-span-3", FormErrors { errors: form_errors } }
        }
    }
}

// ---------------------------------------------------------------------------
// Prizes
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Form)]
struct Prize {
    name: String,
    sponsor: String,
    qty: u32,
}

impl Default for Prize {
    fn default() -> Self {
        Self {
            name: String::new(),
            sponsor: String::new(),
            qty: 1,
        }
    }
}

fn prize_form(namespace: String, initial: Prize) -> FormHandle<Prize> {
    let handle = FormHandle::<Prize>::from_config(
        FormConfig::new(initial)
            .id_namespace(namespace)
            .validation_mode(ValidationMode::on_submit()),
    );
    handle.write_advanced(|core| {
        let f = Prize::fields();
        core.register_sync_field_validator(f.name(), "required", required("What is the prize?"));
        core.register_sync_field_validator(f.qty(), "range", |v: &u32, _| {
            if settings::prize_qty(*v) {
                Vec::new()
            } else {
                vec![format!("Between 1 and {MAX_PRIZE_QTY}.")]
            }
        });
    });
    handle
}

/// How a prize reaches its winner, in the organizer's words.
pub fn delivery_label(d: Delivery) -> &'static str {
    match d {
        Delivery::OnTheSpot => "On the spot",
        Delivery::Later => "Sent later",
    }
}

/// What choosing `d` means for the claim, or what the choice decides.
pub fn delivery_hint(d: Option<Delivery>) -> &'static str {
    match d {
        Some(Delivery::OnTheSpot) => {
            "Handed over at the event when the winner shows their code. Pressing Claimed also marks it delivered."
        }
        Some(Delivery::Later) => {
            "Sent after the raffle, like a voucher by email: mark it delivered in People once it reached them. Their data is kept until then."
        }
        None => "Pick one: it decides whether “Claimed” also marks the prize delivered.",
    }
}

/// The two ways a prize reaches its winner, as a pair of buttons, `value`
/// pressed (if any).
#[component]
pub fn DeliveryPicker(
    value: Option<Delivery>,
    on_pick: EventHandler<Delivery>,
    /// Buttons' size class (`btn-sm`, `btn-xs`).
    #[props(default = "btn-sm")]
    size: &'static str,
    #[props(default)] disabled: bool,
    /// What the group is about, for screen readers.
    #[props(into, default = String::from("How the winner gets the prize"))]
    label: String,
) -> Element {
    // Native radios, styled as buttons (daisyUI shows their aria-label): the
    // browser gives them the keyboard (arrow keys within the group) and says
    // which is checked.
    let name = use_hook(|| format!("delivery-{}", uuid::Uuid::new_v4().simple()));
    rsx! {
        div { class: "join", role: "radiogroup", aria_label: label,
            for d in [Delivery::OnTheSpot, Delivery::Later] {
                input {
                    key: "{d.as_str()}",
                    r#type: "radio",
                    name: "{name}",
                    class: "btn join-item {size} checked:btn-secondary",
                    aria_label: delivery_label(d),
                    checked: value == Some(d),
                    disabled,
                    onchange: move |_| on_pick.call(d),
                }
            }
        }
    }
}

/// Adds a prize, or edits an existing one when `prize_id` is set. A new
/// prize's delivery has to be picked: a prize said to be sent later tells
/// its winner's phone so, which would be wrong for one handed over.
#[component]
pub fn PrizeForm(
    raffle_id: String,
    #[props(default)] prize_id: Option<String>,
    #[props(default)] name: String,
    #[props(default)] sponsor: String,
    /// Not edited here, but kept: the server stores what it is sent.
    #[props(default)]
    description: String,
    #[props(default = 1)] qty: u32,
    #[props(default)] position: i64,
    #[props(default)] delivery: Option<Delivery>,
    on_saved: EventHandler<()>,
    #[props(default)] on_cancel: Option<EventHandler<()>>,
) -> Element {
    let namespace = format!("prize-{}", prize_id.as_deref().unwrap_or("new"));
    let initial = Prize {
        name: name.clone(),
        sponsor: sponsor.clone(),
        qty,
    };
    let form = use_form_handle(move || prize_form(namespace, initial));
    let mut picked = use_signal(|| delivery);
    let fields = Prize::fields();
    let qty_binding = use_number(&form, fields.qty());
    let submit = form.managed_submit();
    let editing = prize_id.is_some();
    let busy = form.is_submitting();
    let form_err = messages(form.visible_form_validation_errors());
    let reset = form.clone();
    let toast = use_toast();

    rsx! {
        form {
            class: "grid gap-x-3 sm:grid-cols-[1fr_12rem_7rem_auto] sm:items-start",
            novalidate: true,
            onsubmit: move |event| {
                let (raffle_id, prize_id, reset) = (raffle_id.clone(), prize_id.clone(), reset.clone());
                let description = description.clone();
                let delivery = picked();
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<Prize>| async move {
                    let Some(delivery) = delivery else {
                        return SubmitError::form("How does the winner get it: on the spot, or sent later?".to_owned()).into();
                    };
                    let v = s.value().clone();
                    let input = PrizeInput {
                        id: prize_id.clone(),
                        name: v.name.clone(),
                        sponsor: v.sponsor,
                        description,
                        qty: v.qty,
                        position,
                        delivery,
                    };
                    match api::save_prize(raffle_id, input).await {
                        Ok(_) => {
                            if prize_id.is_none() {
                                reset.reinitialize(Prize::default());
                                picked.set(None);
                                toast.success(format!("{} added", v.name), ToastOptions::new());
                            }
                            on_saved.call(());
                            SubmitErrors::none()
                        }
                        Err(e) => SubmitError::form(err_text(e)).into(),
                    }
                });
            },
            InputField {
                context: form.text(fields.name()),
                label: "Prize",
                required: true,
                maxlength: "{MAX_TITLE_CHARS}",
                placeholder: "e.g. KCD Budapest ticket",
                class: "w-full",
            }
            InputField {
                context: form.text(fields.sponsor()),
                label: "Sponsor",
                placeholder: "optional",
                class: "w-full",
            }
            InputField {
                context: qty_binding,
                label: "How many",
                r#type: "number",
                min: "1",
                max: "{MAX_PRIZE_QTY}",
                inputmode: "numeric",
                class: "w-full",
            }
            div { class: "flex flex-col gap-2 py-2 sm:col-span-3 sm:col-start-1",
                span { class: "text-sm font-medium", "How does the winner get it?" }
                DeliveryPicker { value: picked(), on_pick: move |d| picked.set(Some(d)) }
                p { class: "text-xs text-base-content/60", "{delivery_hint(picked())}" }
            }
            div { class: "flex gap-2 sm:pt-8",
                Button { color: if editing { ButtonColor::Primary } else { ButtonColor::Secondary }, r#type: "submit", disabled: busy,
                    if editing { Check { class: "size-5" } "Save" } else { Gift { class: "size-5" } "Add prize" }
                }
                if let Some(cancel) = on_cancel {
                    Button { class: "btn-ghost", r#type: "button", onclick: move |_| cancel.call(()), "Cancel" }
                }
            }
            div { class: "sm:col-span-4", FormErrors { errors: form_err } }
        }
    }
}

// ---------------------------------------------------------------------------
// Adding one person
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Form)]
struct Person {
    name: String,
    email: String,
    /// The organizer approved a name the blocklist refuses: it is someone's
    /// real name. Offered only for such a name.
    approved: bool,
    /// The organizer adds them though they were removed at their request:
    /// offered only for an email that was.
    lift_removal: bool,
}

/// One person the organizer adds (the Add people dialog's first tab): an
/// add of one row. Its batch ID is made with the form and again after each
/// add that went in, so a submission whose reply was lost (a network error)
/// is sent again with the same ID, and the server answers the first add
/// instead of adding the person twice. An email of someone `removed` at
/// their request is said so, and they are added only if the organizer adds
/// them anyway.
#[component]
pub fn AddPersonForm(
    raffle_id: String,
    /// Who was removed at their request, as the admin's data has them.
    #[props(default)]
    removed: Vec<RemovedAtRequest>,
    on_added: EventHandler<()>,
) -> Element {
    let form = use_form_handle(|| {
        let handle = FormHandle::<Person>::from_config(
            FormConfig::new(Person::default())
                .id_namespace("add-person")
                .validation_mode(ValidationMode::on_submit()),
        );
        handle.write_advanced(|core| {
            let f = Person::fields();
            core.register_sync_field_validator(f.name(), "required", required("Who is it?"));
            core.register_sync_field_validator(
                f.name(),
                "length",
                |v: &String, ctx: ValidatorContext<'_, Person>| match entrant::added_name(v) {
                    Err(entrant::NameError::Invalid) if !v.trim().is_empty() => {
                        vec![format!(
                            "{MAX_ADDED_NAME_CHARS} characters at most, no control characters."
                        )]
                    }
                    Err(entrant::NameError::Blocked) if !ctx.form().approved => {
                        vec![
                            "This name has a word the blocklist refuses. If it is their real name, approve it below."
                                .to_owned(),
                        ]
                    }
                    _ => Vec::new(),
                },
            );
            core.register_sync_field_validator(f.email(), "email", optional_email());
        });
        handle
    });
    let mut batch_id = use_signal(command_id);
    let fields = Person::fields();
    // The blocklist's substring matching catches real names (Nazir,
    // Buzinkay): such a name can be approved, which lifts the blocklist
    // alone.
    let blocked =
        entrant::added_name(&form.text(fields.name()).value()) == Err(entrant::NameError::Blocked);
    // Removed at their request, by this email: the server leaves them out
    // unless lifted.
    let was_removed = entrant::Email::parse(&form.text(fields.email()).value())
        .is_some_and(|e| removed.iter().any(|r| r.matches(Some(&e.key), None)));
    let submit = form.managed_submit();
    let busy = form.is_submitting();
    let form_err = messages(form.visible_form_validation_errors());
    let reset = form.clone();
    let toast = use_toast();

    rsx! {
        form {
            class: "grid gap-x-3 sm:grid-cols-[1fr_1fr_auto] sm:items-start",
            novalidate: true,
            onsubmit: move |event| {
                let (raffle_id, reset) = (raffle_id.clone(), reset.clone());
                let batch = batch_id();
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<Person>| async move {
                    let v = s.value().clone();
                    let name = v.name.trim().to_owned();
                    let add = AddRequest {
                        batch_id: batch,
                        origin: Origin::One,
                        rows: vec![AddRow {
                            name: name.clone(),
                            email: Some(v.email.trim().to_owned()).filter(|e| !e.is_empty()),
                            list_ref: None,
                            approved: v.approved,
                            lift_removal: v.lift_removal,
                        }],
                        name_format: None,
                    };
                    match api::add_entrants(raffle_id, add).await {
                        // Skipped: someone with this email is in already, or
                        // was removed at their request.
                        Ok(result) if result.added.is_empty() => {
                            let removed = result.skipped.iter().any(|s| s.reason == SkipReason::RemovedAtRequest);
                            SubmitError::form(if removed {
                                "They were removed at their request. Add them anyway only if they told you they changed their mind.".to_owned()
                            } else {
                                "This email is already entered.".to_owned()
                            })
                            .into()
                        }
                        Ok(_) => {
                            reset.reinitialize(Person::default());
                            batch_id.set(command_id());
                            toast.success(format!("{name} is in"), ToastOptions::new());
                            on_added.call(());
                            SubmitErrors::none()
                        }
                        Err(e) => SubmitError::form(err_text(e)).into(),
                    }
                });
            },
            InputField {
                context: form.text(fields.name()),
                label: "Name",
                required: true,
                maxlength: "{MAX_ADDED_NAME_CHARS}",
                placeholder: "Shown on the screen",
                description: format!("As they'd recognize it, a full name too: up to {MAX_ADDED_NAME_CHARS} characters."),
                prefix: rsx! { Smartphone { class: "size-4 opacity-60" } },
                wrapper_attributes: dioxus_primitives::dioxus_attributes::attributes!(span { class: "w-full" }),
            }
            InputField {
                context: form.text(fields.email()),
                label: "Email",
                r#type: "email",
                placeholder: "optional, to deliver the prize",
                class: "w-full",
            }
            div { class: "sm:pt-8",
                Button { color: ButtonColor::Secondary, r#type: "submit", disabled: busy,
                    Plus { class: "size-5" }
                    "Add"
                }
            }
            if was_removed {
                div { class: "sm:col-span-3",
                    SwitchField {
                        context: form.field(fields.lift_removal()),
                        label: "They were removed at their request: add them anyway",
                        description: "They asked to be taken off this raffle. Add them again only if they told you they changed their mind.",
                        color: SwitchColor::Warning,
                    }
                }
            }
            if blocked {
                div { class: "sm:col-span-3",
                    SwitchField {
                        context: form.field(fields.approved()),
                        label: "It's their real name",
                        description: "The blocklist matches parts of words, so it refuses some real names. Approving lifts it for this name only.",
                        color: SwitchColor::Warning,
                    }
                }
            }
            div { class: "sm:col-span-3", FormErrors { errors: form_err } }
        }
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Form)]
struct Settings {
    title: String,
    description: String,
    event_date: String,
    /// Days after the event date, as typed.
    retention_days: String,
    organizer_name: String,
    contact_email: String,
    privacy_email: String,
    privacy_url: String,
    privacy_text: String,
    rules: String,
    test_mode: bool,
    registration: bool,
}

impl From<&RaffleSettings> for Settings {
    fn from(s: &RaffleSettings) -> Self {
        Self {
            title: s.title.clone(),
            description: s.description.clone(),
            event_date: s.event_date.clone(),
            retention_days: s.retention_days.to_string(),
            organizer_name: s.organizer_name.clone(),
            contact_email: s.contact_email.clone(),
            privacy_email: s.privacy_email.clone(),
            privacy_url: s.privacy_url.clone(),
            privacy_text: s.privacy_text.clone(),
            rules: s.rules.clone(),
            test_mode: s.test_mode,
            registration: s.registration,
        }
    }
}

impl From<Settings> for RaffleSettings {
    fn from(s: Settings) -> Self {
        // The end of the event day where the organizer is; the server keeps
        // the stored instant when the day itself didn't change.
        let event_ends_at = dates::end_of_day_local(&s.event_date);
        Self {
            title: s.title,
            description: s.description,
            organizer_name: s.organizer_name,
            contact_email: s.contact_email,
            privacy_email: s.privacy_email,
            privacy_text: s.privacy_text,
            privacy_url: s.privacy_url,
            rules: s.rules,
            event_date: s.event_date,
            event_ends_at,
            retention_days: s.retention_days.trim().parse().unwrap_or(-1),
            test_mode: s.test_mode,
            registration: s.registration,
        }
    }
}

#[component]
pub fn SettingsForm(
    raffle_id: String,
    settings: RaffleSettings,
    /// Whether the fixed settings (the additional rules, the event date, the
    /// retention period, the privacy text and link) can still change: not
    /// once a real entrant is in
    /// ([`locks_settings`](rafi_lifecycle::entrant::Source::locks_settings)).
    rules_editable: bool,
    /// Whether registration can be switched now
    /// ([`AdminActions::edit_registration`](rafi_api::AdminActions::edit_registration)):
    /// before the freeze, not while it is open.
    #[props(default = true)]
    registration_editable: bool,
    /// The installation's periods, for the retention field.
    retention: Retention,
    /// Whether the signed-in user may change settings at all: an
    /// organization's members only run raffles. Everything is read-only
    /// otherwise.
    #[props(default = true)]
    can_edit: bool,
    /// Shown after the privacy fields: what goes with them (the paragraph
    /// to send the people the organizer adds).
    #[props(default = VNode::empty())]
    after_privacy: Element,
    on_saved: EventHandler<()>,
) -> Element {
    let initial = Settings::from(&settings);
    let max_days = retention.max_days;
    // Fixed settings stay editable only while nobody has joined, and only
    // for someone who may edit at all.
    let rules_editable = rules_editable && can_edit;
    let registration_editable = registration_editable && can_edit;
    let form = use_form_handle(move || {
        let handle = FormHandle::<Settings>::from_config(
            FormConfig::new(initial)
                .id_namespace("settings")
                .validation_mode(ValidationMode::on_commit()),
        );
        handle.write_advanced(|core| {
            let f = Settings::fields();
            core.register_sync_field_validator(
                f.title(),
                "required",
                required("The raffle needs a title."),
            );
            core.register_sync_field_validator(
                f.event_date(),
                "required",
                required("The raffle needs an event date."),
            );
            core.register_sync_field_validator(f.contact_email(), "email", optional_email());
            core.register_sync_field_validator(f.privacy_email(), "email", optional_email());
            core.register_sync_field_validator(f.privacy_url(), "url", optional_https_url());
            core.register_sync_field_validator(
                f.retention_days(),
                "range",
                move |value: &String, _ctx: ValidatorContext<'_, _>| match value
                    .trim()
                    .parse::<i64>()
                {
                    Ok(d) if (0..=max_days).contains(&d) => Vec::new(),
                    _ => vec![format!("Between 0 and {max_days} days.")],
                },
            );
        });
        handle
    });
    let toast = use_toast();

    // Follow server-side changes (e.g. another tab) unless there are local edits.
    // Tracked outside reactivity: an effect would subscribe to the form state it
    // resets and loop forever.
    let last_seen = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(settings.clone())));
    if *last_seen.borrow() != settings {
        *last_seen.borrow_mut() = settings.clone();
        let (sync, next) = (form.clone(), Settings::from(&settings));
        spawn(async move {
            if !sync.is_dirty() {
                sync.reinitialize(next);
            }
        });
    }

    let f = Settings::fields();
    let submit = form.managed_submit();
    let dirty = form.is_dirty();
    let busy = form.is_submitting();
    let form_err = messages(form.visible_form_validation_errors());
    let rebase = form.clone();
    let reset = form.clone();
    let as_defaults = form.clone();
    let from_defaults = form.clone();
    let mut saving_defaults = use_signal(|| false);
    // Loading or saving the defaults failed: shown by the buttons.
    let mut defaults_error = use_signal(|| None::<Problem>);
    // Fills the organizer, contact and rules (unless they're fixed) from the
    // owner's defaults, to save with the rest.
    let fill_defaults = move |_| {
        let form = from_defaults.clone();
        spawn(async move {
            match api::get_defaults().await {
                Ok(d) if d == RaffleDefaults::default() => {
                    toast.info("No defaults saved yet.".to_owned(), ToastOptions::new());
                }
                Ok(d) => {
                    let f = Settings::fields();
                    form.text(f.organizer_name()).set_value(d.organizer_name);
                    form.text(f.contact_email()).set_value(d.contact_email);
                    form.text(f.privacy_email()).set_value(d.privacy_email);
                    if rules_editable {
                        form.textarea(f.rules()).set_value(d.rules);
                        form.textarea(f.privacy_text()).set_value(d.privacy_text);
                        form.text(f.privacy_url()).set_value(d.privacy_url);
                        if let Some(days) = d.retention_days {
                            form.text(f.retention_days()).set_value(days.to_string());
                        }
                    }
                    if registration_editable {
                        form.set_field(f.registration(), d.registration);
                    }
                }
                Err(e) => defaults_error.set(Some(e.into())),
            }
        });
    };
    // The organizer, contact and rules as the form has them (saved or not):
    // what the owner's next raffles start with.
    let save_defaults = move |_| {
        let s = as_defaults.snapshot();
        spawn(async move {
            saving_defaults.set(true);
            let res = api::save_defaults(RaffleDefaults {
                organizer_name: s.organizer_name,
                contact_email: s.contact_email,
                privacy_email: s.privacy_email,
                privacy_text: s.privacy_text,
                privacy_url: s.privacy_url,
                rules: s.rules,
                retention_days: s.retention_days.trim().parse().ok(),
                registration: s.registration,
            })
            .await;
            saving_defaults.set(false);
            match res {
                Ok(()) => toast.success(
                    "📌 New raffles will start with this organizer, contacts, privacy text, rules, retention period and registration".to_owned(),
                    ToastOptions::new(),
                ),
                Err(e) => defaults_error.set(Some(e.into())),
            };
        });
    };

    rsx! {
        form {
            class: "grid gap-x-6 gap-y-1 md:grid-cols-2",
            novalidate: true,
            onsubmit: move |event| {
                let (raffle_id, rebase) = (raffle_id.clone(), rebase.clone());
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<Settings>| async move {
                    let value = s.value().clone();
                    match api::save_settings(raffle_id, value.clone().into()).await {
                        Ok(()) => {
                            rebase.reinitialize(value);
                            toast.success("✨ Settings saved".to_owned(), ToastOptions::new());
                            on_saved.call(());
                            SubmitErrors::none()
                        }
                        Err(e) => SubmitError::form(err_text(e)).into(),
                    }
                });
            },
            div { class: "md:col-span-2",
                InputField { context: form.text(f.title()), label: "Title", required: true, maxlength: "{MAX_TITLE_CHARS}", disabled: !can_edit, class: "w-full" }
            }
            div { class: "md:col-span-2",
                TextareaField {
                    context: form.textarea(f.description()),
                    label: "Description",
                    description: "Shown on the intro screen.",
                    rows: "2",
                    disabled: !can_edit,
                    class: "w-full",
                }
            }
            InputField {
                context: form.text(f.organizer_name()),
                label: "Organizer (data controller)",
                description: "Named in the rules and the privacy notice. Needed before registration can open, or the list be frozen.",
                maxlength: "{MAX_TITLE_CHARS}",
                disabled: !can_edit,
                class: "w-full",
            }
            InputField {
                context: form.text(f.contact_email()),
                label: "Contact email",
                description: "Shown in the rules and the privacy notice. Needed before registration can open, or the list be frozen (or a privacy contact).",
                r#type: "email",
                disabled: !can_edit,
                class: "w-full",
            }
            InputField {
                context: form.text(f.privacy_email()),
                label: "Privacy contact (optional)",
                description: "Where privacy requests go, if not the contact email: a DPO, for instance. Can change at any time.",
                r#type: "email",
                disabled: !can_edit,
                class: "w-full",
            }
            InputField {
                context: form.text(f.privacy_url()),
                label: "Your own privacy notice (optional)",
                description: if rules_editable { "An https:// link, shown after the generated notice." } else { "Locked: entrants have seen it." },
                r#type: "url",
                placeholder: "https://",
                readonly: !rules_editable,
                class: "w-full",
            }
            InputField {
                context: form.text(f.event_date()),
                label: "Event date",
                description: if rules_editable { "The day of the draw, at most a year ahead. Entrants' data is erased a set number of days after it." } else { "Locked: entrants were told their erasure date. Postponed? Remove the real entrants (People tab, before the freeze) to unlock it, or duplicate the raffle." },
                r#type: "date",
                required: true,
                readonly: !rules_editable,
                class: "w-full",
            }
            InputField {
                context: form.text(f.retention_days()),
                label: "Retention period (days after the event)",
                description: if rules_editable {
                    format!("Days after the event date. Up to {max_days}; this service's default is {}. Winners are kept until their prize is marked delivered, at the latest {} days longer. With 0, erasure is at midnight at the end of the event day: an evening that runs past midnight needs its draws done before.", retention.default_days, retention.winner_hold_days)
                } else {
                    "Locked: entrants were told their erasure date.".to_owned()
                },
                r#type: "number",
                min: "0",
                max: "{max_days}",
                readonly: !rules_editable,
                class: "w-full",
            }
            div { class: "md:col-span-2",
                TextareaField {
                    context: form.textarea(f.rules()),
                    label: "Additional rules",
                    description: if rules_editable {
                        "One paragraph per line, appended to the standard rules. Put a higher age limit here. Fake entrants don't lock anything; a real one does."
                    } else {
                        "Locked: real entrants are in under these rules. Remove them (People tab, before the freeze) to unlock."
                    },
                    placeholder: "e.g. Entrants must be 18 or older.",
                    readonly: !rules_editable,
                    rows: "3",
                    class: "w-full",
                }
            }
            div { class: "md:col-span-2",
                TextareaField {
                    context: form.textarea(f.privacy_text()),
                    label: "Your own privacy text (optional)",
                    description: if rules_editable {
                        "One paragraph per line, shown after the generated privacy notice, never instead of it."
                    } else {
                        "Locked: entrants have seen it."
                    },
                    readonly: !rules_editable,
                    rows: "3",
                    class: "w-full",
                }
            }
            div { class: "md:col-span-2", {after_privacy} }
            div { class: "mt-2 rounded-box bg-base-200 p-4 md:col-span-2",
                SwitchField {
                    context: form.field(f.registration()),
                    label: "Let people join with the QR code",
                    disabled: !registration_editable,
                    description: if registration_editable || !can_edit {
                        "You can always add people yourself, one by one or from a list."
                    } else {
                        "You can always add people yourself, one by one or from a list. It can't change while registration is open (close it first), or once the list is frozen."
                    },
                }
            }
            div { class: "mt-2 rounded-box bg-warning/10 p-4 md:col-span-2",
                SwitchField {
                    context: form.field(f.test_mode()),
                    label: "Test mode",
                    disabled: !can_edit,
                    description: "Rehearse with fake entrants; a TEST badge shows on every screen. Switching it off removes the fake entrants. It can't change once the list is frozen.",
                    color: SwitchColor::Warning,
                }
            }
            if !can_edit {
                p { class: "text-sm text-base-content/60 md:col-span-2", "Only an organization admin or manager can change the settings." }
            }
            div { class: "mt-4 flex flex-wrap items-center gap-3 md:col-span-2", hidden: !can_edit,
                Button { color: ButtonColor::Primary, r#type: "submit", disabled: !dirty || busy,
                    if busy { span { class: "loading loading-spinner loading-sm" } } else { Check { class: "size-5" } }
                    "Save settings"
                }
                if dirty {
                    Button { class: "btn-ghost", r#type: "button", onclick: move |_| reset.reset(), Undo2 { class: "size-4" } "Discard changes" }
                }
                Button { class: "btn-ghost", r#type: "button", disabled: saving_defaults(), onclick: save_defaults,
                    title: "New raffles will start with this organizer, contact email and additional rules.",
                    BookmarkPlus { class: "size-4" }
                    "Save as my defaults"
                }
                Button { class: "btn-ghost", r#type: "button", onclick: fill_defaults,
                    title: "Fill in the organizer, contact email and additional rules from your defaults.",
                    BookmarkCheck { class: "size-4" }
                    "Use my defaults"
                }
                FormErrors { errors: form_err }
                Problems { error: defaults_error }
            }
        }
    }
}
