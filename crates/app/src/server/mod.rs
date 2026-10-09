//! Server side: participant pages, public and admin APIs. The database and all
//! its SQL are in `rafi-store`.
//!
//! The server keeps no state of its own between requests: everything is in
//! the database, and the pieces that differ per platform sit behind small
//! interfaces from `rafi-platform` in [`AppState`] ([`DrawingEngine`],
//! [`Notifier`], [`RateLimit`]); each deployment plugs in its own.

pub mod admin;
pub mod altcha;
pub mod auth;
pub mod config;
pub mod ids;
pub mod participant;
pub mod public;
pub mod retention;
pub mod service;
pub mod view;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::{ops::Deref, sync::Arc};

use axum::{Extension, Router};
use rafi_drawing::drand::ChainHash;
use rafi_platform::{DrawingEngine, Notifier, RateLimit};
use rafi_store::Db;
use restate_rafi_client::RestateClient;

use self::config::Config;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    /// The drand chain new raffles commit to ([`Config::chain`]).
    pub chain: ChainHash,
    /// The draw engine; `None` without one (no `RESTATE_INGRESS_URL`).
    pub engine: Option<Arc<dyn DrawingEngine>>,
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
        use anyhow::Context as _;

        let db = Db::connect(&cfg.database_url)
            .await
            .context("opening database")?;
        let engine = restate_engine(&cfg);
        Self::from_parts(
            cfg,
            db,
            engine,
            Arc::new(rafi_platform::Polling::default()),
            Arc::new(rafi_platform::Limiter::default()),
        )
    }

    /// The app over a database, a draw engine and the platform's notifier and
    /// rate limits.
    pub fn from_parts(
        cfg: Config,
        db: Db,
        engine: Option<Arc<dyn DrawingEngine>>,
        notifier: Arc<dyn Notifier>,
        limiter: Arc<dyn RateLimit>,
    ) -> anyhow::Result<Self> {
        Ok(Self(Arc::new(Inner {
            chain: cfg.chain()?,
            engine,
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

/// The draw engine `cfg` points at: the `restate-rafi` service through the
/// Restate ingress; `None` without `RESTATE_INGRESS_URL`.
pub fn restate_engine(cfg: &Config) -> Option<Arc<dyn DrawingEngine>> {
    (!cfg.restate.ingress_url.is_empty())
        .then(|| Arc::new(RestateClient::new(cfg.restate.clone())) as Arc<dyn DrawingEngine>)
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
