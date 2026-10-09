//! Runtime configuration from environment variables.

#[cfg(not(target_arch = "wasm32"))]
use std::env;

pub use rafi_drand::DrandConfig;
/// How the app reaches the `restate-rafi` service: through the Restate ingress.
pub use restate_rafi_client::ClientConfig as RestateConfig;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Externally visible base URL, used for QR codes and links.
    pub public_url: String,
    /// Skip Clerk and treat every admin request as user `dev`. Never use in production.
    pub dev_auth: bool,
    /// The draw engine: the `Raffle` virtual object of the `restate-rafi`
    /// service. Without an ingress URL, draws can't start.
    pub restate: RestateConfig,
    /// The drand chain new raffles commit to (the draw engine refuses another).
    pub drand: DrandConfig,
    pub altcha_key: Vec<u8>,
    /// Maximum proof-of-work number (difficulty).
    pub altcha_max: u64,
    pub cookie_secure: bool,
    /// The most entrants a raffle takes (`RAFI_MAX_ENTRANTS`): the hard limit
    /// behind the soft rate limits.
    pub max_entrants: i64,
    /// Erase personal data in the hourly retention run (`RAFI_RETENTION`).
    /// Off for now: personal data is only erased by hand from the admin. The
    /// run itself always happens (expired proof-of-work challenges, ending
    /// live channels).
    pub retention_enabled: bool,
    pub retention_nonwinners_days: i64,
    pub retention_winners_days: i64,
}

impl Config {
    /// From the process environment (the native server).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env() -> Self {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// From any lookup: the process environment natively, the Worker's vars
    /// and secrets on Workers. Empty values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| lookup(name).filter(|v| !v.trim().is_empty());
        let flag = |name: &str| {
            var(name).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        };
        let public_url = var("RAFI_PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_owned();
        let altcha_key = var("RAFI_ALTCHA_KEY")
            .map(|k| k.into_bytes())
            .unwrap_or_else(|| crate::server::ids::random_bytes::<32>().to_vec());
        Self {
            database_url: var("RAFI_DATABASE_URL").unwrap_or_else(|| "sqlite://rafi.db".into()),
            cookie_secure: var("RAFI_COOKIE_SECURE")
                .map(|v| v == "1" || v == "true")
                .unwrap_or_else(|| public_url.starts_with("https://")),
            public_url,
            dev_auth: flag("RAFI_DEV_AUTH"),
            restate: RestateConfig {
                ingress_url: var("RESTATE_INGRESS_URL")
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_owned(),
                api_key: var("RESTATE_API_KEY").unwrap_or_default(),
                timeout_secs: var("RESTATE_TIMEOUT_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(RestateConfig::DEFAULT_TIMEOUT_SECS),
            },
            // Only the chain matters here: the draw engine fetches the beacons.
            drand: DrandConfig {
                mock: flag("RAFI_DRAND_MOCK"),
                ..DrandConfig::default()
            },
            altcha_key,
            altcha_max: var("RAFI_ALTCHA_MAX")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30_000),
            max_entrants: var("RAFI_MAX_ENTRANTS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000),
            retention_enabled: flag("RAFI_RETENTION"),
            retention_nonwinners_days: var("RAFI_RETENTION_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            retention_winners_days: var("RAFI_RETENTION_WINNERS_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(90),
        }
    }

    /// Configuration for tests: in-memory database, mock drand, no Restate
    /// (the tests hand the app their engine).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn for_tests() -> Self {
        Self {
            database_url: "sqlite::memory:".into(),
            public_url: "http://localhost:8080".into(),
            dev_auth: false,
            restate: RestateConfig {
                ingress_url: String::new(),
                api_key: String::new(),
                timeout_secs: RestateConfig::DEFAULT_TIMEOUT_SECS,
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
            max_entrants: 2000,
            retention_enabled: false,
            retention_nonwinners_days: 30,
            retention_winners_days: 90,
        }
    }
}
