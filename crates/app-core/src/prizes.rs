//! A raffle's prizes, while they can change (until the freeze).

use rafi_lifecycle::settings;
use rafi_model::PrizeInput;
use rafi_store::prizes;

use crate::{App, Error, Refusal, Result, ids, owned};

/// Adds a prize, or changes `input.id`; returns its ID.
pub async fn upsert_prize(
    app: &App,
    owner: &str,
    raffle_id: &str,
    input: PrizeInput,
) -> Result<String> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state.prizes_editable() {
        return Err(Refusal::PrizesFrozen.into());
    }
    let name = settings::title(&input.name).ok_or(Refusal::PrizeName)?;
    if !settings::prize_qty(input.qty) {
        return Err(Refusal::PrizeQty.into());
    }
    let fields = prizes::PrizeFields {
        name,
        sponsor: input.sponsor.trim(),
        description: input.description.trim(),
        qty: input.qty,
        position: input.position,
    };
    // The store writes only while prizes can change: a freeze that landed
    // since the read above refuses it.
    let id = match &input.id {
        Some(id) => {
            if !prizes::update(&app.db, id, raffle_id, &fields).await? {
                return Err(frozen_or_gone(app, raffle_id).await);
            }
            id.clone()
        }
        None => {
            let id = ids::code(6).to_lowercase();
            if !prizes::insert(&app.db, &id, raffle_id, &fields).await? {
                return Err(Refusal::PrizesFrozen.into());
            }
            id
        }
    };
    // Before the freeze, phones count the prizes from their quantities.
    app.notifier.notify(raffle_id).await;
    Ok(id)
}

pub async fn delete_prize(app: &App, owner: &str, raffle_id: &str, prize_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state.prizes_editable() {
        return Err(Refusal::PrizesFrozen.into());
    }
    if !prizes::delete(&app.db, prize_id, raffle_id).await? {
        // Already gone is fine; frozen meanwhile is not.
        if let Error::Refused(r) = frozen_or_gone(app, raffle_id).await {
            return Err(r.into());
        }
    }
    app.notifier.notify(raffle_id).await;
    Ok(())
}

/// Why a guarded prize write changed nothing: the prizes froze meanwhile,
/// or there is no such prize.
async fn frozen_or_gone(app: &App, raffle_id: &str) -> Error {
    match rafi_store::raffles::get(&app.db, raffle_id).await {
        Ok(Some(r)) if !r.state.prizes_editable() => Refusal::PrizesFrozen.into(),
        Ok(_) => Error::NotFound,
        Err(e) => e.into(),
    }
}
