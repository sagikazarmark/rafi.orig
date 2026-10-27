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
    let json_href = format!("/api/r/{slug}/verify.json");
    let me = use_hook(me_from_url);
    let mut query = use_signal(String::new);
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
        Some(Err(e)) if e.code == api::codes::CLIENT => rsx! {
            div { class: "rounded-box border-2 border-dashed border-base-300 py-16 text-center",
                p { class: "text-5xl", "🧩" }
                p { class: "mt-3 text-base-content/70", "This page can't read the raffle's record. Reload the page; if that doesn't help, the record was written by a newer version of Rafi than this page." }
                p { class: "mt-1 text-sm text-base-content/50", "{e.message}" }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "rounded-box border-2 border-dashed border-base-300 py-16 text-center",
                p { class: "text-5xl", "🔍" }
                p { class: "mt-3 text-base-content/70", "This raffle hasn't been frozen yet, or it doesn't exist." }
            }
        },
        Some(Ok(d)) => {
            // Public drand chains only: a test chain is never reported as verified.
            let report = rafi_raffle::verify_public(&d.manifest, &d.manifest_hash, &d.events);
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
            let names: HashMap<&str, &str> = d
                .names
                .iter()
                .map(|(id, n)| (id.as_str(), n.as_str()))
                .collect();
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
            let ok = report.ok();
            let manifest = d.manifest.clone();
            let entrants = doc.as_ref().map(|d| d.entrants.len()).unwrap_or_default();
            // Everyone in the frozen list, by name (erased entrants have
            // none left), then by ID.
            let mut listed: Vec<(String, Option<String>)> = doc
                .as_ref()
                .map(|d| {
                    d.entrants
                        .iter()
                        .map(|id| {
                            let name = names.get(id.as_str()).map(|n| (*n).to_owned());
                            (id.to_string(), name)
                        })
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
            let mine = me
                .as_ref()
                .map(|me| (me.clone(), listed.iter().find(|(id, _)| id == me).cloned()));
            let q = query().trim().to_lowercase();
            let shown: Vec<&(String, Option<String>)> = listed
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
                                p { class: "text-sm",
                                    "Your entry, "
                                    if let Some(name) = name { strong { "{name} " } }
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
                                                span { class: "font-semibold", "{names.get(c.published_entrant.as_str()).copied().unwrap_or(\"\")} " }
                                                span { class: "font-mono text-xs text-base-content/50", "{short(c.published_entrant.as_str())}" }
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
                        label { class: "input w-full",
                            Search { class: "size-4 opacity-50" }
                            input { class: "grow", r#type: "search", placeholder: "Find your name", aria_label: "Find your name", value: "{query}", oninput: move |e| query.set(e.value()) }
                        }
                        if shown.is_empty() {
                            p { class: "text-sm text-base-content/60", "Nobody by that name." }
                        } else {
                            ul { class: "flex flex-wrap gap-2",
                                for (id, name) in shown.iter() {
                                    li { key: "{id}",
                                        class: if me.as_deref() == Some(id.as_str()) { "badge badge-lg badge-success gap-2" } else { "badge badge-lg badge-ghost gap-2" },
                                        if let Some(name) = name { span { class: "font-semibold", "{name}" } }
                                        span { class: "font-mono text-xs opacity-60", "{short(id)}" }
                                    }
                                }
                            }
                        }
                        if more {
                            p { class: "text-xs text-base-content/60", "Showing {shown.len()} of {listed.len()}: search to find the rest." }
                        }
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
                                        div { class: "timeline-end mb-2 text-sm", "{describe(e, &names, &item_names)}" }
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
                                "Download the published data and run the independent verifier, which also re-fetches every drand round from the public relays:"
                            }
                            div { class: "mockup-code text-xs",
                                pre { "data-prefix": "$", code { "curl -O {json_href}" } }
                                pre { "data-prefix": "$", code { "python3 verify.py --fetch verify.json" } }
                            }
                            a { class: "btn btn-sm btn-outline w-fit", href: "{json_href}", Download { class: "size-4" } "verify.json" }
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
    };

    rsx! {
        div { class: "min-h-screen bg-base-200",
            header { class: "border-b border-base-300 bg-base-100/85 backdrop-blur",
                div { class: "mx-auto flex max-w-5xl items-center gap-2 px-4 py-2.5",
                    Brand {}
                    span { class: "flex-1" }
                    ThemeToggle {}
                }
            }
            main { class: "mx-auto max-w-5xl space-y-6 px-4 py-8", {body} }
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

fn describe(e: &Event, names: &HashMap<&str, &str>, items: &HashMap<ItemId, String>) -> String {
    let name = |id: &EntrantId| {
        names
            .get(id.as_str())
            .map(|n| n.to_string())
            .unwrap_or_else(|| short(id.as_str()).to_owned())
    };
    let item_name = |id: &ItemId| items.get(id).cloned().unwrap_or_else(|| id.to_string());
    match e {
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
            format!(
                "Draw {k}: #{index} of {pool_size} (pass {pass}) → {}",
                name(entrant)
            )
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
    }
}
