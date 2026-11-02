//! The rest: live channels ended once, proof-of-work replays.

use super::*;

pub(in crate::conformance) async fn live_channels_end_once(db: &Db) -> Outcome {
    // A raffle that finished at 100, and one that hasn't finished.
    raffle_in(db, "r1", RaffleState::Draft).await?;
    db.query("UPDATE raffles SET state = 'finished', finished_at = 100 WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    raffle_in(db, "r2", RaffleState::Open).await?;

    // Live channels end a while after the raffle finished, once.
    check(
        housekeeping::live_channels_due(db, 99)
            .await
            .map_err(fail)?
            .is_empty(),
        "no channel to end yet",
    )?;
    check(
        housekeeping::live_channels_due(db, 100)
            .await
            .map_err(fail)?
            == ["r1"],
        "the finished raffle's channel, not the unfinished one's",
    )?;
    housekeeping::live_ended(db, "r1", 200)
        .await
        .map_err(fail)?;
    housekeeping::live_ended(db, "r1", 300)
        .await
        .map_err(fail)?;
    check(
        housekeeping::live_channels_due(db, 1000)
            .await
            .map_err(fail)?
            .is_empty()
            && raffle(db, "r1").await?.live_ended_at == Some(200),
        "ended once",
    )
}

pub(in crate::conformance) async fn pow_refuses_replays(db: &Db) -> Outcome {
    let redeem = |challenge, expires_at| pow::redeem(db, challenge, expires_at);
    check(
        redeem("c1", 100).await.map_err(fail)?,
        "the first redemption",
    )?;
    check(!redeem("c1", 100).await.map_err(fail)?, "a replay")?;
    check(redeem("c2", 50).await.map_err(fail)?, "another challenge")?;
    pow::purge_expired(db, 60).await.map_err(fail)?;
    check(
        !redeem("c1", 100).await.map_err(fail)?,
        "an unexpired challenge stays redeemed",
    )?;
    check(
        redeem("c2", 50).await.map_err(fail)?,
        "an expired one is forgotten",
    )
}
