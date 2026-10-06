//! Runtime configuration from environment variables.

use std::env;

pub use rafi_drand::DrandConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// Draws run in-process (development, tests, emergency fallback).
    Local,
    /// Draws run in the `Raffle` virtual object of the `restate-rafi` service.
    Restate,
}

/// How the app reaches the `restate-rafi` service: through the Restate ingress.
#[derive(Debug, Clone)]
pub struct RestateConfig {
    /// Ingress base URL: `http://restate-tunnel:8080` (the Cloud tunnel client's
    /// authenticating proxy), `https://<env>.env.eu.restate.cloud:8080`, or a
    /// self-hosted `http://localhost:8080`.
    pub ingress_url: String,
    /// API key for the ingress (Restate Cloud direct). Empty behind the tunnel
    /// proxy and for unauthenticated local servers.
    pub api_key: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Externally visible base URL, used for QR codes and links.
    pub public_url: String,
    /// Skip Clerk and treat every admin request as user `dev`. Never use in production.
    pub dev_auth: bool,
    pub engine: EngineKind,
    pub restate: RestateConfig,
    pub drand: DrandConfig,
    pub altcha_key: Vec<u8>,
    /// Maximum proof-of-work number (difficulty).
    pub altcha_max: u64,
    pub cookie_secure: bool,
    /// Run the hourly erasure job (`RAFI_RETENTION`). Off for now: personal
    /// data is only erased by hand from the admin.
    pub retention_enabled: bool,
    pub retention_nonwinners_days: i64,
    pub retention_winners_days: i64,
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn flag(name: &str) -> bool {
    var(name).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
}

impl Config {
    pub fn from_env() -> Self {
        let public_url = var("RAFI_PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_owned();
        let restate_ingress = var("RESTATE_INGRESS_URL").unwrap_or_default();
        let engine = match var("RAFI_DRAW_ENGINE").as_deref() {
            Some("restate") => EngineKind::Restate,
            Some("local") => EngineKind::Local,
            _ if !restate_ingress.is_empty() => EngineKind::Restate,
            _ => EngineKind::Local,
        };
        let altcha_key = var("RAFI_ALTCHA_KEY")
            .map(|k| k.into_bytes())
            .unwrap_or_else(|| crate::server::ids::random_bytes::<32>().to_vec());
        let defaults = DrandConfig::default();
        Self {
            database_url: var("RAFI_DATABASE_URL").unwrap_or_else(|| "sqlite://rafi.db".into()),
            cookie_secure: var("RAFI_COOKIE_SECURE")
                .map(|v| v == "1" || v == "true")
                .unwrap_or_else(|| public_url.starts_with("https://")),
            public_url,
            dev_auth: flag("RAFI_DEV_AUTH"),
            engine,
            restate: RestateConfig {
                ingress_url: restate_ingress.trim_end_matches('/').to_owned(),
                api_key: var("RESTATE_API_KEY").unwrap_or_default(),
            },
            drand: DrandConfig {
                relays: var("RAFI_DRAND_RELAYS")
                    .map(|v| v.split(',').map(str::to_owned).collect())
                    .unwrap_or(defaults.relays),
                mock: flag("RAFI_DRAND_MOCK"),
                lead_secs: var("RAFI_DRAW_LEAD_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(defaults.lead_secs),
                timeout_secs: var("RAFI_DRAND_TIMEOUT_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(defaults.timeout_secs),
            },
            altcha_key,
            altcha_max: var("RAFI_ALTCHA_MAX")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30_000),
            retention_enabled: flag("RAFI_RETENTION"),
            retention_nonwinners_days: var("RAFI_RETENTION_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            retention_winners_days: var("RAFI_RETENTION_WINNERS_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(90),
        }
    }

    /// Configuration for tests: in-memory database, local engine, mock drand.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn for_tests() -> Self {
        Self {
            database_url: "sqlite::memory:".into(),
            public_url: "http://localhost:8080".into(),
            dev_auth: false,
            engine: EngineKind::Local,
            restate: RestateConfig {
                ingress_url: String::new(),
                api_key: String::new(),
            },
            drand: DrandConfig {
                relays: vec![],
                mock: true,
                lead_secs: 0,
                timeout_secs: 5,
            },
            altcha_key: b"test-key".to_vec(),
            altcha_max: 50,
            cookie_secure: false,
            retention_enabled: false,
            retention_nonwinners_days: 30,
            retention_winners_days: 90,
        }
    }
}
