//! Scheduled erasure of personal data.
//!
//! After a raffle finishes, non-winners are erased after
//! `RAFI_RETENTION_DAYS` (30), winners once their prize is marked delivered or
//! after `RAFI_RETENTION_WINNERS_DAYS` (90). Entrant IDs and the draw log stay,
//! so the published draw remains verifiable.

use std::{collections::HashSet, time::Duration};

use super::{AppState, db, ids, service};

pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match run(&app).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(erased = n, "retention: erased personal data"),
                Err(e) => tracing::error!(error = %e, "retention run failed"),
            }
        }
    });
}

/// One retention pass. Returns the number of entrants erased.
pub async fn run(app: &AppState) -> anyhow::Result<u64> {
    // Raffles finished by Restate while the app was not watching get their dates here.
    app.schedule_purges().await?;
    let now = ids::now();
    let raffles: Vec<db::RaffleRow> = app
        .db
        .query(
            "SELECT * FROM raffles WHERE state IN ('finished', 'purged') AND finished_at IS NOT NULL AND purged_at IS NULL",
        )
        .all()
        .await?;
    let mut erased = 0;
    for raffle in raffles {
        let winners: HashSet<String> = raffle
            .draw_state()
            .map(|s| {
                s.entrant_outcomes()
                    .into_iter()
                    .filter(|(_, (o, _))| *o == rafi_core::state::Outcome::Claimed)
                    .map(|(id, _)| id)
                    .collect()
            })
            .unwrap_or_default();
        let nonwinners_due = raffle.purge_nonwinners_at.is_some_and(|t| t <= now);
        let winners_due = raffle.purge_winners_at.is_some_and(|t| t <= now);
        for e in db::entrants(&app.db, &raffle.id).await? {
            if e.purged {
                continue;
            }
            let is_winner = winners.contains(&e.id);
            let due = if is_winner {
                winners_due || (e.delivered_at.is_some() && nonwinners_due)
            } else {
                nonwinners_due
            };
            if due && service::erase(app, &raffle.id, Some(&e.id)).await? {
                erased += 1;
            }
        }
        if winners_due {
            app.db
                .query("UPDATE raffles SET state = 'purged', purged_at = ? WHERE id = ?")
                .bind(now)
                .bind(&raffle.id)
                .run()
                .await?;
        }
        if erased > 0 {
            app.refresh(&raffle.id).await;
        }
    }
    Ok(erased)
}
