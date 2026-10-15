//! [ALTCHA](https://altcha.org) v2 proof-of-work for Dioxus.
//!
//! The server issues a challenge (with the [`altcha`] crate, re-exported
//! here), the browser solves it, and the form posts the solution back as
//! base64 JSON, the value the official widget posts. The server verifies it
//! with [`altcha::verify_solution`] and records the challenge's nonce so it's
//! accepted only once.
//!
//! - [`solve`] runs in Web Workers on WebCrypto ([`WORKER_JS`]), off the main
//!   thread, falling back to the main thread when workers can't start. It
//!   supports `PBKDF2/SHA-*` (and iterated `SHA-*`, which is slow above
//!   `cost` 1); not `SCRYPT` or `ARGON2ID`.
//! - [`use_altcha`] and [`AltchaField`] put it in a form.
//!
//! The challenge stays raw JSON on the client ([`RawChallenge`]): parsing it
//! into [`altcha::Challenge`] would drop fields it doesn't know and break the
//! signature for a server on another version.
//!
//! WebCrypto needs a secure context (HTTPS or localhost). The workers start
//! from a `blob:` URL, which a Content Security Policy must allow
//! (`worker-src blob:`); otherwise serve [`WORKER_JS`] as a file and set
//! [`SolveOptions::worker_url`].

mod fetch;
mod hook;
mod solve;

use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub use altcha;
pub use fetch::{FetchError, fetch_challenge};
pub use hook::{
    AltchaError, AltchaField, AltchaOptions, AltchaState, Auto, UseAltcha, use_altcha,
    use_altcha_with_options,
};
pub use solve::solve;

/// The solver script. Serve it yourself and set [`SolveOptions::worker_url`]
/// when a Content Security Policy forbids `blob:` workers. Loaded on a page,
/// it defines `altchaSolve(challenge, start, step)`, returning a promise of
/// the solution.
///
/// Its worker protocol, JSON strings both ways, is part of this crate's
/// contract: in, `{"challenge": .., "start": i, "step": n}`; out, a
/// [`Solution`](altcha::Solution) or `{"error": ".."}`.
pub const WORKER_JS: &str = include_str!("worker.js");

/// Whether [`solve`] can start workers from an embedded copy of
/// [`WORKER_JS`] (the `embed-worker` feature). Without it,
/// [`SolveOptions::worker_url`] is required in the browser.
pub const EMBEDS_WORKER: bool = cfg!(feature = "embed-worker");

/// The largest form value [`decode_payload`] accepts, in bytes. A challenge's
/// `data` is free-form, so a client could inflate it.
pub const MAX_PAYLOAD_LEN: usize = 8 * 1024;

/// A challenge as the server sent it, passed through untouched.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawChallenge(Box<RawValue>);

impl PartialEq for RawChallenge {
    fn eq(&self, other: &Self) -> bool {
        self.0.get() == other.0.get()
    }
}

impl RawChallenge {
    /// Wraps a challenge's JSON; `Err` if it isn't JSON.
    pub fn from_json(json: impl Into<String>) -> Result<Self, SolveError> {
        RawValue::from_string(json.into())
            .map(Self)
            .map_err(|e| SolveError::InvalidChallenge(e.to_string()))
    }

    /// Serializes a challenge, e.g. one just issued in the same app.
    pub fn from_challenge(challenge: &altcha::Challenge) -> Self {
        let json = serde_json::to_string(challenge).expect("a Challenge always serializes");
        Self::from_json(json).expect("serde_json writes JSON")
    }

    /// The challenge's JSON.
    pub fn json(&self) -> &str {
        self.0.get()
    }

    /// When it expires, in Unix seconds: `None` if it doesn't (absent or 0).
    pub fn expires_at(&self) -> Option<i64> {
        self.info().ok()?.parameters.expires_at.filter(|&t| t != 0)
    }

    /// How long a solution can still be posted, judged by this device's
    /// clock: until expiry, less a margin (30 s, at most half the time left)
    /// so it doesn't expire in transit. `None` if the challenge doesn't
    /// expire, or the device's clock is too far off to tell (more than a day
    /// left, or already past).
    pub fn usable_for(&self) -> Option<Duration> {
        let left = hook::clock::deadline(0.0, self.expires_at(), hook::clock::wall_ms())?;
        Some(Duration::from_secs_f64(left.max(0.0) / 1000.0))
    }

    /// The two fields the client reads.
    fn info(&self) -> Result<Info, SolveError> {
        serde_json::from_str(self.0.get()).map_err(|e| SolveError::InvalidChallenge(e.to_string()))
    }

    /// Fails unless the algorithm is one [`WORKER_JS`] solves.
    fn check_algorithm(&self) -> Result<(), SolveError> {
        // ASCII case only: `to_uppercase` costs kilobytes of Unicode tables.
        let name = self.info()?.parameters.algorithm;
        let hash = match name.get(..7) {
            Some(head) if head.eq_ignore_ascii_case("PBKDF2/") => &name[7..],
            _ => name.as_str(),
        };
        if ["SHA-256", "SHA-384", "SHA-512"]
            .iter()
            .any(|h| h.eq_ignore_ascii_case(hash))
        {
            Ok(())
        } else {
            Err(SolveError::UnsupportedAlgorithm(name))
        }
    }
}

#[derive(Deserialize)]
struct Info {
    parameters: InfoParameters,
}

#[derive(Deserialize)]
struct InfoParameters {
    algorithm: String,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

/// The form value: base64 (standard, padded) of the JSON
/// `{"challenge": .., "solution": ..}`, with the challenge as received.
pub fn payload(challenge: &RawChallenge, solution: &altcha::Solution) -> String {
    let solution = serde_json::to_string(solution).expect("a Solution always serializes");
    let json = format!(
        r#"{{"challenge":{},"solution":{solution}}}"#,
        challenge.json()
    );
    base64::engine::general_purpose::STANDARD.encode(json)
}

/// Decodes a posted form value, for servers. It doesn't verify anything:
/// pass the result to [`altcha::verify_solution`].
pub fn decode_payload(value: &str) -> Result<altcha::Payload, DecodeError> {
    let value = value.trim();
    if value.len() > MAX_PAYLOAD_LEN {
        return Err(DecodeError::TooLong);
    }
    let json = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| DecodeError::Base64)?;
    serde_json::from_slice(&json).map_err(|e| DecodeError::Json(e.to_string()))
}

/// Why [`decode_payload`] failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    #[error("the payload is longer than {MAX_PAYLOAD_LEN} bytes")]
    TooLong,
    #[error("the payload isn't base64")]
    Base64,
    #[error("the payload isn't a challenge and a solution: {0}")]
    Json(String),
}

/// How [`solve`] runs.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct SolveOptions {
    workers: Option<usize>,
    timeout: Option<Option<Duration>>,
    worker_url: Option<String>,
}

impl SolveOptions {
    /// The default timeout: 90 s, as in `altcha`.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

    pub fn new() -> Self {
        Self::default()
    }

    /// How many workers split the search. Default: the device's
    /// `hardwareConcurrency`, at most 4, or 2 if unknown. (Chromium runs
    /// every worker's PBKDF2 on one thread, so more don't help there.)
    pub fn workers(mut self, n: usize) -> Self {
        self.workers = Some(n.max(1));
        self
    }

    /// Gives up after this much time with the page visible (a hidden page,
    /// e.g. a locked phone, doesn't count). `None`: never. Default: 90 s.
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Loads the workers from this URL, serving [`WORKER_JS`], instead of a
    /// `blob:` URL of the embedded copy. Required without the `embed-worker`
    /// feature.
    pub fn worker_url(mut self, url: impl Into<String>) -> Self {
        self.worker_url = Some(url.into());
        self
    }

    #[cfg_attr(
        not(any(target_arch = "wasm32", feature = "native")),
        allow(dead_code)
    )]
    fn timeout_or_default(&self) -> Option<Duration> {
        self.timeout.unwrap_or(Some(Self::DEFAULT_TIMEOUT))
    }
}

/// Why [`solve`] failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SolveError {
    #[error("unsupported algorithm: {0}")]
    UnsupportedAlgorithm(String),
    #[error("invalid challenge: {0}")]
    InvalidChallenge(String),
    #[error("WebCrypto needs a secure context (HTTPS or localhost)")]
    InsecureContext,
    #[error("solving isn't supported on this target (enable the `native` feature)")]
    UnsupportedTarget,
    #[error("no solution before the timeout")]
    Timeout,
    #[error("the solver failed: {0}")]
    Worker(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge() -> altcha::Challenge {
        altcha::create_challenge(altcha::CreateChallengeOptions {
            cost: 1,
            key_prefix: "0".into(),
            expires_at: Some(4_000_000_000),
            hmac_signature_secret: Some("key".into()),
            ..Default::default()
        })
        .unwrap()
    }

    fn solution(c: &altcha::Challenge) -> altcha::Solution {
        altcha::solve_challenge(altcha::SolveChallengeOptions::new(c))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn payload_round_trips_and_verifies() {
        let c = challenge();
        let s = solution(&c);
        let decoded = decode_payload(&payload(&RawChallenge::from_challenge(&c), &s)).unwrap();
        let result = altcha::verify_solution(altcha::VerifySolutionOptions::new(
            &decoded.challenge,
            &decoded.solution,
            "key",
        ))
        .unwrap();
        assert!(result.verified);
    }

    #[test]
    fn unknown_challenge_fields_survive() {
        let c = challenge();
        let mut json: serde_json::Value = serde_json::to_value(&c).unwrap();
        json["parameters"]["future"] = "field".into();
        json["extra"] = 1.into();
        let raw = RawChallenge::from_json(json.to_string()).unwrap();
        let encoded = payload(&raw, &solution(&c));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let back: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back["challenge"], json);
    }

    #[test]
    fn decode_refuses_junk() {
        assert_eq!(
            decode_payload("not base64!").unwrap_err(),
            DecodeError::Base64
        );
        assert!(matches!(decode_payload("e30="), Err(DecodeError::Json(_)))); // {}
        let long = "A".repeat(MAX_PAYLOAD_LEN + 4);
        assert_eq!(decode_payload(&long).unwrap_err(), DecodeError::TooLong);
    }

    #[test]
    fn reads_algorithm_and_expiry() {
        let raw = RawChallenge::from_challenge(&challenge());
        assert_eq!(raw.expires_at(), Some(4_000_000_000));
        assert_eq!(raw.check_algorithm(), Ok(()));

        let none =
            RawChallenge::from_json(r#"{"parameters":{"algorithm":"SCRYPT","expiresAt":0}}"#)
                .unwrap();
        assert_eq!(none.expires_at(), None);
        assert_eq!(
            none.check_algorithm(),
            Err(SolveError::UnsupportedAlgorithm("SCRYPT".into()))
        );
        for ok in ["SHA-256", "pbkdf2/sha-512", "PBKDF2/SHA-384"] {
            let raw =
                RawChallenge::from_json(format!(r#"{{"parameters":{{"algorithm":"{ok}"}}}}"#))
                    .unwrap();
            assert_eq!(raw.check_algorithm(), Ok(()), "{ok}");
        }
    }
}
