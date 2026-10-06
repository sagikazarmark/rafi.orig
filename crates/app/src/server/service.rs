//! Raffle operations shared by server functions and attendee routes.

use std::collections::HashSet;

use rafi_core::{
    commit::{CommitDoc, UnitRef},
    raffle::{Action, RaffleState},
    state::{Outcome, UnitStatus},
};

use super::{
    AppState,
    config::EngineKind,
    db::{self, RaffleRow},
    engine::Command,
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
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<rafi_core::state::DrawError> for AppError {
    fn from(e: rafi_core::state::DrawError) -> Self {
        Self::Conflict(e.to_string())
    }
}

impl From<AppError> for dioxus::prelude::ServerFnError {
    fn from(e: AppError) -> Self {
        if let AppError::Internal(e) = &e {
            tracing::error!(error = %e, "internal error");
            return dioxus::prelude::ServerFnError::new("internal error");
        }
        dioxus::prelude::ServerFnError::new(e.to_string())
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

fn conflict(msg: impl Into<String>) -> AppError {
    AppError::Conflict(msg.into())
}

pub fn unit_id(item_id: &str, n: i64) -> String {
    format!("{item_id}-{n}")
}

// ---------------------------------------------------------------------------
// Raffles
// ---------------------------------------------------------------------------

pub async fn owned(app: &AppState, owner: &str, id: &str) -> Result<RaffleRow> {
    let raffle = db::raffle(&app.db, id).await?.ok_or(AppError::NotFound)?;
    if raffle.owner_id != owner {
        return Err(AppError::NotFound);
    }
    Ok(raffle)
}

pub async fn list(app: &AppState, owner: &str) -> Result<Vec<RaffleSummary>> {
    let rows: Vec<(String, String, String, String, bool, i64, i64)> = sqlx::query_as(
        "SELECT r.id, r.slug, r.title, r.state, r.test_mode, r.created_at,
                (SELECT COUNT(*) FROM entrants e WHERE e.raffle_id = r.id)
         FROM raffles r WHERE r.owner_id = ? ORDER BY r.created_at DESC",
    )
    .bind(owner)
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, slug, title, state, test_mode, created_at, entrant_count)| RaffleSummary {
                id,
                slug,
                title,
                state: RaffleState::parse(&state).unwrap_or(RaffleState::Draft),
                entrant_count,
                test_mode,
                created_at,
            },
        )
        .collect())
}

pub async fn create(app: &AppState, owner: &str, title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(bad("The title must be 1–120 characters."));
    }
    let id = ids::id();
    let now = ids::now();
    for _ in 0..8 {
        let res = sqlx::query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, engine, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(owner)
        .bind(ids::slug(title))
        .bind(ids::code(4))
        .bind(title)
        .bind(app.cfg.engine.as_str())
        .bind(now)
        .bind(now)
        .execute(&app.db)
        .await;
        match res {
            Ok(_) => {
                db::audit(&app.db, &id, owner, "create", title).await?;
                return Ok(id);
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue,
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
    sqlx::query(
        "UPDATE raffles SET title = ?, description = ?, lang = ?, organizer_name = ?, contact_email = ?, rules = ?,
         event_date = ?, min_age = ?, test_mode = ?, updated_at = ? WHERE id = ?",
    )
    .bind(title)
    .bind(s.description.trim())
    .bind(lang)
    .bind(s.organizer_name.trim())
    .bind(s.contact_email.trim())
    .bind(s.rules.trim())
    .bind(s.event_date.trim())
    .bind(s.min_age.min(99) as i64)
    .bind(s.test_mode)
    .bind(ids::now())
    .bind(id)
    .execute(&app.db)
    .await?;
    app.refresh(id).await;
    Ok(())
}

pub async fn delete(app: &AppState, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    if raffle.state() == RaffleState::Drawing
        && raffle.draw_state().is_some_and(|s| s.pending.is_some())
    {
        return Err(conflict("A draw is in progress."));
    }
    sqlx::query("DELETE FROM raffles WHERE id = ?")
        .bind(id)
        .execute(&app.db)
        .await?;
    db::audit(&app.db, id, owner, "delete", "").await?;
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
    for item in db::items(&app.db, id).await? {
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
    let id = match &input.id {
        Some(id) => {
            let res = sqlx::query(
                "UPDATE items SET name = ?, sponsor = ?, description = ?, qty = ?, position = ? WHERE id = ? AND raffle_id = ?",
            )
            .bind(name)
            .bind(input.sponsor.trim())
            .bind(input.description.trim())
            .bind(input.qty as i64)
            .bind(input.position)
            .bind(id)
            .bind(raffle_id)
            .execute(&app.db)
            .await?;
            if res.rows_affected() == 0 {
                return Err(AppError::NotFound);
            }
            id.clone()
        }
        None => {
            let id = ids::code(6).to_lowercase();
            sqlx::query(
                "INSERT INTO items (id, raffle_id, name, sponsor, description, qty, position) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(raffle_id)
            .bind(name)
            .bind(input.sponsor.trim())
            .bind(input.description.trim())
            .bind(input.qty as i64)
            .bind(input.position)
            .execute(&app.db)
            .await?;
            id
        }
    };
    app.refresh(raffle_id).await;
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
    sqlx::query("DELETE FROM items WHERE id = ? AND raffle_id = ?")
        .bind(item_id)
        .bind(raffle_id)
        .execute(&app.db)
        .await?;
    app.refresh(raffle_id).await;
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
    let res =
        sqlx::query("UPDATE raffles SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(to.as_str())
            .bind(ids::now())
            .bind(id)
            .bind(from.as_str())
            .execute(&app.db)
            .await?;
    if res.rows_affected() == 0 {
        return Err(conflict(
            "The raffle changed in the meantime; reload and try again.",
        ));
    }
    db::audit(&app.db, id, owner, to.as_str(), "").await?;
    app.refresh(id).await;
    Ok(())
}

/// Closes registration, freezes and commits the entrant list, and hands it to the draw engine.
///
/// Calling it again on a frozen raffle re-sends the stored commit document to
/// the engine (idempotent), which recovers from an engine outage at freeze time.
pub async fn start_draw(app: &AppState, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let engine = EngineKind::parse(&raffle.engine);
    if engine == EngineKind::Restate {
        let client = app
            .restate
            .get()
            .ok_or_else(|| conflict("Restate is not configured on this server."))?;
        if !client.healthy().await {
            return Err(conflict(
                "Restate is not reachable. Check the connection, or switch this raffle to the local engine.",
            ));
        }
    }

    let doc = match raffle.state() {
        RaffleState::Open | RaffleState::Closed => {
            let items = db::items(&app.db, id).await?;
            let units: Vec<UnitRef> = items
                .iter()
                .flat_map(|i| {
                    (1..=i.qty).map(move |n| UnitRef {
                        unit_id: unit_id(&i.id, n),
                        item_id: i.id.clone(),
                        item_name: i.name.clone(),
                    })
                })
                .collect();
            if units.is_empty() {
                return Err(conflict("Add at least one prize first."));
            }

            // Close registration first so no join can slip in while we read the list.
            let mut tx = app.db.begin().await?;
            let res = sqlx::query("UPDATE raffles SET state = 'frozen', updated_at = ? WHERE id = ? AND state IN ('open', 'closed')")
                .bind(ids::now())
                .bind(id)
                .execute(&mut *tx)
                .await?;
            if res.rows_affected() == 0 {
                return Err(conflict(
                    "The raffle changed in the meantime; reload and try again.",
                ));
            }
            let entrant_ids: Vec<(String,)> =
                sqlx::query_as("SELECT id FROM entrants WHERE raffle_id = ? AND purged = 0")
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await?;
            if entrant_ids.is_empty() {
                return Err(conflict("Nobody has entered yet."));
            }
            // The app alone decides the chain; the draw engine refuses a mismatch.
            let doc = CommitDoc::new(
                id,
                &app.drand.chain_hash(),
                entrant_ids.into_iter().map(|(e,)| e).collect(),
                units,
            );
            let json = doc.canonical_json();
            let hash = hex::encode(rafi_core::commit::list_hash(&json));
            sqlx::query(
                "UPDATE raffles SET commit_doc = ?, list_hash = ?, frozen_at = ? WHERE id = ?",
            )
            .bind(&json)
            .bind(&hash)
            .bind(ids::now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            db::audit(&app.db, id, owner, "freeze", &hash).await?;
            app.refresh(id).await;
            doc
        }
        RaffleState::Frozen if raffle.draw_state.is_none() => {
            let json = raffle
                .commit_doc
                .as_deref()
                .ok_or_else(|| conflict("Missing commit document."))?;
            serde_json::from_str(json).map_err(anyhow::Error::from)?
        }
        RaffleState::Frozen | RaffleState::Drawing => return Ok(()),
        s => {
            return Err(conflict(format!(
                "Cannot start the draw of a raffle that is {}.",
                s.as_str()
            )));
        }
    };

    match engine {
        EngineKind::Restate => {
            app.watch.wake(id);
            app.restate
                .get()
                .expect("checked above")
                .freeze(&doc)
                .await?
        }
        EngineKind::Local => app.local.freeze(app, &doc).await?,
    }
    Ok(())
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

    // Validate against the projection for immediate, friendly errors.
    match &cmd {
        Command::Draw { unit_id, .. } => {
            if let Some(p) = &state.pending {
                // Drawing the pending unit again retries its committed round
                // (or completes it offline); nothing else may start meanwhile.
                if &p.unit_id != unit_id {
                    return Err(conflict("A draw is already in progress."));
                }
                return send(app, &raffle, owner, id, cmd, command_id).await;
            }
            let unit = state.unit(unit_id).ok_or(AppError::NotFound)?;
            if unit.status != UnitStatus::Pending {
                return Err(conflict("This prize has already been drawn or withdrawn."));
            }
            if state.pool().1.is_empty() {
                return Err(conflict("There are no eligible entrants left."));
            }
        }
        Command::Resolve { unit_id, outcome } => {
            let unit = state.unit(unit_id).ok_or(AppError::NotFound)?;
            if unit.status != UnitStatus::Drawn {
                return Err(conflict("This prize has no undecided winner."));
            }
            if state.pending.is_some() && *outcome != Outcome::Claimed {
                return Err(conflict(
                    "Wait for the current draw to finish: “not here” and “declined” could change who it can pick.",
                ));
            }
        }
        Command::Abandon { unit_id } => {
            if state.pending.as_ref().is_none_or(|p| &p.unit_id != unit_id) {
                return Err(conflict("This prize has no draw in progress."));
            }
        }
        Command::Void { unit_id } => {
            let unit = state.unit(unit_id).ok_or(AppError::NotFound)?;
            if unit.status != UnitStatus::Pending {
                return Err(conflict(
                    "Only prizes that have not been won can be withdrawn.",
                ));
            }
        }
        Command::Finish => {
            if state.pending.is_some() {
                return Err(conflict("Wait for the current draw to finish."));
            }
        }
    }

    send(app, &raffle, owner, id, cmd, command_id).await
}

/// Audits a validated command and hands it to the raffle's engine.
async fn send(
    app: &AppState,
    raffle: &db::RaffleRow,
    owner: &str,
    id: &str,
    cmd: Command,
    command_id: &str,
) -> Result<()> {
    db::audit(
        &app.db,
        id,
        owner,
        "command",
        &serde_json::to_string(&cmd).unwrap_or_default(),
    )
    .await?;
    match EngineKind::parse(&raffle.engine) {
        EngineKind::Restate => {
            let client = app.restate.get().ok_or_else(|| conflict("Restate is not configured on this server; switch this raffle to the local engine."))?;
            app.watch.wake(id);
            client.command(id, &cmd, command_id).await?;
        }
        EngineKind::Local => app.local.command(app, id, cmd).await?,
    }
    Ok(())
}

/// Emergency switch: continue this raffle's draw in-process (e.g. Restate is down).
///
/// The projection becomes the source of truth. A pending draw stays pending on
/// its committed round, and the local engine fetches that round.
pub async fn switch_to_local(app: &AppState, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    if raffle.engine == "local" {
        return Ok(());
    }
    if let Some(state) = raffle.draw_state()
        && !state
            .drand_chain
            .eq_ignore_ascii_case(&app.drand.chain_hash().to_string())
    {
        return Err(conflict(
            "This raffle uses a different drand chain than this server's local engine.",
        ));
    }
    sqlx::query("UPDATE raffles SET engine = 'local', updated_at = ? WHERE id = ?")
        .bind(ids::now())
        .bind(id)
        .execute(&app.db)
        .await?;
    db::audit(&app.db, id, owner, "switch_to_local", "").await?;
    app.local.resume(app, id).await?;
    app.refresh(id).await;
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
    let taken: HashSet<String> =
        sqlx::query_as::<_, (String,)>("SELECT claim_code FROM entrants WHERE raffle_id = ?")
            .bind(raffle_id)
            .fetch_all(&app.db)
            .await?
            .into_iter()
            .map(|(c,)| c)
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
    let now = ids::now();
    let res = sqlx::query(
        "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source, claim_code, token_hash,
                               consent_version, consent_rules_at, consent_marketing, created_at)
         SELECT ?, ?, ?, ?, ?, 'self', ?, ?, ?, ?, ?, ?
         WHERE EXISTS (SELECT 1 FROM raffles WHERE id = ? AND state = 'open')",
    )
    .bind(&id)
    .bind(&raffle.id)
    .bind(&name)
    .bind(email.trim())
    .bind(&email_norm)
    .bind(&claim)
    .bind(ids::token_hash(&token))
    .bind(CONSENT_VERSION)
    .bind(now)
    .bind(marketing)
    .bind(now)
    .bind(&raffle.id)
    .execute(&app.db)
    .await;
    match res {
        Ok(r) if r.rows_affected() == 1 => {}
        Ok(_) => return Err("reg_closed"),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Ok(JoinOutcome::AlreadyIn);
        }
        Err(e) => {
            tracing::error!(error = %e, "join failed");
            return Err("err_rate");
        }
    }
    app.refresh(&raffle.id).await;
    app.refresh_later(
        &raffle.id,
        std::time::Duration::from_secs(super::view::NAME_DELAY_SECS as u64 + 1),
    );
    Ok(JoinOutcome::Joined { token })
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
    let res = sqlx::query(
        "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source, claim_code, consent_version, consent_rules_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'manual', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(raffle_id)
    .bind(&name)
    .bind(raw)
    .bind(norm)
    .bind(&claim)
    .bind(CONSENT_VERSION)
    .bind(ids::now())
    .bind(ids::now())
    .execute(&app.db)
    .await;
    match res {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(conflict("This email is already entered."));
        }
        Err(e) => return Err(e.into()),
    }
    db::audit(&app.db, raffle_id, owner, "add_manual", &id).await?;
    app.refresh(raffle_id).await;
    app.refresh_later(
        raffle_id,
        std::time::Duration::from_secs(super::view::NAME_DELAY_SECS as u64 + 1),
    );
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
    let mut tx = app.db.begin().await?;
    for _ in 0..n {
        let b = ids::random_bytes::<2>();
        let name = format!(
            "{} {}.",
            FIRST[b[0] as usize % FIRST.len()],
            (b'A' + b[1] % 26) as char
        );
        sqlx::query(
            "INSERT INTO entrants (id, raffle_id, display_name, source, claim_code, consent_version, consent_rules_at, created_at)
             VALUES (?, ?, ?, 'fake', ?, ?, ?, ?)",
        )
        .bind(ids::id())
        .bind(raffle_id)
        .bind(name)
        .bind(ids::code(4))
        .bind(CONSENT_VERSION)
        .bind(ids::now())
        .bind(ids::now() - super::view::NAME_DELAY_SECS)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    db::audit(&app.db, raffle_id, owner, "add_fakes", &n.to_string()).await?;
    app.refresh(raffle_id).await;
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
    let entrant = db::entrant(&app.db, raffle_id, entrant_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if entrant.purged {
        return Err(conflict("This entrant's data has been erased."));
    }
    if let Some(name) = patch.display_name {
        let name = validate_name(&name).map_err(|k| bad(crate::i18n::Lang::En.t(k)))?;
        sqlx::query("UPDATE entrants SET display_name = ? WHERE id = ?")
            .bind(name)
            .bind(entrant_id)
            .execute(&app.db)
            .await?;
    }
    if let Some(hidden) = patch.hidden {
        sqlx::query("UPDATE entrants SET hidden = ? WHERE id = ?")
            .bind(hidden)
            .bind(entrant_id)
            .execute(&app.db)
            .await?;
    }
    if let Some(delivered) = patch.delivered {
        if raffle.state() == RaffleState::Draft {
            return Err(conflict("Nothing to deliver yet."));
        }
        let at = delivered.then(ids::now);
        sqlx::query("UPDATE entrants SET delivered_at = ? WHERE id = ?")
            .bind(at)
            .bind(entrant_id)
            .execute(&app.db)
            .await?;
    }
    db::audit(&app.db, raffle_id, owner, "patch_entrant", entrant_id).await?;
    app.refresh(raffle_id).await;
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
        sqlx::query("DELETE FROM entrants WHERE id = ? AND raffle_id = ?")
            .bind(entrant_id)
            .bind(raffle_id)
            .execute(&app.db)
            .await?;
        db::audit(&app.db, raffle_id, owner, "remove_entrant", entrant_id).await?;
    } else {
        erase(app, raffle_id, Some(entrant_id)).await?;
        db::audit(&app.db, raffle_id, owner, "erase_entrant", entrant_id).await?;
    }
    app.refresh(raffle_id).await;
    Ok(())
}

/// Erases personal data of one entrant, or all entrants of a raffle.
pub async fn erase(app: &AppState, raffle_id: &str, entrant_id: Option<&str>) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE entrants SET display_name = '#' || substr(id, 1, 6), email_raw = NULL, email_norm = NULL, token_hash = NULL,
         consent_marketing = 0, purged = 1
         WHERE raffle_id = ?1 AND purged = 0 AND (?2 IS NULL OR id = ?2)",
    )
    .bind(raffle_id)
    .bind(entrant_id)
    .execute(&app.db)
    .await?;
    Ok(res.rows_affected())
}

pub async fn purge(app: &AppState, owner: &str, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if raffle.state() != RaffleState::Finished && raffle.state() != RaffleState::Purged {
        return Err(conflict("Only finished raffles can be purged."));
    }
    erase(app, raffle_id, None).await?;
    sqlx::query("UPDATE raffles SET state = 'purged', purged_at = ?, updated_at = ? WHERE id = ?")
        .bind(ids::now())
        .bind(ids::now())
        .bind(raffle_id)
        .execute(&app.db)
        .await?;
    db::audit(&app.db, raffle_id, owner, "purge", "").await?;
    app.refresh(raffle_id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

pub async fn admin(app: &AppState, owner: &str, id: &str) -> Result<RaffleAdmin> {
    let raffle = owned(app, owner, id).await?;
    let items = db::items(&app.db, id).await?;
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
    let entrants = db::entrants(&app.db, id)
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
    let view = match app.hub.current(id) {
        Some(v) => (*v).clone(),
        None => super::view::build(app, id)
            .await?
            .ok_or(AppError::NotFound)?,
    };
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
    let raffle = db::raffle_by_slug(&app.db, slug)
        .await?
        .ok_or(AppError::NotFound)?;
    let (Some(commit_doc), Some(list_hash)) = (raffle.commit_doc.clone(), raffle.list_hash.clone())
    else {
        return Err(AppError::NotFound);
    };
    let names = db::entrants(&app.db, &raffle.id)
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
