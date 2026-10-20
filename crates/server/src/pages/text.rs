//! The raffle's rules and its privacy notice: text pages.

use dioxus::prelude::*;
use dioxus_icons::lucide::ArrowLeft;

use super::participant::{Raffle, document};

/// Who runs the raffle, as the rules and the privacy notice name them.
#[derive(Debug, Clone, Copy)]
pub struct Organizer<'a> {
    /// Empty if not given.
    pub name: &'a str,
    /// Empty if not given.
    pub contact_email: &'a str,
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
    /// The organizer's own additional rules, one paragraph per line: where an
    /// age limit goes, if the raffle has one.
    pub additional: &'a str,
}

/// A text page: headed sections of paragraphs, with a link back to the
/// participant page.
fn text_page(title: &str, back: String, sections: Vec<(String, Vec<String>)>) -> String {
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
                            p { class: "text-base-content/80", "{p}" }
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
    let prizes: Vec<String> = r
        .prizes
        .iter()
        .map(|i| {
            let sponsor = if i.sponsor.is_empty() {
                String::new()
            } else {
                format!(" ({})", i.sponsor)
            };
            format!("{} × {}{sponsor}", i.qty, i.name)
        })
        .collect();
    let organizer = if r.organizer.name.is_empty() {
        "the organizer".to_owned()
    } else {
        r.organizer.name.to_owned()
    };
    let contact = r.organizer.contact();
    let mut sections = vec![
        ("Organizer".to_owned(), vec![format!("{organizer}{contact}")]),
        ("Prizes".to_owned(), if prizes.is_empty() { vec!["To be announced.".into()] } else { prizes }),
        ("Who can take part".to_owned(), vec![
            "Anyone attending the event in person. Organizers and volunteers cannot win. Sponsor employees cannot win their own sponsor's prizes.".into(),
            "Entry is free. No purchase or donation is necessary. One entry per person.".into(),
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
                .map(str::to_owned)
                .collect(),
        ));
    }
    let title = format!("Rules — {}", r.raffle.title);
    text_page(&title, format!("/j/{}", r.raffle.join_code), sections)
}

/// The privacy notice, as HTML.
pub fn privacy(raffle: Raffle<'_>, organizer: Organizer<'_>) -> String {
    let controller = if organizer.name.is_empty() {
        "The raffle organizer".to_owned()
    } else {
        organizer.name.to_owned()
    };
    let contact = organizer.contact();
    let sections = vec![
        ("Controller".to_owned(), vec![format!("{controller}{contact}")]),
        ("What we collect and why".to_owned(), vec![
            "Your display name and email address, to run the prize draw you entered under the raffle rules (GDPR Art. 6(1)(b)): to tell entries apart, to identify winners and to deliver prizes.".into(),
            "Duplicate and abuse checks (one entry per email address, rate limiting, a proof-of-work check that runs in your browser) are based on our legitimate interest in a fair draw (Art. 6(1)(f)).".into(),
            "If you tick the optional box, we also use your email to tell you about future meetups, based on your consent (Art. 6(1)(a)). You can withdraw it at any time by writing to the contact above.".into(),
            "Your email address is required to enter; a display name is optional. Your email is never shown on the screen or shared with sponsors.".into(),
        ]),
        ("What is public".to_owned(), vec![
            "Your display name appears on the event screen and on the verification page, which lists every entrant's display name so everyone can check they were in. It doesn't have to be your real name: leave it blank and we pick one for you. The organizer can also hide it.".into(),
            "The draw itself uses only random anonymous IDs. Your email address is never published.".into(),
        ]),
        ("Cookies".to_owned(), vec![
            "We set one strictly necessary cookie that remembers your entry on this device. No analytics or tracking.".into(),
        ]),
        ("Processors".to_owned(), vec![
            "The hosting provider of this service stores the application and its database.".into(),
            "The raffle engine (Restate) only receives anonymous IDs, never names or emails.".into(),
            "Clerk, Inc. (USA, EU–US Data Privacy Framework) authenticates the organizers only; participant data is never sent to Clerk.".into(),
            "The randomness comes from the public drand network; no personal data is sent to it.".into(),
        ]),
        ("How long we keep it".to_owned(), vec![
            "Your entry is part of the draw until the raffle has finished. After that, the organizer erases personal data, or erases yours on request. Anonymous IDs and the draw log are kept so the draw stays verifiable.".into(),
        ]),
        ("Your rights".to_owned(), vec![
            "You can request access, correction, erasure, restriction, or object to processing, and withdraw consent, by contacting the controller. You can also lodge a complaint with your local data protection supervisory authority.".into(),
        ]),
    ];
    let title = format!("Privacy notice — {}", raffle.title);
    text_page(&title, format!("/j/{}", raffle.join_code), sections)
}
