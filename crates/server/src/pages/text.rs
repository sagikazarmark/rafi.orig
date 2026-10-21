//! The raffle's rules and its privacy notice, and the installation's own
//! legal documents: text pages.

use dioxus::prelude::*;
use dioxus_icons::lucide::ArrowLeft;
pub use rafi_lifecycle::settings::ErasureDates;

use super::participant::{Raffle, document};

/// Who runs the raffle, as the rules and the privacy notice name them.
#[derive(Debug, Clone, Copy)]
pub struct Organizer<'a> {
    /// Empty if not given.
    pub name: &'a str,
    /// Empty if not given.
    pub contact_email: &'a str,
    /// Where privacy requests go: the privacy email, or else the contact.
    /// Empty if neither.
    pub privacy_contact: &'a str,
    /// The organizer's own privacy text, one paragraph per line; empty if none.
    pub privacy_text: &'a str,
    /// The organizer's own privacy notice; empty if none.
    pub privacy_url: &'a str,
    /// The organizer removed their data from the service: the raffle is
    /// over, its personal data gone, and nobody is left to name.
    pub left: bool,
}

impl Organizer<'_> {
    fn contact(&self) -> String {
        if self.contact_email.is_empty() {
            String::new()
        } else {
            format!(" — {}", self.contact_email)
        }
    }
}

/// Who runs the installation, as every notice names them.
#[derive(Debug, Clone, Copy)]
pub struct Operator<'a> {
    pub name: &'a str,
    pub email: &'a str,
    /// The installation's own privacy notice, at `/privacy`, and its version.
    pub privacy_version: &'a str,
}

/// The one sentence the join form says about erasure.
pub fn erasure_sentence(dates: &ErasureDates) -> String {
    format!(
        "Your name and email are erased on {}; a winner's once their prize is delivered, at the latest on {}.",
        dates.erase.long(),
        dates.winners_erase.long()
    )
}

/// The age line entrants confirm by entering.
pub fn age_line(min_age: u32) -> String {
    format!("By entering you confirm you are {min_age} or older.")
}

/// A prize, as the rules list it.
#[derive(Debug, Clone, Copy)]
pub struct PrizeLine<'a> {
    pub qty: i64,
    pub name: &'a str,
    /// Empty if not sponsored.
    pub sponsor: &'a str,
}

/// What the rules page shows.
#[derive(Debug, Clone)]
pub struct Rules<'a> {
    pub raffle: Raffle<'a>,
    pub organizer: Organizer<'a>,
    pub prizes: Vec<PrizeLine<'a>>,
    /// The organizer's own additional rules, one paragraph per line: where a
    /// higher age limit goes, if the raffle has one.
    pub additional: &'a str,
    /// The installation's minimum age.
    pub min_age: u32,
}

/// What the privacy notice shows.
#[derive(Debug, Clone, Copy)]
pub struct Privacy<'a> {
    pub raffle: Raffle<'a>,
    pub organizer: Organizer<'a>,
    pub operator: Operator<'a>,
    pub retention: ErasureDates,
}

/// A paragraph: plain text, or text with a link after it.
#[derive(Debug, Clone)]
enum Para {
    Text(String),
    Link {
        text: String,
        href: String,
        label: String,
    },
}

impl From<String> for Para {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for Para {
    fn from(s: &str) -> Self {
        Self::Text(s.to_owned())
    }
}

/// A text page: headed sections of paragraphs, with a link back to `back`
/// (the participant page, or the site).
fn text_page(title: &str, back: String, sections: Vec<(String, Vec<Para>)>) -> String {
    let title_owned = title.to_owned();
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-2xl flex-col gap-4 px-4 py-8",
            a { class: "btn btn-ghost btn-sm w-fit", href: "{back}",
                ArrowLeft { class: "size-4" }
                "rafi"
            }
            h1 { class: "text-3xl font-extrabold", "{title_owned}" }
            for (heading , paras) in sections {
                section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                    div { class: "card-body gap-2",
                        h2 { class: "card-title text-lg", "{heading}" }
                        for p in paras {
                            match p {
                                Para::Text(text) => rsx! { p { class: "text-base-content/80", "{text}" } },
                                Para::Link { text, href, label } => rsx! {
                                    p { class: "text-base-content/80",
                                        "{text} "
                                        a { class: "link link-primary", href: "{href}", rel: "noopener", "{label}" }
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    });
    document(title, body, None)
}

/// The raffle rules page, as HTML.
pub fn rules(r: &Rules<'_>) -> String {
    let prizes: Vec<Para> = r
        .prizes
        .iter()
        .map(|i| {
            let sponsor = if i.sponsor.is_empty() {
                String::new()
            } else {
                format!(" ({})", i.sponsor)
            };
            format!("{} × {}{sponsor}", i.qty, i.name).into()
        })
        .collect();
    let organizer = if r.organizer.name.is_empty() {
        "the organizer".to_owned()
    } else {
        r.organizer.name.to_owned()
    };
    let contact = r.organizer.contact();
    let mut sections = vec![
        ("Organizer".to_owned(), vec![format!("{organizer}{contact}").into()]),
        ("Prizes".to_owned(), if prizes.is_empty() { vec!["To be announced.".into()] } else { prizes }),
        ("Who can take part".to_owned(), vec![
            "Anyone attending the event in person. Organizers and volunteers cannot win. Sponsor employees cannot win their own sponsor's prizes.".into(),
            "Entry is free. No purchase or donation is necessary. One entry per person.".into(),
            format!("Entrants must be {} or older: by entering, you confirm you are.", r.min_age).into(),
        ]),
        ("How the draw works".to_owned(), vec![
            "Registration closes when the draw starts and the list of entrants is frozen and published (as a hash and as a list of anonymous IDs).".into(),
            "Each prize is drawn separately. Every draw commits to a future public randomness round from drand (League of Entropy) before that round exists, so nobody — including the organizer — can know or influence the result. Anyone can re-run every draw on the verification page.".into(),
            "Each person can win at most one prize. If more prizes remain than people, previous winners can win again.".into(),
        ]),
        ("Claiming a prize".to_owned(), vec![
            "Winners must be present and claim their prize at the event, by showing the claim code on their phone or identifying themselves to the organizer. If a winner is not present or declines, they are out of the draw and the prize is drawn again.".into(),
            "Prizes cannot be exchanged for cash.".into(),
        ]),
    ];
    if !r.additional.trim().is_empty() {
        sections.push((
            "Additional rules".into(),
            r.additional
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_owned().into())
                .collect(),
        ));
    }
    let title = format!("Rules — {}", r.raffle.title);
    text_page(&title, format!("/j/{}", r.raffle.join_code), sections)
}

/// The raffle's privacy notice, as HTML: the controller, what is collected
/// and why, what is public, the three erasure dates, the operator as the
/// processor (its own notice names its subprocessors), the entrant's rights,
/// then whatever the organizer adds.
pub fn privacy(p: &Privacy<'_>) -> String {
    let Privacy {
        raffle,
        organizer,
        operator,
        retention,
    } = p;
    let controller: Vec<Para> = if organizer.left {
        vec![
            "The raffle organizer, who has since left this service. The raffle's personal data has been erased; only the anonymous draw remains, so it can still be verified.".into(),
        ]
    } else {
        let name = if organizer.name.is_empty() {
            "The raffle organizer".to_owned()
        } else {
            organizer.name.to_owned()
        };
        let contact = if organizer.privacy_contact.is_empty() {
            String::new()
        } else {
            format!(" Privacy requests: {}.", organizer.privacy_contact)
        };
        vec![format!("{name} is the data controller for this raffle.{contact}").into()]
    };
    let mut sections: Vec<(String, Vec<Para>)> = vec![
        ("Controller".to_owned(), controller),
        ("What we collect and why".to_owned(), vec![
            "Your display name and email address, to run the prize draw you entered under the raffle rules (GDPR Art. 6(1)(b)): to tell entries apart, to identify winners and to deliver prizes.".into(),
            "Duplicate and abuse checks (one entry per email address, rate limiting, a proof-of-work check that runs in your browser) are based on our legitimate interest in a fair draw (Art. 6(1)(f)).".into(),
            "Your email address is required to enter; a display name is optional. Your email is never shown on the screen or shared with sponsors.".into(),
        ]),
        ("What is public".to_owned(), vec![
            "Your display name appears on the event screen and on the verification page, which lists every entrant's display name so everyone can check they were in, until your data is erased. It doesn't have to be your real name: leave it blank and we pick one for you. The organizer can also hide it.".into(),
            "The draw itself uses only random anonymous IDs. Your email address is never published.".into(),
        ]),
        ("Cookies".to_owned(), vec![
            "We set one strictly necessary cookie that remembers your entry on this device. No analytics or tracking.".into(),
        ]),
        ("How long we keep it".to_owned(), vec![
            format!(
                "The event is on {}. If you did not win, your name and email are erased on {}, whatever else happens to the raffle. The organizer can erase them earlier, and erases yours on request.",
                retention.event.long(),
                retention.erase.long()
            ).into(),
            format!(
                "If you won, your name and email are kept until the organizer marks your prize delivered, so it can reach you, and erased on {} at the latest.",
                retention.winners_erase.long()
            ).into(),
            "Anonymous IDs and the draw log are kept so the draw stays verifiable. The organizer can export the winners' details to deliver prizes; a copy they exported is outside this service, and its erasure is the organizer's responsibility.".into(),
        ]),
        ("Who processes it".to_owned(), vec![
            Para::Link {
                text: format!(
                    "{} ({}) runs this service and processes the raffle's data on the organizer's behalf, as its processor, under a data processing agreement. Its own privacy notice (version {}) names the providers it relies on: hosting, the raffle engine, organizer sign-in and the public randomness network, none of which receives your email.",
                    operator.name, operator.email, operator.privacy_version
                ),
                href: "/privacy".into(),
                label: "The service's privacy notice".into(),
            },
        ]),
        ("Your rights".to_owned(), vec![
            "You can request access, correction, erasure, restriction, or object to processing, by contacting the controller. You can also lodge a complaint with your local data protection supervisory authority.".into(),
        ]),
    ];
    let mut own: Vec<Para> = organizer
        .privacy_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_owned().into())
        .collect();
    if !organizer.privacy_url.is_empty() {
        own.push(Para::Link {
            text: "The organizer's own privacy notice:".into(),
            href: organizer.privacy_url.to_owned(),
            label: organizer.privacy_url.to_owned(),
        });
    }
    if !own.is_empty() {
        sections.push(("From the organizer".into(), own));
    }
    let title = format!("Privacy notice — {}", raffle.title);
    text_page(&title, format!("/j/{}", raffle.join_code), sections)
}

/// One of the installation's documents (its privacy notice, its terms), as
/// HTML: the Markdown rendered, with its version.
pub fn legal(title: &str, version: &str, markdown_text: &str) -> String {
    let html = markdown(markdown_text);
    let title_owned = title.to_owned();
    let version = version.to_owned();
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-2xl flex-col gap-4 px-4 py-8",
            a { class: "btn btn-ghost btn-sm w-fit", href: "/",
                ArrowLeft { class: "size-4" }
                "rafi"
            }
            p { class: "text-sm text-base-content/60", "{title_owned} · version {version}" }
            article { class: "prose card bg-base-100 p-6 shadow-sm ring-1 ring-base-300 max-w-none",
                dangerous_inner_html: "{html}"
            }
        }
    });
    document(title, body, None)
}

/// Markdown as HTML: CommonMark with tables, strikethrough and footnotes;
/// raw HTML in the source is escaped, never passed through.
pub fn markdown(text: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, html};
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(text, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut out = String::with_capacity(text.len() * 2);
    html::push_html(&mut out, parser);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_renders_and_escapes_html() {
        let html = markdown("# Terms\n\nA **bold** <script>alert(1)</script> line.");
        assert!(html.contains("<h1>Terms</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn the_retention_sentence_names_the_dates() {
        let dates = ErasureDates::of("2026-10-28", 7, 90).unwrap();
        assert_eq!(
            erasure_sentence(&dates),
            "Your name and email are erased on 4 November 2026; a winner's once their prize is delivered, at the latest on 2 February 2027."
        );
    }
}
