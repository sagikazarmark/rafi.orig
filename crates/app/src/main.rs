//! Rafi's entry points: the native server (`server`) or the web app (`web`).
//! Everything else is in the library (`lib.rs`).

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context as _;
    use rafi::server;

    dioxus::logger::initialize_default();
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
    let public_dir = dir("RAFI_PUBLIC_DIR", "target/dx/rafi/release/web/public");
    if !public_dir.join("index.html").is_file() {
        tracing::warn!(
            dir = %public_dir.display(),
            "no web app in RAFI_PUBLIC_DIR: only the attendee pages and APIs are served"
        );
    }
    // The attendee page's WebAssembly and fallback: `./scripts/attendee.sh` writes them here.
    let attendee_dir = dir("RAFI_ATTENDEE_DIR", "crates/attendee/dist");
    if !attendee_dir.join("fallback.js").is_file() {
        tracing::warn!(
            dir = %attendee_dir.display(),
            "no attendee scripts in RAFI_ATTENDEE_DIR (./scripts/attendee.sh): phones can't join or follow the draw"
        );
    }
    let router = server::build_router(
        app,
        auth,
        &server::Dirs {
            public: public_dir,
            attendee: attendee_dir,
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
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
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
    dioxus::launch(rafi::ui::App);
}

#[cfg(not(any(feature = "web", feature = "server")))]
fn main() {
    eprintln!(
        "Build rafi with `--features server` (the server) or `--features web` (the web app)."
    );
}

/// On Cloudflare Workers the entry points are in the library (`worker`
/// feature); the binary has nothing to run.
#[cfg(all(feature = "server", target_arch = "wasm32"))]
fn main() {}
