//! Server side: participant pages, public and admin APIs. The database and all
//! its SQL are in `rafi-store`.
//!
//! What the app does lives in `rafi-app-core` (re-exported as [`service`]):
//! this is its HTTP side, which maps requests onto those use cases and their
//! results onto responses.
//!
//! The server keeps no state of its own between requests: everything is in
//! the database, and the pieces that differ per platform sit behind small
//! interfaces in [`AppState`] ([`Engine`], the live updates'
//! `rafi_live_server::Notifier`, [`RateLimit`]); each deployment plugs in its
//! own.

pub mod admin;
pub mod altcha;
pub mod auth;
pub mod config;
pub mod participant;
pub mod public;
pub mod retention;

pub use rafi_app_core::{self as service, codes, ids, view};

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::{ops::Deref, sync::Arc};

use axum::{Extension, Router};
use rafi_app_core::RateLimit;
use rafi_engine::Engine;
use rafi_engine_restate::RestateClient;
use rafi_live_server::LiveUpdates;
use rafi_store::Db;

use self::config::Config;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    /// What the use cases run against: the database, the drand chain
    /// ([`Config::chain`]), the raffle engine (`None` without
    /// `RESTATE_INGRESS_URL`) and the live updates. `Inner` derefs to it.
    pub core: service::App,
    /// The join rate limits.
    pub limiter: Arc<dyn RateLimit>,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Deref for Inner {
    type Target = service::App;
    fn deref(&self) -> &service::App {
        &self.core
    }
}

impl AppState {
    /// The native server: opens (and migrates) the database; no push (clients
    /// poll), in-memory rate limits.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn new(cfg: Config) -> anyhow::Result<Self> {
        use anyhow::Context as _;

        let db = Db::connect(&cfg.database_url)
            .await
            .context("opening database")?;
        let engine = restate_engine(&cfg);
        Self::from_parts(
            cfg,
            db,
            engine,
            LiveUpdates::polling(),
            Arc::new(service::Limiter::default()),
        )
    }

    /// The app over a database, a raffle engine and the platform's live
    /// updates and rate limits.
    pub fn from_parts(
        cfg: Config,
        db: Db,
        engine: Option<Arc<dyn Engine>>,
        live_updates: LiveUpdates,
        limiter: Arc<dyn RateLimit>,
    ) -> anyhow::Result<Self> {
        let core = service::App {
            chain: cfg.chain()?,
            db,
            engine,
            live_updates,
            policy: service::Policy {
                public_url: cfg.public_url.clone(),
                max_entrants: cfg.max_entrants,
            },
        };
        Ok(Self(Arc::new(Inner { core, limiter, cfg })))
    }

    /// When personal data of a finished raffle is erased (the retention settings).
    pub fn purge_after(&self) -> rafi_store::PurgeAfter {
        rafi_store::PurgeAfter::days(
            self.cfg.retention_nonwinners_days,
            self.cfg.retention_winners_days,
        )
    }
}

/// The raffle engine `cfg` points at: the `restate-rafi` service through the
/// Restate ingress; `None` without `RESTATE_INGRESS_URL`.
pub fn restate_engine(cfg: &Config) -> Option<Arc<dyn Engine>> {
    (!cfg.restate.ingress_url.is_empty())
        .then(|| Arc::new(RestateClient::new(cfg.restate.clone())) as Arc<dyn Engine>)
}

/// The server's own routes: participant pages, public JSON and the admin API.
pub fn routes(app: AppState) -> Router {
    Router::new()
        .merge(participant::routes())
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
    /// The participant page's WebAssembly and fallback: `./scripts/participant.sh`'s
    /// output, served at `/participant/`.
    pub participant: PathBuf,
}

/// The complete HTTP router: [`routes`], the participant scripts at
/// `/participant/`, then the web app from [`Dirs::public`] with a single-page
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
        .nest_service("/participant", ServeDir::new(&dirs.participant))
        .fallback_service(web_app);
    match auth {
        Some(layer) => router.layer(layer),
        None => router,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_restate;
