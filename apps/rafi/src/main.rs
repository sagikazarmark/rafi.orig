//! The native server: `rafi`. Everything but the entry point is in
//! `rafi-server`; on Workers, `rafi-app-worker` serves the same routes.

use anyhow::Context as _;
use rafi_server as server;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cfg = server::config::Config::from_env();
    if cfg.dev_auth {
        tracing::warn!("RAFI_DEV_AUTH is on: admin pages are NOT protected");
    }
    if cfg.drand_chain.eq_ignore_ascii_case("mock") {
        tracing::warn!(
            "RAFI_DRAND_CHAIN is the mock chain: draws use a public test chain and are NOT verifiable as drand"
        );
    }
    if cfg.restate.ingress_url.is_empty() {
        tracing::warn!("RESTATE_INGRESS_URL is not set: draws can't start");
    }
    let app = server::AppState::new(cfg).await?;
    server::spawn_background(&app);
    let auth = if app.cfg.dev_auth {
        None
    } else {
        Some(dioxus_clerk::server::ClerkAuthLayer::from_env().context("Clerk configuration")?)
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

    // `IP` and `PORT`, as the Dioxus CLI and the container image set them.
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

/// Ctrl-C or SIGTERM (`docker stop`).
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
