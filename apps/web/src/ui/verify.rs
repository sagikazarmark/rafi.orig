//! Public verification page: re-runs every draw in the browser.

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_icons::lucide::{
    Check, Dice5, Download, Flag, Lock, Search, ShieldCheck, TriangleAlert, Users, X,
};
use rafi_raffle::{EntrantId, Event, ItemId};

use super::{Brand, ThemeToggle};
use crate::api;

/// The entrant ID after `#me=` in the page's URL: the participant page links
/// here with it, so an entrant can see that they are in the frozen list.
fn me_from_url() -> Option<String> {
    let hash = web_sys::window()?.location().hash().ok()?;
    let id = hash.strip_prefix("#me=")?.trim().to_lowercase();
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

#[component]
pub fn Verify(slug: String) -> Element {
    let json_href = rafi_api::routes::verify_json(&slug);
    // Absolute, so the command runs as shown.
    let json_url = use_hook(|| {
        web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default()
    }) + &json_href;
    let build = use_hook(crate::config::build);
    let verifier = build.verifier_url.clone();
    let me = use_hook(me_from_url);
    let query = use_signal(String::new);
    let data = use_resource(move || {
        let slug = slug.clone();
        async move { api::get_verify(slug).await }
    });

    let body = match &*data.read() {
        None => {
            rsx! { div { class: "grid place-items-center py-24", span { class: "loading loading-dots loading-lg text-primary" } } }
        }
        // A reply this page can't read is a page older than the server (or
        // a record written by a newer one), not a missing raffle.
        Some(Err(e)) if e.code == api::codes::UNREADABLE => rsx! {
            Unreadable { detail: e.message.clone() }
        },
        // One this page reads but whose format it doesn't know: its rules
        // may differ, so it isn't checked by these.
        Some(Ok(d)) if unknown_format(d).is_some() => rsx! {
            Unreadable { detail: unknown_format(d).unwrap_or_default() }
        },
        Some(Err(e)) if e.code != api::codes::NOT_FOUND => rsx! {
            div { role: "alert", class: "alert alert-warning alert-soft",
                TriangleAlert { class: "size-5" }
                span { "Couldn't load this raffle's record. Check your connection and reload the page." }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "rounded-box border-2 border-dashed border-base-300 py-16 text-center",
                p { class: "text-5xl", "🔍" }
                p { class: "mt-3 text-base-content/70", "This raffle hasn't been frozen yet, or it doesn't exist." }
            }
        },
        Some(Ok(d)) => rsx! {
            Record {
                data: d.clone(),
                query,
                me: me.clone(),
                json_href: json_href.clone(),
                json_url: json_url.clone(),
                verifier: verifier.clone(),
            }
        },
    };

    rsx! {
        document::Title { "Check the draw — Rafi" }
        div { class: "min-h-screen bg-base-200",
            header { class: "border-b border-base-300 bg-base-100/85 backdrop-blur",
                div { class: "mx-auto flex max-w-5xl items-center gap-2 px-4 py-2.5",
                    Brand {}
                    span { class: "flex-1" }
                    ThemeToggle {}
                }
            }
            main { class: "mx-auto max-w-5xl space-y-6 px-4 py-8", {body} }
            super::Colophon { info: build }
        }
    }
}

/// An entrant in the frozen list: their ID and their name, if they have
/// one left (an erased entrant hasn't).
type Listed = (String, Option<String>);

/// What the page shows of one record that doesn't depend on the search:
/// every draw replayed and every beacon checked, which is the slow part
/// (a BLS check per draw), so it is worked out once per record
/// ([`Checked::of`]), not on each keystroke in the search box.
#[derive(PartialEq)]
struct Checked {
    report: rafi_raffle::Report,
    /// The manifest, if it reads.
    doc: Option<rafi_raffle::Manifest>,
    /// Entrant ID → display name.
    names: HashMap<String, String>,
    item_names: HashMap<ItemId, String>,
    /// Everyone in the frozen list, by name (erased entrants have none
    /// left), then by ID.
    listed: Vec<Listed>,
}

#[cfg(test)]
thread_local! {
    /// How many times [`Checked::of`] ran on this thread.
    static CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Checked {
    fn of(d: &rafi_api::VerifyData) -> Self {
        #[cfg(test)]
        CHECKS.with(|n| n.set(n.get() + 1));
        // Public drand chains only: a test chain is never reported as verified.
        let report = rafi_raffle::verify_public(&d.manifest, &d.manifest_hash, &d.events);
        let names: HashMap<String, String> = d.names.iter().cloned().collect();
        let doc: Option<rafi_raffle::Manifest> = serde_json::from_str(&d.manifest).ok();
        let item_names: HashMap<ItemId, String> = doc
            .as_ref()
            .map(|d| {
                d.items
                    .iter()
                    .map(|u| (u.id.clone(), u.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut listed: Vec<Listed> = doc
            .as_ref()
            .map(|d| {
                d.entrants
                    .iter()
                    .map(|id| (id.to_string(), names.get(id.as_str()).cloned()))
                    .collect()
            })
            .unwrap_or_default();
        listed.sort_by_cached_key(|(id, name)| {
            (
                name.is_none(),
                name.as_deref().map(str::to_lowercase),
                id.clone(),
            )
        });
        Self {
            report,
            doc,
            names,
            item_names,
            listed,
        }
    }
}

/// A record this page reads, checked: the verdict, the draws, the entrants
/// and the event log. The search box's text is `query`, which only
/// [`Entrants`] reads, so typing in it doesn't render this again, and the
/// check runs once per record whatever does.
#[component]
fn Record(
    data: ReadSignal<rafi_api::VerifyData>,
    query: Signal<String>,
    me: Option<String>,
    json_href: String,
    json_url: String,
    verifier: Option<String>,
) -> Element {
    let checked = use_memo(move || Checked::of(&data.read()));
    let d = data.read();
    let c = checked.read();
    let Checked {
        report,
        doc,
        names,
        item_names,
        listed,
    } = &*c;
    let test_chain = !report.beacons_verified
        && report.manifest_hash_ok
        && report.errors.is_empty()
        && report.draws.iter().all(|c| c.ok);
    let chain = report
        .beacon
        .as_ref()
        .and_then(|b| b.drand_chain())
        .map(ToString::to_string)
        .unwrap_or_default();
    let ok = report.ok();
    let manifest = d.manifest.clone();
    let entrants = doc.as_ref().map(|d| d.entrants.len()).unwrap_or_default();
    let mine = me
        .as_ref()
        .map(|me| (me.clone(), listed.iter().find(|(id, _)| id == me).cloned()));
    let prizes = doc.as_ref().map(|d| d.items.len()).unwrap_or_default();
    rsx! {
        div { class: "space-y-2",
            p { class: "badge badge-soft badge-primary gap-1 font-semibold", ShieldCheck { class: "size-3.5" } "Independent verification" }
            h1 { class: "text-4xl font-extrabold", "{d.title}" }
        }
        if d.test_mode {
            div { role: "note", class: "alert alert-warning shadow-sm",
                span { class: "text-3xl", "🧪" }
                div {
                    p { class: "font-display text-xl font-extrabold", "TEST MODE: a rehearsal." }
                    p { class: "text-sm opacity-90", "The organizer ran this raffle to try it out. Its entrants may be made up, and its prizes aren't real." }
                }
            }
        }
        match &mine {
            Some((_, Some((id, name)))) => rsx! {
                div { role: "status", class: "alert alert-success alert-soft",
                    span { class: "text-2xl", "🎟️" }
                    div {
                        p { class: "font-semibold", "You're in." }
                        p { class: "text-sm wrap-anywhere",
                            "Your entry, "
                            if let Some(name) = name { strong { bdi { "{name}" } " " } }
                            span { class: "font-mono", "{short(id)}…" }
                            ", is in the frozen entrant list that every draw below was made from."
                        }
                    }
                }
            },
            Some((id, None)) => rsx! {
                div { role: "alert", class: "alert alert-warning alert-soft",
                    TriangleAlert { class: "size-5" }
                    p { class: "text-sm",
                        "The entry "
                        span { class: "font-mono", "{short(id)}…" }
                        " from your link is not in this raffle's frozen entrant list. If you think it should be, ask the organizer."
                    }
                }
            },
            None => rsx! {},
        }
        if ok {
            div { role: "status", class: "alert alert-success shadow-sm animate-pop",
                span { class: "text-3xl", "✅" }
                div {
                    p { class: "font-display text-xl font-extrabold", "All {report.draws.len()} draws check out." }
                    p { class: "text-sm opacity-90", "Your browser re-ran every draw from the published entrant list and the public drand randomness." }
                }
            }
        } else if test_chain {
            div { role: "alert", class: "alert alert-info shadow-sm",
                span { class: "text-3xl", "🧪" }
                div {
                    p { class: "font-display text-xl font-extrabold", "Test chain: not publicly verifiable." }
                    p { class: "text-sm opacity-90", "These draws used a development test chain instead of public drand randomness. They replay consistently, but anyone could have produced their randomness." }
                }
            }
        } else {
            div { role: "alert", class: "alert alert-error shadow-sm",
                span { class: "text-3xl", "❌" }
                p { class: "font-display text-xl font-extrabold", "Verification failed." }
            }
        }
        if !report.abandoned.is_empty() {
            div { role: "alert", class: "alert alert-warning alert-soft items-start",
                TriangleAlert { class: "size-5" }
                div { class: "space-y-1",
                    p { class: "font-semibold", "Some committed rounds did not decide their draw:" }
                    ul { class: "list-inside list-disc text-sm",
                        for r in report.abandoned.iter() {
                            li { key: "{r.k}",
                                "Draw {r.k} ({item_names.get(&r.item).cloned().unwrap_or_else(|| r.item.to_string())}): round {r.round} was abandoned by the organizer (pool of {r.pool.len()})."
                            }
                        }
                    }
                    p { class: "text-sm", "Run the independent verifier with --fetch to see whom each of those rounds would have picked." }
                }
            }
        }
        div { class: "stats stats-vertical w-full bg-base-100 shadow-sm ring-1 ring-base-300 sm:stats-horizontal",
            div { class: "stat",
                div { class: "stat-title", "Entrants" }
                div { class: "stat-value text-primary", "{entrants}" }
            }
            div { class: "stat",
                div { class: "stat-title", "Prizes" }
                div { class: "stat-value text-secondary", "{prizes}" }
            }
            div { class: "stat",
                div { class: "stat-title", "Manifest hash" }
                div { class: "stat-value font-mono text-base", "{&report.manifest_hash.to_string()[..16]}…" }
                div { class: "stat-desc", if report.manifest_hash_ok { "✓ matches the published hash" } else { "✗ does not match" } }
            }
        }
        section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
            div { class: "card-body",
                h2 { class: "card-title", Dice5 { class: "size-5 text-primary" } "Draws" }
                div { class: "overflow-x-auto",
                    table { class: "table",
                        thead { tr { th { "#" } th { "Prize" } th { "drand round" } th { "Pool" } th { "Winner" } th {} } }
                        tbody {
                            for c in report.draws.iter() {
                                tr { key: "{c.k}",
                                    td { class: "font-mono", "{c.k}" }
                                    td { class: "font-semibold", "{item_names.get(&c.item).cloned().unwrap_or_else(|| c.item.to_string())}" }
                                    td {
                                        if report.beacons_verified {
                                            a { class: "link link-primary font-mono", href: "https://api.drand.sh/{chain}/public/{c.round}", target: "_blank", "{c.round}" }
                                        } else {
                                            span { class: "font-mono", "{c.round} " }
                                            span { class: "badge badge-info badge-soft", "test chain" }
                                        }
                                    }
                                    td { "{c.pool_size}" }
                                    td {
                                        div { class: "max-w-64 wrap-break-word",
                                            bdi { class: "font-semibold", "{names.get(c.published_entrant.as_str()).map(String::as_str).unwrap_or(\"\")}" }
                                            " "
                                            span { class: "font-mono text-xs text-base-content/50", "{short(c.published_entrant.as_str())}" }
                                        }
                                    }
                                    td {
                                        if c.ok {
                                            span { class: "badge badge-success badge-soft", "✓" }
                                        } else {
                                            span { class: "badge badge-error", "expected {c.expected_entrant}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !report.errors.is_empty() {
                    ul { class: "list-inside list-disc text-error",
                        for e in report.errors.iter() { li { "{e}" } }
                    }
                }
            }
        }
        section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
            div { class: "card-body gap-3",
                h2 { class: "card-title", Users { class: "size-5 text-primary" } "Entrants" }
                p { class: "text-sm text-base-content/70",
                    "Everyone in the frozen list, which the manifest hash commits to. Find your name to check you were in."
                }
                if let Some(sources) = d.sources {
                    Sources { sources }
                }
                Entrants { checked, query, me: me.clone() }
                p { class: "text-xs text-base-content/60",
                    "Hidden names show as ★ ★ ★. Once the organizer erases personal data, only the IDs remain."
                }
            }
        }
        div { class: "grid gap-4 md:grid-cols-2",
            section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                div { class: "card-body",
                    h2 { class: "card-title", "Event log" }
                    ul { class: "timeline timeline-vertical timeline-compact",
                        for (i , e) in d.events.iter().enumerate() {
                            li { key: "{i}",
                                if i > 0 { hr {} }
                                div { class: "timeline-middle text-primary", {event_icon(e)} }
                                {
                                    let (text, name) = describe(e, names, item_names);
                                    rsx! {
                                        div { class: "timeline-end mb-2 text-sm wrap-anywhere",
                                            "{text}"
                                            if let Some(name) = name { bdi { "{name}" } }
                                        }
                                    }
                                }
                                if i + 1 < d.events.len() { hr {} }
                            }
                        }
                    }
                }
            }
            section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                div { class: "card-body",
                    h2 { class: "card-title", "Check it yourself" }
                    p { class: "text-sm text-base-content/70",
                        "Download the published data and run the independent verifier, "
                        if let Some(url) = &verifier {
                            a { class: "link", href: "{url}", target: "_blank", rel: "noopener", "verify.py" }
                        } else {
                            "verify.py"
                        }
                        " (Python 3, no dependencies), which also re-fetches every drand round from the public relays:"
                    }
                    div { class: "mockup-code text-xs",
                        pre { "data-prefix": "$", code { "curl -o verify.json {json_url}" } }
                        pre { "data-prefix": "$", code { "python3 verify.py --fetch verify.json" } }
                    }
                    div { class: "flex flex-wrap gap-2",
                        a { class: "btn btn-sm btn-outline w-fit", href: "{json_href}", Download { class: "size-4" } "verify.json" }
                        if let Some(url) = &verifier {
                            a { class: "btn btn-sm btn-outline w-fit", href: "{url}", target: "_blank", rel: "noopener", Download { class: "size-4" } "verify.py" }
                        }
                    }
                    details { class: "collapse collapse-arrow mt-2 bg-base-200",
                        summary { class: "collapse-title text-sm font-semibold", "Manifest" }
                        div { class: "collapse-content",
                            pre { class: "whitespace-pre-wrap break-all font-mono text-xs", "{manifest}" }
                        }
                    }
                }
            }
        }
    }
}

/// The search box over the frozen list and the entrants it finds: what
/// typing in it renders again, filtering the list [`Checked`] sorted once.
#[component]
fn Entrants(checked: Memo<Checked>, query: Signal<String>, me: Option<String>) -> Element {
    let c = checked.read();
    let listed = &c.listed;
    let q = query().trim().to_lowercase();
    let shown: Vec<&Listed> = listed
        .iter()
        .filter(|(id, name)| {
            q.is_empty()
                || id.starts_with(&q)
                || name
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase().contains(&q))
        })
        .take(200)
        .collect();
    let more = listed.len() > shown.len() && q.is_empty();
    rsx! {
        label { class: "input w-full",
            Search { class: "size-4 opacity-50" }
            input { class: "grow", r#type: "search", placeholder: "Find your name", aria_label: "Find your name", value: "{query}", oninput: move |e| query.set(e.value()) }
        }
        if shown.is_empty() {
            p { class: "text-sm text-base-content/60", "Nobody by that name." }
        } else {
            ul { class: "flex flex-wrap gap-2",
                for (id, name) in shown.iter() {
                    // A badge that grows to fit a long name (the
                    // organizer gives up to 64 characters): wrapped,
                    // in full.
                    li { key: "{id}",
                        class: if me.as_deref() == Some(id.as_str()) { "badge badge-lg badge-success h-auto min-h-(--size) max-w-full flex-wrap gap-x-2 py-1" } else { "badge badge-lg badge-ghost h-auto min-h-(--size) max-w-full flex-wrap gap-x-2 py-1" },
                        if let Some(name) = name { bdi { class: "font-semibold wrap-anywhere", "{name}" } }
                        span { class: "font-mono text-xs opacity-60", "{short(id)}" }
                    }
                }
            }
        }
        if more {
            p { class: "text-xs text-base-content/60", "Showing {shown.len()} of {listed.len()}: search to find the rest." }
        }
    }
}

fn event_icon(e: &Event) -> Element {
    let class = "size-4";
    match e {
        Event::Announced { .. } => rsx! { Lock { class } },
        Event::Drawn { .. } => rsx! { Dice5 { class } },
        Event::Stalled { .. } | Event::Abandoned { .. } => rsx! { TriangleAlert { class } },
        Event::Resolved { .. } => rsx! { Check { class } },
        Event::Voided { .. } => rsx! { X { class } },
        Event::Finished => rsx! { Flag { class } },
    }
}

/// The first characters of an entrant ID, enough to tell them apart.
fn short(id: &str) -> &str {
    &id[..8.min(id.len())]
}

/// What an event says, and the name of the entrant it is about, if any,
/// which follows the text: apart, so it is rendered isolated from it (a
/// right-to-left name would otherwise reorder the text around it).
fn describe(
    e: &Event,
    names: &HashMap<String, String>,
    items: &HashMap<ItemId, String>,
) -> (String, Option<String>) {
    let name = |id: &EntrantId| {
        names
            .get(id.as_str())
            .map(|n| n.to_string())
            .unwrap_or_else(|| short(id.as_str()).to_owned())
    };
    let item_name = |id: &ItemId| items.get(id).cloned().unwrap_or_else(|| id.to_string());
    let text = match e {
        Event::Announced { k, item, round, .. } => format!(
            "Draw {k}: {} committed to drand round {round}",
            item_name(item)
        ),
        Event::Drawn {
            k,
            entrant,
            pool_size,
            index,
            pass,
            ..
        } => {
            return (
                format!("Draw {k}: #{index} of {pool_size} (pass {pass}) → "),
                Some(name(entrant)),
            );
        }
        Event::Stalled {
            k, round, reason, ..
        } => format!("Draw {k}: round {round} not available yet ({reason}); still pending"),
        Event::Abandoned {
            k, round, reason, ..
        } => format!("Draw {k}: committed round {round} abandoned ({reason})"),
        Event::Resolved { k, resolution } => {
            format!("Draw {k}: {}", format!("{resolution:?}").to_lowercase())
        }
        Event::Voided { item } => format!("{} withdrawn", item_name(item)),
        Event::Finished => "Finished 🎉".into(),
    };
    (text, None)
}

/// Why this page can't check `d`, if it can't: a `format` it doesn't know
/// ([`rafi_api::VERIFY_FORMAT`]), as `tools/verify.py` refuses one.
fn unknown_format(d: &rafi_api::VerifyData) -> Option<String> {
    (d.format != rafi_api::VERIFY_FORMAT).then(|| {
        format!(
            "The record is in format {}; this page reads format {}.",
            d.format,
            rafi_api::VERIFY_FORMAT
        )
    })
}

/// A record this page can't check: a reply it can't read, or one in a
/// format it doesn't know (`verify.json`'s `format`), which a newer
/// version of Rafi wrote. Never a verdict: the page says why and stops.
#[component]
fn Unreadable(detail: String) -> Element {
    rsx! {
        div { class: "rounded-box border-2 border-dashed border-base-300 py-16 text-center",
            p { class: "text-5xl", "🧩" }
            p { class: "mt-3 text-base-content/70", "This page can't read the raffle's record. Reload the page; if that doesn't help, the record was written by a newer version of Rafi than this page." }
            p { class: "mt-1 text-sm text-base-content/50", "{detail}" }
        }
    }
}

/// How the frozen list's entrants got in: who joined on their phones and
/// whom the organizer added. The draw is fair over the list it was given;
/// an added entrant is the organizer's word, and this makes the list's
/// make-up visible ("320 entrants: 8 joined on their phones, 312 added by
/// the organizer").
#[component]
fn Sources(sources: rafi_api::SourceCounts) -> Element {
    rsx! {
        p { class: "text-sm font-semibold", "{sources.text()}" }
        if sources.added > 0 {
            p { class: "text-xs text-base-content/60",
                "People the organizer added are on the organizer's word: winners must be at the event to claim, and the organizer checks it is them."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record in a format this page doesn't know is not checked: the
    /// page says a newer Rafi wrote it.
    #[test]
    fn a_newer_format_is_not_checked() {
        let record = |format: u32| -> rafi_api::VerifyData {
            serde_json::from_value(serde_json::json!({
                "format": format, "title": "Meetup", "state": "finished",
                "manifest": "{}", "manifest_hash": "00", "events": [], "names": [],
            }))
            .unwrap()
        };
        assert_eq!(
            unknown_format(&record(1)),
            None,
            "the format this page reads"
        );
        let detail = unknown_format(&record(2)).expect("a newer format");
        let html = dioxus_ssr::render_element(rsx! { Unreadable { detail } });
        assert!(
            html.contains("written by a newer version of Rafi"),
            "{html}"
        );
        assert!(html.contains("format 2"), "{html}");
    }

    thread_local! {
        static QUERY: std::cell::Cell<Option<Signal<String>>> = const { std::cell::Cell::new(None) };
    }

    /// The page's record with a search box the test types into.
    fn searched(data: rafi_api::VerifyData) -> Element {
        let query = use_signal(String::new);
        use_hook(|| QUERY.set(Some(query)));
        rsx! {
            Record {
                data,
                query,
                me: None,
                json_href: "/api/v/meetup/verify.json",
                json_url: "https://rafi.example/api/v/meetup/verify.json",
                verifier: None,
            }
        }
    }

    /// Typing in the search filters the list the record's check sorted,
    /// without checking the record again: the BLS checks run once per
    /// record, not per keystroke.
    #[test]
    fn the_search_does_not_check_the_record_again() {
        let manifest = rafi_raffle::Manifest::new(
            "r1",
            rafi_raffle::BeaconSource::QUICKNET,
            vec![
                "ab12cd34ef".into(),
                "0011223344".into(),
                "ffeeddccbb".into(),
            ],
            vec![rafi_raffle::ManifestItem {
                id: "item-0".into(),
                name: "Mug".into(),
            }],
        );
        let data: rafi_api::VerifyData = serde_json::from_value(serde_json::json!({
            "format": rafi_api::VERIFY_FORMAT, "title": "Meetup", "state": "finished",
            "manifest": manifest.canonical_json(),
            "manifest_hash": manifest.hash().to_string(),
            "events": [],
            // The third has been erased: only the ID is left.
            "names": [["ab12cd34ef", "Anna"], ["0011223344", "Bob"]],
        }))
        .unwrap();
        CHECKS.set(0);
        let mut dom = VirtualDom::new_with_props(searched, data);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("Meetup") && html.contains("Mug"), "{html}");
        assert!(
            html.contains("Anna") && html.contains("Bob") && html.contains("ffeeddcc"),
            "{html}"
        );
        let mut query = QUERY.get().expect("the search's signal");
        for (q, shown, hidden) in [
            ("ann", &["Anna"][..], &["Bob", "ffeeddcc"][..]),
            ("FFEE", &["ffeeddcc"], &["Anna", "Bob"]),
            (
                "zed",
                &["Nobody by that name."],
                &["Anna", "Bob", "ffeeddcc"],
            ),
        ] {
            dom.in_runtime(|| query.set(q.to_owned()));
            dom.render_immediate_to_vec();
            let html = dioxus_ssr::render(&dom);
            // The entrants' list, between the search and the note under it.
            let list = &html[html.find("Find your name").expect("the search")
                ..html.find("Hidden names show").expect("the note")];
            for name in shown {
                assert!(list.contains(name), "{q}: {name} in {list}");
            }
            for name in hidden {
                assert!(!list.contains(name), "{q}: {name} not in {list}");
            }
        }
        assert_eq!(CHECKS.get(), 1, "one check for one record");
    }

    #[test]
    fn the_sources_in_words() {
        let sources = rafi_api::SourceCounts {
            joined: 8,
            added: 312,
            fake: 0,
        };
        let html = dioxus_ssr::render_element(rsx! { Sources { sources } });
        assert!(
            html.contains("320 entrants: 8 joined on their phones, 312 added by the organizer"),
            "{html}"
        );
        assert!(html.contains("organizer&#39;s word"), "{html}");
        let sources = rafi_api::SourceCounts {
            joined: 3,
            added: 0,
            fake: 0,
        };
        let html = dioxus_ssr::render_element(rsx! { Sources { sources } });
        assert!(
            html.contains("3 entrants: 3 joined on their phones, 0 added by the organizer")
                && !html.contains("word"),
            "{html}"
        );
    }
}
