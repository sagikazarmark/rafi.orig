//! The raffle's rules and its privacy notice: text pages, in English and
//! Hungarian.

use dioxus::prelude::*;

use crate::{
    i18n::Lang,
    icons::I,
    participant::{Raffle, document},
};

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
    pub min_age: i64,
    pub prizes: Vec<PrizeLine<'a>>,
    /// The organizer's own additional rules, one paragraph per line.
    pub additional: &'a str,
}

/// A text page: headed sections of paragraphs, with a link back to the
/// participant page.
fn text_page(
    lang: Lang,
    title: &str,
    back: String,
    sections: Vec<(String, Vec<String>)>,
) -> String {
    let title_owned = title.to_owned();
    let back_icon = I::ArrowLeft.svg("size-4");
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-2xl flex-col gap-4 px-4 py-8",
            a { class: "btn btn-ghost btn-sm w-fit", href: "{back}", span { dangerous_inner_html: "{back_icon}" } "rafi" }
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
    document(lang, title, body, None)
}

/// The raffle rules page, as HTML.
pub fn rules(lang: Lang, r: &Rules<'_>) -> String {
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
    let min_age = r.min_age;
    let mut sections = match lang {
        Lang::En => vec![
            ("Organizer".to_owned(), vec![format!("{organizer}{contact}")]),
            ("Prizes".to_owned(), if prizes.is_empty() { vec!["To be announced.".into()] } else { prizes }),
            ("Who can take part".to_owned(), vec![
                format!("Anyone attending the event in person who is at least {min_age} years old. Organizers and volunteers cannot win. Sponsor employees cannot win their own sponsor's prizes."),
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
        ],
        Lang::Hu => vec![
            ("Szervező".to_owned(), vec![format!("{organizer}{contact}")]),
            ("Nyeremények".to_owned(), if prizes.is_empty() { vec!["Hamarosan.".into()] } else { prizes }),
            ("Ki vehet részt".to_owned(), vec![
                format!("Bárki, aki személyesen jelen van a rendezvényen és legalább {min_age} éves. A szervezők és önkéntesek nem nyerhetnek. A szponzorok munkatársai a saját cégük nyereményeit nem nyerhetik meg."),
                "A részvétel ingyenes, vásárláshoz vagy adományhoz nem kötött. Személyenként egy jelentkezés.".into(),
            ]),
            ("A sorsolás menete".to_owned(), vec![
                "A sorsolás indulásakor a jelentkezés lezárul, a résztvevők listáját rögzítjük és nyilvánosságra hozzuk (hash formájában és anonim azonosítók listájaként).".into(),
                "Minden nyereményt külön sorsolunk. Minden sorsolás egy jövőbeli, nyilvános drand (League of Entropy) véletlenszám-körre hivatkozik, mielőtt az létezne, így senki — a szervező sem — tudhatja előre vagy befolyásolhatja az eredményt. Az ellenőrző oldalon bárki újrafuttathatja a sorsolásokat.".into(),
                "Mindenki legfeljebb egy nyereményt nyerhet. Ha több nyeremény marad, mint résztvevő, a korábbi nyertesek újra nyerhetnek.".into(),
            ]),
            ("A nyeremény átvétele".to_owned(), vec![
                "A nyertesnek jelen kell lennie, és a helyszínen kell átvennie a nyereményt a telefonján látható átvételi kóddal vagy a szervezőnél azonosítva magát. Ha a nyertes nincs jelen vagy visszautasítja, kiesik a sorsolásból, és a nyereményt újrasorsoljuk.".into(),
                "A nyeremény készpénzre nem váltható.".into(),
            ]),
        ],
    };
    if !r.additional.trim().is_empty() {
        let heading = if lang == Lang::Hu {
            "További szabályok"
        } else {
            "Additional rules"
        };
        sections.push((
            heading.into(),
            r.additional
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect(),
        ));
    }
    let title = format!("{} — {}", lang.t("rules"), r.raffle.title);
    text_page(lang, &title, format!("/j/{}", r.raffle.join_code), sections)
}

/// The privacy notice, as HTML: personal data of non-winners is erased
/// `nonwinners_days` after the event, winners' at the latest `winners_days`.
pub fn privacy(
    lang: Lang,
    raffle: Raffle<'_>,
    organizer: Organizer<'_>,
    nonwinners_days: i64,
    winners_days: i64,
) -> String {
    let controller = if organizer.name.is_empty() {
        "The raffle organizer".to_owned()
    } else {
        organizer.name.to_owned()
    };
    let contact = organizer.contact();
    let (days_nw, days_w) = (nonwinners_days, winners_days);
    let sections = match lang {
        Lang::En => vec![
            ("Controller".to_owned(), vec![format!("{controller}{contact}")]),
            ("What we collect and why".to_owned(), vec![
                "Your display name and email address, to run the prize draw you entered under the raffle rules (GDPR Art. 6(1)(b)): to tell entries apart, to identify winners and to deliver prizes.".into(),
                "Duplicate and abuse checks (one entry per email address, rate limiting, a proof-of-work check that runs in your browser) are based on our legitimate interest in a fair draw (Art. 6(1)(f)).".into(),
                "If you tick the optional box, we also use your email to tell you about future meetups, based on your consent (Art. 6(1)(a)). You can withdraw it at any time by writing to the contact above.".into(),
                "Providing your name and email is required to enter. Your email is never shown on the screen or shared with sponsors.".into(),
            ]),
            ("What is public".to_owned(), vec![
                "Your display name may appear on the event screen and on the verification page. The published entrant list contains only random anonymous IDs.".into(),
            ]),
            ("Cookies".to_owned(), vec![
                "We set one strictly necessary cookie that remembers your entry on this device, and one that remembers your language choice. No analytics or tracking.".into(),
            ]),
            ("Processors".to_owned(), vec![
                "Hetzner Online GmbH (Germany) hosts the application and database. Backups are stored with an EU object storage provider.".into(),
                "Restate GmbH (Germany) runs the raffle engine; it only receives anonymous IDs, never names or emails.".into(),
                "Clerk, Inc. (USA, EU–US Data Privacy Framework) authenticates the organizers only; participant data is never sent to Clerk.".into(),
                "The randomness comes from the public drand network; no personal data is sent to it.".into(),
            ]),
            ("How long we keep it".to_owned(), vec![
                format!("Personal data of entrants who did not win is erased {days_nw} days after the event. Winners' data is erased once the prize is delivered, at the latest {days_w} days after the event. Anonymous IDs and the draw log are kept so the draw stays verifiable."),
            ]),
            ("Your rights".to_owned(), vec![
                "You can request access, correction, erasure, restriction, or object to processing, and withdraw consent, by contacting the controller. You can also lodge a complaint with the Hungarian National Authority for Data Protection and Freedom of Information (NAIH, naih.hu).".into(),
            ]),
        ],
        Lang::Hu => vec![
            ("Adatkezelő".to_owned(), vec![format!("{controller}{contact}")]),
            ("Milyen adatot és miért kezelünk".to_owned(), vec![
                "A megjelenítendő nevedet és az e-mail címedet a sorsolás lebonyolításához kezeljük, amelyre a szabályok szerint jelentkeztél (GDPR 6. cikk (1) b)): a jelentkezések megkülönböztetéséhez, a nyertesek azonosításához és a nyeremények átadásához.".into(),
                "A duplikációk és visszaélések kiszűrése (egy jelentkezés e-mail címenként, kéréskorlátozás, a böngésződben futó proof-of-work ellenőrzés) a tisztességes sorsoláshoz fűződő jogos érdekünkön alapul (6. cikk (1) f)).".into(),
                "Ha bejelölöd a nem kötelező négyzetet, a hozzájárulásod alapján (6. cikk (1) a)) e-mailben értesítünk a következő meetupokról. A hozzájárulást bármikor visszavonhatod a fenti elérhetőségen.".into(),
                "A név és e-mail cím megadása a részvétel feltétele. Az e-mail címed sehol nem jelenik meg, és nem adjuk át szponzoroknak.".into(),
            ]),
            ("Mi nyilvános".to_owned(), vec![
                "A megjelenítendő neved megjelenhet a kivetítőn és az ellenőrző oldalon. A közzétett résztvevői lista csak véletlenszerű anonim azonosítókat tartalmaz.".into(),
            ]),
            ("Sütik".to_owned(), vec![
                "Egy feltétlenül szükséges sütit használunk, amely ezen az eszközön megjegyzi a jelentkezésedet, és egyet a nyelvválasztáshoz. Analitika vagy követés nincs.".into(),
            ]),
            ("Adatfeldolgozók".to_owned(), vec![
                "Hetzner Online GmbH (Németország): az alkalmazás és az adatbázis tárhelye. A biztonsági mentések EU-s objektumtárolóban vannak.".into(),
                "Restate GmbH (Németország): a sorsolómotor; csak anonim azonosítókat kap, nevet vagy e-mail címet soha.".into(),
                "Clerk, Inc. (USA, EU–USA adatvédelmi keretrendszer): kizárólag a szervezők bejelentkezése; résztvevői adat nem kerül a Clerkhez.".into(),
                "A véletlenszám a nyilvános drand hálózatból származik; oda személyes adat nem kerül.".into(),
            ]),
            ("Meddig őrizzük".to_owned(), vec![
                format!("A nem nyertes résztvevők személyes adatait a rendezvény után {days_nw} nappal töröljük. A nyertesek adatait a nyeremény átadása után, legkésőbb {days_w} nappal a rendezvény után töröljük. Az anonim azonosítókat és a sorsolási naplót megőrizzük, hogy a sorsolás ellenőrizhető maradjon."),
            ]),
            ("Jogaid".to_owned(), vec![
                "Kérhetsz hozzáférést, helyesbítést, törlést, korlátozást, tiltakozhatsz az adatkezelés ellen, és visszavonhatod a hozzájárulásodat az adatkezelőnél. Panaszt tehetsz a Nemzeti Adatvédelmi és Információszabadság Hatóságnál (NAIH, naih.hu).".into(),
            ]),
        ],
    };
    let title = format!("{} — {}", lang.t("privacy"), raffle.title);
    text_page(lang, &title, format!("/j/{}", raffle.join_code), sections)
}
