//! A raffle's prizes, while they can change (until the freeze), and how
//! each reaches its winner (until the raffle is over).

use rafi_api::PrizeInput;
use rafi_lifecycle::{prize::Delivery, settings, start};
use rafi_store::{RaffleRow, audit, prizes};

use crate::{Actor, App, Error, Refusal, Result, ids, owned, unwritten};

/// Adds a prize, or changes `input.id`; returns its ID.
pub async fn upsert_prize(
    app: &App,
    actor: Actor<'_>,
    raffle_id: &str,
    input: PrizeInput,
) -> Result<String> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    if !raffle.state.prizes_editable() {
        return Err(Refusal::PrizesFrozen.into());
    }
    let name = settings::title(&input.name).ok_or(Refusal::PrizeName)?;
    if !settings::prize_qty(input.qty) {
        return Err(Refusal::PrizeQty.into());
    }
    // The raffle's items, with this prize as it would be: within the cap
    // (which the freeze checks again).
    let others: u32 = prizes::list(&app.db, raffle_id)
        .await?
        .iter()
        .filter(|p| Some(&p.id) != input.id.as_ref())
        .map(|p| u32::try_from(p.qty).unwrap_or(0))
        .sum();
    if others.saturating_add(input.qty) > settings::MAX_ITEMS {
        return Err(Refusal::Start(start::Refusal::TooManyItems).into());
    }
    let fields = prizes::PrizeFields {
        name,
        sponsor: input.sponsor.trim(),
        description: input.description.trim(),
        qty: input.qty,
        position: input.position,
        delivery: input.delivery,
    };
    // The store writes only while prizes can change: a freeze that landed
    // since the read above refuses it.
    let id = match &input.id {
        Some(id) => {
            if !prizes::update(&app.db, id, raffle_id, &fields).await? {
                return Err(unwritten(app, raffle_id, frozen, Error::NotFound).await);
            }
            id.clone()
        }
        None => {
            let id = ids::id();
            if !prizes::insert(&app.db, &id, raffle_id, &fields).await? {
                return Err(Refusal::PrizesFrozen.into());
            }
            id
        }
    };
    // Before the freeze, phones count the prizes from their quantities.
    app.live_updates.notify(raffle_id).await;
    Ok(id)
}

pub async fn delete_prize(
    app: &App,
    actor: Actor<'_>,
    raffle_id: &str,
    prize_id: &str,
) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    if !raffle.state.prizes_editable() {
        return Err(Refusal::PrizesFrozen.into());
    }
    if !prizes::delete(&app.db, prize_id, raffle_id).await? {
        // Already gone is fine; frozen meanwhile is not.
        if let Error::Refused(r) = unwritten(app, raffle_id, frozen, Error::NotFound).await {
            return Err(r.into());
        }
    }
    app.live_updates.notify(raffle_id).await;
    Ok(())
}

/// Changes how a prize reaches its winner, which it may until the raffle is
/// over, past the freeze too. It applies to the claims made from then on:
/// claiming an item of a prize handed over on the spot marks its winner's
/// prize delivered (the raffle's store does, with the claim). Audited, as
/// it decides which winners' data goes on the non-winners' clock.
pub async fn set_prize_delivery(
    app: &App,
    actor: Actor<'_>,
    raffle_id: &str,
    prize_id: &str,
    delivery: Delivery,
) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    if !raffle.state.prize_delivery_editable() {
        return Err(Refusal::PrizeDeliveryFixed.into());
    }
    if !prizes::set_delivery(&app.db, prize_id, raffle_id, delivery).await? {
        // Over since the read above, or no such prize.
        let fixed = |r: &RaffleRow| {
            (!r.state.prize_delivery_editable()).then_some(Refusal::PrizeDeliveryFixed)
        };
        return Err(unwritten(app, raffle_id, fixed, Error::NotFound).await);
    }
    let detail = format!("{prize_id} {}", delivery.as_str());
    let now = app.clock.now();
    audit::record(
        &app.db,
        raffle_id,
        actor,
        audit::Action::SetPrizeDelivery,
        &detail,
        now,
    )
    .await?;
    // A winner's phone says whether the prize is still to be sent.
    app.live_updates.notify(raffle_id).await;
    Ok(())
}

/// Why a prize write was refused, if the raffle's state is why: its prizes
/// froze.
fn frozen(r: &RaffleRow) -> Option<Refusal> {
    (!r.state.prizes_editable()).then_some(Refusal::PrizesFrozen)
}
