//! Rafi's server: the HTTP side of the app. Participant pages, public and
//! admin APIs, over the use cases in `rafi-app` (re-exported as
//! [`service`]): it maps requests onto them and their results onto
//! responses. The pages render in `pages`; the database and all its SQL
//! are in `rafi-store`.
//!
//! The native binary (`rafi`, `apps/rafi`) serves it with the web app's files;
//! on Cloudflare Workers, `rafi-worker` does.
//!
//! The server keeps no state of its own between requests: everything is in
//! the database, and the pieces that differ per platform sit behind small
//! interfaces in [`AppState`] ([`Engine`], the live updates'
//! `rafi_live_server::Notifier`, [`RateLimit`]); each deployment plugs in its
//! own.

pub mod admin;
pub mod auth;
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod housekeeping;
mod pages;
pub mod participant;
pub mod public;

pub use rafi_app::{self as service, ids, view};

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::{ops::Deref, sync::Arc};

use axum::{Extension, Router};
use rafi_app::RateLimit;
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
    /// `RAFI_ENGINE_INGRESS_URL`), the live updates and the join's rate limits.
    /// `Inner` derefs to it.
    pub core: service::App,
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
    /// updates and rate limits. The configuration is checked
    /// ([`Config::validate`]): a production server with no operator or
    /// documents doesn't start.
    pub fn from_parts(
        cfg: Config,
        db: Db,
        engine: Option<Arc<dyn Engine>>,
        live_updates: LiveUpdates,
        limiter: Arc<dyn RateLimit>,
    ) -> anyhow::Result<Self> {
        cfg.validate()?;
        let core = service::App {
            chain: cfg.chain()?,
            db,
            engine,
            live_updates,
            limiter,
            policy: service::Policy {
                public_url: cfg.public_url.clone(),
                max_entrants: cfg.max_entrants,
                proof_of_work: service::ProofOfWork::new(cfg.altcha_key.clone(), cfg.altcha_cost)?,
                retention: cfg.retention,
                min_age: cfg.min_age,
            },
        };
        Ok(Self(Arc::new(Inner { core, cfg })))
    }
}

/// The raffle engine `cfg` points at: the `rafi-restate` service through the
/// Restate ingress; `None` without `RAFI_ENGINE_INGRESS_URL`.
pub fn restate_engine(cfg: &Config) -> Option<Arc<dyn Engine>> {
    (!cfg.restate.ingress_url.is_empty())
        .then(|| Arc::new(RestateClient::new(cfg.restate.clone())) as Arc<dyn Engine>)
}

/// The server's own routes: participant pages, public JSON and the admin API,
/// with the [security headers](security_headers) on every response.
pub fn routes(app: AppState) -> Router {
    Router::new()
        .merge(participant::routes())
        .merge(public::routes())
        .merge(admin::routes())
        .layer(axum::middleware::from_fn(security_headers))
        .layer(Extension(app))
}

/// The paths of the web app that may be shown inside another page: the
/// projector (a venue's signage) and the verification page (a meetup's
/// site). Nothing else may be framed: not the admin, not the join page.
pub fn frameable(path: &str) -> bool {
    path.starts_with("/s/") || path.starts_with("/v/")
}

/// The headers every response carries, and that a browser acts on:
///
/// - `X-Content-Type-Options: nosniff`: a response is what its type says;
/// - `Referrer-Policy: strict-origin-when-cross-origin`: a link out of a
///   page (the organizer's own privacy notice, from a join page whose URL
///   holds the join code) carries the origin, not the path;
/// - `Content-Security-Policy: frame-ancestors` (and `X-Frame-Options` for
///   older browsers): no page is framed, except the ones [`frameable`] names,
///   so the admin can't be clickjacked.
///
/// On Workers, the web app's files are served by Workers Assets with the
/// same headers from `_headers` (`scripts/app-assets.sh`). HSTS is the
/// edge's: Cloudflare's, or the reverse proxy's natively.
pub async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{HeaderValue, header};
    let frameable = frameable(req.uri().path());
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    if !frameable {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'none'"),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    resp
}

/// Starts the native background work: the hourly housekeeping run.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_background(app: &AppState) {
    housekeeping::spawn(app.clone());
}

/// Where the static files are.
#[cfg(not(target_arch = "wasm32"))]
pub struct Dirs {
    /// The web app (admin, projector, verification page): `dx bundle`'s output.
    pub public: PathBuf,
    /// The participant build: `./scripts/participant.sh`'s output,
    /// `apps/participant/dist/public`. Served at `/participant/`: `assets/`
    /// (the module and its glue, named by their hash) and `fallback.js`, with
    /// `.br` and `.gz` next to each where the script could make them. Its
    /// `index.html`, the page's loader, isn't served: this server has it
    /// compiled in ([`check_participant`](Self::check_participant)). The
    /// proof-of-work solver there, `altcha-worker.js`, is a route in [`routes`].
    pub participant: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl Dirs {
    /// Checks that [`participant`](Self::participant) holds the build this
    /// server was compiled against: its `index.html` is the loader the page
    /// carries. Another build's files have other names, which the page
    /// wouldn't link; a server doesn't start on one.
    pub fn check_participant(&self) -> anyhow::Result<()> {
        use anyhow::Context as _;
        let path = self.participant.join("index.html");
        let loader = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "reading {}: RAFI_PARTICIPANT_DIR is the participant build's dist/public (./scripts/participant.sh)",
                path.display()
            )
        })?;
        anyhow::ensure!(
            loader == pages::LOADER,
            "{} is not the participant build this server was built with: run ./scripts/participant.sh and build the server again, or point RAFI_PARTICIPANT_DIR at that build",
            path.display()
        );
        Ok(())
    }
}

/// `Cache-Control` for the participant's hashed assets: kept for good.
#[cfg(not(target_arch = "wasm32"))]
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The complete HTTP router: [`routes`], the participant build's files at
/// `/participant/`, then the web app from [`Dirs::public`] with a single-page
/// fallback to its `index.html`, all behind the Clerk auth layer unless dev
/// auth is on. On Workers, Workers Assets serves the files instead.
///
/// The participant's assets are named by their hash, so a phone keeps them
/// for good (`Cache-Control: immutable`; a deploy links new names). Its
/// fallback keeps a fixed name, so a phone must revalidate it (`no-cache`,
/// with the `ETag` `ServeDir` derives from the file). Either header goes on
/// a file that was found, never on a 404, which a browser could otherwise
/// keep. The files are served precompressed where `participant.sh` left a
/// `.br` or `.gz`: the module crosses venue wifi, where the uncompressed
/// size would be felt.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_router(
    app: AppState,
    auth: Option<dioxus_clerk::server::ClerkAuthLayer>,
    dirs: &Dirs,
) -> Router {
    use axum::{
        http::{HeaderValue, StatusCode, header::CACHE_CONTROL},
        response::Response,
    };
    use tower_http::{
        services::{ServeDir, ServeFile},
        set_header::SetResponseHeaderLayer,
    };

    /// `value` as the `Cache-Control` of a response that found its file.
    fn cache_control(value: &'static str) -> impl Fn(&Response) -> Option<HeaderValue> + Clone {
        move |resp| {
            (resp.status().is_success() || resp.status() == StatusCode::NOT_MODIFIED)
                .then(|| HeaderValue::from_static(value))
        }
    }

    let web_app = Router::new()
        .fallback_service(
            ServeDir::new(&dirs.public).fallback(ServeFile::new(dirs.public.join("index.html"))),
        )
        .layer(axum::middleware::from_fn(security_headers));
    let assets = Router::new()
        .fallback_service(
            ServeDir::new(dirs.participant.join("assets"))
                .precompressed_br()
                .precompressed_gzip(),
        )
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            cache_control(IMMUTABLE),
        ));
    let fallback = Router::new()
        .route_service(
            "/participant/fallback.js",
            ServeFile::new(dirs.participant.join("fallback.js"))
                .precompressed_br()
                .precompressed_gzip(),
        )
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            cache_control("no-cache"),
        ));
    let router = routes(app)
        .nest("/participant/assets", assets)
        .merge(fallback)
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
