//! Public verification page: re-runs every draw in the browser.

use std::collections::HashMap;

use dioxus::prelude::*;
use rafi_core::{
    state::{DrawMode, Event},
    verify::Replaced,
};

use super::{Brand, ThemeToggle};
use crate::{
    api,
    icons::{I, Icon},
};

#[component]
pub fn Verify(slug: String) -> Element {
    let json_href = format!("/api/r/{slug}/verify.json");
    let data = use_resource(move || {
        let slug = slug.clone();
        async move { api::get_verify(slug).await }
    });

    let body = match &*data.read() {
        None => {
            rsx! { div { class: "grid place-items-center py-24", span { class: "loading loading-dots loading-lg text-primary" } } }
        }
        Some(Err(_)) => rsx! {
            div { class: "rounded-box border-2 border-dashed border-base-300 py-16 text-center",
                p { class: "text-5xl", "🔍" }
                p { class: "mt-3 text-base-content/70", "This raffle hasn't been frozen yet, or it doesn't exist." }
            }
        },
        Some(Ok(d)) => {
            // Public drand chains only: a test chain is never reported as verified.
            let report = rafi_core::verify::verify_public(&d.commit_doc, &d.list_hash, &d.events);
            let test_chain = report.chain_unknown()
                && report.list_hash_ok
                && report.errors.is_empty()
                && report.draws.iter().all(|c| c.ok);
            let chain = report.chain.clone();
            let names: HashMap<&str, &str> = d
                .names
                .iter()
                .map(|(id, n)| (id.as_str(), n.as_str()))
                .collect();
            let doc: Option<rafi_core::commit::CommitDoc> =
                serde_json::from_str(&d.commit_doc).ok();
            let unit_names: HashMap<String, String> = doc
                .as_ref()
                .map(|d| {
                    d.units
                        .iter()
                        .map(|u| (u.unit_id.clone(), u.item_name.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let ok = report.ok();
            let commit_doc = d.commit_doc.clone();
            let offline = report.draws.iter().any(|c| c.mode == DrawMode::Offline);
            let entrants = doc
                .as_ref()
                .map(|d| d.entrant_ids.len())
                .unwrap_or_default();
            let prizes = doc.as_ref().map(|d| d.units.len()).unwrap_or_default();
            rsx! {
                div { class: "space-y-2",
                    p { class: "badge badge-soft badge-primary gap-1 font-semibold", Icon { i: I::ShieldCheck, class: "size-3.5" } "Independent verification" }
                    h1 { class: "text-4xl font-extrabold", "{d.title}" }
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
                if !report.replaced.is_empty() {
                    div { role: "alert", class: "alert alert-warning alert-soft items-start",
                        Icon { i: I::Alert }
                        div { class: "space-y-1",
                            p { class: "font-semibold", "Some committed rounds did not decide their draw:" }
                            ul { class: "list-inside list-disc text-sm",
                                for r in report.replaced.iter() {
                                    li { key: "{r.k}",
                                        "Draw {r.k} ({unit_names.get(&r.unit_id).cloned().unwrap_or_else(|| r.unit_id.clone())}): round {r.round} was "
                                        match r.replaced {
                                            Replaced::Abandoned => "abandoned by the organizer",
                                            Replaced::Offline => "replaced by an offline draw",
                                        }
                                        " (pool of {r.pool.len()})."
                                    }
                                }
                            }
                            p { class: "text-sm", "Run the independent verifier with --fetch to see whom each of those rounds would have picked." }
                        }
                    }
                }
                if offline {
                    div { role: "alert", class: "alert alert-warning alert-soft",
                        Icon { i: I::Alert }
                        "Some draws used offline server randomness (drand was unreachable), so they can't be checked independently."
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
                        div { class: "stat-title", "Entrant list hash" }
                        div { class: "stat-value font-mono text-base", "{&report.list_hash[..16]}…" }
                        div { class: "stat-desc", if report.list_hash_ok { "✓ matches the published hash" } else { "✗ does not match" } }
                    }
                }
                section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                    div { class: "card-body",
                        h2 { class: "card-title", Icon { i: I::Dice, class: "size-5 text-primary" } "Draws" }
                        div { class: "overflow-x-auto",
                            table { class: "table",
                                thead { tr { th { "#" } th { "Prize" } th { "drand round" } th { "Pool" } th { "Winner" } th {} } }
                                tbody {
                                    for c in report.draws.iter() {
                                        tr { key: "{c.k}",
                                            td { class: "font-mono", "{c.k}" }
                                            td { class: "font-semibold", "{unit_names.get(&c.unit_id).cloned().unwrap_or_else(|| c.unit_id.clone())}" }
                                            td {
                                                if c.mode == DrawMode::Offline {
                                                    span { class: "badge badge-warning badge-soft", "offline" }
                                                } else if report.chain_known {
                                                    a { class: "link link-primary font-mono", href: "https://api.drand.sh/{chain}/public/{c.round}", target: "_blank", "{c.round}" }
                                                } else {
                                                    span { class: "font-mono", "{c.round} " }
                                                    span { class: "badge badge-info badge-soft", "test chain" }
                                                }
                                            }
                                            td { "{c.pool_size}" }
                                            td {
                                                span { class: "font-semibold", "{names.get(c.published_entrant.as_str()).copied().unwrap_or(\"\")} " }
                                                span { class: "font-mono text-xs text-base-content/50", "{&c.published_entrant[..8.min(c.published_entrant.len())]}" }
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
                div { class: "grid gap-4 md:grid-cols-2",
                    section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                        div { class: "card-body",
                            h2 { class: "card-title", "Event log" }
                            ul { class: "timeline timeline-vertical timeline-compact",
                                for (i , e) in d.events.iter().enumerate() {
                                    li { key: "{i}",
                                        if i > 0 { hr {} }
                                        div { class: "timeline-middle text-primary", Icon { i: event_icon(e), class: "size-4" } }
                                        div { class: "timeline-end mb-2 text-sm", "{describe(e, &names, &unit_names)}" }
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
                            a { class: "btn btn-sm btn-outline w-fit", href: "{json_href}", Icon { i: I::Download, class: "size-4" } "verify.json" }
                            details { class: "collapse collapse-arrow mt-2 bg-base-200",
                                summary { class: "collapse-title text-sm font-semibold", "Commit document" }
                                div { class: "collapse-content",
                                    pre { class: "whitespace-pre-wrap break-all font-mono text-xs", "{commit_doc}" }
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

fn event_icon(e: &Event) -> I {
    match e {
        Event::Announce { .. } => I::Lock,
        Event::Draw { .. } => I::Dice,
        Event::Stall { .. } | Event::Abandon { .. } => I::Alert,
        Event::Resolve { .. } => I::Check,
        Event::Void { .. } => I::X,
        Event::Finish => I::Flag,
    }
}

fn describe(e: &Event, names: &HashMap<&str, &str>, units: &HashMap<String, String>) -> String {
    let name = |id: &str| {
        names
            .get(id)
            .map(|n| n.to_string())
            .unwrap_or_else(|| id[..8.min(id.len())].to_owned())
    };
    let unit = |id: &str| units.get(id).cloned().unwrap_or_else(|| id.to_owned());
    match e {
        Event::Announce {
            k, unit_id, round, ..
        } => format!(
            "Draw {k}: {} committed to drand round {round}",
            unit(unit_id)
        ),
        Event::Draw {
            k,
            entrant_id,
            pool_size,
            index,
            pass,
            ..
        } => {
            format!(
                "Draw {k}: #{index} of {pool_size} (pass {pass}) → {}",
                name(entrant_id)
            )
        }
        Event::Stall {
            k, round, reason, ..
        } => format!("Draw {k}: round {round} not available yet ({reason}); still pending"),
        Event::Abandon {
            k, round, reason, ..
        } => format!("Draw {k}: committed round {round} abandoned ({reason})"),
        Event::Resolve { k, outcome } => {
            format!("Draw {k}: {}", format!("{outcome:?}").to_lowercase())
        }
        Event::Void { unit_id } => format!("{} withdrawn", unit(unit_id)),
        Event::Finish => "Finished 🎉".into(),
    }
}
