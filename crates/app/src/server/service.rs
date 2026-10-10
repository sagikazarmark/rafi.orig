//! Raffle operations shared by the admin API and the participant routes.
//!
//! Every write that changes what phones see (the participant view) notifies
//! clients ([`AppState::notifier`]). Joins don't: they never change it. The
//! drawing engine notifies after its own writes.

use std::collections::HashSet;

use rafi_drawing::{
    BeaconSource, Command, Decision, DrawRequest, Error as DrawingError, ErrorKind, ItemId,
    Manifest, ManifestHash, ManifestItem, Outcome, Resolution,
};
use rafi_engine::{DrawingEngine, EngineError};
use rafi_raffle::{
    self as raffle, RaffleState,
    entrant::{self, Email, NameError, Removal},
    settings,
    start::{self, Start},
};
use rafi_store::{RaffleRow, audit, entrants, prizes, raffles};

use super::{AppState, ids};
use crate::model::{
    AdminEntrant, EntrantPatch, EntrantSource, Lifecycle, Prize, PrizeInput, RaffleAdmin,
    RaffleSettings, RaffleSummary, VerifyData,
};

/// Version of the consent text shown on the join form.
pub const CONSENT_VERSION: &str = "2026-10-01";
pub const MAX_FAKE_ENTRANTS: u32 = 500;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    /// A service the request needs (the drawing engine) is unreachable; retry.
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<rafi_store::Error> for AppError {
    fn from(e: rafi_store::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<DrawingError> for AppError {
    fn from(e: DrawingError) -> Self {
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

/// The drawing engine, or a conflict if this server has none.
fn engine(app: &AppState) -> Result<&dyn DrawingEngine> {
    app.engine
        .as_deref()
        .ok_or_else(|| conflict("Restate is not configured on this server."))
}

/// What the organizer sees of a call to the drawing engine. A call the engine
/// accepted but hasn't settled yet ([`EngineError::Pending`]) succeeds: it
/// applies once, and the screens follow the projection.
fn engine_result(result: std::result::Result<(), EngineError>) -> Result<()> {
    match result {
        Ok(()) | Err(EngineError::Pending) => Ok(()),
        Err(EngineError::Rejected { kind, message }) => Err(match kind {
            ErrorKind::NotFound => AppError::NotFound,
            ErrorKind::BadRequest => AppError::BadRequest(message),
            ErrorKind::Conflict => AppError::Conflict(message),
            ErrorKind::Invariant => {
                AppError::Internal(anyhow::anyhow!("drawing engine invariant: {message}"))
            }
        }),
        Err(EngineError::Unavailable(e)) => {
            tracing::warn!(error = %format!("{e:#}"), "drawing engine unavailable");
            Err(AppError::Unavailable(
                "The drawing engine is not reachable right now. Try again in a moment.".into(),
            ))
        }
    }
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
            state: r.state,
            id: r.id,
            slug: r.slug,
            title: r.title,
            entrant_count: r.entrant_count,
            test_mode: r.test_mode,
            created_at: r.created_at,
        })
        .collect())
}

/// The organizer-facing refusal of a title that isn't one.
fn title(input: &str) -> Result<&str> {
    settings::title(input).ok_or_else(|| {
        bad(format!(
            "The title must be 1–{} characters.",
            settings::MAX_TITLE_CHARS
        ))
    })
}

pub async fn create(app: &AppState, owner: &str, title: &str) -> Result<String> {
    let title = self::title(title)?;
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
    let title = title(&s.title)?;
    if !settings::texts_fit(&s.description, &s.rules) {
        return Err(bad("Description or rules are too long."));
    }
    let lang = if s.lang == "hu" { "hu" } else { "en" };
    if raffle.test_mode != s.test_mode && !raffle.state.test_mode_editable() {
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
    if !raffle::drawing::deletable(raffle.state, raffle.drawing()?.as_ref()) {
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

/// Copies settings and prizes into a new draft raffle (no entrants), in one
/// write: a copy is whole or not there.
pub async fn duplicate(app: &AppState, owner: &str, id: &str) -> Result<String> {
    let raffle = owned(app, owner, id).await?;
    let title = settings::copy_title(&raffle.title);
    let prizes = prizes::list(&app.db, id).await?;
    let copies: Vec<(String, prizes::PrizeFields)> = prizes
        .iter()
        .map(|prize| {
            (
                ids::code(6).to_lowercase(),
                prizes::PrizeFields {
                    name: &prize.name,
                    sponsor: &prize.sponsor,
                    description: &prize.description,
                    qty: prize.qty.max(0) as u32,
                    position: prize.position,
                },
            )
        })
        .collect();
    let settings = raffles::Settings {
        title: &title,
        description: &raffle.description,
        lang: &raffle.lang,
        organizer_name: &raffle.organizer_name,
        contact_email: &raffle.contact_email,
        rules: &raffle.rules,
        event_date: &raffle.event_date,
        min_age: raffle.min_age.clamp(0, 99) as u32,
        test_mode: raffle.test_mode,
    };
    let new_id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert_with(
            &app.db,
            &raffles::NewRaffle {
                id: &new_id,
                owner_id: owner,
                slug: &ids::slug(&title),
                join_code: &ids::code(4),
                title: &title,
            },
            &settings,
            &copies,
        )
        .await;
        match res {
            Ok(()) => {
                audit::record(&app.db, &new_id, owner, "create", &title).await?;
                return Ok(new_id);
            }
            Err(e) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(conflict(
        "Could not allocate a unique join code; please try again.",
    ))
}

pub async fn upsert_prize(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    input: PrizeInput,
) -> Result<String> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state.prizes_editable() {
        return Err(conflict(
            "Prizes cannot change after the entrant list is frozen.",
        ));
    }
    let name = settings::title(&input.name).ok_or_else(|| {
        bad(format!(
            "The prize name must be 1–{} characters.",
            settings::MAX_TITLE_CHARS
        ))
    })?;
    if !settings::prize_qty(input.qty) {
        return Err(bad(format!(
            "The quantity must be between 1 and {}.",
            settings::MAX_PRIZE_QTY
        )));
    }
    let fields = prizes::PrizeFields {
        name,
        sponsor: input.sponsor.trim(),
        description: input.description.trim(),
        qty: input.qty,
        position: input.position,
    };
    let id = match &input.id {
        Some(id) => {
            if !prizes::update(&app.db, id, raffle_id, &fields).await? {
                return Err(AppError::NotFound);
            }
            id.clone()
        }
        None => {
            let id = ids::code(6).to_lowercase();
            prizes::insert(&app.db, &id, raffle_id, &fields).await?;
            id
        }
    };
    // Before the freeze, phones count the prizes from their quantities.
    app.notifier.notify(raffle_id).await;
    Ok(id)
}

pub async fn delete_prize(
    app: &AppState,
    owner: &str,
    raffle_id: &str,
    prize_id: &str,
) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state.prizes_editable() {
        return Err(conflict(
            "Prizes cannot change after the entrant list is frozen.",
        ));
    }
    prizes::delete(&app.db, prize_id, raffle_id).await?;
    app.notifier.notify(raffle_id).await;
    Ok(())
}

pub async fn lifecycle(app: &AppState, owner: &str, id: &str, action: Lifecycle) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let command = match action {
        Lifecycle::Publish => raffle::Command::Publish,
        Lifecycle::Unpublish => raffle::Command::Unpublish,
        Lifecycle::Open => raffle::Command::Open,
        Lifecycle::Close => raffle::Command::Close,
    };
    let from = raffle.state;
    let to = from.apply(command).ok_or_else(|| {
        conflict(format!(
            "Cannot {command:?} a raffle that is {}.",
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

/// Closes registration, freezes and commits the entrant list, and hands it to the drawing engine.
///
/// Calling it again on a frozen raffle re-sends the stored commit document to
/// the engine (idempotent), which recovers from an engine outage at freeze time.
/// A raffle left frozen without a commit document (the freeze stopped between
/// its writes) gets one built from its now-fixed entrant list.
pub async fn start_draw(app: &AppState, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let engine = engine(app)?;
    let info = engine.info().await.map_err(|e| {
        tracing::warn!(error = %e, "drawing engine unavailable");
        AppError::Unavailable(
            "Restate is not reachable. Check the connection and try again.".into(),
        )
    })?;
    // The app decides the beacon source; the engine would refuse another
    // when the drawing opens, after the manifest is stored. Refuse before
    // anything is frozen.
    let beacon = BeaconSource::Drand(app.chain);
    if info.beacon != beacon {
        return Err(conflict(format!(
            "The drawing engine draws from {}, but raffles here commit to {beacon}. Fix the configuration before starting the draw.",
            info.beacon
        )));
    }

    let committed = raffle.manifest.is_some();
    let opened = raffle.drawing.is_some();
    let next = start::next(raffle.state, committed, opened).map_err(refused_start)?;
    let manifest = match next {
        Start::Freeze { from } => {
            let items = items(app, id).await?;
            let entrants = entrants::count_active(&app.db, id).await?;
            start::ready(usize::try_from(entrants).unwrap_or(0), items.len())
                .map_err(refused_start)?;
            // Close registration first so no join can slip in while we read the list.
            if !raffles::freeze(&app.db, id, from).await? {
                return Err(conflict(
                    "The raffle changed in the meantime; reload and try again.",
                ));
            }
            app.notifier.notify(id).await;
            write_manifest(app, owner, id, items, from).await?
        }
        Start::Commit { unfreeze_to } => {
            write_manifest(app, owner, id, items(app, id).await?, unfreeze_to).await?
        }
        Start::Resend => {
            let json = raffle.manifest.as_deref().unwrap_or_default();
            serde_json::from_str(json).map_err(anyhow::Error::from)?
        }
        Start::Started => return Ok(()),
    };

    // The engine writes the opened drawing and nudges the channel itself.
    engine_result(engine.open(&manifest).await)
}

/// The organizer-facing message for a draw that can't start.
fn refused_start(refusal: start::Refusal) -> AppError {
    match refusal {
        start::Refusal::State(s) => conflict(format!(
            "Cannot start the draw of a raffle that is {}.",
            s.as_str()
        )),
        start::Refusal::NoEntrants => conflict("Nobody has entered yet."),
        start::Refusal::NoPrizes => conflict("Add at least one prize first."),
    }
}

/// The drawing's items for a raffle's prizes (one per copy), in draw order.
/// Errors if there are none.
async fn items(app: &AppState, id: &str) -> Result<Vec<ManifestItem>> {
    let prizes = prizes::list(&app.db, id).await?;
    let items = rafi_raffle::drawing::items(prizes.iter().map(|i| {
        (
            i.id.as_str(),
            i.name.as_str(),
            u64::try_from(i.qty).unwrap_or(0),
        )
    }));
    if items.is_empty() {
        return Err(refused_start(start::Refusal::NoPrizes));
    }
    Ok(items)
}

/// Writes the manifest of a frozen raffle: builds it from its entrants and
/// stores it with its hash.
///
/// The store has no interactive transactions, so freezing and writing the
/// manifest are separate writes ([`rafi_store::raffles`]); this is the second,
/// and is safe to repeat. If another attempt wrote one first, that one is
/// returned. If nobody is left to draw from (everyone was removed while
/// freezing), the raffle goes back to `unfreeze_to`.
async fn write_manifest(
    app: &AppState,
    owner: &str,
    id: &str,
    items: Vec<ManifestItem>,
    unfreeze_to: RaffleState,
) -> Result<Manifest> {
    let entrant_ids = entrants::active_ids(&app.db, id).await?;
    if let Err(refusal) = start::ready(entrant_ids.len(), items.len()) {
        let to = RaffleState::Frozen
            .unfreeze(unfreeze_to)
            .ok_or_else(|| anyhow::anyhow!("cannot unfreeze to {}", unfreeze_to.as_str()))?;
        raffles::unfreeze(&app.db, id, to).await?;
        app.notifier.notify(id).await;
        return Err(refused_start(refusal));
    }
    // The app alone decides the chain; the drawing engine refuses a mismatch.
    let manifest =
        rafi_raffle::drawing::manifest(id, BeaconSource::Drand(app.chain), entrant_ids, items);
    let json = manifest.canonical_json();
    let hash = ManifestHash::of(&json).to_string();
    if !raffles::set_manifest(&app.db, id, &json, &hash).await? {
        // Another attempt wrote one first (or the raffle moved on).
        let stored = raffles::get(&app.db, id)
            .await?
            .and_then(|r| r.manifest)
            .ok_or_else(|| conflict("The raffle changed in the meantime; reload and try again."))?;
        return Ok(serde_json::from_str(&stored).map_err(anyhow::Error::from)?);
    }
    audit::record(&app.db, id, owner, "freeze", &hash).await?;
    Ok(manifest)
}

/// Sends a draw command to the raffle's engine.
pub async fn command(
    app: &AppState,
    owner: &str,
    id: &str,
    cmd: impl Into<Command>,
    command_id: &str,
) -> Result<()> {
    let cmd = cmd.into();
    let raffle = owned(app, owner, id).await?;
    if !raffle.state.drawable() {
        return Err(conflict("The draw has not started (or is already over)."));
    }
    let state = raffle.drawing()?.ok_or_else(|| {
        conflict(
            "The drawing engine has not confirmed the frozen list yet; press “Start the draw” again.",
        )
    })?;

    // Checked against the projection for immediate, friendly errors: a draw
    // the engine refused would go unnoticed.
    state.admit(&cmd).map_err(|e| refusal(&cmd, e))?;

    send(app, owner, id, cmd, command_id).await
}

/// The organizer-facing message for a command the draw rules refuse.
fn refusal(cmd: &Command, e: DrawingError) -> AppError {
    let message = match (cmd, &e) {
        (_, DrawingError::UnknownItem(_)) => return AppError::NotFound,
        (_, DrawingError::Finished) => "The raffle is finished.",
        (Command::Draw(_), DrawingError::DrawInProgress) => "A draw is already in progress.",
        (Command::Draw(_), DrawingError::ItemNotAvailable(_)) => {
            "This prize has already been drawn or withdrawn."
        }
        (_, DrawingError::NoEligibleEntrants) => "There are no eligible entrants left.",
        (Command::Decide(Decision::Resolve { .. }), DrawingError::ItemNotDrawn(_)) => {
            "This prize has no undecided winner."
        }
        (Command::Decide(Decision::Resolve { .. }), DrawingError::DrawInProgress) => {
            "Wait for the current draw to finish: “not here” and “declined” could change who it can pick."
        }
        (Command::Decide(Decision::Abandon { .. }), DrawingError::NoDrawInProgress) => {
            "This prize has no draw in progress."
        }
        (Command::Decide(Decision::Abandon { .. }), DrawingError::DrawInProgress) => {
            "The draw is still waiting for its drand round; it can be abandoned if it stalls."
        }
        (Command::Decide(Decision::Void { .. }), DrawingError::ItemNotAvailable(_)) => {
            "Only prizes that have not been won can be withdrawn."
        }
        (_, DrawingError::DrawInProgress) => "Wait for the current draw to finish.",
        _ => return AppError::Conflict(e.to_string()),
    };
    conflict(message)
}

/// Hands a validated command to the drawing engine, which writes the new
/// state and nudges the channel itself, and audits it once the engine took it
/// (applied, or accepted and queued). A command the engine refused, or
/// couldn't be asked, isn't audited: the organizer's retry (same command ID,
/// applied once) is.
async fn send(app: &AppState, owner: &str, id: &str, cmd: Command, command_id: &str) -> Result<()> {
    let engine = engine(app)?;
    let drawing = raffle::drawing::drawing_id(id);
    engine_result(match &cmd {
        Command::Draw(request) => engine.draw(&drawing, request, command_id).await,
        Command::Decide(decision) => engine.decide(&drawing, decision, command_id).await,
    })?;
    let (action, detail) = audited(&cmd);
    audit::record(&app.db, id, owner, action, &detail).await?;
    Ok(())
}

/// How the audit log records a command: the organizer's request as a verb,
/// like every other audit row, and what it was about.
fn audited(cmd: &Command) -> (&'static str, String) {
    match cmd {
        Command::Draw(DrawRequest { item }) => ("draw", item.to_string()),
        Command::Decide(Decision::Resolve { item, resolution }) => {
            let resolution = match resolution {
                Resolution::Claimed => "claimed",
                Resolution::NotHere => "not_here",
                Resolution::Declined => "declined",
            };
            ("resolve", format!("{item} {resolution}"))
        }
        Command::Decide(Decision::Void { item }) => ("void", item.to_string()),
        Command::Decide(Decision::Abandon { item }) => ("abandon", item.to_string()),
        Command::Decide(Decision::Finish) => ("finish", String::new()),
    }
}

// ---------------------------------------------------------------------------
// Entrants
// ---------------------------------------------------------------------------

/// The display name `input` stands for, or the i18n key of why not.
pub fn validate_name(input: &str) -> Result<String, &'static str> {
    entrant::display_name(input).map_err(|e| match e {
        NameError::Invalid => "err_name",
        NameError::Blocked => "err_name_blocked",
    })
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

/// Self-service join from the participant page. Errors are i18n keys.
pub async fn join(
    app: &AppState,
    raffle: &RaffleRow,
    name: &str,
    email: &str,
    rules: bool,
    marketing: bool,
) -> std::result::Result<JoinOutcome, &'static str> {
    if !raffle.state.joinable() {
        return Err("reg_closed");
    }
    let name = validate_name(name)?;
    let email = Email::parse(email).ok_or("err_email")?;
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
            email_raw: &email.raw,
            email_norm: &email.key,
            claim_code: &claim,
            token_hash: &ids::token_hash(&token),
            consent_version: CONSENT_VERSION,
            consent_marketing: marketing,
        },
        app.cfg.max_entrants,
    )
    .await;
    match res {
        Ok(false) => return not_joined(app, raffle, &email.key).await,
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
        Ok(Some(r)) if r.state.joinable()
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
    if !raffle.state.entrants_editable() {
        return Err(conflict("The entrant list is frozen."));
    }
    let name = validate_name(name).map_err(|k| bad(crate::i18n::Lang::En.t(k)))?;
    let email = match email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => Some(Email::parse(e).ok_or_else(|| bad("Invalid email address."))?),
        None => None,
    };
    let id = ids::id();
    let claim = unique_claim_code(app, raffle_id).await?;
    let res = entrants::insert_manual(
        &app.db,
        &entrants::Manual {
            id: &id,
            raffle_id,
            display_name: &name,
            email_raw: email.as_ref().map(|e| e.raw.as_str()),
            email_norm: email.as_ref().map(|e| e.key.as_str()),
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
    if !raffle.state.entrants_editable() {
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
        if !raffle.state.deliverable() {
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
    match raffle.state.removal() {
        Removal::Delete => {
            entrants::delete(&app.db, raffle_id, entrant_id).await?;
            audit::record(&app.db, raffle_id, owner, "remove_entrant", entrant_id).await?;
        }
        Removal::Erase => {
            entrants::erase(&app.db, raffle_id, Some(entrant_id)).await?;
            audit::record(&app.db, raffle_id, owner, "erase_entrant", entrant_id).await?;
        }
    }
    app.notifier.notify(raffle_id).await;
    Ok(())
}

/// Erases every entrant's personal data of a finished raffle now, and ends
/// its live channel.
pub async fn purge(app: &AppState, owner: &str, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if raffle.state.apply(raffle::Command::Purge).is_none() {
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
    if !raffle.state.is_over() {
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
    let prizes = prizes::list(&app.db, id).await?;
    let drawing = raffle.drawing()?;
    let outcomes = drawing
        .as_ref()
        .map(|s| s.entrant_outcomes())
        .unwrap_or_default();
    let item_names: std::collections::HashMap<ItemId, String> = drawing
        .as_ref()
        .map(|s| {
            s.items()
                .iter()
                .map(|item| (item.id.clone(), item.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let entrants = entrants::list(&app.db, id)
        .await?
        .into_iter()
        .map(|e| {
            let outcome = outcomes.get(e.id.as_str());
            AdminEntrant {
                outcome: outcome.map(|o| o.0),
                prize: outcome.and_then(|o| item_names.get(&o.1).cloned()),
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
        prizes: prizes
            .into_iter()
            .map(|i| Prize {
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
    let (Some(manifest), Some(manifest_hash)) =
        (raffle.manifest.clone(), raffle.manifest_hash.clone())
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
        state: raffle.state,
        events: raffle
            .drawing()?
            .map(|s| s.events().to_vec())
            .unwrap_or_default(),
        manifest,
        manifest_hash,
        names,
    })
}
