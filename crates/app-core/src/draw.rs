//! Starting the draw and sending it the organizer's commands.

use rafi_engine::EngineError;
use rafi_lifecycle::{
    self as lifecycle, RaffleState,
    start::{self, Start},
};
use rafi_raffle::{
    BeaconSource, Command, Decision, DrawRequest, Manifest, ManifestHash, ManifestItem, RaffleId,
    Resolution,
};
use rafi_store::{audit, entrants, prizes, raffles};

use crate::{App, Error, Refusal, Result, owned};

/// A call to the drawing engine, as the organizer sees it. A call the engine
/// accepted but hasn't settled yet ([`EngineError::Pending`]) succeeds: it
/// applies once, and the screens follow the projection.
fn confirmed(result: std::result::Result<(), EngineError>) -> Result<()> {
    match result {
        Ok(()) | Err(EngineError::Pending) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Closes registration, freezes the entrant list, writes the drawing's
/// manifest from it, and hands that to the drawing engine.
///
/// Each step is its own write ([`start`]), so calling it again picks up
/// where an attempt stopped: a raffle left frozen without a manifest gets
/// one built from its now-fixed entrant list; one with a manifest the engine
/// hasn't opened is sent it again (opening is idempotent), which recovers
/// from an engine outage at freeze time.
pub async fn start_draw(app: &App, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let engine = app.engine()?;
    let info = engine.info().await?;
    // The app decides the beacon source; the engine would refuse another
    // when the drawing opens, after the manifest is stored. Refuse before
    // anything is frozen.
    let beacon = BeaconSource::Drand(app.chain);
    if info.beacon != beacon {
        return Err(Refusal::BeaconMismatch {
            engine: info.beacon,
            raffles: beacon,
        }
        .into());
    }

    let next = start::next(
        raffle.state,
        raffle.manifest.is_some(),
        raffle.log.is_some(),
    )
    .map_err(Refusal::Start)?;
    let manifest = match next {
        Start::Freeze { from } => {
            let items = items(app, id).await?;
            let entrants = entrants::count_active(&app.db, id).await?;
            start::ready(usize::try_from(entrants).unwrap_or(0), items.len())
                .map_err(Refusal::Start)?;
            // Close registration first so no join can slip in while we read the list.
            if !raffles::freeze(&app.db, id, from).await? {
                return Err(Refusal::Changed.into());
            }
            app.notifier.notify(id).await;
            write_manifest(app, owner, id, items, from).await?
        }
        Start::WriteManifest { unfreeze_to } => {
            write_manifest(app, owner, id, items(app, id).await?, unfreeze_to).await?
        }
        Start::Resend => {
            let json = raffle.manifest.as_deref().unwrap_or_default();
            serde_json::from_str(json).map_err(anyhow::Error::from)?
        }
        Start::Started => return Ok(()),
    };

    // The engine writes the opened drawing and nudges the channel itself.
    confirmed(engine.open(&manifest).await)
}

/// The drawing's items for a raffle's prizes (one per copy), in draw order.
async fn items(app: &App, id: &str) -> Result<Vec<ManifestItem>> {
    let prizes = prizes::list(&app.db, id).await?;
    Ok(lifecycle::raffle::items(prizes.iter().map(|i| {
        (
            i.id.as_str(),
            i.name.as_str(),
            u64::try_from(i.qty).unwrap_or(0),
        )
    })))
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
    app: &App,
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
        return Err(Refusal::Start(refusal).into());
    }
    // The app alone decides the chain; the drawing engine refuses a mismatch.
    let manifest =
        lifecycle::raffle::manifest(id, BeaconSource::Drand(app.chain), entrant_ids, items);
    let json = manifest.canonical_json();
    let hash = ManifestHash::of(&json).to_string();
    if !raffles::set_manifest(&app.db, id, &json, &hash).await? {
        // Another attempt wrote one first (or the raffle moved on).
        let stored = raffles::get(&app.db, id)
            .await?
            .and_then(|r| r.manifest)
            .ok_or(Refusal::Changed)?;
        return Ok(serde_json::from_str(&stored).map_err(anyhow::Error::from)?);
    }
    audit::record(&app.db, id, owner, "freeze", &hash).await?;
    Ok(manifest)
}

/// Sends a draw command to the raffle's engine.
pub async fn command(
    app: &App,
    owner: &str,
    id: &str,
    cmd: impl Into<Command>,
    command_id: &str,
) -> Result<()> {
    let cmd = cmd.into();
    let raffle = owned(app, owner, id).await?;
    if !raffle.state.drawable() {
        return Err(Refusal::NotDrawing.into());
    }
    let state = raffle.drawn()?.ok_or(Refusal::NotOpened)?;

    // Checked against the projection for immediate, friendly errors: a draw
    // the engine refused would go unnoticed.
    state.admit(&cmd).map_err(|error| match error {
        rafi_raffle::Error::UnknownItem(_) => Error::NotFound,
        error => Refusal::Drawing {
            command: Some(cmd.clone()),
            error,
        }
        .into(),
    })?;

    send(app, owner, id, cmd, command_id).await
}

/// Hands a validated command to the drawing engine, which writes the new
/// state and nudges the channel itself, and audits it once the engine took it
/// (applied, or accepted and queued). A command the engine refused, or
/// couldn't be asked, isn't audited: the organizer's retry (same command ID,
/// applied once) is.
async fn send(app: &App, owner: &str, id: &str, cmd: Command, command_id: &str) -> Result<()> {
    let engine = app.engine()?;
    let raffle = RaffleId::new(id);
    confirmed(match &cmd {
        Command::Draw(request) => engine.draw(&raffle, request, command_id).await,
        Command::Decide(decision) => engine.decide(&raffle, decision, command_id).await,
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
