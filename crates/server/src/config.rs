//! Runtime configuration from environment variables.

#[cfg(not(target_arch = "wasm32"))]
use std::env;

use anyhow::{anyhow, bail};
use rafi_raffle::drand::{ChainHash, chains};

/// How the app reaches the `rafi-restate` service: through the Restate ingress.
pub use rafi_engine_restate::ClientConfig as RestateConfig;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Externally visible base URL, used for QR codes and links.
    pub public_url: String,
    /// Skip Clerk and treat every admin request as user `dev`. Never use in production.
    pub dev_auth: bool,
    /// The raffle engine: the `Raffle` virtual object of the `rafi-restate`
    /// service. Without an ingress URL, draws can't start.
    pub restate: RestateConfig,
    /// The drand chain new raffles commit to (`RAFI_DRAND_CHAIN`, a chain
    /// hash; quicknet when unset). The app decides it ([`Config::chain`]); the
    /// raffle engine only checks it. `mock` (or `RAFI_DRAND_MOCK=1`) is the mock
    /// chain, in development builds only.
    pub drand_chain: String,
    /// The proof-of-work's HMAC key (`RAFI_ALTCHA_KEY`). Random when unset,
    /// so challenges die with the process.
    pub altcha_key: String,
    /// The proof-of-work's difficulty (`RAFI_ALTCHA_COST`): PBKDF2 iterations
    /// per attempt, about 256 attempts per solve.
    pub altcha_cost: u32,
    pub cookie_secure: bool,
    /// The most entrants a raffle takes (`RAFI_MAX_ENTRANTS`): the hard limit
    /// behind the soft rate limits.
    pub max_entrants: i64,
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
        // Hex: the key is a string, and raw random bytes aren't UTF-8.
        let altcha_key =
            var("RAFI_ALTCHA_KEY").unwrap_or_else(|| hex::encode(crate::ids::random_bytes::<32>()));
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
            // Only the chain matters here: the raffle engine fetches the beacons.
            drand_chain: var("RAFI_DRAND_CHAIN")
                .or_else(|| flag("RAFI_DRAND_MOCK").then(|| MOCK.to_owned()))
                .unwrap_or_else(|| chains::QUICKNET_HASH.to_string()),
            altcha_key,
            altcha_cost: var("RAFI_ALTCHA_COST")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_ALTCHA_COST),
            max_entrants: var("RAFI_MAX_ENTRANTS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000),
        }
    }

    /// The chain new raffles commit to. Release builds commit only to public
    /// drand chains; the mock chain (whose key is public: its beacons can be
    /// forged) needs a development build (`mock-drand`).
    pub fn chain(&self) -> anyhow::Result<ChainHash> {
        resolve_chain(&self.drand_chain, mock_chain())
    }

    /// Configuration for tests: in-memory database, mock drand, no Restate
    /// (the tests hand the app their engine).
    #[cfg(test)]
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
            drand_chain: MOCK.into(),
            altcha_key: "test-key".into(),
            altcha_cost: 1,
            cookie_secure: false,
            max_entrants: 2000,
        }
    }
}

/// `RAFI_ALTCHA_COST` when unset. Provisional: to be set from measurements
/// on old phones (docs/dioxus-altcha-plan.md, M4).
pub const DEFAULT_ALTCHA_COST: u32 = 1000;

/// The alias for the mock chain in `RAFI_DRAND_CHAIN`.
const MOCK: &str = "mock";

/// The mock chain's hash, in builds that may commit to it.
fn mock_chain() -> Option<ChainHash> {
    #[cfg(any(test, feature = "mock-drand"))]
    return Some(rafi_raffle::drand::mock_chain().chain().hash());
    #[cfg(not(any(test, feature = "mock-drand")))]
    None
}

/// The chain `configured` names, by the policy of a build whose mock chain is
/// `mock` (`None`: no mock chain allowed).
fn resolve_chain(configured: &str, mock: Option<ChainHash>) -> anyhow::Result<ChainHash> {
    let configured = configured.trim();
    if configured.eq_ignore_ascii_case(MOCK) {
        return mock.ok_or_else(|| {
            anyhow!(
                "RAFI_DRAND_CHAIN is the mock chain, but this build has no `mock-drand` feature"
            )
        });
    }
    let chain: ChainHash = configured
        .parse()
        .map_err(|e| anyhow!("RAFI_DRAND_CHAIN {configured:?}: {e}"))?;
    if chains::by_hash(&chain).is_some() || Some(chain) == mock {
        return Ok(chain);
    }
    bail!("RAFI_DRAND_CHAIN {chain} is not a public drand chain this build knows");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_builds_commit_only_to_public_chains() {
        let quicknet = chains::QUICKNET_HASH;
        let mock = rafi_raffle::drand::mock_chain().chain().hash();
        assert_eq!(
            resolve_chain(&quicknet.to_string(), None).unwrap(),
            quicknet
        );
        assert!(resolve_chain("mock", None).is_err());
        assert!(resolve_chain(&mock.to_string(), None).is_err());
        assert!(resolve_chain("not a hash", None).is_err());
        // Development builds also take the mock chain, by name or hash.
        assert_eq!(resolve_chain("mock", Some(mock)).unwrap(), mock);
        assert_eq!(resolve_chain(&mock.to_string(), Some(mock)).unwrap(), mock);
    }

    #[test]
    fn the_chain_defaults_to_quicknet() {
        let cfg = Config::from_lookup(|_| None);
        assert_eq!(cfg.chain().unwrap(), chains::QUICKNET_HASH);
        let cfg = Config::from_lookup(|name| (name == "RAFI_DRAND_MOCK").then(|| "1".into()));
        assert_eq!(cfg.drand_chain, "mock");
    }
}
