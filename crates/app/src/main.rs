//! Rafi: live, verifiable meetup raffles.
//!
//! Two builds of one crate: `web` is the admin, projector and verification page (a
//! Dioxus single-page app, `dx bundle --platform web`); `server` is the native HTTP
//! server (attendee pages, public and admin JSON APIs, and the web app's files).

// Shared modules are compiled for both the server and the browser; some items
// are only used on one side.
#![cfg_attr(not(feature = "server"), allow(dead_code))]

#[cfg_attr(not(feature = "web"), allow(dead_code))]
mod api;
// Installed from the dioxus-daisyui-components registry (`dx components add`); not every
// export is used.
#[cfg(feature = "web")]
#[allow(unused_imports, dead_code)]
mod components;
#[cfg_attr(not(feature = "web"), allow(dead_code))]
mod i18n;
#[cfg_attr(not(feature = "web"), allow(dead_code))]
mod icons;
#[cfg_attr(not(feature = "web"), allow(dead_code))]
mod model;
#[cfg(feature = "server")]
mod server;
mod style;
#[cfg(feature = "web")]
mod ui;

#[cfg(feature = "server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context as _;

    dioxus::logger::initialize_default();
    let cfg = server::config::Config::from_env();
    if cfg.dev_auth {
        tracing::warn!("RAFI_DEV_AUTH is on: admin pages are NOT protected");
    }
    if cfg.drand.mock {
        tracing::warn!(
            "RAFI_DRAND_MOCK is on: draws use a public test chain and are NOT verifiable as drand"
        );
    }
    let app = server::AppState::new(cfg).await?;
    server::spawn_background(&app);
    let auth = if app.cfg.dev_auth {
        None
    } else {
        Some(dioxus_clerk::server::ClerkAuthLayer::from_env().context("Clerk configuration")?)
    };

    // The web bundle: `dx bundle --platform web --release` writes it here.
    let public_dir = std::env::var_os("RAFI_PUBLIC_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "target/dx/rafi/release/web/public".into());
    if !public_dir.join("index.html").is_file() {
        tracing::warn!(
            dir = %public_dir.display(),
            "no web app in RAFI_PUBLIC_DIR: only the attendee pages and APIs are served"
        );
    }
    let router = server::build_router(app, auth, &public_dir);

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
#[cfg(feature = "server")]
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

#[cfg(all(feature = "web", not(feature = "server")))]
fn main() {
    dioxus::launch(ui::App);
}

#[cfg(not(any(feature = "web", feature = "server")))]
fn main() {
    eprintln!(
        "Build rafi with `--features server` (the server) or `--features web` (the web app)."
    );
}
