//! Starting the draw and sending it the organizer's commands.

use rafi_engine::EngineError;
use rafi_lifecycle::{
    self as lifecycle, RaffleState,
    prize::OnClaim,
    start::{self, Start},
};
use rafi_raffle::{
    BeaconSource, Command, Decision, DrawRequest, EntrantId, Manifest, ManifestHash, ManifestItem,
    RaffleId, Resolution,
};
use rafi_store::{audit, entrants, prizes, raffles};

use crate::{Actor, App, Error, Refusal, Result, owned};

/// A call to the raffle engine, as the organizer sees it. A call the engine
/// accepted but hasn't settled yet ([`EngineError::Pending`]) succeeds: it
/// applies once, and the screens follow the projection.
fn confirmed(result: std::result::Result<(), EngineError>) -> Result<()> {
    match result {
        Ok(()) | Err(EngineError::Pending) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Freezes the entrant list (closing registration, if open), writes the raffle's
/// manifest from it, and hands that to the raffle engine. A raffle without
/// registration freezes from Published, after the checks opening would have
/// made (a named controller and contact, the erasure date not passed); if
/// nobody is left to draw from, it goes back there.
///
/// Each step is its own write ([`start`]), so calling it again picks up
/// where an attempt stopped: a raffle left frozen without a manifest gets
/// one built from its now-fixed entrant list; one with a manifest the engine
/// hasn't opened is sent it again (opening is idempotent), which recovers
/// from an engine outage at freeze time.
pub async fn start_draw(app: &App, actor: Actor<'_>, id: &str) -> Result<()> {
    let raffle = owned(app, actor.owner, id).await?;
    let now = app.clock.now();
    let engine = app.engine()?;
    let info = engine.info().await?;
    // The app decides the beacon source; the engine would refuse another
    // when the raffle opens, after the manifest is stored. Refuse before
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
        raffle.registration,
        raffle.manifest.is_some(),
        raffle.opened(),
    )
    .map_err(Refusal::Start)?;
    let manifest = match next {
        Start::Freeze { from } => {
            // From Published, no opening checked the notice and the date.
            if from == RaffleState::Ready {
                crate::raffles::ready_for_entrants(&raffle, now)?;
            }
            // Checked before anything is frozen, for the organizer's
            // refusal; the manifest reads the prizes again once they are
            // locked (`write_manifest`).
            let items = items(app, id).await?;
            let entrants = entrants::count_active(&app.db, id).await?;
            start::ready(usize::try_from(entrants).unwrap_or(0), items.len())
                .map_err(Refusal::Start)?;
            let fakes = entrants::count_fakes(&app.db, id).await?;
            start::genuine(raffle.test_mode, usize::try_from(fakes).unwrap_or(0))
                .map_err(Refusal::Start)?;
            // Close registration first so no join can slip in while we read
            // the list; guarded on the setting the freeze was decided with.
            if !raffles::freeze(&app.db, id, from, raffle.registration, now).await? {
                return Err(Refusal::Changed.into());
            }
            app.live_updates.notify(id).await;
            write_manifest(app, actor, id, from, now).await?
        }
        Start::WriteManifest { unfreeze_to } => {
            write_manifest(app, actor, id, unfreeze_to, now).await?
        }
        Start::Resend => {
            let json = raffle.manifest.as_deref().unwrap_or_default();
            serde_json::from_str(json).map_err(anyhow::Error::from)?
        }
        Start::Started => return Ok(()),
    };

    // The engine writes the opened raffle and nudges the channel itself.
    confirmed(engine.open(&manifest).await)
}

/// The raffle's items for its prizes (one per copy), in draw order.
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

/// Writes the manifest of a frozen raffle `now`: builds it from its entrants
/// and prizes and stores it with its hash.
///
/// The store has no interactive transactions, so freezing and writing the
/// manifest are separate writes ([`rafi_store::raffles`]); this is the second,
/// and is safe to repeat. Both lists are read here, after the freeze, which
/// locks them: a prize edit that landed while the freeze was being decided
/// is in the manifest, like the join that did. If another attempt wrote one
/// first, that one is returned. If nobody is left to draw from (everyone was
/// removed while freezing), or nothing (every prize was deleted), the raffle
/// goes back to `unfreeze_to`.
async fn write_manifest(
    app: &App,
    actor: Actor<'_>,
    id: &str,
    unfreeze_to: RaffleState,
    now: i64,
) -> Result<Manifest> {
    let items = items(app, id).await?;
    let entrant_ids = entrants::active_ids(&app.db, id).await?;
    if let Err(refusal) = start::ready(entrant_ids.len(), items.len()) {
        let to = RaffleState::Frozen
            .unfreeze(unfreeze_to)
            .ok_or_else(|| anyhow::anyhow!("cannot unfreeze to {}", unfreeze_to.as_str()))?;
        raffles::unfreeze(&app.db, id, to, now).await?;
        app.live_updates.notify(id).await;
        return Err(Refusal::Start(refusal).into());
    }
    // The app alone decides the chain; the raffle engine refuses a mismatch.
    let manifest =
        lifecycle::raffle::manifest(id, BeaconSource::Drand(app.chain), entrant_ids, items);
    let json = manifest.canonical_json();
    let hash = ManifestHash::of(&json).to_string();
    if !raffles::set_manifest(&app.db, id, &json, &hash, now).await? {
        // Another attempt wrote one first (or the raffle moved on).
        let stored = raffles::get(&app.db, id)
            .await?
            .and_then(|r| r.manifest)
            .ok_or(Refusal::Changed)?;
        return Ok(serde_json::from_str(&stored).map_err(anyhow::Error::from)?);
    }
    audit::record(&app.db, id, actor, "freeze", &hash, now).await?;
    Ok(manifest)
}

/// Sends a draw command to the raffle's engine.
pub async fn command(
    app: &App,
    actor: Actor<'_>,
    id: &str,
    cmd: impl Into<Command>,
    command_id: &str,
) -> Result<()> {
    let cmd = cmd.into();
    let raffle = owned(app, actor.owner, id).await?;
    if !raffle.state.drawable() {
        return Err(Refusal::NotDrawing.into());
    }
    let state = raffles::drawn(&app.db, &raffle)
        .await?
        .ok_or(Refusal::NotOpened)?;

    // Checked against the projection for immediate, friendly errors: a draw
    // the engine refused would go unnoticed.
    state.admit(&cmd).map_err(|error| match error {
        rafi_raffle::Error::UnknownItem(_) => Error::NotFound,
        error => Refusal::Raffle {
            command: Some(cmd.clone()),
            error,
        }
        .into(),
    })?;

    objected_winner(app, id, &state, &cmd).await?;
    let delivery = delivery_on_claim(app, id, &state, &cmd).await?;
    send(app, actor, id, cmd, command_id).await?;
    // The store changes the winner's delivery with the claim, as it projects
    // it; the log says that the organizer's claim did.
    if let Some((action, winner)) = delivery {
        let now = app.clock.now();
        audit::record(&app.db, id, actor, action, winner.as_str(), now).await?;
    }
    Ok(())
}

/// Refuses `cmd` if it resolves a draw whose winner took themselves off
/// ([`object_entrant`](crate::object_entrant)) any other way than declined
/// ([`objected_may_be`](lifecycle::entrant::objected_may_be)): they said
/// they don't take part, so they claim nothing, and "Not here" would
/// record an absence. Checked before the engine is asked, against the
/// projection (`state`, which admitted it) and the entrant as stored.
async fn objected_winner(
    app: &App,
    id: &str,
    state: &rafi_raffle::Raffle,
    cmd: &Command,
) -> Result<()> {
    let Command::Decide(Decision::Resolve {
        item, resolution, ..
    }) = cmd
    else {
        return Ok(());
    };
    if lifecycle::entrant::objected_may_be(*resolution) {
        return Ok(());
    }
    let Some(winner) = state.item(item).and_then(|i| i.winner().cloned()) else {
        return Ok(());
    };
    let objected = entrants::get(&app.db, id, winner.as_str())
        .await?
        .is_some_and(|e| e.objected_at.is_some());
    if objected {
        Err(Refusal::ObjectedWinner.into())
    } else {
        Ok(())
    }
}

/// What `cmd` does to a winner's delivery, if it is a claim that changes it
/// ([`lifecycle::raffle::on_claim`]): `deliver` for a prize handed over on
/// the spot, `undeliver` for one sent later to a winner marked delivered
/// (with more items than people, one can claim two), and the winner, as
/// `state` (the projection, which admitted it) has the item.
async fn delivery_on_claim(
    app: &App,
    id: &str,
    state: &rafi_raffle::Raffle,
    cmd: &Command,
) -> Result<Option<(&'static str, EntrantId)>> {
    let Command::Decide(Decision::Resolve {
        item,
        resolution: Resolution::Claimed,
        ..
    }) = cmd
    else {
        return Ok(None);
    };
    let Some((k, winner)) = state
        .item(item)
        .and_then(|i| Some((i.k()?, i.winner()?.clone())))
    else {
        return Ok(None);
    };
    let deliveries = prizes::deliveries(&app.db, id).await?;
    let claimed_as = prizes::claimed_deliveries(&app.db, id).await?;
    let delivered = || async {
        Ok::<_, Error>(
            entrants::get(&app.db, id, winner.as_str())
                .await?
                .is_some_and(|e| e.delivered_at.is_some() && !e.erased),
        )
    };
    let effect = lifecycle::raffle::on_claim(
        state,
        k,
        |p| deliveries.get(p).copied(),
        |k| claimed_as.get(&k).copied(),
    );
    let action = match effect {
        OnClaim::Deliver if !delivered().await? => "deliver",
        OnClaim::Undeliver if delivered().await? => "undeliver",
        _ => return Ok(None),
    };
    Ok(Some((action, winner)))
}

/// Hands a validated command to the raffle engine, which writes the new
/// state and nudges the channel itself, and audits it once the engine took it
/// (applied, or accepted and queued). A command the engine refused, or
/// couldn't be asked, isn't audited: the organizer's retry (same command ID,
/// applied once) is.
async fn send(app: &App, actor: Actor<'_>, id: &str, cmd: Command, command_id: &str) -> Result<()> {
    let engine = app.engine()?;
    let raffle = RaffleId::new(id);
    confirmed(match &cmd {
        Command::Draw(request) => engine.draw(&raffle, request, command_id).await,
        Command::Decide(decision) => engine.decide(&raffle, decision, command_id).await,
    })?;
    let (action, detail) = audited(&cmd);
    audit::record(&app.db, id, actor, action, &detail, app.clock.now()).await?;
    Ok(())
}

/// How the audit log records a command: the organizer's request as a verb,
/// like every other audit row, and what it was about.
fn audited(cmd: &Command) -> (&'static str, String) {
    match cmd {
        Command::Draw(DrawRequest { item }) => ("draw", item.to_string()),
        Command::Decide(Decision::Resolve {
            item, resolution, ..
        }) => {
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
