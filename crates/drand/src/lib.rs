//! Rafi's drand settings on top of [`drand`]: which chain, which relays,
//! how far ahead a draw commits, and how long a draw waits for its beacon.
//!
//! The chain is quicknet, or (with `mock = true` and the `mock` feature) a
//! signing test chain with a public seed: its beacons are real BLS signatures,
//! but anyone can forge them, and its chain hash marks every raffle using it as
//! not publicly verifiable.

use std::time::{Duration, SystemTime};

use anyhow::{Context, bail, ensure};
pub use drand::client::{Error as FetchError, Fetch};
use drand::{
    ChainHash, ChainInfo, Schedule, UnixTime, VerifiedBeacon, Verifier, chains,
    client::{Client, Strategy},
    http::Relay,
};
use serde::{Deserialize, Serialize};

/// Beacon and timing settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DrandConfig {
    /// Relays to ask, in order: `v2:https://…`, `v1:https://…`, or a bare URL (v1).
    pub relays: Vec<String>,
    /// Use the public test chain instead of quicknet (development and tests only:
    /// NOT verifiable as drand; needs the `mock` feature).
    pub mock: bool,
    /// Seconds between starting a draw and its committed round. At least one
    /// quicknet period (3 s); 0 only with `mock`.
    pub lead_secs: u64,
    /// Give up fetching a beacon after this many seconds (the draw stays
    /// pending on its round; pressing Draw again retries the same round).
    pub timeout_secs: u64,
}

impl Default for DrandConfig {
    fn default() -> Self {
        Self {
            relays: vec![
                "v2:https://api.drand.sh".into(),
                "v2:https://api2.drand.sh".into(),
                "v2:https://api3.drand.sh".into(),
                "v1:https://drand.cloudflare.com".into(),
            ],
            mock: false,
            lead_secs: 6,
            timeout_secs: 60,
        }
    }
}

/// The public seed of the mock chain. Anyone can sign its beacons.
pub const MOCK_SEED: [u8; 32] = *b"rafi mock drand: not verifiable!";

/// The mock (test) chain: pinned parameters, so the app and the drawing engine
/// agree on its hash across restarts.
#[cfg(feature = "mock")]
pub fn mock_chain() -> drand::TestChain {
    let q = chains::quicknet();
    let period = u32::try_from(q.period().as_secs())
        .ok()
        .and_then(std::num::NonZeroU32::new)
        .expect("quicknet period");
    drand::TestChain::new(
        MOCK_SEED,
        drand::Scheme::BlsUnchainedG1Rfc9380,
        period,
        q.genesis_time().as_secs(),
        "rafi-test",
    )
    .expect("supported scheme")
}

/// Chain info for a chain hash found in a manifest: the public drand
/// chains, and the mock chain when this build supports it.
pub fn resolve_chain(hash: &ChainHash) -> Option<ChainInfo> {
    if let Some(info) = chains::by_hash(hash) {
        return Some(info);
    }
    #[cfg(feature = "mock")]
    {
        let mock = mock_chain().chain().clone();
        if mock.hash() == *hash {
            return Some(mock);
        }
    }
    None
}

impl DrandConfig {
    /// The settings from flat variables, for hosts without a configuration
    /// file (Workers' vars): `RAFI_DRAND_RELAYS` (comma-separated),
    /// `RAFI_DRAND_MOCK` (`1`), `RAFI_DRAND_LEAD_SECS` and
    /// `RAFI_DRAND_TIMEOUT_SECS`. Anything missing or unparsable is the
    /// default.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let defaults = Self::default();
        let number = |name, default| {
            get(name)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(default)
        };
        Self {
            relays: get("RAFI_DRAND_RELAYS")
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .filter(|relays| !relays.is_empty())
                .unwrap_or(defaults.relays),
            mock: get("RAFI_DRAND_MOCK").is_some_and(|v| v.trim() == "1"),
            lead_secs: number("RAFI_DRAND_LEAD_SECS", defaults.lead_secs),
            timeout_secs: number("RAFI_DRAND_TIMEOUT_SECS", defaults.timeout_secs),
        }
    }

    /// The chain draws use.
    pub fn chain(&self) -> anyhow::Result<ChainInfo> {
        if !self.mock {
            return Ok(chains::quicknet());
        }
        #[cfg(feature = "mock")]
        return Ok(mock_chain().chain().clone());
        #[cfg(not(feature = "mock"))]
        bail!("drand mock mode is not available in this build (enable the `mock-drand` feature)");
    }

    /// The configured relays.
    pub fn parsed_relays(&self) -> anyhow::Result<Vec<Relay>> {
        self.relays
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .map(|r| {
                r.parse()
                    .with_context(|| format!("invalid drand relay {r:?}"))
            })
            .collect()
    }

    /// Checks the settings against the chain.
    pub fn validate(&self) -> anyhow::Result<()> {
        let chain = self.chain()?;
        // Draw events record only the signature, so chained schemes cannot be replayed.
        ensure!(
            !chain.scheme().is_chained(),
            "chained drand schemes are not supported"
        );
        let period = chain.period().as_secs();
        if !self.mock && self.lead_secs < period {
            bail!(
                "drand lead_secs must be at least the chain period ({period}s), or the committed round may already be public"
            );
        }
        ensure!(self.timeout_secs > 0, "drand timeout_secs must be positive");
        if !self.mock {
            ensure!(
                !self.parsed_relays()?.is_empty(),
                "no drand relays configured"
            );
        }
        Ok(())
    }
}

/// Fetches and verifies beacons for Rafi's configured chain.
#[derive(Clone, Debug)]
pub struct DrandClient {
    cfg: DrandConfig,
    client: Client,
}

impl DrandClient {
    /// Validates the configuration and builds the client (fails at startup on
    /// bad settings, a mock request without the `mock` feature, or an
    /// unverifiable chain).
    pub fn new(cfg: DrandConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let chain = cfg.chain()?;
        let builder = Client::builder(chain)
            .strategy(Strategy::default())
            .request_timeout(Duration::from_secs(5))
            .user_agent("rafi (+https://github.com/sagikazarmark/rafi)");
        let client = if cfg.mock {
            #[cfg(feature = "mock")]
            {
                builder.test_transport(mock_chain()).build()?
            }
            #[cfg(not(feature = "mock"))]
            unreachable!("validate rejects mock without the feature")
        } else {
            builder
                .relays(cfg.parsed_relays()?)
                .http_client(reqwest::Client::new())
                .build()?
        };
        Ok(Self { cfg, client })
    }

    pub fn config(&self) -> &DrandConfig {
        &self.cfg
    }

    pub fn is_mock(&self) -> bool {
        self.cfg.mock
    }

    pub fn chain(&self) -> &ChainInfo {
        self.client.chain()
    }

    pub fn chain_hash(&self) -> ChainHash {
        self.client.chain().hash()
    }

    pub fn schedule(&self) -> Schedule {
        self.client.chain().schedule()
    }

    /// Verifies a persisted beacon again (e.g. after a durable step returned it).
    pub fn verifier(&self) -> &Verifier {
        self.client.verifier()
    }

    /// The underlying `drand` client (e.g. for `restate_drand::Drand`).
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The round a draw starting at `now_unix` commits to
    /// ([`Schedule::commit_round`], the same rule as the Restate engine's), and
    /// when it is due (Unix seconds).
    pub fn target_round(&self, now_unix: u64) -> (u64, u64) {
        let schedule = self.schedule();
        let now = UnixTime::from_secs(i64::try_from(now_unix).unwrap_or(i64::MAX));
        let round = schedule.commit_round(now, Duration::from_secs(self.cfg.lead_secs));
        let due = schedule
            .round_time(round)
            .and_then(|t| u64::try_from(t.as_secs()).ok())
            .unwrap_or_default();
        (round, due)
    }

    /// One attempt across the relays, no sleeping (for a durable retried step).
    pub async fn fetch(&self, round: u64) -> Result<Fetch, FetchError> {
        self.client.fetch(round).await
    }

    /// Waits until `round` is due, then fetches it for up to `timeout_secs`
    /// (the same budget the Restate engine retries for after its durable sleep).
    pub async fn wait_for(&self, round: u64) -> Result<VerifiedBeacon, FetchError> {
        let until_due = self
            .schedule()
            .round_time(round)
            .map(|due| due.saturating_duration_since(UnixTime::from(SystemTime::now())))
            .unwrap_or_default();
        let timeout = until_due + Duration::from_secs(self.cfg.timeout_secs);
        self.client.wait_for(round, timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_flat_variables() {
        let vars = |name: &str| match name {
            "RAFI_DRAND_RELAYS" => Some(" v2:https://a , ,v1:https://b".to_owned()),
            "RAFI_DRAND_MOCK" => Some("1".to_owned()),
            "RAFI_DRAND_LEAD_SECS" => Some("9".to_owned()),
            "RAFI_DRAND_TIMEOUT_SECS" => Some("soon".to_owned()),
            _ => None,
        };
        let cfg = DrandConfig::from_lookup(vars);
        assert_eq!(cfg.relays, ["v2:https://a", "v1:https://b"]);
        assert!(cfg.mock);
        assert_eq!(cfg.lead_secs, 9);
        assert_eq!(
            cfg.timeout_secs,
            DrandConfig::default().timeout_secs,
            "unparsable is the default"
        );
        assert_eq!(DrandConfig::from_lookup(|_| None), DrandConfig::default());
    }

    #[test]
    fn config_deserializes_with_defaults() {
        let cfg: DrandConfig = serde_json::from_str(r#"{"lead_secs": 9}"#).unwrap();
        assert_eq!(cfg.lead_secs, 9);
        assert_eq!(cfg.relays, DrandConfig::default().relays);
        assert!(
            serde_json::from_str::<DrandConfig>(r#"{"lead": 9}"#).is_err(),
            "unknown fields are rejected"
        );
    }

    #[test]
    fn relays_parse_with_and_without_prefix() {
        let cfg = DrandConfig {
            relays: vec![
                "https://drand.cloudflare.com".into(),
                " v2:https://api.drand.sh/".into(),
                "".into(),
            ],
            ..DrandConfig::default()
        };
        let relays = cfg.parsed_relays().unwrap();
        assert_eq!(
            relays.len(),
            2,
            "empty entries (a trailing comma) are ignored"
        );
        assert_eq!(relays[0].to_string(), "v1:https://drand.cloudflare.com");
        assert_eq!(relays[1].to_string(), "v2:https://api.drand.sh");
        assert!(DrandClient::new(cfg).is_ok());
    }

    #[test]
    fn real_chains_need_a_lead_of_at_least_one_period() {
        for lead in [0, 1, 2] {
            let cfg = DrandConfig {
                lead_secs: lead,
                ..DrandConfig::default()
            };
            assert!(cfg.validate().is_err(), "lead {lead}");
        }
        assert!(DrandConfig::default().validate().is_ok());
        let cfg = DrandConfig {
            relays: vec!["ftp://nope".into()],
            ..DrandConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn quicknet_target_round_matches_the_old_rule() {
        let client = DrandClient::new(DrandConfig::default()).unwrap();
        let g = 1_692_803_367u64;
        for now in g..g + 30 {
            let (round, due) = client.target_round(now);
            assert_eq!(round, (now + 6 - g) / 3 + 1);
            assert!(due > now && due <= now + 6);
        }
    }

    /// Both engines commit with [`Schedule::commit_round`]: the Restate engine
    /// (`restate_drand::Drand::commit_round`) reads its clock in milliseconds,
    /// the local engine ([`DrandClient::target_round`]) in whole seconds. Both
    /// must pick the same round, and a due time not yet past, at any instant.
    #[test]
    fn both_engines_commit_to_the_same_round() {
        let q = chains::quicknet();
        let genesis_ms = q.genesis_time().as_secs() * 1000;
        for lead_secs in [3u64, 6, 7] {
            let client = DrandClient::new(DrandConfig {
                lead_secs,
                ..DrandConfig::default()
            })
            .unwrap();
            for offset_ms in (0..20_000).step_by(137) {
                let now_ms = genesis_ms + 1_000_000_000 + offset_ms;
                let restate = q.schedule().commit_round(
                    UnixTime::from_millis(now_ms),
                    Duration::from_secs(lead_secs),
                );
                let (local, due) = client.target_round(u64::try_from(now_ms / 1000).unwrap());
                assert_eq!(local, restate, "lead {lead_secs}, now {now_ms} ms");
                assert!(
                    due * 1000 >= u64::try_from(now_ms).unwrap(),
                    "a lead of a period or more commits to a round not out yet"
                );
            }
        }
    }

    #[cfg(not(feature = "mock"))]
    #[test]
    fn mock_needs_the_feature() {
        let cfg = DrandConfig {
            mock: true,
            lead_secs: 0,
            ..DrandConfig::default()
        };
        assert!(DrandClient::new(cfg).is_err());
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn mock_chain_serves_verified_past_rounds() {
        let client = DrandClient::new(DrandConfig {
            mock: true,
            lead_secs: 0,
            ..DrandConfig::default()
        })
        .unwrap();
        assert!(client.chain_hash() != chains::QUICKNET_HASH);
        assert_eq!(
            resolve_chain(&client.chain_hash()),
            Some(client.chain().clone())
        );
        let Fetch::Ready(b) = client.fetch(7).await.unwrap() else {
            panic!("past rounds are ready")
        };
        assert_eq!(b.chain_hash(), client.chain_hash());
        assert_eq!(
            mock_chain().chain().hash(),
            client.chain_hash(),
            "the mock chain is pinned"
        );
    }
}
