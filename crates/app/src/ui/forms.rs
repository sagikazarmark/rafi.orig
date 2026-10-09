//! Organizer forms: dioform for typed state, validation and managed async
//! submit; the dioxus-daisyui-components registry for the fields, which read
//! dioform's field metadata (errors, required, touched) through `dioxus-field`.

use dioform::prelude::*;
use dioxus::prelude::*;

use crate::{
    api::{self, ApiError},
    components::{
        button::{Button, ButtonColor, ButtonSize},
        field::{Field, FieldDescription, FieldError, FieldLabel},
        input::{InputField, InputSize},
        select::{Select, SelectList, SelectOption, SelectTrigger, SelectValue},
        switch::{SwitchColor, SwitchField},
        textarea::TextareaField,
        toast::{ToastOptions, use_toast},
    },
    icons::{I, Icon},
    model::{PrizeInput, RaffleSettings},
};

pub fn err_text(e: ApiError) -> String {
    e.message
}

/// Form-level errors (e.g. the server rejected the submission).
#[component]
fn FormErrors(errors: Vec<String>) -> Element {
    rsx! {
        for e in errors {
            div { role: "alert", class: "alert alert-error alert-soft py-2 text-sm", Icon { i: I::Alert, class: "size-4" } "{e}" }
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

fn optional_email<M: 'static>()
-> impl for<'a> Fn(&'a String, ValidatorContext<'a, M>) -> Vec<String> + 'static {
    |value: &String, _ctx: ValidatorContext<'_, M>| {
        let v = value.trim();
        if v.is_empty() || (v.contains('@') && v.contains('.')) {
            Vec::new()
        } else {
            vec!["That does not look like an email.".to_owned()]
        }
    }
}

// ---------------------------------------------------------------------------
// New raffle
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Form)]
struct NewRaffle {
    title: String,
}

#[component]
pub fn CreateRaffleForm(on_created: EventHandler<String>) -> Element {
    let form = use_form_handle(|| {
        let handle = FormHandle::<NewRaffle>::from_config(
            FormConfig::new(NewRaffle::default())
                .id_namespace("new-raffle")
                .validation_mode(ValidationMode::on_submit()),
        );
        handle.write_advanced(|core| {
            core.register_sync_field_validator(
                NewRaffle::fields().title(),
                "required",
                required("Give your raffle a name."),
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
            class: "flex flex-col gap-3 sm:flex-row sm:items-end",
            novalidate: true,
            onsubmit: move |event| {
                let reset = reset.clone();
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<NewRaffle>| async move {
                    match api::create_raffle(s.value().title.clone()).await {
                        Ok(id) => {
                            reset.reinitialize(NewRaffle::default());
                            on_created.call(id);
                            SubmitErrors::none()
                        }
                        Err(e) => SubmitError::form(err_text(e)).into(),
                    }
                });
            },
            div { class: "min-w-0 flex-1",
                InputField {
                    context: form.text(fields.title()),
                    label: "New raffle",
                    size: InputSize::Lg,
                    required: true,
                    maxlength: "120",
                    placeholder: "e.g. “Cloud Native Budapest — October”",
                    prefix: rsx! { Icon { i: I::Sparkles, class: "size-5 text-secondary" } },
                    wrapper_attributes: dioxus_primitives::dioxus_attributes::attributes!(span { class: "w-full" }),
                }
                FormErrors { errors: form_errors }
            }
            Button { color: ButtonColor::Primary, size: ButtonSize::Lg, class: "hover-wiggle sm:mb-6", r#type: "submit", disabled: busy,
                if busy { span { class: "loading loading-spinner loading-sm" } } else { Icon { i: I::Plus } }
                "Create"
            }
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
            if (1..=500).contains(v) {
                Vec::new()
            } else {
                vec!["Between 1 and 500.".to_owned()]
            }
        });
    });
    handle
}

/// Adds a prize, or edits an existing one when `prize_id` is set.
#[component]
pub fn PrizeForm(
    raffle_id: String,
    #[props(default)] prize_id: Option<String>,
    #[props(default)] name: String,
    #[props(default)] sponsor: String,
    #[props(default = 1)] qty: u32,
    #[props(default)] position: i64,
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
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<Prize>| async move {
                    let v = s.value().clone();
                    let input = PrizeInput {
                        id: prize_id.clone(),
                        name: v.name.clone(),
                        sponsor: v.sponsor,
                        description: String::new(),
                        qty: v.qty,
                        position,
                    };
                    match api::save_prize(raffle_id, input).await {
                        Ok(_) => {
                            if prize_id.is_none() {
                                reset.reinitialize(Prize::default());
                                toast.success(format!("🎁 {} added", v.name), ToastOptions::new());
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
                maxlength: "120",
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
                max: "500",
                inputmode: "numeric",
                class: "w-full",
            }
            div { class: "flex gap-2 sm:pt-[1.85rem]",
                Button { color: if editing { ButtonColor::Primary } else { ButtonColor::Secondary }, r#type: "submit", disabled: busy,
                    if editing { Icon { i: I::Check } "Save" } else { Icon { i: I::Gift } "Add prize" }
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
// Entrants without a phone
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Form)]
struct Entrant {
    name: String,
    email: String,
}

#[component]
pub fn AddEntrantForm(raffle_id: String, on_added: EventHandler<()>) -> Element {
    let form = use_form_handle(|| {
        let handle = FormHandle::<Entrant>::from_config(
            FormConfig::new(Entrant::default())
                .id_namespace("add-entrant")
                .validation_mode(ValidationMode::on_submit()),
        );
        handle.write_advanced(|core| {
            let f = Entrant::fields();
            core.register_sync_field_validator(f.name(), "required", required("Who is it?"));
            core.register_sync_field_validator(f.name(), "length", |v: &String, _| {
                if v.trim().chars().count() > 24 {
                    vec!["24 characters at most.".to_owned()]
                } else {
                    Vec::new()
                }
            });
            core.register_sync_field_validator(f.email(), "email", optional_email());
        });
        handle
    });
    let fields = Entrant::fields();
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
                let _ = submit.on_submit_async(event, move |s: SubmissionSnapshot<Entrant>| async move {
                    let v = s.value().clone();
                    let email = Some(v.email.trim().to_owned()).filter(|e| !e.is_empty());
                    match api::add_entrant(raffle_id, v.name.clone(), email).await {
                        Ok(_) => {
                            reset.reinitialize(Entrant::default());
                            toast.success(format!("🙋 {} is in", v.name.trim()), ToastOptions::new());
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
                maxlength: "24",
                placeholder: "Shown on the screen",
                prefix: rsx! { Icon { i: I::Phone, class: "size-4 opacity-60" } },
                wrapper_attributes: dioxus_primitives::dioxus_attributes::attributes!(span { class: "w-full" }),
            }
            InputField {
                context: form.text(fields.email()),
                label: "Email",
                r#type: "email",
                placeholder: "optional, to deliver the prize",
                class: "w-full",
            }
            div { class: "sm:pt-[1.85rem]",
                Button { color: ButtonColor::Secondary, r#type: "submit", disabled: busy,
                    Icon { i: I::Plus }
                    "Add entrant"
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
    lang: String,
    event_date: String,
    organizer_name: String,
    contact_email: String,
    min_age: u32,
    rules: String,
    test_mode: bool,
}

impl From<&RaffleSettings> for Settings {
    fn from(s: &RaffleSettings) -> Self {
        Self {
            title: s.title.clone(),
            description: s.description.clone(),
            lang: if s.lang == "hu" {
                "hu".into()
            } else {
                "en".into()
            },
            event_date: s.event_date.clone(),
            organizer_name: s.organizer_name.clone(),
            contact_email: s.contact_email.clone(),
            min_age: s.min_age,
            rules: s.rules.clone(),
            test_mode: s.test_mode,
        }
    }
}

impl From<Settings> for RaffleSettings {
    fn from(s: Settings) -> Self {
        Self {
            title: s.title,
            description: s.description,
            lang: s.lang,
            organizer_name: s.organizer_name,
            contact_email: s.contact_email,
            rules: s.rules,
            event_date: s.event_date,
            min_age: s.min_age,
            test_mode: s.test_mode,
        }
    }
}

const LANGUAGES: [(&str, &str); 2] = [("en", "🇬🇧 English"), ("hu", "🇭🇺 Magyar")];

#[component]
pub fn SettingsForm(
    raffle_id: String,
    settings: RaffleSettings,
    on_saved: EventHandler<()>,
) -> Element {
    let initial = Settings::from(&settings);
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
            core.register_sync_field_validator(f.contact_email(), "email", optional_email());
            core.register_sync_field_validator(f.min_age(), "range", |v: &u32, _| {
                if *v <= 99 {
                    Vec::new()
                } else {
                    vec!["0 to 99.".to_owned()]
                }
            });
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
    let min_age = use_number(&form, f.min_age());
    let submit = form.managed_submit();
    let dirty = form.is_dirty();
    let busy = form.is_submitting();
    let form_err = messages(form.visible_form_validation_errors());
    let rebase = form.clone();
    let reset = form.clone();

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
            InputField { context: form.text(f.title()), label: "Title", required: true, maxlength: "120", class: "w-full" }
            Field { context: form.select(f.lang()),
                FieldLabel { "Language" }
                Select::<String> { class: "w-full",
                    SelectTrigger { class: "w-full", SelectValue { placeholder: "Choose a language" } }
                    SelectList { class: "z-20 w-full",
                        for (index , (code , label)) in LANGUAGES.into_iter().enumerate() {
                            SelectOption::<String> {
                                key: "{code}",
                                value: code.to_owned(),
                                index,
                                text_value: Some(label.to_owned()),
                                "{label}"
                            }
                        }
                    }
                }
                FieldDescription { "Of the projector screen and the attendee page (attendees can switch)." }
                FieldError {}
            }
            div { class: "md:col-span-2",
                TextareaField {
                    context: form.textarea(f.description()),
                    label: "Description",
                    description: "Shown on the intro screen.",
                    rows: "2",
                    class: "w-full",
                }
            }
            InputField { context: form.text(f.event_date()), label: "Event date", r#type: "date", class: "w-full" }
            InputField { context: min_age, label: "Minimum age", r#type: "number", min: "0", max: "99", inputmode: "numeric", class: "w-full" }
            InputField {
                context: form.text(f.organizer_name()),
                label: "Organizer (data controller)",
                description: "Named in the rules and the privacy notice.",
                class: "w-full",
            }
            InputField {
                context: form.text(f.contact_email()),
                label: "Contact email",
                description: "For privacy requests.",
                r#type: "email",
                class: "w-full",
            }
            div { class: "md:col-span-2",
                TextareaField {
                    context: form.textarea(f.rules()),
                    label: "Additional rules",
                    description: "One paragraph per line, appended to the standard rules.",
                    rows: "3",
                    class: "w-full",
                }
            }
            div { class: "mt-2 rounded-box bg-warning/10 p-4 md:col-span-2",
                SwitchField {
                    context: form.field(f.test_mode()),
                    label: "🧪 Test mode",
                    description: "Rehearse with fake entrants; a TEST badge shows on every screen.",
                    color: SwitchColor::Warning,
                }
            }
            div { class: "mt-4 flex flex-wrap items-center gap-3 md:col-span-2",
                Button { color: ButtonColor::Primary, r#type: "submit", disabled: !dirty || busy,
                    if busy { span { class: "loading loading-spinner loading-sm" } } else { Icon { i: I::Check } }
                    "Save settings"
                }
                if dirty {
                    Button { class: "btn-ghost", r#type: "button", onclick: move |_| reset.reset(), Icon { i: I::Undo, class: "size-4" } "Discard changes" }
                }
                FormErrors { errors: form_err }
            }
        }
    }
}
