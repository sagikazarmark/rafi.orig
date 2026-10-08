//! Server side: database, attendee pages, public and admin APIs, and the
//! draw engine's client.
//!
//! The server keeps no state of its own between requests: everything is in
//! the database, and the pieces that differ per platform sit behind small
//! interfaces from `rafi-platform` in [`AppState`] ([`Notifier`],
//! [`RateLimit`]); each deployment plugs in its own.

pub mod admin;
pub mod altcha;
pub mod attendee;
pub mod auth;
pub mod config;
pub mod db;
pub mod engine;
pub mod ids;
pub mod public;
pub mod retention;
pub mod service;
pub mod view;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::{ops::Deref, sync::Arc};

use anyhow::Context as _;
use axum::{Extension, Router};
use rafi_drand::DrandClient;
use rafi_platform::{Notifier, RateLimit};

use self::{config::Config, db::Db, engine::RestateClient};

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    /// The drand chain new raffles commit to.
    pub drand: DrandClient,
    /// The draw engine; `None` without `RESTATE_INGRESS_URL`.
    pub restate: Option<RestateClient>,
    /// Live updates to clients (natively: none, clients poll).
    pub notifier: Arc<dyn Notifier>,
    /// The join rate limits.
    pub limiter: Arc<dyn RateLimit>,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    /// The native server: opens (and migrates) the database; no push (clients
    /// poll), in-memory rate limits.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn new(cfg: Config) -> anyhow::Result<Self> {
        let db = db::connect(&cfg.database_url)
            .await
            .context("opening database")?;
        Self::from_parts(
            cfg,
            db,
            Arc::new(rafi_platform::Polling::default()),
            Arc::new(rafi_platform::Limiter::default()),
        )
    }

    /// The app over a database and the platform's notifier and rate limits.
    pub fn from_parts(
        cfg: Config,
        db: Db,
        notifier: Arc<dyn Notifier>,
        limiter: Arc<dyn RateLimit>,
    ) -> anyhow::Result<Self> {
        let restate =
            (!cfg.restate.ingress_url.is_empty()).then(|| RestateClient::new(cfg.restate.clone()));
        Ok(Self(Arc::new(Inner {
            drand: DrandClient::new(cfg.drand.clone()).context("drand configuration")?,
            restate,
            notifier,
            limiter,
            db,
            cfg,
        })))
    }

    /// How clients follow `raffle_id`: the deployment's transport while its
    /// live updates run, plain polling once they've ended.
    pub fn transport(&self, raffle_id: &str, live_ended: bool) -> rafi_live::LiveTransport {
        if live_ended {
            rafi_live::LiveTransport::poll()
        } else {
            self.notifier.transport(raffle_id)
        }
    }

    /// When personal data of a finished raffle is erased (the retention settings).
    pub fn purge_after(&self) -> rafi_store::PurgeAfter {
        rafi_store::PurgeAfter::days(
            self.cfg.retention_nonwinners_days,
            self.cfg.retention_winners_days,
        )
    }
}

/// The server's own routes: attendee pages, public JSON and the admin API.
pub fn routes(app: AppState) -> Router {
    Router::new()
        .merge(attendee::routes())
        .merge(public::routes())
        .merge(admin::routes())
        .layer(Extension(app))
}

/// Starts the native background work: the hourly retention run.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_background(app: &AppState) {
    if !app.cfg.retention_enabled {
        tracing::info!("automatic erasure of personal data is off (RAFI_RETENTION)");
    }
    retention::spawn(app.clone());
}

/// Where the static files are.
#[cfg(not(target_arch = "wasm32"))]
pub struct Dirs {
    /// The web app (admin, projector, verification page): `dx bundle`'s output.
    pub public: PathBuf,
    /// The attendee page's WebAssembly and fallback: `./scripts/attendee.sh`'s
    /// output, served at `/attendee/`.
    pub attendee: PathBuf,
}

/// The complete HTTP router: [`routes`], the attendee scripts at
/// `/attendee/`, then the web app from [`Dirs::public`] with a single-page
/// fallback to its `index.html`, all behind the Clerk auth layer unless dev
/// auth is on. On Workers, Workers Assets serves the files instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_router(
    app: AppState,
    auth: Option<dioxus_clerk::server::ClerkAuthLayer>,
    dirs: &Dirs,
) -> Router {
    use tower_http::services::{ServeDir, ServeFile};

    let web_app =
        ServeDir::new(&dirs.public).fallback(ServeFile::new(dirs.public.join("index.html")));
    let router = routes(app)
        .nest_service("/attendee", ServeDir::new(&dirs.attendee))
        .fallback_service(web_app);
    match auth {
        Some(layer) => router.layer(layer),
        None => router,
    }
}

#[cfg(test)]
mod fake_engine;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_restate;
