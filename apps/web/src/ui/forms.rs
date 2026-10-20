//! Organizer forms: dioform for typed state, validation and managed async
//! submit; the dioxus-daisyui-components registry for the fields, which read
//! dioform's field metadata (errors, required, touched) through `dioxus-field`.

use dioform::prelude::*;
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    BookmarkCheck, BookmarkPlus, Check, Gift, Plus, Smartphone, Sparkles, TriangleAlert, Undo2,
};
use rafi_lifecycle::{
    entrant::{self, MAX_NAME_CHARS},
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
};
use rafi_api::{PrizeInput, RaffleDefaults, RaffleSettings};

pub fn err_text(e: ApiError) -> String {
    e.message
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
                    maxlength: "{MAX_TITLE_CHARS}",
                    placeholder: "e.g. “Cloud Native Budapest — October”",
                    prefix: rsx! { Sparkles { class: "size-5 text-secondary" } },
                    wrapper_attributes: dioxus_primitives::dioxus_attributes::attributes!(span { class: "w-full" }),
                }
                FormErrors { errors: form_errors }
            }
            Button { color: ButtonColor::Primary, size: ButtonSize::Lg, class: "hover-wiggle sm:mb-6", r#type: "submit", disabled: busy,
                if busy { span { class: "loading loading-spinner loading-sm" } } else { Plus { class: "size-5" } }
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
            if settings::prize_qty(*v) {
                Vec::new()
            } else {
                vec![format!("Between 1 and {MAX_PRIZE_QTY}.")]
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
            div { class: "flex gap-2 sm:pt-[1.85rem]",
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
                match entrant::display_name(v) {
                    Err(entrant::NameError::Invalid) if !v.trim().is_empty() => {
                        vec![format!(
                            "{MAX_NAME_CHARS} characters at most, no control characters."
                        )]
                    }
                    Err(entrant::NameError::Blocked) => {
                        vec!["This name is not allowed.".to_owned()]
                    }
                    _ => Vec::new(),
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
                maxlength: "{MAX_NAME_CHARS}",
                placeholder: "Shown on the screen",
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
            div { class: "sm:pt-[1.85rem]",
                Button { color: ButtonColor::Secondary, r#type: "submit", disabled: busy,
                    Plus { class: "size-5" }
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
    event_date: String,
    organizer_name: String,
    contact_email: String,
    rules: String,
    test_mode: bool,
}

impl From<&RaffleSettings> for Settings {
    fn from(s: &RaffleSettings) -> Self {
        Self {
            title: s.title.clone(),
            description: s.description.clone(),
            event_date: s.event_date.clone(),
            organizer_name: s.organizer_name.clone(),
            contact_email: s.contact_email.clone(),
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
            organizer_name: s.organizer_name,
            contact_email: s.contact_email,
            rules: s.rules,
            event_date: s.event_date,
            test_mode: s.test_mode,
        }
    }
}

#[component]
pub fn SettingsForm(
    raffle_id: String,
    settings: RaffleSettings,
    /// Whether the additional rules can still change: not once an entrant
    /// accepted them.
    rules_editable: bool,
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
                    if rules_editable {
                        form.textarea(f.rules()).set_value(d.rules);
                    }
                }
                Err(e) => toast.error(err_text(e), ToastOptions::new()),
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
                rules: s.rules,
            })
            .await;
            saving_defaults.set(false);
            match res {
                Ok(()) => toast.success(
                    "📌 New raffles will start with this organizer, contact and rules".to_owned(),
                    ToastOptions::new(),
                ),
                Err(e) => toast.error(err_text(e), ToastOptions::new()),
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
                InputField { context: form.text(f.title()), label: "Title", required: true, maxlength: "{MAX_TITLE_CHARS}", class: "w-full" }
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
            InputField { context: form.text(f.event_date()), label: "Event date", r#type: "date", class: "w-full" }
            div { class: "md:col-span-2",
                TextareaField {
                    context: form.textarea(f.rules()),
                    label: "Additional rules",
                    description: if rules_editable {
                        "One paragraph per line, appended to the standard rules. Put any age limit here."
                    } else {
                        "Locked: entrants have accepted these rules."
                    },
                    placeholder: "e.g. Entrants must be 18 or older.",
                    readonly: !rules_editable,
                    rows: "3",
                    class: "w-full",
                }
            }
            div { class: "mt-2 rounded-box bg-warning/10 p-4 md:col-span-2",
                SwitchField {
                    context: form.field(f.test_mode()),
                    label: "🧪 Test mode",
                    description: "Rehearse with fake entrants; a TEST badge shows on every screen. Switching it off removes the fake entrants. It can't change once the list is frozen.",
                    color: SwitchColor::Warning,
                }
            }
            div { class: "mt-4 flex flex-wrap items-center gap-3 md:col-span-2",
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
            }
        }
    }
}
