//! drand quicknet beacon client.
//!
//! Every configured relay is asked for the round; all that answer must agree.
//! A future round is reported as [`Fetch::NotYet`] (HTTP 425), so callers can
//! retry with their own policy: a durable Restate `run` retry, or
//! [`DrandClient::fetch_until`] in-process.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use rafi_core::drand::{Beacon, QUICKNET_CHAIN, randomness_from_signature, round_time};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Beacon and timing settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DrandConfig {
    /// Relay base URLs; every reachable relay must agree on the beacon.
    pub relays: Vec<String>,
    /// Use deterministic fake beacons (development and tests only: NOT verifiable).
    pub mock: bool,
    /// Seconds between starting a draw and the committed round (>= 3 for fairness).
    pub lead_secs: u64,
    /// Give up fetching a beacon after this many seconds.
    pub timeout_secs: u64,
}

impl Default for DrandConfig {
    fn default() -> Self {
        Self {
            relays: vec![
                "https://api.drand.sh".into(),
                "https://drand.cloudflare.com".into(),
            ],
            mock: false,
            lead_secs: 6,
            timeout_secs: 60,
        }
    }
}

/// Result of a single fetch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    Ready(Beacon),
    /// The round has not been published yet (HTTP 425).
    NotYet,
}

#[derive(Clone)]
pub struct DrandClient {
    http: reqwest::Client,
    cfg: DrandConfig,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl DrandClient {
    pub fn new(mut cfg: DrandConfig) -> Self {
        for relay in &mut cfg.relays {
            *relay = relay.trim().trim_end_matches('/').to_owned();
        }
        cfg.relays.retain(|r| !r.is_empty());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("rafi (+https://github.com/sagikazarmark/rafi)")
            .build()
            .expect("http client");
        Self { http, cfg }
    }

    pub fn config(&self) -> &DrandConfig {
        &self.cfg
    }

    pub fn is_mock(&self) -> bool {
        self.cfg.mock
    }

    /// One attempt to fetch `round` from every relay. All relays that answer
    /// must agree; at least one must answer.
    pub async fn fetch(&self, round: u64) -> anyhow::Result<Fetch> {
        if self.cfg.mock {
            if round_time(round) > now() && self.cfg.lead_secs > 0 {
                return Ok(Fetch::NotYet);
            }
            return Ok(Fetch::Ready(mock_beacon(round)));
        }
        if self.cfg.relays.is_empty() {
            bail!("no drand relays configured");
        }
        let mut beacons = Vec::new();
        let mut not_yet = false;
        let mut errors = Vec::new();
        for relay in &self.cfg.relays {
            let url = format!("{relay}/{QUICKNET_CHAIN}/public/{round}");
            match self.http.get(&url).send().await {
                Ok(resp) if resp.status().as_u16() == 425 => not_yet = true,
                Ok(resp) if resp.status().is_success() => match resp.json::<Beacon>().await {
                    Ok(b) => beacons.push((relay.clone(), b)),
                    Err(e) => errors.push(format!("{relay}: {e}")),
                },
                Ok(resp) => errors.push(format!("{relay}: HTTP {}", resp.status())),
                Err(e) => errors.push(format!("{relay}: {e}")),
            }
        }
        let Some((_, first)) = beacons.first() else {
            if not_yet {
                return Ok(Fetch::NotYet);
            }
            bail!("no drand relay answered: {}", errors.join("; "));
        };
        for (relay, b) in &beacons[1..] {
            if b.signature != first.signature {
                bail!("drand relays disagree on round {round} ({relay})");
            }
        }
        first.checked_randomness(round).context("invalid beacon")?;
        Ok(Fetch::Ready(first.clone()))
    }

    /// Waits until `round` is published and returns its beacon, retrying until
    /// the configured timeout.
    pub async fn fetch_until(&self, round: u64) -> anyhow::Result<Beacon> {
        let wait = round_time(round) as i64 - now() as i64;
        if wait > 0 && !(self.cfg.mock && self.cfg.lead_secs == 0) {
            tokio::time::sleep(Duration::from_millis(wait as u64 * 1000 + 300)).await;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.cfg.timeout_secs);
        let mut delay = Duration::from_millis(250);
        loop {
            match self.fetch(round).await {
                Ok(Fetch::Ready(b)) => return Ok(b),
                Ok(Fetch::NotYet) => {}
                Err(e) => tracing::warn!(round, error = %e, "drand fetch failed"),
            }
            if tokio::time::Instant::now() + delay > deadline {
                return Err(anyhow!(
                    "drand round {round} unavailable after {}s",
                    self.cfg.timeout_secs
                ));
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(2));
        }
    }
}

/// Deterministic fake beacon used with `mock = true`.
pub fn mock_beacon(round: u64) -> Beacon {
    let a = Sha256::digest([b"rafi-mock-drand".as_slice(), &round.to_be_bytes()].concat());
    let b = Sha256::digest(a);
    let sig = [a.as_slice(), &b[..16]].concat();
    Beacon {
        round,
        randomness: hex::encode(randomness_from_signature(&sig)),
        signature: hex::encode(sig),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_beacons_are_valid() {
        let b = mock_beacon(42);
        assert!(b.checked_randomness(42).is_ok());
        assert_eq!(b, mock_beacon(42));
    }

    #[tokio::test]
    async fn mock_client_serves_past_rounds() {
        let client = DrandClient::new(DrandConfig {
            mock: true,
            lead_secs: 0,
            ..DrandConfig::default()
        });
        assert_eq!(client.fetch(7).await.unwrap(), Fetch::Ready(mock_beacon(7)));
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
}
