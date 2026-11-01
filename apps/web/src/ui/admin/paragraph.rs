//! The paragraph the organizer sends the people they add.

use dioxus_icons::lucide::{Copy, Send};
use rafi_api::NameFormat;

use super::*;

/// How a name format reads on its button.
fn format_label(format: NameFormat) -> &'static str {
    match format {
        NameFormat::Full => "Full names",
        NameFormat::FirstInitial => "First name and initial",
    }
}

/// The paragraph to send the people the organizer adds
/// ([`RaffleAdmin::paragraph_to_send`](rafi_api::RaffleAdmin::paragraph_to_send)),
/// to copy and edit: from the saved settings, its links under the
/// installation's address. Before the event, on the event page or in a
/// confirmation email, it is how people on a list learn they are in, that
/// their name will be shown, and how to stay out.
///
/// With `format`, the names are shown so (the list tab's choice); without,
/// the organizer picks one here to see how the paragraph reads.
#[component]
pub(super) fn ParagraphToSend(
    admin: AdminData,
    #[props(default)] format: Option<NameFormat>,
) -> Element {
    let mut picked = use_signal(NameFormat::default);
    let shown = format.unwrap_or(picked());
    let text = admin.paragraph_to_send(shown);
    // The rules and the privacy notice are served once the raffle is
    // public: the paragraph's links lead nowhere before.
    let published = admin.view.state.is_public();
    let toast = use_toast();
    let copied = text.clone();

    rsx! {
        div { class: "space-y-2 rounded-box bg-base-200 p-4",
            div { class: "flex flex-wrap items-center gap-2",
                Send { class: "size-4 text-primary" }
                h3 { class: "flex-1 font-semibold", "What to tell the people you add" }
                if format.is_none() {
                    div { class: "join", role: "group", aria_label: "How their names are shown",
                        for f in NameFormat::ALL {
                            button {
                                key: "{f.as_str()}",
                                r#type: "button",
                                class: if f == shown { "btn btn-xs join-item btn-active btn-secondary" } else { "btn btn-xs join-item" },
                                aria_pressed: "{f == shown}",
                                onclick: move |_| picked.set(f),
                                "{format_label(f)}"
                            }
                        }
                    }
                }
                Button {
                    size: ButtonSize::Sm,
                    r#type: "button",
                    title: "Copy the paragraph",
                    onclick: move |_| {
                        copy_to_clipboard(copied.clone());
                        toast.info("📋 Paragraph copied".to_owned(), ToastOptions::new());
                    },
                    Copy { class: "size-4" }
                    "Copy"
                }
            }
            p { class: "text-sm text-base-content/70",
                "People you add are in the draw without seeing the rules or the privacy notice, and their names are shown on the screen and the verification page. Tell them before the event: put this on the event page or in the confirmation email, and edit it as you like."
            }
            textarea {
                class: "textarea w-full text-sm",
                readonly: true,
                rows: "5",
                aria_label: "The paragraph to send",
                value: "{text}",
            }
            if !published {
                p { class: "text-sm text-warning", "The links work once the raffle is published." }
            }
        }
    }
}
