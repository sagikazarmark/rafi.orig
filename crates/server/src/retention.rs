//! The native schedule of the retention run
//! ([`rafi_app::retention`]): hourly, in a task. On Workers the Cron
//! Trigger runs it.

use std::time::Duration;

use super::{AppState, service::retention};

/// Runs [`retention::run`] every hour.
pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match retention::run(&app).await {
                Ok(r) if r == retention::Report::default() => {}
                Ok(r) => tracing::info!(
                    erased = r.erased,
                    channels_ended = r.channels_ended,
                    "retention run"
                ),
                Err(e) => tracing::error!(error = %e, "retention run failed"),
            }
        }
    });
}
