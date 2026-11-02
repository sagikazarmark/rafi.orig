//! A failed call, as the organizer sees it: a toast, or, when the server
//! gave the failure a reference (its request ID, for a server error), an
//! alert in the page that stays until closed, with the reference to copy:
//! the vendored toasts take no buttons.

use dioxus::{
    html::{ScrollBehavior, ScrollLogicalPosition, ScrollToOptions},
    prelude::*,
};
use dioxus_icons::lucide::{Copy, TriangleAlert, X};

use super::copy_to_clipboard;
use crate::{
    api::ApiError,
    components::toast::{ToastOptions, use_toast},
};

/// What went wrong, and the reference whoever runs the installation finds
/// in the server's log, when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    pub reference: Option<String>,
}

impl Problem {
    /// A problem the page noticed itself: no reference.
    pub fn notice(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reference: None,
        }
    }
}

impl From<ApiError> for Problem {
    fn from(e: ApiError) -> Self {
        Self {
            message: e.message,
            reference: e.request_id,
        }
    }
}

/// Shows what's set on `error`, then clears it: a problem without a
/// reference as a toast (until closed), one with a reference as an
/// [`ErrorAlert`] here, until closed.
#[component]
pub fn Problems(error: Signal<Option<Problem>>) -> Element {
    let toast = use_toast();
    // Each with a number of its own, its key: closing one leaves the rest.
    let mut alerts = use_signal(Vec::<(u64, Problem)>::new);
    let mut next = use_signal(|| 0_u64);
    use_effect(move || {
        let Some(problem) = error() else {
            return;
        };
        error.set(None);
        if problem.reference.is_some() {
            let n = *next.peek();
            next.set(n + 1);
            alerts.write().push((n, problem));
        } else {
            toast.error(
                "That didn't work".to_owned(),
                ToastOptions::new()
                    .description(problem.message)
                    .permanent(true),
            );
        }
    });
    rsx! {
        for (n , problem) in alerts() {
            ErrorAlert {
                key: "{n}",
                message: problem.message,
                reference: problem.reference,
                scroll: true,
                on_close: move |_| {
                    alerts.write().retain(|(m, _)| *m != n);
                },
            }
        }
    }
}

/// A failure in the page: its message and, if the server gave one, the
/// reference in monospace with a button that copies it; `on_close` adds a
/// close button, `children` more actions (Retry). `scroll` brings it into
/// view when it appears, for one raised by an action further down the page.
#[component]
pub fn ErrorAlert(
    message: String,
    reference: Option<String>,
    on_close: Option<EventHandler<()>>,
    #[props(default)] scroll: bool,
    #[props(default = VNode::empty())] children: Element,
) -> Element {
    let toast = use_toast();
    rsx! {
        div {
            role: "alert",
            class: "alert alert-error alert-soft items-start",
            onmounted: move |e| async move {
                if scroll {
                    let _ = e
                        .scroll_to_with_options(ScrollToOptions {
                            behavior: ScrollBehavior::Smooth,
                            vertical: ScrollLogicalPosition::Nearest,
                            horizontal: ScrollLogicalPosition::Nearest,
                        })
                        .await;
                }
            },
            TriangleAlert { class: "size-5" }
            div { class: "space-y-1",
                p { "{message}" }
                if let Some(reference) = reference {
                    p { class: "flex flex-wrap items-center gap-2 text-sm",
                        "Reference:"
                        code { class: "font-mono select-all", "{reference}" }
                        button {
                            class: "btn btn-xs",
                            r#type: "button",
                            title: "Copy the reference, for whoever runs this Rafi installation",
                            onclick: move |_| {
                                copy_to_clipboard(reference.clone());
                                toast.info("Reference copied".to_owned(), ToastOptions::new());
                            },
                            Copy { class: "size-3.5" }
                            "Copy reference"
                        }
                    }
                }
            }
            div { class: "flex items-center gap-1",
                {children}
                if let Some(close) = on_close {
                    button {
                        class: "btn btn-ghost btn-sm btn-square",
                        r#type: "button",
                        aria_label: "Close",
                        onclick: move |_| close.call(()),
                        X { class: "size-4" }
                    }
                }
            }
        }
    }
}

/// The alert, server-rendered natively.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::toast::ToastProvider;

    #[test]
    fn a_problem_from_an_api_error_keeps_its_reference() {
        let e = ApiError::new("internal", "Something went wrong.").with_request_id("req-42");
        assert_eq!(
            Problem::from(e),
            Problem {
                message: "Something went wrong.".to_owned(),
                reference: Some("req-42".to_owned()),
            }
        );
        let e = ApiError::new("state", "The list is frozen.");
        assert_eq!(Problem::from(e), Problem::notice("The list is frozen."));
    }

    #[test]
    fn the_alert_shows_the_message_the_reference_and_copy() {
        #[component]
        fn Shown() -> Element {
            rsx! {
                ToastProvider {
                    ErrorAlert {
                        message: "Something went wrong.",
                        reference: Some("req-42".to_owned()),
                        on_close: |_| {},
                    }
                }
            }
        }
        let html = dioxus_ssr::render_element(rsx! { Shown {} });
        for expected in [
            r#"role="alert""#,
            "Something went wrong.",
            r#"<code class="font-mono select-all">req-42</code>"#,
            "Copy reference",
            r#"aria-label="Close""#,
        ] {
            assert!(html.contains(expected), "{expected} in {html}");
        }
    }

    #[test]
    fn without_a_reference_or_close_there_is_neither() {
        #[component]
        fn Shown() -> Element {
            rsx! {
                ToastProvider {
                    ErrorAlert { message: "The list is frozen.", reference: None }
                }
            }
        }
        let html = dioxus_ssr::render_element(rsx! { Shown {} });
        assert!(html.contains("The list is frozen."), "{html}");
        assert!(!html.contains("Copy reference"), "{html}");
        assert!(!html.contains("Close"), "{html}");
    }
}
