use dioxus::core::AttributeValue;
use dioxus::prelude::*;
use dioxus_primitives::alert_dialog;
use dioxus_primitives::dioxus_attributes::attributes;
use dioxus_primitives::merge_attributes;

/// daisyUI's placement axis for an alert dialog, which places the box within the
/// modal.
///
/// [`AlertDialogPlacement::Default`] emits no class. It is close to
/// [`AlertDialogPlacement::Middle`] (both centre the box) but not a synonym
/// for it: daisyUI's explicit middle also caps the box's height at the viewport
/// less `5em`, where an unclassed modal lets it grow to the full viewport.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum AlertDialogPlacement {
    #[default]
    Default,
    Top,
    Middle,
    Bottom,
    /// The inline start edge, which follows the writing direction: the left in
    /// a left-to-right document, the right in a right-to-left one.
    Start,
    /// The inline end edge, mirroring [`AlertDialogPlacement::Start`].
    End,
}

impl AlertDialogPlacement {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::Top,
        Self::Middle,
        Self::Bottom,
        Self::Start,
        Self::End,
    ];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Top => "modal-top",
            Self::Middle => "modal-middle",
            Self::Bottom => "modal-bottom",
            Self::Start => "modal-start",
            Self::End => "modal-end",
        }
    }
}

/// daisyUI's colour axis for the two buttons an alert dialog ends with.
///
/// These are the button component's classes, written out again rather than
/// depended on: the registry uses no cross-component dependencies, because a
/// bare dependency name resolves against the upstream registry rather than this
/// one. The Tailwind contract already requires every class name to be a literal
/// in scanned source, so there is nothing to share but the literal itself.
///
/// [`AlertDialogButtonColor::Default`] emits no class at all, which is
/// daisyUI's own uncoloured button.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum AlertDialogButtonColor {
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

impl AlertDialogButtonColor {
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
            Self::Neutral => "btn-neutral",
            Self::Primary => "btn-primary",
            Self::Secondary => "btn-secondary",
            Self::Accent => "btn-accent",
            Self::Info => "btn-info",
            Self::Success => "btn-success",
            Self::Warning => "btn-warning",
            Self::Error => "btn-error",
        }
    }
}

/// Whether [`AlertDialogTitle`] emits the utilities that make it look like a
/// title.
///
/// daisyUI has no class for a modal's title, so the ones it uses in its own
/// examples are emitted here instead. That inverts the usual convention:
/// [`AlertDialogTitleAppearance::Default`] emits classes and
/// [`AlertDialogTitleAppearance::None`] emits nothing, because a utility this
/// component emits only ties with a caller's, and a tie is settled by
/// generated-stylesheet order rather than by the class attribute. Switching
/// ours off is the way to win it (ADR-0004).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum AlertDialogTitleAppearance {
    #[default]
    Default,
    None,
}

impl AlertDialogTitleAppearance {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Default, Self::None];

    /// The Tailwind utilities for this value, as complete string literals so
    /// Tailwind's scanner can see them.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "text-lg font-bold",
            Self::None => "",
        }
    }
}

/// Whether [`AlertDialogDescription`] emits the utilities that space it from
/// the title.
///
/// The inverse convention of the other axes, for the reason
/// [`AlertDialogTitleAppearance`] records.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum AlertDialogDescriptionAppearance {
    #[default]
    Default,
    None,
}

impl AlertDialogDescriptionAppearance {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Default, Self::None];

    /// The Tailwind utilities for this value, as complete string literals so
    /// Tailwind's scanner can see them.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "py-4",
            Self::None => "",
        }
    }
}

/// The outer element of an alert dialog, carrying daisyUI's `modal` classes.
///
/// This is where the open state is **lifted** (ADR-0006), exactly as the
/// dialog's is: daisyUI hides `.modal` outright and reveals it only through
/// `.modal.modal-open`, which matches no attribute the primitive sets. So the
/// class is emitted from Rust and this component owns the state to emit it.
///
/// What differs from the dialog is that the primitive unmounts **this element**
/// when the dialog closes, rather than only the box inside it, so a closed
/// alert dialog is not a hidden modal, it is nothing at all. The class still
/// has to be emitted, because the element is mounted before it is revealed and
/// stays mounted through the exit animation.
///
/// The dim behind the box is drawn by this element, which is how daisyUI draws
/// it.
///
/// Classes passed by the caller concatenate with this element's own; every
/// other attribute the caller passes overrides them.
#[component]
pub fn AlertDialogRoot(
    /// daisyUI's placement axis.
    #[props(default)]
    placement: AlertDialogPlacement,
    /// The id of this element. Declared rather than left to the attribute
    /// list, because the primitive generates one and then looks the element up
    /// by it; an id that arrived as an attribute would be written over the one
    /// it is looking for.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    /// The controlled open state of the dialog.
    #[props(default)]
    open: ReadSignal<Option<bool>>,
    /// The state the dialog starts in when it is not controlled.
    #[props(default)]
    default_open: bool,
    /// Called when the open state changes.
    #[props(default)]
    on_open_change: Callback<bool>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let mut uncontrolled = use_signal(|| default_open);

    // Both are read here rather than inside the markup, and eagerly rather
    // than only when the other is absent, so that this component subscribes to
    // whichever of them is driving and re-renders, which is what puts the
    // modifier class below on the element and takes it off again.
    let is_open = open().unwrap_or(uncontrolled());

    let placement = placement.class();
    // Tier 2, and the whole reason the state is lifted: daisyUI's own modifier
    // class, emitted from Rust as a complete literal so that Tailwind's
    // scanner sees it too.
    let state = if is_open { "modal-open" } else { "" };

    let base = attributes!(div {
        class: "modal {placement} {state}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        alert_dialog::AlertDialogRoot {
            id,
            open: Some(is_open),
            on_open_change: move |open| {
                uncontrolled.set(open);
                on_open_change.call(open);
            },
            attributes: merged,
            {children}
        }
    }
}

/// The box an alert dialog's content sits in, carrying daisyUI's `modal-box`
/// class.
///
/// This is the element that traps focus while the dialog is open and, unlike
/// the dialog's box, the one that does *not* dismiss on a click outside itself.
/// That is the point of an alert dialog: the decision has to be taken rather
/// than dismissed.
///
/// It must stay a direct child of [`AlertDialogRoot`]: daisyUI's open rule
/// reaches the box through a child combinator, so a wrapper between them would
/// leave the box invisible.
#[component]
pub fn AlertDialogContent(
    /// The id of this element, declared for the reason [`AlertDialogRoot`]'s
    /// is.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let base = attributes!(div { class: "modal-box" });
    let mut merged = merge_attributes(vec![base, attributes]);

    // The merged class travels through the primitive's own `class` prop, which
    // is the one part that takes one. Left in the attribute list it would
    // arrive at an element that already has a class attribute on it, and which
    // of the two lands is a question about the renderer rather than about this
    // component.
    let class = take_class(&mut merged);

    rsx! {
        alert_dialog::AlertDialogContent { id, class: Some(class), attributes: merged, {children} }
    }
}

/// An alert dialog's title, which is what the dialog is announced by.
///
/// The primitive points the dialog's `aria-labelledby` at this element, so an
/// alert dialog with no title is an unnamed one.
#[component]
pub fn AlertDialogTitle(
    /// Whether to emit the utilities that make this look like a title.
    #[props(default)]
    appearance: AlertDialogTitleAppearance,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let appearance = appearance.class();

    let base = attributes!(h2 {
        class: "{appearance}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        alert_dialog::AlertDialogTitle { attributes: merged, {children} }
    }
}

/// An alert dialog's description, which is what it is announced by after its
/// title.
///
/// The primitive points the dialog's `aria-describedby` at this element.
#[component]
pub fn AlertDialogDescription(
    /// Whether to emit the utilities that space this from the title.
    #[props(default)]
    appearance: AlertDialogDescriptionAppearance,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let appearance = appearance.class();

    let base = attributes!(p {
        class: "{appearance}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        alert_dialog::AlertDialogDescription { attributes: merged, {children} }
    }
}

/// The row an alert dialog ends with, carrying daisyUI's `modal-action` class.
///
/// Unlike the dialog's, this row is not optional in practice: an alert dialog
/// that cannot be dismissed by clicking outside it needs somewhere to say yes
/// and somewhere to say no.
#[component]
pub fn AlertDialogActions(
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let base = attributes!(div {
        class: "modal-action",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        alert_dialog::AlertDialogActions { attributes: merged, {children} }
    }
}

/// The button that takes the decision, carrying daisyUI's `btn` class.
///
/// It closes the dialog before running the caller's handler, which is the
/// primitive's doing rather than this component's.
#[component]
pub fn AlertDialogAction(
    /// daisyUI's colour axis.
    #[props(default)]
    color: AlertDialogButtonColor,
    /// Called when the button is pressed, after the dialog has closed.
    #[props(default)]
    on_click: Option<EventHandler<MouseEvent>>,
    #[props(extends = GlobalAttributes)]
    #[props(extends = button)]
    attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let color = color.class();

    let base = attributes!(button {
        class: "btn {color}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        alert_dialog::AlertDialogAction { on_click, attributes: merged, {children} }
    }
}

/// The button that declines it, carrying daisyUI's `btn` class.
///
/// It closes the dialog the same way [`AlertDialogAction`] does. The two are
/// separate parts because what they mean is separate: which one is the
/// dangerous one is the caller's to say, through the colour axis.
#[component]
pub fn AlertDialogCancel(
    /// daisyUI's colour axis.
    #[props(default)]
    color: AlertDialogButtonColor,
    /// Called when the button is pressed, after the dialog has closed.
    #[props(default)]
    on_click: Option<EventHandler<MouseEvent>>,
    #[props(extends = GlobalAttributes)]
    #[props(extends = button)]
    attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let color = color.class();

    let base = attributes!(button {
        class: "btn {color}",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        alert_dialog::AlertDialogCancel { on_click, attributes: merged, {children} }
    }
}

/// An alert dialog, as one component rather than as [`AlertDialogRoot`] wrapped
/// around [`AlertDialogContent`].
///
/// The collapse is legal for the reason the dialog's is: daisyUI's modal has
/// nothing between those two elements and no caller content can go there.
///
/// **Caller attributes land on the box**, which is the element worth reaching:
/// a class here sizes or repaints the box. The outer element is reached through
/// `placement`, or by dropping to the parts.
#[component]
pub fn AlertDialog(
    /// daisyUI's placement axis, which styles the outer element.
    #[props(default)]
    placement: AlertDialogPlacement,
    /// The id of the box.
    #[props(default)]
    id: ReadSignal<Option<String>>,
    /// The controlled open state of the dialog.
    #[props(default)]
    open: ReadSignal<Option<bool>>,
    /// The state the dialog starts in when it is not controlled.
    #[props(default)]
    default_open: bool,
    /// Called when the open state changes.
    #[props(default)]
    on_open_change: Callback<bool>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! {
        AlertDialogRoot { placement, open, default_open, on_open_change,
            AlertDialogContent { id, attributes, {children} }
        }
    }
}

/// Takes the class out of a merged attribute list, so that it can be passed to
/// a primitive that takes one as a prop of its own.
///
/// `merge_attributes` has already concatenated the caller's class with this
/// component's by the time this runs, so there is exactly one to take, as
/// long as it is text, which is the only kind of class `rsx!` produces and the
/// only kind that could have been concatenated in the first place. Anything
/// else is left where it is, to travel on as an attribute.
fn take_class(attributes: &mut Vec<Attribute>) -> String {
    let class = attributes.iter().position(|attribute| {
        attribute.name == "class" && matches!(attribute.value, AttributeValue::Text(_))
    });

    match class.map(|index| attributes.remove(index).value) {
        Some(AttributeValue::Text(class)) => class,
        _ => String::new(),
    }
}
