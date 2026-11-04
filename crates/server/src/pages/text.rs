//! The raffle's rules and its privacy notice, and the operator's own legal
//! documents: text pages.

use dioxus::prelude::*;
use dioxus_icons::lucide::ArrowLeft;
pub use rafi_lifecycle::settings::ErasureDates;

pub use crate::config::Hosting;

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

/// Who runs the installation, as every notice names them, and what of the
/// notice depends on them: their documents, and where they run it.
#[derive(Debug, Clone, Copy)]
pub struct Operator<'a> {
    pub name: &'a str,
    pub email: &'a str,
    /// The operator's own privacy notice's version (at `/privacy`); `None`
    /// when they haven't provided one.
    pub privacy_version: Option<&'a str>,
    /// Whether they have provided terms (with the data processing
    /// agreement, at `/terms`).
    pub terms: bool,
    /// Where the service runs: the hosting the notice names.
    pub hosting: Hosting,
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
    document(title, body, None, None)
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
            "The raffle is for the people attending the event in person. Entrants are the people who join with the QR code while registration is open, and the people the organizer adds (for example, from the event's guest list). If the organizer added you, you can ask them to take you off.".into(),
            "Organizers and volunteers cannot win. Sponsor employees cannot win their own sponsor's prizes.".into(),
            "Entry is free. No purchase or donation is necessary. One entry per person.".into(),
            format!("Entrants must be {} or older: by joining, you confirm you are; the organizer is responsible for the people it adds meeting the age limit.", r.min_age).into(),
        ]),
        ("How the draw works".to_owned(), vec![
            "When the draw starts, the list of entrants is frozen and published (as a hash and as a list of anonymous IDs): nobody can join or be added after that.".into(),
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
    text_page(&title, rafi_api::routes::join(r.raffle.join_code), sections)
}

/// The raffle's privacy notice, as HTML: the controller, what is collected
/// and why from someone who joins and from someone the organizer adds, what
/// is public, the three erasure dates, the operator as the processor and
/// what the software itself decides of the processing ([`processors`]), the
/// entrant's rights, then whatever the organizer adds. Built in, whatever
/// documents the operator has provided.
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
            "If you join with the QR code: your display name and email address, to run the prize draw you entered under the raffle rules (GDPR Art. 6(1)(b)): to tell entries apart, to identify winners and to deliver prizes.".into(),
            "Duplicate and abuse checks (one entry per email address, rate limiting, a proof-of-work check that runs in your browser when you join) are based on our legitimate interest in a fair draw (Art. 6(1)(f)).".into(),
            "To join with the QR code, your email address is required: without it you can't join. A display name is optional. Nobody's email is ever shown on the screen or shared with sponsors.".into(),
            "The draw is automated and random, from public randomness anyone can check; it makes no decision with legal or similarly significant effects on you.".into(),
        ]),
        ("If the organizer added you".to_owned(), added(raffle, organizer)),
        ("What is public".to_owned(), vec![
            "Your display name appears on the event screen and on the verification page, which lists every entrant's display name so everyone can check they were in, until your data is erased. If you join with the QR code, it doesn't have to be your real name: leave it blank and we pick one for you. The organizer can hide anyone's name.".into(),
            "The draw itself uses only random anonymous IDs. Your email address is never published.".into(),
        ]),
        ("Cookies".to_owned(), vec![
            "When you join with the QR code, we set one strictly necessary cookie that remembers your entry on that device. No analytics or tracking.".into(),
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
            after_erasure(operator).into(),
        ]),
        ("Who processes it".to_owned(), processors(operator)),
        ("Your rights".to_owned(), vec![
            "You can request access, correction, erasure, restriction, or object to processing, and receive the data you gave in a portable form, by contacting the controller. You can also lodge a complaint with your local data protection supervisory authority.".into(),
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
    text_page(&title, rafi_api::routes::join(raffle.join_code), sections)
}

/// The notice's section for the people the organizer adds (one by one, or
/// from a list) rather than joining themselves: what Art. 14 asks of data
/// that didn't come from its person on a form (the categories, the source,
/// the purpose and the basis, with the interest named), then the right to
/// object, on its own as Art. 21(4) wants it, with what it does before and
/// after the freeze. Always there: in any raffle, the organizer may add
/// anyone by hand.
fn added(raffle: &Raffle<'_>, organizer: &Organizer<'_>) -> Vec<Para> {
    let who = if organizer.left || organizer.name.is_empty() {
        "the organizer".to_owned()
    } else {
        organizer.name.to_owned()
    };
    let ask = if organizer.left || organizer.privacy_contact.is_empty() {
        who
    } else {
        format!("{who} ({})", organizer.privacy_contact)
    };
    vec![
        format!(
            "The organizer may add you to this raffle themselves, one by one or from a list, instead of you joining with the QR code. Then we hold your name and, if the organizer gives it, your email address: nothing else. They come from you, at the event, or from the organizer's records about the event, such as its registration or guest list. They are used for this prize draw only, on the basis of the organizer's legitimate interest in rewarding the attendees of “{}” with a prize draw (GDPR Art. 6(1)(f)).",
            raffle.title
        )
        .into(),
        format!(
            "You can object to this at any time: ask {ask} to take you off. Before the list of entrants is frozen for the draw, you are removed, and stay out if the organizer adds the list again. After that, your name is hidden and your data is erased when the raffle is over; only your anonymous ID stays in the draw, so it can still be verified."
        )
        .into(),
        "Your name is shown on the event screen and on the verification page, as everyone's is, and erased on the same dates as everyone's (below).".into(),
    ]
}

/// What stays after erasure, and where erased data may linger: the
/// hosting's point-in-time recovery (on Cloudflare, D1's Time Travel, whose
/// 30 days are the Workers paid plan's), and backups, which only the
/// operator's notice can put a time on.
fn after_erasure(operator: &Operator<'_>) -> String {
    let lingers = match operator.hosting {
        Hosting::Cloudflare => {
            "Erased data can stay in the database's point-in-time recovery (Cloudflare D1 Time Travel) for up to 30 days, and in any backup the operator keeps"
        }
        Hosting::Native => {
            "Backups and the hosting provider's point-in-time recovery may hold erased data a while longer"
        }
    };
    let how_long = if operator.privacy_version.is_some() {
        "; the operator's privacy notice says how long"
    } else {
        ""
    };
    format!(
        "After erasure, the anonymous entry keeps when it joined, how it joined, whether a prize was delivered and whether its name was hidden. {lingers}{how_long}."
    )
}

/// Who processes the raffle's data: the operator, as the organizer's
/// processor (under the data processing agreement only if their terms are
/// there to hold one), then what the software itself decides, the same
/// whatever the operator has written: the hosting, and the providers that
/// receive none of an entrant's data. Then the operator's own privacy notice,
/// or that there is none.
fn processors(operator: &Operator<'_>) -> Vec<Para> {
    let runs = format!(
        "{} ({}) runs this service and processes the raffle's data on the organizer's behalf, as its processor",
        operator.name, operator.email
    );
    let runs = if operator.terms {
        Para::Link {
            text: format!("{runs}, under the data processing agreement in its terms."),
            href: "/terms".into(),
            label: "The service's terms".into(),
        }
    } else {
        format!("{runs}.").into()
    };
    let hosting = match operator.hosting {
        Hosting::Cloudflare => {
            "The service runs on Cloudflare (Cloudflare, Inc., a US company), whose Workers, D1 database and Durable Objects hold the raffle's data on the operator's behalf."
        }
        Hosting::Native => {
            "The service runs on hosting the operator chose, which holds the raffle's data on the operator's behalf."
        }
    };
    let software = format!(
        "How the Rafi software works: {hosting} The raffle engine (Restate), organizer sign-in (Clerk) and the public randomness network (drand) receive none of your personal data."
    );
    let own = match operator.privacy_version {
        Some(version) => Para::Link {
            text: format!(
                "The operator's own privacy notice (version {version}) covers the service as they run it."
            ),
            href: "/privacy".into(),
            label: "The service's privacy notice".into(),
        },
        None => "The operator hasn't published a privacy notice of its own.".into(),
    };
    vec![runs, software.into(), own]
}

/// One of the operator's documents (their privacy notice, their terms), as
/// HTML: the Markdown rendered, with its version; or, when they haven't
/// provided it (`None`), a page that says so, `missing` naming what isn't
/// there ("a privacy notice").
pub fn legal(title: &str, doc: Option<(&str, &str)>, missing: &str) -> String {
    let title_owned = title.to_owned();
    let content = match doc {
        Some((version, markdown_text)) => {
            let html = markdown(markdown_text);
            let version = version.to_owned();
            rsx! {
                p { class: "text-sm text-base-content/60", "{title_owned} · version {version}" }
                article { class: "prose card bg-base-100 p-6 shadow-sm ring-1 ring-base-300 max-w-none",
                    dangerous_inner_html: "{html}"
                }
            }
        }
        None => rsx! {
            h1 { class: "text-3xl font-extrabold", "{title_owned}" }
            p { class: "text-base-content/80", "The operator of this service hasn't provided {missing}." }
        },
    };
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-2xl flex-col gap-4 px-4 py-8",
            a { class: "btn btn-ghost btn-sm w-fit", href: "/",
                ArrowLeft { class: "size-4" }
                "rafi"
            }
            {content}
        }
    });
    document(title, body, None, None)
}

/// Whether a link may point there: a web page, a mail address or a place on
/// this page. The documents are the operator's, so this guards against a
/// slip (a pasted `javascript:` or `data:` link), not an attacker.
fn safe_link(dest: &str) -> bool {
    let dest = dest.trim();
    let lower = dest.to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || dest.starts_with('#')
        || dest.starts_with('/')
}

/// Markdown as HTML: CommonMark with tables, strikethrough and footnotes;
/// raw HTML in the source is escaped, never passed through, and a link or
/// image to anything but a web page, a mail address or this page is
/// rendered as its text. Tables' columns are left unaligned: pulldown-cmark
/// aligns them with `style` attributes, which the pages' Content-Security-Policy
/// doesn't apply (`crate::csp`), so the markup carries none.
fn markdown(text: &str) -> String {
    use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd, html};
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES;
    let mut dropped: usize = 0;
    let parser = Parser::new_ext(text, options).filter_map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Some(Event::Text(raw)),
        Event::Start(Tag::Table(columns)) => Some(Event::Start(Tag::Table(
            columns.iter().map(|_| Alignment::None).collect(),
        ))),
        Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. })
            if !safe_link(&dest_url) =>
        {
            dropped += 1;
            None
        }
        Event::End(TagEnd::Link | TagEnd::Image) if dropped > 0 => {
            dropped -= 1;
            None
        }
        other => Some(other),
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
        // Links to a page, a mail address or this document stay; anything
        // else is rendered as its text.
        let html = markdown(
            "[site](https://example.org) [mail](mailto:p@example.org) [here](#top) [own](/privacy) [bad](javascript:alert(1)) [data](data:text/html,x)",
        );
        assert!(
            html.contains(r#"<a href="https://example.org">site</a>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="mailto:p@example.org">mail</a>"#),
            "{html}"
        );
        assert!(html.contains(r##"<a href="#top">here</a>"##), "{html}");
        assert!(html.contains(r#"<a href="/privacy">own</a>"#), "{html}");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(!html.contains("data:"), "{html}");
        assert!(
            html.contains("bad") && html.contains("data"),
            "the text stays: {html}"
        );
    }

    #[test]
    fn markdown_tables_carry_no_style() {
        let html = markdown("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n");
        assert!(
            html.contains("<table>") && html.contains("<td>3</td>"),
            "{html}"
        );
        assert!(!html.contains("style="), "{html}");
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
