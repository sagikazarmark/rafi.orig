//! The native schedule of the housekeeping run
//! ([`rafi_app::housekeeping`]): hourly, in a task. On Workers the Cron
//! Trigger runs it.

use std::time::Duration;

use super::{AppState, service::housekeeping};

/// Runs [`housekeeping::run`] every hour.
pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match housekeeping::run(&app).await {
                Ok(r) if r == housekeeping::Report::default() => {}
                Ok(r) => tracing::info!(channels_ended = r.channels_ended, "housekeeping run"),
                Err(e) => tracing::error!(error = %e, "housekeeping run failed"),
            }
        }
    });
}
