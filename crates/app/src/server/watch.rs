//! Picks up the draw states the `restate-rafi` service writes into the
//! database and pushes them to the screens.
//!
//! The service is a separate process, so nothing in the app hears about its
//! writes. It only writes when the app has asked Restate to do something, so
//! the app watches a raffle from the moment it sends a command ([`Watch::wake`])
//! until things settle: while a draw is pending, and for a while after the last
//! change. With nothing to watch, nothing is queried.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::Notify;

use super::{AppState, db};

const INTERVAL: Duration = Duration::from_millis(250);
/// How long a raffle stays watched after its last command or change.
const SETTLE: Duration = Duration::from_secs(30);

/// The raffles being watched, and when each last saw activity.
#[derive(Clone, Default)]
pub struct Watch {
    active: Arc<Mutex<HashMap<String, Instant>>>,
    wake: Arc<Notify>,
}

impl Watch {
    /// Starts (or extends) watching a raffle. Call before asking Restate to change it.
    pub fn wake(&self, raffle_id: &str) {
        self.active
            .lock()
            .expect("watch lock")
            .insert(raffle_id.to_owned(), Instant::now());
        self.wake.notify_one();
    }

    pub fn is_idle(&self) -> bool {
        self.active.lock().expect("watch lock").is_empty()
    }

    fn active(&self) -> Vec<(String, Instant)> {
        let active = self.active.lock().expect("watch lock");
        active.iter().map(|(id, at)| (id.clone(), *at)).collect()
    }

    fn touch(&self, raffle_id: &str) {
        if let Some(at) = self.active.lock().expect("watch lock").get_mut(raffle_id) {
            *at = Instant::now();
        }
    }

    /// Stops watching, unless it was woken again after `since`.
    fn settle(&self, raffle_id: &str, since: Instant) {
        let mut active = self.active.lock().expect("watch lock");
        if active.get(raffle_id).is_some_and(|at| *at <= since) {
            active.remove(raffle_id);
        }
    }
}

pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        // Draws Restate may still be finishing from before a restart.
        match unsettled(&app).await {
            Ok(ids) => ids.iter().for_each(|id| app.watch.wake(id)),
            Err(e) => tracing::warn!(error = %e, "looking for unfinished Restate draws failed"),
        }
        let mut watcher = Watcher::default();
        loop {
            if app.watch.is_idle() {
                app.watch.wake.notified().await;
            }
            if let Err(e) = watcher.poll(&app).await {
                tracing::warn!(error = %e, "checking for new draw states failed");
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

/// Restate raffles with a draw in flight.
async fn unsettled(app: &AppState) -> anyhow::Result<Vec<String>> {
    let rows: Vec<db::RaffleRow> = sqlx::query_as(
        "SELECT * FROM raffles WHERE engine = 'restate' AND state IN ('frozen', 'drawing')",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|r| r.draw_state.is_none() || r.draw_state().is_some_and(|s| s.pending.is_some()))
        .map(|r| r.id)
        .collect())
}

/// The draw versions last seen of the watched raffles.
#[derive(Default)]
pub struct Watcher {
    seen: HashMap<String, i64>,
}

impl Watcher {
    /// One pass over the watched raffles: refreshes those whose draw state
    /// changed since the last pass and returns their IDs; stops watching those
    /// that have settled.
    pub async fn poll(&mut self, app: &AppState) -> anyhow::Result<Vec<String>> {
        let mut changed = Vec::new();
        for (id, at) in app.watch.active() {
            let raffle = db::raffle(&app.db, &id).await?;
            let Some(raffle) = raffle.filter(|r| r.engine == "restate") else {
                app.watch.settle(&id, at);
                self.seen.remove(&id);
                continue;
            };
            if self.seen.insert(id.clone(), raffle.draw_version) != Some(raffle.draw_version) {
                app.watch.touch(&id);
                changed.push(id);
                continue;
            }
            let pending = raffle.draw_state().is_some_and(|s| s.pending.is_some());
            if !pending && at.elapsed() >= SETTLE {
                app.watch.settle(&id, at);
                self.seen.remove(&id);
            }
        }
        if !changed.is_empty() {
            app.schedule_purges().await?;
            for id in &changed {
                app.refresh(id).await;
            }
        }
        Ok(changed)
    }
}
