//! Rafi's drand settings on top of [`drand_beacon`]: which chain, which relays,
//! how far ahead a draw commits, and how long a draw waits for its beacon.
//!
//! The chain is quicknet, or (with `mock = true` and the `mock` feature) a
//! signing test chain with a public seed: its beacons are real BLS signatures,
//! but anyone can forge them, and its chain hash marks every raffle using it as
//! not publicly verifiable.

use std::time::Duration;

use anyhow::{Context, bail, ensure};
pub use drand_beacon::client::{Error as FetchError, Fetch};
use drand_beacon::{
    ChainHash, ChainInfo, Schedule, VerifiedBeacon, Verifier, chains,
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

/// The mock (test) chain: pinned parameters, so the app and the draw engine
/// agree on its hash across restarts.
#[cfg(feature = "mock")]
pub fn mock_chain() -> drand_beacon::TestChain {
    let q = chains::quicknet();
    drand_beacon::TestChain::new(
        MOCK_SEED,
        drand_beacon::Scheme::BlsUnchainedG1Rfc9380,
        q.period(),
        q.genesis_time(),
        "rafi-test",
    )
    .expect("supported scheme")
}

/// Chain info for a chain hash found in a commit document: the public drand
/// chains, and the mock chain when this build supports it.
pub fn resolve_chain(hash: &ChainHash) -> Option<ChainInfo> {
    if let Some(info) = chains::by_hash(hash) {
        return Some(info);
    }
    #[cfg(feature = "mock")]
    {
        let mock = mock_chain().info();
        if mock.hash() == *hash {
            return Some(mock);
        }
    }
    None
}

impl DrandConfig {
    /// The chain draws use.
    pub fn chain(&self) -> anyhow::Result<ChainInfo> {
        if !self.mock {
            return Ok(chains::quicknet());
        }
        #[cfg(feature = "mock")]
        return Ok(mock_chain().info());
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
        let period = u64::from(chain.period().get());
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
                .http(reqwest::Client::new())
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

    /// The round a draw starting at `now_unix` commits to, and when it is due.
    pub fn target_round(&self, now_unix: u64) -> (u64, u64) {
        let schedule = self.schedule();
        let round = rafi_core::drand::target_round(&schedule, now_unix, self.cfg.lead_secs);
        (round, rafi_core::drand::round_time(&schedule, round))
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
            .round_time_ms(round)
            .map(|due| due.saturating_sub(now_ms()).max(0) as u64)
            .unwrap_or_default();
        let timeout = Duration::from_millis(until_due) + Duration::from_secs(self.cfg.timeout_secs);
        self.client.wait_for(round, timeout).await
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(client.chain_hash() != chains::QUICKNET);
        assert_eq!(
            resolve_chain(&client.chain_hash()),
            Some(client.chain().clone())
        );
        let Fetch::Ready(b) = client.fetch(7).await.unwrap() else {
            panic!("past rounds are ready")
        };
        assert_eq!(b.chain(), &client.chain_hash());
        assert_eq!(
            mock_chain().info().hash(),
            client.chain_hash(),
            "the mock chain is pinned"
        );
    }
}
