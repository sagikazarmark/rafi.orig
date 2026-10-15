//! What anyone may read of a public raffle: the participant page's reads
//! (the raffle by its join code, the device's own entry, what phones see),
//! the prizes the rules list, and how many raffles are live.

use rafi_live_client::{Follow, ParticipantView};
use rafi_store::{EntrantRow, PrizeRow, RaffleRow, entrants, live::Snapshot, prizes, raffles};

use crate::{App, Error, Result, codes, ids, view};

/// Raffle `slug`; not found unless the public may see it.
pub async fn raffle(app: &App, slug: &str) -> Result<RaffleRow> {
    public(raffles::by_slug(&app.db, slug).await?)
}

/// The raffle joined with `code`, as typed (it is normalized); not found
/// unless the public may see it.
pub async fn raffle_by_join_code(app: &App, code: &str) -> Result<RaffleRow> {
    public(raffles::by_join_code(&app.db, &codes::normalize(code)).await?)
}

fn public(raffle: Option<RaffleRow>) -> Result<RaffleRow> {
    raffle
        .filter(|r| r.state.is_public())
        .ok_or(Error::NotFound)
}

/// The entrant of raffle `raffle_id` holding the participant cookie `token`.
pub async fn entrant(app: &App, raffle_id: &str, token: &str) -> Result<Option<EntrantRow>> {
    Ok(entrants::by_token(&app.db, raffle_id, &ids::token_hash(token)).await?)
}

/// What phones see of a raffle, and how they follow it.
#[derive(Debug, Clone)]
pub struct Participant {
    pub view: ParticipantView,
    pub follow: Follow,
}

/// What phones see of raffle `raffle_id` now, read in one snapshot; `None`
/// if it's gone.
pub async fn participant(app: &App, raffle_id: &str) -> Result<Option<Participant>> {
    let Some(snapshot) = Snapshot::read(&app.db, raffle_id).await? else {
        return Ok(None);
    };
    Ok(Some(Participant {
        view: view::participant(&snapshot),
        follow: app.follow(raffle_id, !snapshot.channel_open()),
    }))
}

/// Raffle `raffle_id`'s prizes, in order (the rules list them).
pub async fn prizes(app: &App, raffle_id: &str) -> Result<Vec<PrizeRow>> {
    Ok(prizes::list(&app.db, raffle_id).await?)
}

/// How many raffles are live right now. The deploy pipeline refuses to
/// restart the server while there are any.
pub async fn live_count(app: &App) -> Result<i64> {
    Ok(raffles::live_count(&app.db).await?)
}
