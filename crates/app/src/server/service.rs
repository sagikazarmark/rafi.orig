//! Raffle operations shared by the admin API and the attendee routes.
//!
//! Every write that changes what phones see (the participant view) notifies
//! clients ([`AppState::notifier`]). Joins don't: they never change it. The
//! draw engine notifies after its own writes.

use std::collections::HashSet;

use rafi_core::{
    commit::{CommitDoc, UnitRef},
    engine::ErrorKind,
    raffle::{Action, RaffleState},
    state::{DrawError, Outcome},
};
use rafi_store::{RaffleRow, audit, entrants, items, raffles};

use super::{
    AppState,
    engine::{Command, RestateClient},
    ids,
};
use crate::model::{
    AdminEntrant, EntrantPatch, EntrantSource, Item, ItemInput, Lifecycle, RaffleAdmin,
    RaffleSettings, RaffleSummary, VerifyData,
};

/// Version of the consent text shown on the join form.
pub const CONSENT_VERSION: &str = "2026-10-01";
pub const MAX_NAME_CHARS: usize = 24;
pub const MAX_FAKE_ENTRANTS: u32 = 500;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<rafi_store::Error> for AppError {
    fn from(e: rafi_store::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<DrawError> for AppError {
    fn from(e: DrawError) -> Self {
        match e.kind() {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::BadRequest => Self::BadRequest(e.to_string()),
            ErrorKind::Conflict => Self::Conflict(e.to_string()),
            ErrorKind::Invariant => Self::Internal(e.into()),
        }
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

fn conflict(msg: impl Into<String>) -> AppError {
    AppError::Conflict(msg.into())
}

/// The draw engine, or a conflict if this server has none.
fn engine(app: &AppState) -> Result<&RestateClient> {
    app.restate
        .as_ref()
        .ok_or_else(|| conflict("Restate is not configured on this server."))
}

// ---------------------------------------------------------------------------
// Raffles
// ---------------------------------------------------------------------------

pub async fn owned(app: &AppState, owner: &str, id: &str) -> Result<RaffleRow> {
    let raffle = raffles::get(&app.db, id).await?.ok_or(AppError::NotFound)?;
    if raffle.owner_id != owner {
        return Err(AppError::NotFound);
    }
    Ok(raffle)
}

pub async fn list(app: &AppState, owner: &str) -> Result<Vec<RaffleSummary>> {
    Ok(raffles::list_by_owner(&app.db, owner)
        .await?
        .into_iter()
        .map(|r| RaffleSummary {
            state: RaffleState::parse(&r.state).unwrap_or(RaffleState::Draft),
            id: r.id,
            slug: r.slug,
            title: r.title,
            entrant_count: r.entrant_count,
            test_mode: r.test_mode,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn create(app: &AppState, owner: &str, title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(bad("The title must be 1–120 characters."));
    }
    let id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert(
            &app.db,
            &raffles::NewRaffle {
                id: &id,
                owner_id: owner,
                slug: &ids::slug(title),
                join_code: &ids::code(4),
                title,
            },
        )
        .await;
        match res {
            Ok(_) => {
                audit::record(&app.db, &id, owner, "create", title).await?;
                return Ok(id);
            }
            Err(e) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(conflict(
        "Could not allocate a unique join code; please try again.",
    ))
}

pub fn settings_of(r: &RaffleRow) -> RaffleSettings {
    RaffleSettings {
        title: r.title.clone(),
        description: r.description.clone(),
        lang: r.lang.clone(),
        organizer_name: r.organizer_name.clone(),
        contact_email: r.contact_email.clone(),
        rules: r.rules.clone(),
        event_date: r.event_date.clone(),
        min_age: r.min_age.max(0) as u32,
        test_mode: r.test_mode,
    }
}

pub async fn update_settings(
    app: &AppState,
    owner: &str,
    id: &str,
    s: RaffleSettings,
) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let title = s.title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(bad("The title must be 1–120 characters."));
    }
    if s.description.chars().count() > 2000 || s.rules.chars().count() > 5000 {
        return Err(bad("Description or rules are too long."));
    }
    let lang = if s.lang == "hu" { "hu" } else { "en" };
    if raffle.test_mode != s.test_mode && !raffle.state().entrants_editable() {
        return Err(conflict("Test mode cannot change after the draw started."));
    }
    raffles::update_settings(
        &app.db,
        id,
        &raffles::Settings {
            title,
            description: s.description.trim(),
            lang,
            organizer_name: s.organizer_name.trim(),
            contact_email: s.contact_email.trim(),
            rules: s.rules.trim(),
            event_date: s.event_date.trim(),
            min_age: s.min_age.min(99),
            test_mode: s.test_mode,
        },
    )
    .await?;
    // Nothing phones see: the participant view has no settings.
    Ok(())
}

pub async fn delete(app: &AppState, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    if raffle.state() == RaffleState::Drawing
        && raffle.draw_state().is_some_and(|s| s.pending.is_some())
    {
        return Err(conflict("A draw is in progress."));
    }
    raffles::delete(&app.db, id).await?;
    audit::record(&app.db, id, owner, "delete", "").await?;
    // Best effort: the raffle is gone either way, and its channel finds nothing to show.
    if let Err(e) = app.notifier.end(id).await {
        tracing::warn!(raffle_id = id, error = %e, "ending the live channel of a deleted raffle failed");
    }
    Ok(())
}

/// Copies settings and prizes into a new draft raffle (no entrants).
pub async fn duplicate(app: &AppState, owner: &str, id: &str) -> Result<String> {
    let raffle = owned(app, owner, id).await?;
    let new_id = create(
        app,
        owner,
        &format!(
            "{} (copy)",
            raffle.title.chars().take(110).collect::<String>()
        ),
    )
    .await?;
    let mut s = settings_of(&raffle);
    s.title = format!(
        "{} (copy)",
        raffle.title.chars().take(110).collect::<String>()
    );
    update_settings(app, owner, &new_id, s).await?;
    for item in items::list(&app.db, id).await? {
        upsert_item(
            app,
            owner,
            &new_id,
            ItemInput {
                id: None,
                name: item.name,
                sponsor: item.sponsor,
                description: item.description,
                qty: item.qty.max(0) as u32,
                position: item.position,
            },
        )
        .await?;
    }
    Ok(new_id)
}

pub async fn upsert_item(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    input: ItemInput,
) -> Result<String> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state().items_editable() {
        return Err(conflict(
            "Prizes cannot change after the entrant list is frozen.",
        ));
    }
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(bad("The prize name must be 1–120 characters."));
    }
    if input.qty == 0 || input.qty > 500 {
        return Err(bad("The quantity must be between 1 and 500."));
    }
    let fields = items::ItemFields {
        name,
        sponsor: input.sponsor.trim(),
        description: input.description.trim(),
        qty: input.qty,
        position: input.position,
    };
    let id = match &input.id {
        Some(id) => {
            if !items::update(&app.db, id, raffle_id, &fields).await? {
                return Err(AppError::NotFound);
            }
            id.clone()
        }
        None => {
            let id = ids::code(6).to_lowercase();
            items::insert(&app.db, &id, raffle_id, &fields).await?;
            id
        }
    };
    // Before the freeze, phones count the prizes from the items.
    app.notifier.notify(raffle_id).await;
    Ok(id)
}

pub async fn delete_item(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    item_id: &str,
) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state().items_editable() {
        return Err(conflict(
            "Prizes cannot change after the entrant list is frozen.",
        ));
    }
    items::delete(&app.db, item_id, raffle_id).await?;
    app.notifier.notify(raffle_id).await;
    Ok(())
}

pub async fn lifecycle(app: &AppState, owner: &str, id: &str, action: Lifecycle) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let action = match action {
        Lifecycle::Publish => Action::Publish,
        Lifecycle::Unpublish => Action::Unpublish,
        Lifecycle::Open => Action::Open,
        Lifecycle::Close => Action::Close,
    };
    let from = raffle.state();
    let to = from.apply(action).ok_or_else(|| {
        conflict(format!(
            "Cannot {action:?} a raffle that is {}.",
            from.as_str()
        ))
    })?;
    if !raffles::transition(&app.db, id, from, to).await? {
        return Err(conflict(
            "The raffle changed in the meantime; reload and try again.",
        ));
    }
    audit::record(&app.db, id, owner, to.as_str(), "").await?;
    app.notifier.notify(id).await;
    Ok(())
}

/// Closes registration, freezes and commits the entrant list, and hands it to the draw engine.
///
/// Calling it again on a frozen raffle re-sends the stored commit document to
/// the engine (idempotent), which recovers from an engine outage at freeze time.
/// A raffle left frozen without a commit document (the freeze stopped between
/// its writes) gets one built from its now-fixed entrant list.
pub async fn start_draw(app: &AppState, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let client = engine(app)?;
    if !client.healthy().await {
        return Err(conflict(
            "Restate is not reachable. Check the connection and try again.",
        ));
    }

    let doc = match raffle.state() {
        from @ (RaffleState::Open | RaffleState::Closed) => {
            let units = units(app, id).await?;
            if entrants::count_active(&app.db, id).await? == 0 {
                return Err(conflict("Nobody has entered yet."));
            }
            // Close registration first so no join can slip in while we read the list.
            if !raffles::freeze(&app.db, id).await? {
                return Err(conflict(
                    "The raffle changed in the meantime; reload and try again.",
                ));
            }
            app.notifier.notify(id).await;
            commit(app, owner, id, units, from).await?
        }
        RaffleState::Frozen if raffle.draw_state.is_none() => match raffle.commit_doc.as_deref() {
            Some(json) => serde_json::from_str(json).map_err(anyhow::Error::from)?,
            // A freeze that stopped before writing its commit document: the
            // entrant list is fixed since the raffle froze, so commit it now.
            None => commit(app, owner, id, units(app, id).await?, RaffleState::Closed).await?,
        },
        RaffleState::Frozen | RaffleState::Drawing => return Ok(()),
        s => {
            return Err(conflict(format!(
                "Cannot start the draw of a raffle that is {}.",
                s.as_str()
            )));
        }
    };

    // The engine writes the frozen draw state and nudges the channel itself.
    client.freeze(&doc).await?;
    Ok(())
}

/// The prize units of a raffle, in draw order. Errors if there are none.
async fn units(app: &AppState, id: &str) -> Result<Vec<UnitRef>> {
    let items = items::list(&app.db, id).await?;
    let units = rafi_core::commit::units(items.iter().map(|i| {
        (
            i.id.as_str(),
            i.name.as_str(),
            u64::try_from(i.qty).unwrap_or(0),
        )
    }));
    if units.is_empty() {
        return Err(conflict("Add at least one prize first."));
    }
    Ok(units)
}

/// Commits the entrant list of a frozen raffle: builds the commit document
/// from its entrants and stores it with the list hash.
///
/// The store has no interactive transactions, so freezing and committing are
/// separate writes ([`rafi_store::raffles`]); this is the second, and is safe
/// to repeat. If another attempt committed first, its document is returned.
/// If nobody is left to draw from (everyone was removed while freezing), the
/// raffle goes back to `unfreeze_to`.
async fn commit(
    app: &AppState,
    owner: &str,
    id: &str,
    units: Vec<UnitRef>,
    unfreeze_to: RaffleState,
) -> Result<CommitDoc> {
    let entrant_ids = entrants::active_ids(&app.db, id).await?;
    if entrant_ids.is_empty() {
        raffles::unfreeze(&app.db, id, unfreeze_to).await?;
        app.notifier.notify(id).await;
        return Err(conflict("Nobody has entered yet."));
    }
    // The app alone decides the chain; the draw engine refuses a mismatch.
    let doc = CommitDoc::new(id, &app.drand.chain_hash(), entrant_ids, units);
    let json = doc.canonical_json();
    let hash = hex::encode(rafi_core::commit::list_hash(&json));
    if !raffles::commit(&app.db, id, &json, &hash).await? {
        // Another attempt committed first (or the raffle moved on).
        let stored = raffles::get(&app.db, id)
            .await?
            .and_then(|r| r.commit_doc)
            .ok_or_else(|| conflict("The raffle changed in the meantime; reload and try again."))?;
        return Ok(serde_json::from_str(&stored).map_err(anyhow::Error::from)?);
    }
    audit::record(&app.db, id, owner, "freeze", &hash).await?;
    Ok(doc)
}

/// Sends a draw command to the raffle's engine.
pub async fn command(
    app: &AppState,
    owner: &str,
    id: &str,
    cmd: Command,
    command_id: &str,
) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    if !raffle.state().drawable() {
        return Err(conflict("The draw has not started (or is already over)."));
    }
    let state = raffle.draw_state().ok_or_else(|| {
        conflict(
            "The draw engine has not confirmed the frozen list yet; press “Start the draw” again.",
        )
    })?;

    // Validate against the projection for immediate, friendly errors: a dry
    // run of the engine's own rules on a copy. Draws are accepted
    // asynchronously, so a draw the engine refused would go unnoticed.
    let mut dry = state.clone();
    let checked = match &cmd {
        // Drawing the pending unit again retries its committed round (or
        // completes it offline); nothing else may start meanwhile.
        Command::Draw { unit_id, .. } => dry.begin(unit_id, 0, 0).map(|_| ()),
        // While a draw is waiting for its round, the engine would queue the
        // command behind it; only "handed over" is let through (it can't
        // change the draw). The rest waits for the draw to land or stall.
        Command::Resolve {
            outcome: Outcome::Claimed,
            ..
        } => dry.apply(&cmd),
        _ if dry.pending.is_some() && !dry.stalled() => Err(DrawError::DrawInProgress),
        _ => dry.apply(&cmd),
    };
    checked.map_err(|e| refusal(&cmd, e))?;

    send(app, owner, id, cmd, command_id).await
}

/// The organizer-facing message for a command the draw rules refuse.
fn refusal(cmd: &Command, e: DrawError) -> AppError {
    let message = match (cmd, &e) {
        (_, DrawError::UnknownUnit(_)) => return AppError::NotFound,
        (_, DrawError::Finished) => "The raffle is finished.",
        (Command::Draw { .. }, DrawError::DrawInProgress) => "A draw is already in progress.",
        (Command::Draw { .. }, DrawError::UnitNotPending(_)) => {
            "This prize has already been drawn or withdrawn."
        }
        (_, DrawError::NoEligibleEntrants) => "There are no eligible entrants left.",
        (Command::Resolve { .. }, DrawError::UnitNotDrawn(_)) => {
            "This prize has no undecided winner."
        }
        (Command::Resolve { .. }, DrawError::DrawInProgress) => {
            "Wait for the current draw to finish: “not here” and “declined” could change who it can pick."
        }
        (Command::Abandon { .. }, DrawError::NoDrawInProgress) => {
            "This prize has no draw in progress."
        }
        (Command::Abandon { .. }, DrawError::DrawInProgress) => {
            "The draw is still waiting for its drand round; it can be abandoned if it stalls."
        }
        (Command::Void { .. }, DrawError::UnitNotPending(_)) => {
            "Only prizes that have not been won can be withdrawn."
        }
        (_, DrawError::DrawInProgress) => "Wait for the current draw to finish.",
        _ => return AppError::Conflict(e.to_string()),
    };
    conflict(message)
}

/// Audits a validated command and hands it to the draw engine, which writes
/// the new state and nudges the channel itself.
async fn send(app: &AppState, owner: &str, id: &str, cmd: Command, command_id: &str) -> Result<()> {
    let client = engine(app)?;
    audit::record(
        &app.db,
        id,
        owner,
        "command",
        &serde_json::to_string(&cmd).unwrap_or_default(),
    )
    .await?;
    client.command(id, &cmd, command_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Entrants
// ---------------------------------------------------------------------------

/// Words that are not accepted as display names (substring match, lowercase).
const BLOCKED: &[&str] = &[
    "fuck", "shit", "cunt", "nigger", "faggot", "hitler", "nazi", "bitch", "whore", "porn",
    "kurva", "geci", "fasz", "picsa", "buzi", "cigány", "köcsög", "kocsog", "baszd", "bazd",
    "anyád", "anyad",
];

pub fn validate_name(name: &str) -> Result<String, &'static str> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME_CHARS {
        return Err("err_name");
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("err_name");
    }
    let lower = name.to_lowercase();
    if BLOCKED.iter().any(|w| lower.contains(w)) {
        return Err("err_name_blocked");
    }
    Ok(name)
}

async fn unique_claim_code(app: &AppState, raffle_id: &str) -> Result<String> {
    let taken: HashSet<String> = entrants::claim_codes(&app.db, raffle_id)
        .await?
        .into_iter()
        .collect();
    for _ in 0..50 {
        let code = ids::code(4);
        if !taken.contains(&code) {
            return Ok(code);
        }
    }
    Ok(ids::code(6))
}

pub enum JoinOutcome {
    Joined { token: String },
    AlreadyIn,
}

/// Self-service join from the attendee page. Errors are i18n keys.
pub async fn join(
    app: &AppState,
    raffle: &RaffleRow,
    name: &str,
    email: &str,
    rules: bool,
    marketing: bool,
) -> std::result::Result<JoinOutcome, &'static str> {
    if !raffle.state().joinable() {
        return Err("reg_closed");
    }
    let name = validate_name(name)?;
    let email_norm = rafi_core::email::normalize(email).ok_or("err_email")?;
    if email.trim().chars().count() > 254 {
        return Err("err_email");
    }
    if !rules {
        return Err("err_rules");
    }
    let id = ids::id();
    let token = ids::token();
    let claim = unique_claim_code(app, &raffle.id)
        .await
        .map_err(|_| "err_rate")?;
    // Only while registration is open, and only below the entrant cap: the
    // hard limit behind the (soft) rate limits.
    let res = entrants::join(
        &app.db,
        &entrants::Join {
            id: &id,
            raffle_id: &raffle.id,
            display_name: &name,
            email_raw: email.trim(),
            email_norm: &email_norm,
            claim_code: &claim,
            token_hash: &ids::token_hash(&token),
            consent_version: CONSENT_VERSION,
            consent_marketing: marketing,
        },
        app.cfg.max_entrants,
    )
    .await;
    match res {
        Ok(false) => return not_joined(app, raffle, &email_norm).await,
        Ok(true) => {}
        Err(e) if e.is_unique_violation() => {
            return Ok(JoinOutcome::AlreadyIn);
        }
        Err(e) => {
            tracing::error!(error = %e, "join failed");
            return Err("err_rate");
        }
    }
    // No nudge: a join doesn't change what phones see. Screens poll.
    Ok(JoinOutcome::Joined { token })
}

/// Why a join inserted nothing: registration closed, the email is already
/// in, or the raffle is full.
async fn not_joined(
    app: &AppState,
    raffle: &RaffleRow,
    email_norm: &str,
) -> std::result::Result<JoinOutcome, &'static str> {
    let open = matches!(
        raffles::get(&app.db, &raffle.id).await,
        Ok(Some(r)) if r.state().joinable()
    );
    if !open {
        return Err("reg_closed");
    }
    let already = entrants::email_taken(&app.db, &raffle.id, email_norm)
        .await
        .map_err(|_| "err_rate")?;
    if already {
        return Ok(JoinOutcome::AlreadyIn);
    }
    Err("err_full")
}

pub async fn add_manual(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    name: &str,
    email: Option<String>,
) -> Result<String> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state().entrants_editable() {
        return Err(conflict("The entrant list is frozen."));
    }
    let name = validate_name(name).map_err(|k| bad(crate::i18n::Lang::En.t(k)))?;
    let (raw, norm) = match email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => (
            Some(e.to_owned()),
            Some(rafi_core::email::normalize(e).ok_or_else(|| bad("Invalid email address."))?),
        ),
        None => (None, None),
    };
    let id = ids::id();
    let claim = unique_claim_code(app, raffle_id).await?;
    let res = entrants::insert_manual(
        &app.db,
        &entrants::Manual {
            id: &id,
            raffle_id,
            display_name: &name,
            email_raw: raw.as_deref(),
            email_norm: norm.as_deref(),
            claim_code: &claim,
            consent_version: CONSENT_VERSION,
        },
    )
    .await;
    match res {
        Ok(_) => {}
        Err(e) if e.is_unique_violation() => {
            return Err(conflict("This email is already entered."));
        }
        Err(e) => return Err(e.into()),
    }
    audit::record(&app.db, raffle_id, owner, "add_manual", &id).await?;
    Ok(id)
}

/// Adds fake entrants for rehearsals (test mode only).
pub async fn add_fakes(app: &AppState, owner: &str, raffle_id: &str, n: u32) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.test_mode {
        return Err(conflict("Fake entrants are only allowed in test mode."));
    }
    if !raffle.state().entrants_editable() {
        return Err(conflict("The entrant list is frozen."));
    }
    const FIRST: &[&str] = &[
        "Anna", "Bence", "Csilla", "Dávid", "Eszter", "Ferenc", "Gábor", "Hanna", "István",
        "Judit", "Kata", "Levente", "Márton", "Nóra", "Orsi", "Péter", "Réka", "Sára", "Tamás",
        "Zsófi", "Alex", "Sam", "Robin", "Kim", "Max",
    ];
    let n = n.min(MAX_FAKE_ENTRANTS);
    let fakes: Vec<entrants::Fake> = (0..n)
        .map(|_| {
            let b = ids::random_bytes::<2>();
            entrants::Fake {
                id: ids::id(),
                display_name: format!(
                    "{} {}.",
                    FIRST[b[0] as usize % FIRST.len()],
                    (b'A' + b[1] % 26) as char
                ),
                claim_code: ids::code(4),
            }
        })
        .collect();
    // Their names show on the screen right away.
    entrants::insert_fakes(
        &app.db,
        raffle_id,
        CONSENT_VERSION,
        ids::now() - super::view::NAME_DELAY_SECS,
        &fakes,
    )
    .await?;
    audit::record(&app.db, raffle_id, owner, "add_fakes", &n.to_string()).await?;
    Ok(())
}

pub async fn patch_entrant(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    entrant_id: &str,
    patch: EntrantPatch,
) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    let entrant = entrants::get(&app.db, raffle_id, entrant_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if entrant.purged {
        return Err(conflict("This entrant's data has been erased."));
    }
    let renamed_or_hidden = patch.display_name.is_some() || patch.hidden.is_some();
    if let Some(name) = patch.display_name {
        let name = validate_name(&name).map_err(|k| bad(crate::i18n::Lang::En.t(k)))?;
        entrants::set_display_name(&app.db, entrant_id, &name).await?;
    }
    if let Some(hidden) = patch.hidden {
        entrants::set_hidden(&app.db, entrant_id, hidden).await?;
    }
    if let Some(delivered) = patch.delivered {
        if raffle.state() == RaffleState::Draft {
            return Err(conflict("Nothing to deliver yet."));
        }
        entrants::set_delivered(&app.db, entrant_id, delivered.then(ids::now)).await?;
    }
    audit::record(&app.db, raffle_id, owner, "patch_entrant", entrant_id).await?;
    // A drawn entrant's name is on the phones.
    if renamed_or_hidden {
        app.notifier.notify(raffle_id).await;
    }
    Ok(())
}

/// Removes an entrant before the freeze; afterwards, erases their personal data
/// (the anonymous ID must stay in the published list).
pub async fn remove_entrant(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    entrant_id: &str,
) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if raffle.state().entrants_editable() {
        entrants::delete(&app.db, raffle_id, entrant_id).await?;
        audit::record(&app.db, raffle_id, owner, "remove_entrant", entrant_id).await?;
    } else {
        entrants::erase(&app.db, raffle_id, Some(entrant_id)).await?;
        audit::record(&app.db, raffle_id, owner, "erase_entrant", entrant_id).await?;
    }
    app.notifier.notify(raffle_id).await;
    Ok(())
}

/// Erases every entrant's personal data of a finished raffle now, and ends
/// its live channel.
pub async fn purge(app: &AppState, owner: &str, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if raffle.state() != RaffleState::Finished && raffle.state() != RaffleState::Purged {
        return Err(conflict("Only finished raffles can be purged."));
    }
    entrants::erase(&app.db, raffle_id, None).await?;
    raffles::mark_purged(&app.db, raffle_id).await?;
    audit::record(&app.db, raffle_id, owner, "purge", "").await?;
    // Destroying the channel publishes the final (erased) view first.
    app.notifier.end(raffle_id).await?;
    Ok(())
}

/// Ends live updates of a finished raffle: phones stop listening and the
/// raffle's live channel is destroyed. Safe to repeat (the channel is
/// destroyed again).
pub async fn end_live(app: &AppState, owner: &str, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !matches!(raffle.state(), RaffleState::Finished | RaffleState::Purged) {
        return Err(conflict(
            "Live updates can only end once the raffle has finished.",
        ));
    }
    raffles::end_live(&app.db, raffle_id).await?;
    if raffle.live_ended_at.is_none() {
        audit::record(&app.db, raffle_id, owner, "end_live", "").await?;
    }
    app.notifier.end(raffle_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

pub async fn admin(app: &AppState, owner: &str, id: &str) -> Result<RaffleAdmin> {
    let raffle = owned(app, owner, id).await?;
    let items = items::list(&app.db, id).await?;
    let outcomes = raffle
        .draw_state()
        .map(|s| s.entrant_outcomes())
        .unwrap_or_default();
    let unit_names: std::collections::HashMap<String, String> = raffle
        .draw_state()
        .map(|s| {
            s.units
                .into_iter()
                .map(|u| (u.unit_id, u.item_name))
                .collect()
        })
        .unwrap_or_default();
    let entrants = entrants::list(&app.db, id)
        .await?
        .into_iter()
        .map(|e| {
            let outcome = outcomes.get(&e.id);
            AdminEntrant {
                outcome: outcome.map(|o| o.0),
                prize: outcome.and_then(|o| unit_names.get(&o.1).cloned()),
                id: e.id,
                display_name: e.display_name,
                email: e.email_raw,
                source: EntrantSource::parse(&e.source),
                hidden: e.hidden,
                claim_code: e.claim_code,
                marketing: e.consent_marketing,
                created_at: e.created_at,
                delivered: e.delivered_at.is_some(),
                purged: e.purged,
            }
        })
        .collect();
    let view = super::view::build(app, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(RaffleAdmin {
        id: raffle.id.clone(),
        settings: settings_of(&raffle),
        items: items
            .into_iter()
            .map(|i| Item {
                id: i.id,
                name: i.name,
                sponsor: i.sponsor,
                description: i.description,
                qty: i.qty.max(0) as u32,
                position: i.position,
            })
            .collect(),
        entrants,
        view,
        public_url: app.cfg.public_url.clone(),
    })
}

pub async fn export_csv(app: &AppState, owner: &str, id: &str) -> Result<String> {
    let data = admin(app, owner, id).await?;
    fn esc(s: &str) -> String {
        // Quote, and neutralize spreadsheet formula injection.
        let s = if s.starts_with(['=', '+', '-', '@']) {
            format!("'{s}")
        } else {
            s.to_owned()
        };
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    let mut out = String::from(
        "entrant_id,display_name,email,source,claim_code,outcome,prize,delivered,marketing_consent\n",
    );
    for e in data.entrants.iter().filter(|e| e.outcome.is_some()) {
        let outcome = match e.outcome {
            Some(Outcome::Claimed) => "claimed",
            Some(Outcome::Drawn) => "drawn",
            Some(Outcome::NotHere) => "not_here",
            Some(Outcome::Declined) => "declined",
            None => "",
        };
        out.push_str(
            &[
                esc(&e.id),
                esc(&e.display_name),
                esc(e.email.as_deref().unwrap_or("")),
                esc(e.source.as_str()),
                esc(&e.claim_code),
                esc(outcome),
                esc(e.prize.as_deref().unwrap_or("")),
                esc(if e.delivered { "yes" } else { "no" }),
                esc(if e.marketing { "yes" } else { "no" }),
            ]
            .join(","),
        );
        out.push('\n');
    }
    Ok(out)
}

pub async fn verify_data(app: &AppState, slug: &str) -> Result<VerifyData> {
    let raffle = raffles::by_slug(&app.db, slug)
        .await?
        .ok_or(AppError::NotFound)?;
    let (Some(commit_doc), Some(list_hash)) = (raffle.commit_doc.clone(), raffle.list_hash.clone())
    else {
        return Err(AppError::NotFound);
    };
    let names = entrants::list(&app.db, &raffle.id)
        .await?
        .into_iter()
        .map(|e| {
            let name = if e.hidden {
                super::view::HIDDEN_NAME.to_owned()
            } else {
                e.display_name
            };
            (e.id, name)
        })
        .collect();
    Ok(VerifyData {
        title: raffle.title.clone(),
        state: raffle.state(),
        events: raffle.draw_state().map(|s| s.events).unwrap_or_default(),
        commit_doc,
        list_hash,
        names,
    })
}
