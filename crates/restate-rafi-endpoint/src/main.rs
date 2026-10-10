//! Serves the Rafi raffle engine (the `Raffle` virtual object and the
//! `EngineInfo` service) for Restate: logging
//! that survives replays, request identity verification, and a graceful stop
//! on SIGTERM as well as Ctrl-C.

mod config;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rafi_drand::DrandClient;
use rafi_platform::{Notify, Polling};
use restate_config::ConfigureEndpointExt;
use restate_ext::shutdown_signal;
use restate_sdk::{endpoint::Endpoint, filter::ReplayAwareFilter, http_server::HttpServer};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Configuration file (JSON, YAML or TOML); `RESTATE_RAFI_*` variables override it.
    #[arg(long, value_name = "FILE", env = "CONFIG_FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
                )
                // Restate re-runs handler code while replaying a journal; log each line once.
                .with_filter(ReplayAwareFilter),
        )
        .init();

    let config = Config::load(cli.config.as_deref())?;
    if config.drand.mock {
        warn!("drand.mock is on: draws use a public test chain and are NOT verifiable as drand");
    }
    if config.endpoint.identity_keys.is_empty() {
        warn!(
            "no endpoint.identity_keys: requests are not verified to come from your Restate server"
        );
    }

    // Bind first, so a taken port is an error rather than a panic.
    let listener = TcpListener::bind(&config.endpoint.listener)
        .await
        .with_context(|| format!("binding {}", config.endpoint.listener))?;
    info!(address = %listener.local_addr()?, database = config.database.url, "serving the Rafi raffle engine");

    let endpoint = endpoint(config)?;
    HttpServer::new(endpoint)
        .serve_with_cancel(listener, shutdown_signal())
        .await;

    info!("stopped");
    Ok(())
}

/// The SDK endpoint: endpoint-wide settings, then the `Raffle` object with
/// its policy and the `EngineInfo` service.
/// Fails on invalid configuration, so the process exits before accepting requests.
fn endpoint(config: Config) -> Result<Endpoint> {
    let drand = DrandClient::new(config.drand).context("invalid drand configuration")?;
    let (raffle, info) = rafi_engine_restate::services(
        drand,
        config.database.raffle_store()?,
        // Natively there is no push: the app's clients poll the database.
        Notify::new(Polling::default(), rafi_lifecycle::drawing::raffle_id),
    );
    Ok(Endpoint::builder()
        .configure(&config.endpoint)?
        .bind(config.services.raffle.apply(raffle)?)
        .bind(info)
        .build())
}
