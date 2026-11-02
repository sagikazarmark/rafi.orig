//! What the organizer reads (the admin page, the winners' export) and what
//! anyone reads to verify a raffle.

use std::collections::HashMap;

use rafi_api::{
    AdminActions, AdminEntrant, EntrantActions, LifecycleCommand, Prize, RaffleAdmin,
    RemovedAtRequest, Retention, SourceCounts, StartStep, UndoBatch, VerifyData,
};
use rafi_lifecycle::{
    self as lifecycle, RaffleState,
    settings::{self, ErasureDates},
    start::{self, Start},
};
use rafi_raffle::{ItemId, Outcome, Raffle};
use rafi_store::{EntrantRow, RaffleRow, audit, entrants, raffles, view::Full};

use crate::{
    Actor, App, Error, Result,
    raffles::{missing_for_open, settings_of},
    view,
};

/// Everything the admin page shows of raffle `id`, from one read.
pub async fn admin(app: &App, owner: &str, id: &str) -> Result<RaffleAdmin> {
    let full = Full::read(&app.db, id).await?.ok_or(Error::NotFound)?;
    if full.raffle.owner_id != owner {
        return Err(Error::NotFound);
    }
    let now = app.clock.now();
    let view = view::of(app, &full, now)?;
    let raffle = &full.raffle;
    let retention = retention_of(app, raffle, now);
    let entrants = admin_entrants(&full, full.drawn.as_ref(), retention.winners_erase_at);
    // Read apart from the snapshot: only the list tab's preview and the
    // one-person form match against them, and an add checks them again.
    let removed = entrants::removals(&app.db, id)
        .await?
        .into_iter()
        .map(|r| RemovedAtRequest {
            email_key: r.email_norm,
            list_ref: r.list_ref,
            removed_at: r.removed_at,
        })
        .collect();
    let held_winners = entrants.iter().filter(|e| e.held_until.is_some()).count();
    let personal_data_left = full.entrants.iter().any(|e| !e.erased);
    let add_list = lifecycle::entrant::may_add(
        lifecycle::entrant::Origin::List,
        raffle.state,
        raffle.test_mode,
        raffle.past_erase_date(now),
    );
    let actions = AdminActions {
        start: start::next(
            raffle.state,
            raffle.registration,
            raffle.manifest.is_some(),
            raffle.opened(),
        )
        .ok()
        .map(|step| match step {
            Start::Freeze { .. } => StartStep::Freeze,
            Start::WriteManifest { .. } | Start::Resend => StartStep::Retry,
            Start::Started => StartStep::Started,
        }),
        // The header's buttons, with the registration setting: no opening
        // without it.
        lifecycle: LifecycleCommand::ALL
            .into_iter()
            .filter(|&c| raffle.state.apply(c.into(), raffle.registration).is_some())
            .collect(),
        // Opening, or the freeze from Published without registration, checks
        // them: said while that step is ahead.
        missing: if raffle.state.opening_ahead() {
            missing_for_open(raffle)
                .into_iter()
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        },
        edit_registration: raffle.state.registration_editable(),
        draw: raffle.state.frozen_or_later(),
        export: raffle.state.frozen_or_later(),
        purge: raffle
            .state
            .apply(lifecycle::Command::Purge, raffle.registration)
            .is_some()
            && (raffle.state == RaffleState::Finished || personal_data_left),
        edit_rules: settings::rules_editable(full.entrants.iter().map(|e| e.source)),
        removal: raffle.state.removal(raffle.past_erase_date(now)),
        deliver_all: raffle.state.is_over() && held_winners > 0,
        delete: lifecycle::raffle::deletable(full.drawn.as_ref()),
        undo: raffle
            .state
            .entrants_editable()
            .then(|| last_add(&full.entrants))
            .flatten(),
        add_list: add_list.is_ok(),
        add_list_refusal: add_list.err(),
    };
    Ok(RaffleAdmin {
        id: raffle.id.clone(),
        settings: settings_of(raffle),
        prizes: full
            .prizes
            .iter()
            .map(|i| Prize {
                id: i.id.clone(),
                name: i.name.clone(),
                sponsor: i.sponsor.clone(),
                description: i.description.clone(),
                qty: i.qty.max(0) as u32,
                position: i.position,
                delivery: i.delivery,
            })
            .collect(),
        entrants,
        view,
        public_url: app.policy.public_url.clone(),
        actions,
        retention: Retention {
            held_winners,
            ..retention
        },
        removed,
    })
}

/// The add "Undo last add" takes back: of the adds with entrants on the
/// list, the one made last ([`added_at`](crate::entrants::added_at); then
/// the greatest batch ID, so it is the same on every read), with how many
/// it has.
fn last_add(entrants: &[EntrantRow]) -> Option<UndoBatch> {
    let mut adds: HashMap<&str, (i64, u32)> = HashMap::new();
    for e in entrants {
        if let (Some(batch), Some(at)) = (e.batch_id.as_deref(), crate::entrants::added_at(e)) {
            let add = adds.entry(batch).or_insert((at, 0));
            add.0 = add.0.max(at);
            add.1 += 1;
        }
    }
    adds.into_iter()
        .max_by_key(|&(batch, (entered, _))| (entered, batch))
        .map(|(batch, (_, count))| UndoBatch {
            batch_id: batch.to_owned(),
            count,
        })
}

/// When `raffle`'s personal data is erased, as of `now`.
fn retention_of(app: &App, raffle: &RaffleRow, now: i64) -> Retention {
    let hold = app.policy.retention.winner_hold_days;
    let (erase_date, winners_erase_date) =
        match ErasureDates::of(&raffle.event_date, raffle.retention_days, hold) {
            Some(dates) => (dates.erase.long(), dates.winners_erase.long()),
            None => (String::new(), String::new()),
        };
    let past_event = raffle.event_ends_at <= now;
    Retention {
        erase_at: raffle.erase_at,
        erase_date,
        winners_erase_at: app.policy.retention.winners_erase_at(raffle.erase_at),
        winners_erase_date,
        held_winners: 0,
        past_erase_date: raffle.past_erase_date(now),
        unfinished_past_event: past_event && raffle.state.drawable(),
        expires_unfrozen: past_event && raffle.state.expires_with(raffle.manifest.is_some()),
        erasure_imminent: !raffle.state.is_over()
            && !raffle.past_erase_date(now)
            && raffle.erase_at - now < 24 * 60 * 60,
        default_days: app.policy.retention.days,
        max_days: app.policy.retention.max_days,
        winner_hold_days: hold,
    }
}

/// The entrants as the organizer sees them, with what the raffle made of
/// each and what the organizer can do with each. A claimed winner whose
/// prize isn't delivered is held until `winners_erase_at`.
fn admin_entrants(full: &Full, drawn: Option<&Raffle>, winners_erase_at: i64) -> Vec<AdminEntrant> {
    let outcomes = drawn.map(Raffle::entrant_outcomes).unwrap_or_default();
    let item_names: HashMap<ItemId, String> = drawn
        .map(|s| {
            s.items()
                .iter()
                .map(|item| (item.id.clone(), item.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    full.entrants
        .iter()
        .map(|e| {
            let outcome = outcomes.get(e.id.as_str());
            let deliverable = lifecycle::raffle::deliverable(drawn, &e.id);
            let state = full.raffle.state;
            let actions = EntrantActions {
                object: !e.erased
                    && e.objected_at.is_none()
                    && lifecycle::entrant::may_object(state, e.source, deliverable).is_ok(),
                unobject: !e.erased
                    && e.objected_at.is_some()
                    && lifecycle::entrant::may_unobject(state).is_ok(),
            };
            AdminEntrant {
                outcome: outcome.map(|o| o.0),
                prize: outcome.and_then(|o| item_names.get(&o.1).cloned()),
                id: e.id.clone(),
                display_name: e.display_name.clone(),
                email: e.email_raw.clone(),
                source: e.source,
                hidden: e.hidden,
                claim_code: e.claim_code.clone(),
                created_at: e.created_at,
                delivered: e.delivered_at.is_some(),
                deliverable,
                held_until: (deliverable && e.delivered_at.is_none() && !e.erased)
                    .then_some(winners_erase_at),
                erased: e.erased,
                batch_id: e.batch_id.clone(),
                list_ref: e.list_ref.clone(),
                objected: e.objected_at.is_some(),
                actions,
                no_phone: e.source.no_phone(),
            }
        })
        .collect()
}

/// The drawn entrants as CSV: who won what, and how to reach them. Every
/// export is audited (who, when, how many rows), never stored.
///
/// The format (also in `docs/reference/routes.md`): UTF-8 with a byte order
/// mark (Excel on Windows reads accented names wrong without one), one header
/// line, every field double-quoted with `"` doubled, and a field that starts
/// with `=`, `+`, `-`, `@`, a tab or a carriage return prefixed with `'` so a
/// spreadsheet doesn't run it as a formula (`csv_field`). Columns:
/// `entrant_id,display_name,email,source,claim_code,outcome,prize,delivered`.
pub async fn export_csv(app: &App, actor: Actor<'_>, id: &str) -> Result<String> {
    let full = Full::read(&app.db, id).await?.ok_or(Error::NotFound)?;
    if full.raffle.owner_id != actor.owner {
        return Err(Error::NotFound);
    }
    let hold = app.policy.retention.winners_erase_at(full.raffle.erase_at);
    let esc = csv_field;
    let mut out = String::from(
        "\u{FEFF}entrant_id,display_name,email,source,claim_code,outcome,prize,delivered\n",
    );
    let mut rows = 0;
    for e in admin_entrants(&full, full.drawn.as_ref(), hold)
        .iter()
        .filter(|e| e.outcome.is_some())
    {
        rows += 1;
        let outcome = e.outcome.map_or("", Outcome::as_str);
        out.push_str(
            &[
                esc(&e.id),
                esc(&e.display_name),
                esc(e.email.as_deref().unwrap_or("")),
                esc(e.source.as_str()),
                esc(e.claim_code.as_deref().unwrap_or("")),
                esc(outcome),
                esc(e.prize.as_deref().unwrap_or("")),
                esc(if e.delivered { "yes" } else { "no" }),
            ]
            .join(","),
        );
        out.push('\n');
    }
    audit::record(
        &app.db,
        id,
        actor,
        audit::Action::ExportCsv,
        &rows.to_string(),
        app.clock.now(),
    )
    .await?;
    Ok(out)
}

/// One field of the winners' CSV ([`export_csv`]): quoted, with `"` doubled,
/// and with a leading `'` if it starts with a character a spreadsheet would
/// take for a formula: `=`, `+`, `-`, `@`, and the tab and carriage return
/// some read past to one (OWASP's list). Names and prizes are typed by
/// entrants and organizers, and the export is opened in a spreadsheet.
fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Everything needed to verify the draws of raffle `slug`, once its
/// manifest is published. One snapshot ([`Full::read`]): the names, the
/// events and the version agree, so the `ETag` built from `v` is honest.
pub async fn verify_data(app: &App, slug: &str) -> Result<VerifyData> {
    let id = raffles::by_slug(&app.db, slug)
        .await?
        .ok_or(Error::NotFound)?
        .id;
    let full = Full::read(&app.db, &id).await?.ok_or(Error::NotFound)?;
    let raffle = full.raffle;
    let (Some(manifest), Some(manifest_hash)) = (raffle.manifest, raffle.manifest_hash) else {
        return Err(Error::NotFound);
    };
    let sources = source_counts(&manifest, &full.entrants);
    // Erased entrants have no name left: the page shows the start of their ID.
    let names = full
        .entrants
        .into_iter()
        .filter(|e| !e.erased)
        .map(|e| {
            let name = rafi_live_client::public_name(
                &e.id,
                Some((!e.hidden).then_some(e.display_name.as_str())),
            );
            (e.id, name)
        })
        .collect();
    Ok(VerifyData {
        format: rafi_api::VERIFY_FORMAT,
        v: raffle.version.max(0) as u64,
        title: raffle.title,
        state: raffle.state,
        test_mode: raffle.test_mode,
        events: full.drawn.map(|s| s.events().to_vec()).unwrap_or_default(),
        manifest,
        manifest_hash,
        names,
        sources,
    })
}

/// How the entrants in `manifest` got in, counted from their rows in
/// `entrants`. After the freeze the rows are the manifest's entrants (none
/// is added or deleted; erasing one keeps its row and its source), so the
/// counts are what the list held when it was frozen; they are matched by
/// ID all the same. `None` if the manifest doesn't read, or names someone
/// without a row: counts that don't add up to the list aren't published.
fn source_counts(manifest: &str, entrants: &[EntrantRow]) -> Option<SourceCounts> {
    let manifest: rafi_raffle::Manifest = serde_json::from_str(manifest).ok()?;
    let sources: HashMap<String, lifecycle::entrant::Source> = entrants
        .iter()
        .map(|e| (e.id.to_ascii_lowercase(), e.source))
        .collect();
    let counted = manifest
        .entrants
        .iter()
        .map(|id| sources.get(id.as_str()).copied())
        .collect::<Option<Vec<_>>>()?;
    Some(SourceCounts::of(counted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_are_quoted_and_never_formulas() {
        assert_eq!(csv_field("Anna"), "\"Anna\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field(""), "\"\"");
        for (input, cell) in [
            ("=SUM(A1)", "'=SUM(A1)"),
            ("+1", "'+1"),
            ("-1", "'-1"),
            ("@cmd", "'@cmd"),
            ("\t=1+1", "'\t=1+1"),
            ("\r=1+1", "'\r=1+1"),
        ] {
            assert_eq!(csv_field(input), format!("\"{cell}\""), "{input:?}");
        }
        // Only the first character counts.
        assert_eq!(csv_field("a=b"), "\"a=b\"");
        assert_eq!(csv_field(" =1"), "\" =1\"");
    }
}
