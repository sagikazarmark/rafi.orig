use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use dioxus::core::{AttributeValue, ListenerCallback};
use dioxus::prelude::*;
use dioxus_field::{
    AttributeSurface, Binding, ChangeOrigin, FieldControlOptions, FieldMeta, FieldSurface,
    NameSurface, ValiditySurface, merge_attributes, use_binding, use_field_meta,
    use_focus_registration,
};
use dioxus_primitives::dioxus_attributes::attributes;
use dioxus_primitives::select;

static NEXT_FOCUS_SCOPE_ID: AtomicUsize = AtomicUsize::new(0);

/// daisyUI's colour axis for a select, which colours the trigger's border and
/// the ring it takes on focus.
///
/// [`SelectColor::Default`] emits no class at all, which is daisyUI's own
/// uncoloured select rather than a synonym for [`SelectColor::Neutral`].
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectColor {
    #[default]
    Default,
    Neutral,
    Primary,
    Secondary,
    Accent,
    Info,
    Success,
    Warning,
    Error,
}

impl SelectColor {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::Neutral,
        Self::Primary,
        Self::Secondary,
        Self::Accent,
        Self::Info,
        Self::Success,
        Self::Warning,
        Self::Error,
    ];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Neutral => "select-neutral",
            Self::Primary => "select-primary",
            Self::Secondary => "select-secondary",
            Self::Accent => "select-accent",
            Self::Info => "select-info",
            Self::Success => "select-success",
            Self::Warning => "select-warning",
            Self::Error => "select-error",
        }
    }
}

/// daisyUI's size axis for a select, which sizes the trigger rather than the
/// popup under it.
///
/// [`SelectSize::Default`] emits no class, which renders at the same size as
/// daisyUI's explicit `select-md`.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectSize {
    Xs,
    Sm,
    #[default]
    Default,
    Lg,
    Xl,
}

impl SelectSize {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Xs, Self::Sm, Self::Default, Self::Lg, Self::Xl];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Xs => "select-xs",
            Self::Sm => "select-sm",
            Self::Default => "",
            Self::Lg => "select-lg",
            Self::Xl => "select-xl",
        }
    }
}

/// daisyUI's side axis for the popup, which is the side of the trigger it
/// opens on.
///
/// The popup is a borrowed dropdown (ADR-0005), so this is the dropdown's axis
/// and it inherits the dropdown's rule with it: every value emits a class,
/// including the default one (ADR-0008). daisyUI's unclassed dropdown places
/// its content wherever it would have fallen in flow, which is under the
/// trigger only for as long as the trigger is the one thing written before it;
/// `dropdown-bottom` pins it under the element instead.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectSide {
    /// Above the trigger.
    Top,
    /// Under the trigger, which is where daisyUI puts an unplaced dropdown.
    #[default]
    Bottom,
    /// To the left of the trigger, in either writing direction; daisyUI's
    /// horizontal placements are physical rather than logical.
    Left,
    /// To the right of the trigger, mirroring [`SelectSide::Left`].
    Right,
}

impl SelectSide {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Top, Self::Bottom, Self::Left, Self::Right];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Top => "dropdown-top",
            Self::Bottom => "dropdown-bottom",
            Self::Left => "dropdown-left",
            Self::Right => "dropdown-right",
        }
    }
}

/// daisyUI's align axis for the popup, which is where it sits along the
/// side [`SelectSide`] opened it on.
///
/// Every value emits a class here too, for the reason [`SelectSide`]
/// records.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectAlign {
    /// The inline start edge, which follows the writing direction: the left in
    /// a left-to-right document, the right in a right-to-left one. This is
    /// where daisyUI puts an unaligned dropdown.
    #[default]
    Start,
    /// Centred on the trigger.
    Center,
    /// The inline end edge, mirroring [`SelectAlign::Start`].
    End,
}

impl SelectAlign {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Start, Self::Center, Self::End];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Start => "dropdown-start",
            Self::Center => "dropdown-center",
            Self::End => "dropdown-end",
        }
    }
}

/// daisyUI's size axis for the menu inside the popup, which sizes the options
/// rather than the box they are in.
///
/// It is a separate axis from [`SelectSize`] because daisyUI's are separate:
/// `select-lg` sizes a field and `menu-lg` sizes a list, and nothing in daisyUI
/// ties one to the other.
///
/// It is an axis at all, rather than something a caller passes through `class`,
/// because the split (ADR-0005) leaves it unreachable otherwise: `menu-lg` only
/// works on the element carrying `menu`, and that is the list rendered inside
/// the box, one element below where a caller's classes land.
///
/// [`SelectListSize::Default`] emits no class, which renders at the same size
/// as daisyUI's explicit `menu-md`.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectListSize {
    Xs,
    Sm,
    #[default]
    Default,
    Lg,
    Xl,
}

impl SelectListSize {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Xs, Self::Sm, Self::Default, Self::Lg, Self::Xl];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Xs => "menu-xs",
            Self::Sm => "menu-sm",
            Self::Default => "",
            Self::Lg => "menu-lg",
            Self::Xl => "menu-xl",
        }
    }
}

/// Whether [`SelectList`] emits the utilities that draw the box the options sit
/// in.
///
/// daisyUI's `dropdown-content` only positions the element: the fill, the
/// corners and the shadow in its own examples are Tailwind utilities on the
/// same element. They are emitted here instead, which inverts the usual
/// convention: [`SelectListAppearance::Default`] emits classes and
/// [`SelectListAppearance::None`] emits nothing. A utility this component emits
/// only ties with a caller's, and a tie is settled by generated-stylesheet
/// order rather than by the class attribute, so switching ours off is the way
/// to win it (ADR-0004).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectListAppearance {
    #[default]
    Default,
    None,
}

impl SelectListAppearance {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Default, Self::None];

    /// The Tailwind utilities for this value, as complete string literals so
    /// Tailwind's scanner can see them.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "bg-base-100 rounded-box shadow-sm",
            Self::None => "",
        }
    }
}

/// Whether [`SelectValue`] emits the utility that tells a placeholder apart
/// from a chosen value.
///
/// daisyUI has no class for it (the element it fades is a native `select`'s
/// own placeholder, which this component does not have) so the utility is
/// emitted here, keyed on the `data-placeholder` attribute the primitive
/// already sets. The default arm therefore emits and
/// [`SelectValueAppearance::None`] emits nothing, the inverse of the usual
/// convention and for the reason [`SelectListAppearance`] records (ADR-0004).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum SelectValueAppearance {
    #[default]
    Default,
    None,
}

impl SelectValueAppearance {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Default, Self::None];

    /// The Tailwind utilities for this value, as complete string literals so
    /// Tailwind's scanner can see them. The opacity matches what daisyUI fades
    /// a native select's placeholder by.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "data-[placeholder=true]:opacity-50",
            Self::None => "",
        }
    }
}

/// The selected value, as [`Select`] holds it for the options to read.
///
/// This is the second half of the lift ADR-0006 describes, and the half that is
/// this component's own: daisyUI marks the chosen row of a menu with
/// `menu-active`, and `.menu` matches no ARIA attribute at all: not the
/// `aria-selected` the primitive puts on the option, not anything else, so the
/// chosen option would otherwise render identically to every other one.
///
/// `SelectContext` is private, so an option cannot read the value from the
/// primitive. [`Select`] therefore provides it alongside, and
/// [`SelectOption`] compares its own value against it.
struct Selection<T: Clone + PartialEq + 'static> {
    value: Memo<Option<T>>,
}

// Written out rather than derived: a derived `Clone` would demand `T: Clone` of
// the context itself, and a derived `Copy` would demand `T: Copy`, neither of
// which the value inside needs, because a `Memo` is `Copy` whatever it holds.
impl<T: Clone + PartialEq + 'static> Clone for Selection<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + PartialEq + 'static> Copy for Selection<T> {}

#[derive(Clone, PartialEq)]
struct SelectFieldMeta {
    meta: FieldMeta,
    required: Option<bool>,
    disabled: Option<bool>,
    name: Option<String>,
}

/// The outer element of a select, carrying daisyUI's `dropdown` classes.
///
/// Both the open state and the value are **lifted** here (ADR-0006), and
/// neither lift is cosmetic. daisyUI hides `.dropdown-content` outright unless
/// `dropdown-open` is on this element, and marks a menu's chosen row with
/// `menu-active`; it matches no attribute the primitive sets for either. So
/// both classes have to be emitted from Rust, and this component has to know
/// both pieces of state to emit them. Open state is seeded from `default_open`;
/// value state resolves through a Binding seeded from `default_value`. The
/// component always hands the primitive controlled state and intercepts its
/// change callbacks, which leaves controlled and uncontrolled callers working.
///
/// Binding and metadata resolve from explicit props, Field Context, then
/// standalone state. Metadata is forwarded privately to [`SelectTrigger`],
/// which is the interactive control in this Compound widget.
///
/// Classes passed by the caller concatenate with this element's own; every
/// other attribute the caller passes overrides them.
#[component]
pub fn Select<T: Clone + PartialEq + 'static>(
    /// daisyUI's side axis for the popup.
    #[props(default)]
    side: SelectSide,
    /// daisyUI's align axis for the popup.
    #[props(default)]
    align: SelectAlign,
    /// An explicit Field binding, which wins over Field Context.
    binding: Option<Binding<Option<T>>>,
    /// Explicit Field metadata, which wins over Field Context. Its control
    /// attributes are rendered by [`SelectTrigger`].
    meta: Option<FieldMeta>,
    /// The controlled value of the select. `Some` makes it controlled, and the
    /// signal's own `None` means nothing is selected.
    #[props(default)]
    value: Option<ReadSignal<Option<T>>>,
    /// The value the select starts on when it is not controlled. `None` shows
    /// the placeholder until an option is chosen.
    #[props(default)]
    default_value: Option<T>,
    /// Called with the selected value after user interaction.
    on_change: Option<EventHandler<Option<T>>>,
    /// Called after every value change ends its interaction unit.
    on_commit: Option<EventHandler<()>>,
    /// Called when focus leaves the trigger, popup listbox, and options as one
    /// logical focus scope.
    on_focus_exit: Option<EventHandler<()>>,
    /// The controlled open state of the popup.
    #[props(default)]
    open: ReadSignal<Option<bool>>,
    /// The state the popup starts in when it is not controlled. The primitive
    /// closes a popup that nothing in it is focused, so a select that has to
    /// open with the page is one a caller controls; see the component's
    /// documentation.
    #[props(default)]
    default_open: bool,
    /// Called when the open state changes.
    #[props(default)]
    on_open_change: Callback<bool>,
    /// Whether the select is required. Omission falls back to Field metadata.
    /// The button trigger exposes this only as `data-required`; neither native
    /// nor ARIA required state applies to its role.
    #[props(default)]
    required: Option<bool>,
    /// Whether the select is disabled, which leaves the trigger inert and the
    /// popup unopenable. Omission falls back to Field metadata.
    #[props(default)]
    disabled: Option<bool>,
    /// The field name projected onto the trigger. Omission falls back to Field
    /// metadata. This Compound widget has no hidden input and does not
    /// participate in native form submission.
    #[props(default)]
    name: Option<String>,
    /// Whether arrow-key navigation wraps around at the ends of the list. The
    /// default repeats the primitive's own, since a prop declared here has to
    /// carry one.
    #[props(default = ReadSignal::new(Signal::new(true)))]
    roving_loop: ReadSignal<bool>,
    /// How long the typeahead buffer holds what has been typed before it is
    /// cleared. The default repeats the primitive's own.
    #[props(default = ReadSignal::new(Signal::new(Duration::from_millis(1000))))]
    typeahead_timeout: ReadSignal<Duration>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let mut uncontrolled_open = use_signal(|| default_open);
    let binding = use_binding(binding, default_value.clone());
    let meta = use_field_meta(meta);
    let resolved_disabled = disabled.unwrap_or_else(|| meta.disabled());
    let field_value = SelectFieldMeta {
        meta,
        required,
        disabled,
        name,
    };
    let mut field = use_context_provider(|| Signal::new(field_value.clone()));
    let field_changed = {
        let current = field.peek();
        *current != field_value
    };
    if field_changed {
        field.set(field_value);
    }

    // Read here rather than inside the markup, and eagerly rather than only
    // when the other is absent, so that this component subscribes to whichever
    // of them is driving and re-renders, which is what puts the modifier class
    // below on the element and takes it off again.
    let is_open = open().unwrap_or(uncontrolled_open());

    // The value goes on to the options through a context rather than through a
    // class on this element, because the class it decides (`menu-active`) is
    // on the option itself.
    let binding_value = binding.read;
    let selected = use_memo(move || match value {
        Some(value) => value(),
        None => binding_value(),
    });
    use_context_provider(|| Selection { value: selected });
    let controlled: ReadSignal<Option<T>> = selected.into();

    let side = side.class();
    let align = align.class();
    // Tier 2, and half of why the state is lifted: daisyUI's own modifier
    // class, emitted from Rust as a complete literal so that Tailwind's scanner
    // sees it too.
    let state = if is_open { "dropdown-open" } else { "" };

    let base = attributes!(div {
        class: "dropdown {side} {align} {state}",
    });
    let focus_scope_id = use_hook(|| NEXT_FOCUS_SCOPE_ID.fetch_add(1, Ordering::Relaxed));
    let focus_locator = attributes!(div {
        "data-select-focus-scope": "{focus_scope_id}",
    });
    let mut merged = merge_attributes(vec![base, attributes, focus_locator]);
    let caller_focus_in = take_event_listener(&mut merged, "onfocusin");
    let caller_focus_out = take_event_listener(&mut merged, "onfocusout");
    let focus_generation = use_hook(|| Rc::new(Cell::new(0_u64)));
    let focus_active = use_hook(|| Rc::new(Cell::new(false)));
    let focus_in_generation = Rc::clone(&focus_generation);
    let focus_out_generation = Rc::clone(&focus_generation);
    let close_generation = focus_generation;
    let focus_in_active = Rc::clone(&focus_active);
    let focus_out_active = Rc::clone(&focus_active);
    let close_active = focus_active;
    let focus_exit_binding = binding.clone();
    let report_focus_exit = use_callback(move |()| {
        focus_exit_binding.focus_exit();
        if let Some(handler) = &on_focus_exit {
            handler.call(());
        }
    });
    let focus_scope = attributes!(div {
        onfocusin: move |event: FocusEvent| {
            if let Some(listener) = &caller_focus_in {
                listener.call(event.into_any());
            }
            focus_in_active.set(true);
            focus_in_generation.set(focus_in_generation.get().wrapping_add(1));
        },
        onfocusout: move |event: FocusEvent| {
            if let Some(listener) = &caller_focus_out {
                listener.call(event.into_any());
            }
            let generation = focus_out_generation.get().wrapping_add(1);
            focus_out_generation.set(generation);
            let focus_out_generation = Rc::clone(&focus_out_generation);
            let focus_out_active = Rc::clone(&focus_out_active);
            spawn(async move {
                let mut deferred = document::eval("setTimeout(() => dioxus.send(true), 0);");
                let _: Result<bool, _> = deferred.recv().await;
                if focus_out_generation.get() == generation {
                    focus_out_generation.set(generation.wrapping_add(1));
                    focus_out_active.set(false);
                    report_focus_exit.call(());
                }
            });
        },
    });
    merged = merge_attributes(vec![merged, focus_scope]);
    let mut previous_open = use_hook(|| CopyValue::new(is_open));
    use_effect(move || {
        let is_open = open().unwrap_or(uncontrolled_open());
        let was_open = previous_open.cloned();
        previous_open.set(is_open);
        if was_open && !is_open && close_active.get() {
            let generation = close_generation.get();
            let close_generation = Rc::clone(&close_generation);
            let close_active = Rc::clone(&close_active);
            let script = format!(
                r#"
                setTimeout(() => {{
                    const scope = document.querySelector(
                        '[data-select-focus-scope="{focus_scope_id}"]'
                    );
                    dioxus.send(scope != null && scope.contains(document.activeElement));
                }}, 0);
                "#,
            );
            spawn(async move {
                let mut deferred = document::eval(&script);
                let focus_remains_inside: Result<bool, _> = deferred.recv().await;
                if close_generation.get() == generation
                    && close_active.get()
                    && matches!(focus_remains_inside, Ok(false))
                {
                    close_generation.set(generation.wrapping_add(1));
                    close_active.set(false);
                    report_focus_exit.call(());
                }
            });
        }
    });
    let change_binding = binding.clone();
    let commit_binding = binding;

    rsx! {
        select::Select::<T> {
            value: Some(controlled),
            on_value_change: move |value: Option<T>| {
                change_binding.write(value.clone(), ChangeOrigin::User);
                if let Some(handler) = &on_change {
                    handler.call(value);
                }
                commit_binding.commit();
                if let Some(handler) = &on_commit {
                    handler.call(());
                }
            },
            open: Some(is_open),
            on_open_change: move |open| {
                uncontrolled_open.set(open);
                on_open_change.call(open);
            },
            disabled: resolved_disabled,
            roving_loop,
            typeahead_timeout,
            attributes: merged,
            {children}
        }
    }
}

fn take_event_listener(attributes: &mut Vec<Attribute>, name: &str) -> Option<ListenerCallback> {
    let index = attributes.iter().position(|attribute| {
        attribute.name == name && matches!(attribute.value, AttributeValue::Listener(_))
    })?;
    match attributes.remove(index).value {
        AttributeValue::Listener(listener) => Some(listener),
        _ => unreachable!(),
    }
}

/// The field a select opens from, carrying daisyUI's `select` class.
///
/// It is a `button` rather than daisyUI's `select` element, and the class
/// carries over intact: `.select` lays out an inline flex row, pads the inline
/// end by `1.75rem`, and paints the caret into that space as a background
/// image, none of which asks the element to be a form control.
/// Producer-defined invalidity emits `select-error` when no colour is passed.
/// Field metadata attributes and focus registration land on this trigger rather
/// than on [`Select`]'s structural outer element.
///
/// Unlike the button and the checkbox, this takes no `extends = button` list.
/// The primitive renders the element and puts `disabled` on it from
/// [`Select`]'s own prop, and `type` on it as well, so extending them here
/// would offer a second and conflicting way to set the two attributes worth
/// reaching.
#[component]
pub fn SelectTrigger(
    /// An explicit colour, or `None` to derive error colour from Field metadata.
    #[props(default)]
    color: Option<SelectColor>,
    /// daisyUI's size axis.
    #[props(default)]
    size: SelectSize,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let field = use_context::<Signal<SelectFieldMeta>>();
    let field = field.read().clone();
    let meta = field.meta;
    let color = color.map_or_else(
        || {
            if meta.invalid() { "select-error" } else { "" }
        },
        SelectColor::class,
    );
    let size = size.class();
    let mut control: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let focus_control = use_callback(move |()| {
        if let Some(control) = control() {
            spawn(async move {
                let _ = control.set_focus(true).await;
            });
        }
    });
    use_focus_registration(focus_control);

    let base = attributes!(button {
        class: "select {color} {size}",
    });
    let meta_attributes = meta.attributes_for(
        &FieldControlOptions::new()
            .disabled(field.disabled)
            .required(field.required)
            .name(field.name.map(Rc::from))
            .surface(FieldSurface {
                required: AttributeSurface::Omit,
                disabled: AttributeSurface::Native,
                validity: ValiditySurface::Omit,
                name: NameSurface::Native,
            }),
    );
    let mounted = attributes!(button {
        onmounted: move |event: MountedEvent| control.set(Some(event.data())),
    });
    let merged = merge_attributes(vec![meta_attributes, base, attributes, mounted]);

    rsx! {
        select::SelectTrigger { attributes: merged, {children} }
    }
}

/// What the trigger shows: the chosen option's text, or the placeholder.
///
/// The text is the primitive's (it is the `text_value` of the option that is
/// selected, which defaults to the option's own value) and so is the
/// `data-placeholder` attribute that says which of the two is showing.
#[component]
pub fn SelectValue(
    /// Whether to emit the utility that fades the placeholder.
    #[props(default)]
    appearance: SelectValueAppearance,
    /// What to show when no option is selected. The default repeats the
    /// primitive's own, since a prop declared here has to carry one.
    #[props(default = ReadSignal::new(Signal::new(String::from("Select an option"))))]
    placeholder: ReadSignal<String>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
) -> Element {
    let appearance = appearance.class();

    let base = attributes!(span {
        class: "{appearance}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        select::SelectValue { placeholder, attributes: merged }
    }
}

/// The box a select's options sit in, carrying daisyUI's `dropdown-content`
/// class and holding the `menu` list.
///
/// This is the dropdown's split, borrowed whole (ADR-0005). daisyUI puts both
/// classes on one `ul`, which is impossible here for the same three reasons it
/// is impossible there: the primitive's list element is a hardcoded `div` with
/// no `as` prop, its context is private, and every visual rule `.menu` has for
/// a row is written against a literal `li`. So the positioning and the box go
/// on the primitive's element, and `menu` goes on a list rendered inside it.
///
/// The list is marked presentational, as [`SelectOption`]'s own wrapper is: the
/// primitive gives this element the `listbox` role and the options the `option`
/// role, and a plain list between them would break the ownership a screen
/// reader announces option counts and positions from.
///
/// **The list is emitted whether the popup is open or not**, unlike the
/// dropdown's. The primitive keeps this part's children mounted while the popup
/// is closed, so that every option registers the text the trigger displays when
/// it is the chosen one, so the list cannot be rendered only while the popup
/// is. It is hidden instead, by a utility that fires exactly when the list is
/// not inside the box: `[:not(.dropdown-content)>&]:hidden`. That is also what
/// keeps the closing transition intact, since the box stays in the document
/// through it and the list stays inside the box.
///
/// Classes passed by the caller concatenate with the box's own, which is the
/// element worth reaching: a width or a fill belongs on the box rather than on
/// the list inside it.
#[component]
pub fn SelectList(
    /// Whether to emit the utilities that draw the box.
    #[props(default)]
    appearance: SelectListAppearance,
    /// daisyUI's size axis for the menu, which sizes the options.
    #[props(default)]
    size: SelectListSize,
    /// The id of this element. Declared rather than left to the attribute
    /// list, because the primitive generates one and then points the trigger's
    /// `aria-controls` at it; an id that arrived as an attribute would be
    /// written over the one the trigger names.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let appearance = appearance.class();
    let size = size.class();

    let base = attributes!(div {
        class: "dropdown-content {appearance}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        select::SelectList { id, attributes: merged,
            // The list is stretched to the box rather than left to size itself,
            // which is the one place the split has to be papered over: `.menu`
            // is `width: fit-content`, and on daisyUI's single element that is
            // the box's width as well, where here it would leave a caller's
            // width on the box with a shrink-wrapped list inside it.
            ul {
                role: "none",
                class: "menu w-full {size} [:not(.dropdown-content)>&]:hidden",
                {children}
            }
        }
    }
}

/// One option of a select, wrapped in the list item daisyUI's `menu` styles it
/// through.
///
/// The wrapper is marked presentational for the reason [`SelectList`]'s list
/// is, and costs nothing behaviourally: options register with the primitive's
/// focus collection by the `index` they are given rather than by where they sit
/// in the DOM.
///
/// The keyboard highlight needs no class (daisyUI's own rule matches
/// `:focus-visible` as well as its `.menu-focus`, and the primitive moves real
/// DOM focus onto this element) and neither does the hover one. What does need
/// one is the chosen option, which is what [`Select`] lifts the value for.
///
/// Everything the caller passes travels to the option rather than to the
/// wrapper, which is the element daisyUI styles and the primitive gives the
/// `option` role to.
#[component]
pub fn SelectOption<T: Clone + PartialEq + 'static>(
    /// What this option is worth, which is what the select's value becomes when
    /// it is chosen.
    value: ReadSignal<T>,
    /// Where this option falls in the keyboard navigation order.
    index: ReadSignal<usize>,
    /// What the trigger shows and the typeahead matches on when this option is
    /// the chosen one. The primitive falls back to the option's own value,
    /// which only works where that value is a string.
    #[props(default)]
    text_value: ReadSignal<Option<String>>,
    /// Whether this option is disabled.
    #[props(default)]
    disabled: ReadSignal<bool>,
    /// The id of this element, declared for the reason [`SelectList`]'s is:
    /// the primitive generates one and reports it as the focused option.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let selection = use_context::<Selection<T>>();

    // Tier 2 on the selected state, on the option rather than on its wrapper:
    // daisyUI's active row is `li > .menu-active`, and `.menu` matches no ARIA
    // attribute at all, so `aria-selected`, which the primitive does set, is
    // invisible to it.
    let selected = if *selection.value.read() == Some(value()) {
        "menu-active"
    } else {
        ""
    };

    // Tier 2 on the disabled state as well, and this one on the wrapper:
    // daisyUI mutes a disabled row through `.menu-disabled` on the list item or
    // a `disabled` attribute on the row, and the primitive sets neither: it
    // reports the state as `aria-disabled` and `data-disabled`, which daisyUI
    // matches nowhere.
    let state = if disabled() { "menu-disabled" } else { "" };

    let base = attributes!(div {
        class: "{selected}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        li { role: "none", class: "{state}",
            select::SelectOption::<T> {
                value,
                index,
                text_value,
                disabled,
                id,
                attributes: merged,
                {children}
            }
        }
    }
}

/// A run of options under one label, which is what a screen reader announces
/// them as belonging to.
///
/// Nothing is emitted here. daisyUI's grouped menu is flat: a `menu-title`
/// list item followed by the rows it labels, and this element is the
/// primitive's `role="group"`, which stands between the list and those rows.
/// It is left unstyled rather than given `display: contents` because `.menu`
/// lays its children out in a column and stretches them, which is what a plain
/// block between the two does anyway.
#[component]
pub fn SelectGroup(
    /// Whether every option in the group is disabled.
    #[props(default)]
    disabled: ReadSignal<bool>,
    /// The id of this element.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! {
        select::SelectGroup { disabled, id, attributes, {children} }
    }
}

/// The label of a [`SelectGroup`], wrapped in the list item daisyUI's
/// `menu-title` styles.
///
/// The class is on the wrapper rather than on the primitive's element because
/// that is where daisyUI's is: `.menu-title` is written for a list item, and
/// `.menu`'s row rules exclude one that carries it, which is what keeps a
/// label from being padded and highlighted like an option.
///
/// Everything the caller passes travels to the primitive's element, as it does
/// on [`SelectOption`]. That is also the element the group's `aria-labelledby`
/// points at, which is why `id` is a prop of its own.
#[component]
pub fn SelectGroupLabel(
    /// The id of this element. Declared rather than left to the attribute list,
    /// because the primitive generates one and points the group's
    /// `aria-labelledby` at it; an id that arrived as an attribute would leave
    /// the group named after an element that is no longer there.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! {
        li { role: "none", class: "menu-title",
            select::SelectGroupLabel { id, attributes, {children} }
        }
    }
}
