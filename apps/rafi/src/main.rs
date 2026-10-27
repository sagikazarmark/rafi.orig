//! The native server: `rafi`. Everything but the entry point is in
//! `rafi-server`; on Workers, `rafi-worker` serves the same routes.
//!
//! `rafi reapply-erasures` is the step after a database restore: it erases
//! again whatever the erasure log says was erased, purges again what was
//! purged, runs housekeeping once, and exits.

use anyhow::Context as _;
use rafi_server as server;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cfg = server::config::Config::from_env()?;
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("reapply-erasures") => return reapply_erasures(cfg).await,
        Some(other) => {
            anyhow::bail!("unknown command {other:?}; the only one is `reapply-erasures`")
        }
    }
    if cfg.dev_auth {
        if !cfg!(feature = "dev") {
            anyhow::bail!(
                "RAFI_DEV_AUTH is set, but this build has no `dev` feature: a production build never runs without sign-in (build with --features dev for local development)"
            );
        }
        tracing::warn!("RAFI_DEV_AUTH is on: admin pages are NOT protected");
    }
    if cfg.drand_chain.eq_ignore_ascii_case("mock") {
        tracing::warn!(
            "RAFI_DRAND_CHAIN is the mock chain: draws use a public test chain and are NOT verifiable as drand"
        );
    }
    if cfg.restate.ingress_url.is_empty() {
        tracing::warn!("RAFI_ENGINE_INGRESS_URL is not set: draws can't start");
    }
    let app = server::AppState::new(cfg).await?;
    server::spawn_background(&app);
    let auth = if app.cfg.dev_auth {
        None
    } else {
        Some(
            server::auth::clerk_layer(
                &app.cfg,
                dioxus_clerk::server::ClerkAuthLayerConfig::from_env()
                    .context("Clerk configuration")?,
            )
            .context("Clerk configuration")?,
        )
    };

    let dir = |var: &str, default: &str| {
        std::env::var_os(var)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| default.into())
    };
    // The web bundle: `dx bundle --platform web --release` writes it here.
    let public_dir = dir("RAFI_PUBLIC_DIR", "target/dx/rafi-web/release/web/public");
    if !public_dir.join("index.html").is_file() {
        tracing::warn!(
            dir = %public_dir.display(),
            "no web app in RAFI_PUBLIC_DIR: only the participant pages and APIs are served"
        );
    }
    // The participant page's WebAssembly and fallback: `./scripts/participant.sh` writes them here.
    let participant_dir = dir("RAFI_PARTICIPANT_DIR", "apps/participant/dist");
    if !participant_dir.join("fallback.js").is_file() {
        tracing::warn!(
            dir = %participant_dir.display(),
            "no participant scripts in RAFI_PARTICIPANT_DIR (./scripts/participant.sh): phones can't join or follow the draw"
        );
    }
    let router = server::build_router(
        app,
        auth,
        &server::Dirs {
            public: public_dir,
            participant: participant_dir,
        },
    );

    // `IP` and `PORT`, as the Dioxus CLI sets them.
    let ip = std::env::var("IP").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let addr = format!("{ip}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("listening on {addr}"))?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// After a restore: re-applies the erasure log and the purges, then runs
/// housekeeping, so what was due is erased again too.
async fn reapply_erasures(cfg: server::config::Config) -> anyhow::Result<()> {
    let app = server::AppState::new(cfg).await?;
    let done = server::service::reapply_erasures(&app).await?;
    tracing::info!(
        entrants = done.entrants,
        raffles = done.raffles,
        organizers = done.organizers,
        "erasures re-applied; housekeeping ran"
    );
    println!(
        "re-erased {} entrant(s), re-purged {} raffle(s), re-removed {} organizer(s); housekeeping ran",
        done.entrants, done.raffles, done.organizers
    );
    Ok(())
}

/// Ctrl-C or SIGTERM.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
