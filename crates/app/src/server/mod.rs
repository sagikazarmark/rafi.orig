//! Server side: database, live hub, draw engines, attendee pages, public and admin APIs.

pub mod admin;
pub mod altcha;
pub mod attendee;
pub mod auth;
pub mod config;
pub mod db;

pub mod engine;
pub mod hub;
pub mod ids;
pub mod public;
pub mod ratelimit;
pub mod retention;
pub mod service;
pub mod view;
pub mod watch;

use std::{ops::Deref, path::Path, sync::Arc};

use anyhow::Context as _;
use axum::{Extension, Router};
use rafi_core::state::DrawState;
use rafi_drand::DrandClient;

use self::{
    config::{Config, EngineKind},
    db::Db,
    engine::{LocalEngine, RestateClient},
    hub::Hub,
};

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    pub hub: Hub,
    pub drand: DrandClient,
    pub local: LocalEngine,
    pub restate: std::sync::OnceLock<RestateClient>,
    pub watch: watch::Watch,
    pub limiter: ratelimit::Limiter,
    pub altcha: altcha::Altcha,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub async fn new(cfg: Config) -> anyhow::Result<Self> {
        let db = db::connect(&cfg.database_url)
            .await
            .context("opening database")?;
        let restate = std::sync::OnceLock::new();
        if cfg.engine == EngineKind::Restate && !cfg.restate.ingress_url.is_empty() {
            let _ = restate.set(RestateClient::new(cfg.restate.clone()));
        }
        Ok(Self(Arc::new(Inner {
            drand: DrandClient::new(cfg.drand.clone()).context("drand configuration")?,
            altcha: altcha::Altcha::new(cfg.altcha_key.clone(), cfg.altcha_max),
            limiter: ratelimit::Limiter::default(),
            local: LocalEngine::default(),
            hub: Hub::default(),
            watch: watch::Watch::default(),
            restate,
            db,
            cfg,
        })))
    }

    /// Stores a state of the local draw engine (idempotent, keeps the highest
    /// version) and pushes the new view to every screen. The Restate engine
    /// writes its states itself; [`watch`] pushes those.
    pub async fn project(&self, state: &DrawState) -> anyhow::Result<()> {
        rafi_store::save_draw_state(&self.db, state, self.purge_after()).await?;
        self.schedule_purges().await?;
        self.refresh(&state.raffle_id).await;
        Ok(())
    }

    /// When personal data of a finished raffle is erased (the retention settings).
    pub fn purge_after(&self) -> rafi_store::PurgeAfter {
        rafi_store::PurgeAfter::days(
            self.cfg.retention_nonwinners_days,
            self.cfg.retention_winners_days,
        )
    }

    /// Dates the erasure of personal data for raffles that just finished,
    /// whichever engine finished them.
    pub async fn schedule_purges(&self) -> anyhow::Result<()> {
        let day = 86_400;
        self.db
            .query(
                "UPDATE raffles SET
                    purge_nonwinners_at = finished_at + ?1,
                    purge_winners_at = finished_at + ?2
                 WHERE finished_at IS NOT NULL AND purge_nonwinners_at IS NULL",
            )
            .bind(self.cfg.retention_nonwinners_days * day)
            .bind(self.cfg.retention_winners_days * day)
            .run()
            .await?;
        Ok(())
    }

    /// Rebuilds the public view of a raffle and publishes it.
    pub async fn refresh(&self, raffle_id: &str) {
        match view::build(self, raffle_id).await {
            Ok(Some(v)) => self.hub.publish(v),
            Ok(None) => {}
            Err(e) => tracing::error!(raffle_id, error = %e, "building view failed"),
        }
    }

    /// Republishes a raffle's view a bit later (recent names appear with a delay).
    pub fn refresh_later(&self, raffle_id: &str, after: std::time::Duration) {
        let (app, id) = (self.clone(), raffle_id.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            app.refresh(&id).await;
        });
    }
}

/// The server's own routes: attendee pages, SSE, public JSON and the admin API.
pub fn routes(app: AppState) -> Router {
    Router::new()
        .merge(attendee::routes())
        .merge(public::routes())
        .merge(admin::routes())
        .layer(Extension(app))
}

/// Starts background work (Restate draw states, retention purge if enabled, hub GC).
pub fn spawn_background(app: &AppState) {
    watch::spawn(app.clone());
    let resume = app.clone();
    tokio::spawn(async move {
        if let Err(e) = resume.local.resume_all(&resume).await {
            tracing::error!(error = %e, "resuming pending local draws failed");
        }
    });
    if app.cfg.retention_enabled {
        retention::spawn(app.clone());
    } else {
        tracing::info!("automatic erasure of personal data is off (RAFI_RETENTION)");
    }
    let hub = app.hub.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            tick.tick().await;
            hub.gc();
        }
    });
}

/// The complete HTTP router: [`routes`], then the web app (admin, projector,
/// verification page) from `public_dir` with a single-page fallback to its
/// `index.html`, all behind the Clerk auth layer unless dev auth is on.
pub fn build_router(
    app: AppState,
    auth: Option<dioxus_clerk::server::ClerkAuthLayer>,
    public_dir: &Path,
) -> Router {
    use tower_http::services::{ServeDir, ServeFile};

    let web_app = ServeDir::new(public_dir).fallback(ServeFile::new(public_dir.join("index.html")));
    let router = routes(app).fallback_service(web_app);
    match auth {
        Some(layer) => router.layer(layer),
        None => router,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_restate;
