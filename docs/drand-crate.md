# `drand-beacon`: a reusable drand crate (design plan)

> **Historical record.** Names are of their time: `rafi-core` is now `rafi-raffle` (`746f3a0`; the drawing is
> `rafi-drawing`), the `restate-rafi` library is `rafi-engine-restate` (the name stays the native binary's), and the
> commit document is the drawing's manifest. Offline draws were dropped later (`7b8721f`): a stalled draw's only exit
> is an abandon. [PLAN.md](PLAN.md) describes the current state.

> **Published (2026-10-07).** The crate moved to its own repository,
> [sagikazarmark/drand-rs](https://github.com/sagikazarmark/drand-rs), and is published as
> [`drand`](https://crates.io/crates/drand) 0.2 (with [`restate-drand`](https://crates.io/crates/restate-drand) 0.2);
> Rafi depends on the published crates. This plan is kept as written under the working name `drand-beacon`;
> the published API differs in places (e.g. `UnixTime`, `VerifiedBeacon::chain_hash`, `fetch_budget`, the journal
> step names `drand: clock` / `drand: fetch`). Still open: filing the `drand_core` issues and asking drand to list
> the crate.

Status: **approved by all three reviewers** (crypto/protocol, Rust API, Rafi integration) at revision 4, 2026-10-06, with their final non-blocking notes folded in. Revision 4 adds a pool freeze while a draw is pending, abandon/UI/test details (§10.6), and the API nits (Chain error without `#[from]`, `Timeout.last`, `round_at_ms`, `latest()` edge cases, `test_transport` skipping the relay/TLS checks, a pure Send + Sync `TestChain` with `beacon_id`).

**What changed in revision 3:**
- Selective abort: a beacon failure never clears the pending draw (new `Stall` event, §10.6).
- TestChain mock mode, the chain config and the freeze check move into M2.
- Exact `is_retryable` and `NotYet` semantics.
- `Beacon` serde chosen by format (human-readable or binary).
- `ClientBuilder::test_transport`, a millisecond `Clock`, error tidy-ups, `NonZeroU32` period, and TLS checked at build time.
- Rafi's `lead_secs` is checked against the chain period.
- The mock chain's parameters are pinned.

Facts were checked against the live relays, the drand Go source (`crypto/schemes.go`, `common/chain/info.go`, `time.go`), kyber, arkworks 0.6 and the published crates that day. A scratch build verified quicknet r1000, default r1/r2 and evmnet r1000 with arkworks 0.6, using the encodings and hash-to-curve parameters given in §2 and §6.

**What changed since revision 1 (from review):**
- Canonical-encoding rules: a relay must not be able to choose between several randomness values.
- `ChainInfo` can only be built from pinned or explicit sources.
- The client is native-only; the core and the I/O-free `http` module work on wasm and no_std, and never read the clock.
- Split error types, with `is_retryable()`.
- Exact BN254 parameters; BN254 and `watch` moved to 0.2.
- Milestones reordered so Rafi's verification fix ships first.
- The Rafi migration is fully specified: chain resolution, vectors, freeze-time chain check, round arithmetic, config, and a selective-abort fix.

## 1. Why

Rafi has to fetch drand beacons and verify them. Today it only checks `randomness == sha256(signature)`, which anyone can satisfy. It also requires the relays that answer to agree, so a single relay controls the draw whenever the others fail or are blocked. The Rust ecosystem has nothing we can just adopt:

| Crate | State | Gaps |
|---|---|---|
| `drand_core` 0.0.19 (thibmeu, Jul 2026) | Maintained, blocking `ureq`, arkworks | Blocking only; one relay per client; v1 only (v2 payloads fail to parse: requires `randomness` and `schemeID`); no evmnet; `HttpClient` round time `genesis + round*period` is off by one period and `get_by_unix_time` returns the previous round; HTTP is hardwired |
| `drand-verify` 0.6.2 (Nois, Dec 2023) | Verification only, zkcrypto `bls12_381` | Unmaintained; no networking (on purpose); no evmnet; no chain-info model |
| `drand-client-rs` 0.1.0 (2023), `drand` 0.1.0 (2022) | Abandoned | — |

The official Go (`drand/go-clients`) and JS (`drand-client`) clients have several relays with failover or racing, watch streams, v1 and v2, verification rooted in chain info, health checks and chain discovery. No Rust crate does.

**Goal:** one small, correct crate for drand that works in async Rust services, durable executors (Restate, Temporal) and no_std/wasm verifiers. Rafi is the first user.

## 2. Verified facts

- **Live chains:**

  | beacon_id | chain hash | scheme | period | sig / key group | sig / key bytes |
  |---|---|---|---|---|---|
  | `default` | `8990e7a9…b2ce` | `pedersen-bls-chained` | 30s | BLS12-381 G2 / G1 | 96 / 48 compressed |
  | `quicknet` | `52db9ba7…e971` | `bls-unchained-g1-rfc9380` | 3s | BLS12-381 G1 / G2 | 48 / 96 compressed |
  | `evmnet` | `04f1e906…c8c3` | `bls-bn254-unchained-on-g1` | 3s | BN254 G1 / G2 | 64 / 128 uncompressed |

  No live chain uses `pedersen-bls-unchained` or the deprecated `bls-unchained-on-g1`.
- **Chain hash** (all three match): `sha256(u32be(period) ‖ i64be(genesis_time) ‖ public_key ‖ genesis_seed ‖ beacon_id, the last omitted when "" or "default")`. **The scheme is not hashed.** The v1 `groupHash` is the `genesis_seed`.
- **Messages:**
  - Chained: `sha256(prev ‖ u64be(round))`. Go skips `prev` when it is empty. Round 1's `prev` is the genesis seed (32 bytes); an empty prev fails.
  - BLS12-381 unchained: `sha256(u64be(round))`.
  - BN254: `keccak256(u64be(round))`.
  - Randomness is `sha256(signature bytes)` for every scheme, evmnet included.
- **Domain separation tags:** quicknet `BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_NUL_`. Pedersen schemes `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_`. Deprecated `bls-unchained-on-g1`: the G2 tag on G1. evmnet `BLS_SIG_BN254G1_XMD:KECCAK-256_SVDW_RO_NUL_`.
- **BN254 parameters** (live evmnet verifies only with all of these):
  - hash_to_field: L = 48, count = 2.
  - `expand_message_xmd` with Keccak-256 and **Z_pad = 136 bytes** (the Keccak-256 rate).
  - SVDW with Z = 1 and sgn0 = parity.
  - Q0 + Q1, no cofactor clearing (G1 cofactor 1).
  - G1 points: `x‖y`, 32-byte big-endian each.
  - G2 key: `x.c1‖x.c0‖y.c1‖y.c0` (imaginary part first, the EIP-197 layout); the evmnet key is in the G2 subgroup.
  - arkworks 0.6 has no SVDW. Its `DefaultFieldHasher`/`ExpanderXmd` uses Z_pad = L rather than the hash block size. That is right for SHA-256/BLS12-381 only by coincidence (64 = 64) and wrong for Keccak/BN254, so we must not use it for BN254.
- **HTTP API:**
  - v1: `{base}/{chain}/info`, `{base}/{chain}/public/{round|latest}`, `{base}/chains`. Beacons include `randomness`; info uses `hash`, `groupHash`, `schemeID`, `metadata.beaconID`.
  - v2: `{base}/v2/chains`, `/v2/chains/{hash}/{info|health|rounds/{n|latest|next}}`, and the same paths under `/v2/beacons/{id}`. Beacons omit `randomness`; info uses `chain_hash`, `genesis_seed`, `scheme`, `beacon_id`.
  - **`/public/0` returns the latest beacon.**
- **Relays:**
  - api.drand.sh, api2 and api3 serve v1 and v2; drand.cloudflare.com serves v1 for all three chains.
  - A far-future round gets 425 (drand.sh) or 404 (Cloudflare).
  - Within about 2.5s of the due time, relays long-poll and then return 200, 425 or 500. Beacons appear about 0.1–0.9s after the nominal time.
  - `rounds/next` long-polled and returned 500, so we don't rely on it. `latest` is cached (`max-age=2`).

## 3. Scope

**0.1:**
- Chain model with built-in pinned chains.
- Round arithmetic.
- v1/v2 beacon and info parsing.
- Strict verification of `default`, `quicknet` and `pedersen-bls-unchained` (BLS12-381).
- I/O-free `http` module (URLs and response classification).
- Native async `reqwest` client: `fetch`, `latest`, `wait_for`, hedged failover.
- Deterministic `TestChain`.

**0.2:** `bn254` feature (evmnet), whose parameters are already pinned in §2; `watch` stream; health and chain discovery; `ClientBuilder::chain_hash` (fetch and pin a chain that isn't built in).

**Out:** libp2p gossipsub, gRPC, timelock (`tlock` exists), the `blst` backend, response caching, verifying the deprecated `bls-unchained-on-g1` (it gets a named `Scheme` variant but is reported as unsupported).

## 4. Packaging

- **Name:** `drand-beacon` (decided 2026-10-07; free on crates.io).
  - **Why it fits:** it names what the crate handles (fetching and verifying beacons), and it still reads right when the client feature is off.
  - **Rejected:**
    - `drand-client`: suggests the official drand JS/Go clients, and undersells the client-less core.
    - `drand`: a transfer is uncertain, the name implies an official implementation, and it collides with the Rust node's package name.
    - `drand2`: confusable with drand's API v2 and Go `drand/v2`.
    - `deerand`: looks related to thibmeu's `dee`, and reads like a typosquat for a crypto crate.
  - **Keywords:** `drand`, `randomness`, `bls`, `beacon`, `verifiable`. The description mentions "client" for search.
- **Shape:** one crate with features. License MIT OR Apache-2.0. MSRV 1.85 (edition 2024), which is also the floor set by hashbrown 0.17 via ark-ec, reqwest 0.13 and rustls-platform-verifier.
- **Where:** develop as `crates/drand-beacon` in this workspace with no `workspace = true` inheritance and its own CI job, so extracting it before publishing 0.1.0 is a plain move.

| Feature | Default | Enables |
|---|---|---|
| `bls12-381` | yes | `ark-bls12-381`, `ark-ec`, `ark-ff`, `ark-serialize` (0.6, `default-features = false`) |
| `client` | no | Native only (`cfg(not(all(target_arch = "wasm32", target_os = "unknown")))`): `reqwest` 0.13 (`default-features = false`, no TLS), `tokio` (`time`), `futures-util`, `tracing` |
| `rustls` / `native-tls` | no | Forward to reqwest's TLS features. A built-in HTTPS client needs one of them; alternatively pass your own `reqwest::Client` |
| `test-chain` | no | `TestChain` (signing; implies `bls12-381`); `ClientBuilder::test_transport` when `client` is also on |
| `bn254` (0.2) | no | `ark-bn254`, `sha3` 0.10, our XMD and SVDW |

Always on, and no_std + alloc throughout: `serde` (derive), `serde_json` (alloc), `hex` (alloc), `sha2` 0.10 (pinned: arkworks 0.6 is on `digest` 0.10), `thiserror` 2 (no default features). There is **no `std` feature**: the core never reads the clock and uses `core::error::Error`. The no_std + wasm32 core build was proven in a scratch crate.

## 5. API sketch

```rust
// ── chain ────────────────────────────────────────────────────────────────
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scheme {
    PedersenBlsChained, PedersenBlsUnchained, BlsUnchainedG1Rfc9380,
    BlsBn254UnchainedOnG1,
    BlsUnchainedOnG1Deprecated,     // parsed; Verifier::new → UnsupportedScheme
    Other(UnknownScheme),           // opaque; .as_str()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChainHash([u8; 32]);   // from_bytes, as_bytes, AsRef<[u8]>; Display/FromStr/serde as hex

/// Chain parameters. There is deliberately no `Deserialize`: a ChainInfo
/// comes from `chains::*`, from `from_json` against an expected hash, or from
/// an explicit `new` (caller vouches for the parameters).
#[derive(Clone, Debug, PartialEq, Eq, Serialize /* v2 shape */)]
pub struct ChainInfo { /* private: public_key, period: NonZeroU32, genesis_time, genesis_seed, scheme, beacon_id */ }
impl ChainInfo {
    pub fn new(public_key: Vec<u8>, period: NonZeroU32, genesis_time: i64, genesis_seed: Vec<u8>,
               scheme: Scheme, beacon_id: impl Into<String>) -> Self;
    /// Parses a v1 or v2 /info body; errors unless the computed hash == `expected`.
    pub fn from_json(body: &[u8], expected: &ChainHash) -> Result<Self, Error>;
    pub fn hash(&self) -> ChainHash;          // always computed, never read from JSON
    pub fn schedule(&self) -> Schedule;
    pub fn scheme(&self) -> &Scheme;  /* + getters for the other fields */
}

pub mod chains {
    pub fn quicknet() -> ChainInfo;           // owned (cheap; no LazyLock, so no_std works)
    pub fn default_chain() -> ChainInfo;
    pub fn evmnet() -> ChainInfo;             // verifying it needs `bn254` (0.2)
    pub fn by_hash(hash: &ChainHash) -> Option<ChainInfo>;
    pub const QUICKNET: ChainHash; pub const DEFAULT: ChainHash; pub const EVMNET: ChainHash;
}

// ── schedule (pure; callers pass `now`) ──────────────────────────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule { genesis_time: i64, period: NonZeroU32 }   // no division by zero
impl Schedule {
    pub fn round_at(&self, unix_secs: i64) -> Option<u64>;   // latest due at or before; None before genesis
    /// `round_at(now_ms.div_euclid(1000))`; the only ms→s conversion callers should use.
    pub fn round_at_ms(&self, unix_ms: i64) -> Option<u64>;
    pub fn round_time_ms(&self, round: u64) -> Option<i64>;
    pub fn round_time(&self, round: u64) -> Option<i64>;     // None for round 0 or overflow
    /// First round due strictly after `unix_secs` (== round_at(t) + 1 after genesis).
    /// Its randomness is unknown at `unix_secs`, assuming fewer than a threshold of
    /// drand nodes collude and the local clock is right. Leave at least one period of margin.
    pub fn round_after(&self, unix_secs: i64) -> u64;
}

// ── beacon ───────────────────────────────────────────────────────────────
/// Untrusted beacon. Serde branches on `is_human_readable()`:
/// - Human-readable (JSON etc.): serializes the v1 shape (hex, with `randomness`,
///   `previous_signature` skipped when None); deserializes v1, v2, its own output
///   and Rafi's old `{round, signature, randomness}` journal shape.
///   `randomness`, if present, must equal sha256(signature); `previous_signature: ""` = None.
/// - Binary (postcard, bincode): a fixed `(round, signature bytes, Option<bytes>)`
///   tuple, never skipped, never `deserialize_any`.
/// `VerifiedBeacon` and `ChainHash` (hex string vs `[u8; 32]`) follow the same rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beacon { pub round: u64, pub signature: Vec<u8>, pub previous_signature: Option<Vec<u8>> }
impl Beacon {
    pub fn from_hex(round: u64, signature: &str, previous_signature: Option<&str>) -> Result<Self, Error>;
    pub fn randomness(&self) -> [u8; 32];
}

/// Obtainable only from `Verifier`; Serialize (beacon fields + `chain`), never Deserialize.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerifiedBeacon { /* chain: ChainHash, beacon: Beacon, randomness: [u8; 32] */ }
impl VerifiedBeacon {
    pub fn round(&self) -> u64; pub fn randomness(&self) -> &[u8; 32];
    /// Hash of the verifying chain. Callers that accept several chains must compare it.
    pub fn chain(&self) -> &ChainHash;
    pub fn beacon(&self) -> &Beacon; pub fn into_beacon(self) -> Beacon;
}

// ── verify ───────────────────────────────────────────────────────────────
#[derive(Clone, Debug)]
pub struct Verifier { /* decoded, subgroup-checked key; scheme; chain hash; genesis seed */ }
impl Verifier {
    /// Rejects unsupported schemes and keys that are malformed, non-canonical,
    /// the wrong length for the scheme's group, at infinity, or outside the subgroup.
    pub fn new(chain: &ChainInfo) -> Result<Self, Error>;
    /// For unchained schemes any `previous_signature` is ignored, and the
    /// VerifiedBeacon carries None, so no unauthenticated bytes are kept.
    pub fn verify(&self, beacon: &Beacon) -> Result<VerifiedBeacon, Error>;
    /// Also requires `beacon.round == round`.
    pub fn verify_round(&self, round: u64, beacon: &Beacon) -> Result<VerifiedBeacon, Error>;
}

/// Core error: no_std, Clone + PartialEq + Eq + Send + Sync + 'static.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[non_exhaustive] Parse { what: &'static str },
    #[non_exhaustive] Length { what: &'static str, expected: usize, got: usize },
    RoundZero, RandomnessMismatch,
    #[non_exhaustive] RoundMismatch { expected: u64, got: u64 },
    #[non_exhaustive] ChainHashMismatch { expected: ChainHash, got: ChainHash },
    UnsupportedScheme(Scheme), BadPublicKey, NonCanonicalSignature,
    InvalidSignature, MissingPreviousSignature,
}

// ── http (I/O-free; usable on wasm with any HTTP stack) ──────────────────
pub mod http {
    #[non_exhaustive] #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ApiVersion { V1, V2 }
    #[non_exhaustive] #[derive(Clone, Copy, Debug)] pub enum RoundRef { Number(NonZeroU64), Latest }
    #[derive(Clone, Debug)] pub struct Relay { /* base (validated http/https, no trailing slash), version */ }
    impl Relay {
        pub fn v1(base: &str) -> Result<Self, Error>;  pub fn v2(base: &str) -> Result<Self, Error>;
        pub fn info_url(&self, chain: &ChainHash) -> String;
        pub fn round_url(&self, chain: &ChainHash, round: RoundRef) -> String;
    }
    pub mod relays {  // per chain: all four serve default, quicknet and evmnet today
        pub fn defaults(chain: &ChainHash) -> Vec<Relay>; // api, api2, api3 (v2), cloudflare (v1); empty for unknown chains
    }
    pub const MAX_BODY: usize = 64 * 1024;

    #[non_exhaustive] pub enum Answer { Beacon(Beacon), NotYet, NotFound, Retry, Unexpected(u16) }
    /// Classifies one HTTP answer; pass the request time `now_ms` (unix ms):
    /// 2xx → parse (a numeric request must match the round) → Beacon; a bad body → Err(crate::Error)
    /// 425 → NotYet (whatever the time; the caller decides what a late NotYet means)
    /// 404 → NotYet while now < due + GRACE (2s), else NotFound
    /// 5xx or 429 → Retry
    /// any other status → Unexpected(status)
    pub fn classify(req: RoundRef, status: u16, body: &[u8], schedule: &Schedule, now_ms: i64) -> Result<Answer, crate::Error>;
}

// ── client (feature "client", native only) ───────────────────────────────
// On wasm32-unknown-unknown the module is cfg'd out without an error, so
// mixed-target workspaces still build. TLS features forward weakly (`reqwest?/rustls`).
pub mod client {
    pub use reqwest;                         // public dependency, pinned to 0.13

    #[non_exhaustive] pub enum Fetch { Ready(VerifiedBeacon), NotYet { due_at_ms: i64 } }

    /// Ask relays in order; start the next after `hedge_delay` or on failure;
    /// the first verified beacon wins. A zero delay asks all at once.
    #[non_exhaustive] #[derive(Clone, Debug)]
    pub struct Strategy { pub hedge_delay: Duration }
    impl Strategy { pub fn hedged(delay: Duration) -> Self; }
    impl Default for Strategy { /* 500ms */ }

    /// Wall clock in unix milliseconds. The default reads SystemTime. A test clock
    /// must advance with tokio's (paused) time: `wait_for` sleeps on tokio and
    /// computes due times from the Clock.
    pub trait Clock: Send + Sync + 'static { fn now_unix_ms(&self) -> i64; }

    #[derive(Clone)] pub struct Client { /* Arc<Inner>: transport, ChainInfo, Verifier, relays, Strategy, timeout, Clock */ }
    impl Client {
        /// Chain must come from `chains::*`, `ChainInfo::from_json` or `ChainInfo::new`.
        pub fn builder(chain: ChainInfo) -> ClientBuilder;
        pub fn chain(&self) -> &ChainInfo;
        pub fn verifier(&self) -> &Verifier;
        /// One attempt, no sleeping: what a durable executor's retried step calls.
        /// Ready if any relay gave a verified beacon. NotYet if none did and
        /// now < due + grace, where `now` is read when the attempt finishes (each answer is
        /// still classified with its own request time). Otherwise Err(client::Error::Relays).
        pub async fn fetch(&self, round: u64) -> Result<Fetch, client::Error>;
        /// Fetches round_at(now); on NotYet falls back to round - 1. Never returns NotYet.
        /// Before genesis, or when round 1 is not yet available: Err(NotStarted), which is retryable.
        pub async fn latest(&self) -> Result<VerifiedBeacon, client::Error>;
        /// Sleeps until due + 250ms, then calls `fetch` with jittered backoff (250ms ×2,
        /// max 2s). Retries on NotYet and on retryable errors; stops on non-retryable
        /// errors or after `timeout`.
        pub async fn wait_for(&self, round: u64, timeout: Duration) -> Result<VerifiedBeacon, client::Error>;
    }
    impl ClientBuilder {
        pub fn relays(self, relays: Vec<Relay>) -> Self;     // default http::relays::defaults(chain)
        pub fn strategy(self, s: Strategy) -> Self;
        /// Applied per request, so it also covers a client passed via `.http(..)`. Default 5s;
        /// must exceed the ~2.5s relay long-poll.
        pub fn request_timeout(self, d: Duration) -> Self;
        pub fn user_agent(self, ua: impl Into<String>) -> Self;  // also per request
        pub fn http(self, c: reqwest::Client) -> Self;
        pub fn clock(self, c: impl Clock) -> Self;
        /// In-process transport serving a TestChain: NotYet before due, signed beacons after.
        /// Uses the builder's Clock and Strategy.
        /// `build()` then skips the relay and TLS checks.
        #[cfg(feature = "test-chain")] pub fn test_transport(self, chain: TestChain) -> Self;
        /// Errors: unsupported scheme or bad key (Chain), no relays, no TLS feature
        /// enabled and no `.http(..)` client, or a reqwest build failure.
        pub fn build(self) -> Result<Client, client::Error>;
    }
    // All returned futures are Send (checked at compile time).

    #[non_exhaustive] #[derive(Debug, thiserror::Error)]
    pub enum Error {
        /// Build-time chain problem (UnsupportedScheme, BadPublicKey) or RoundZero.
        /// No `#[from]`: per-relay verification failures must become
        /// `RelayErrorKind::Invalid`, never a terminal error through a stray `?`.
        #[error(transparent)] Chain(crate::Error),
        #[error("client configuration: {0}")] Config(&'static str),          // no relays, no TLS
        #[error("building the HTTP client")] Http(#[source] reqwest::Error),
        /// The round is due + grace and no relay produced a beacon.
        #[error("all {} relays failed", .0.len())] Relays(Vec<RelayError>),
        /// `latest()` before genesis, or before round 1 is available.
        #[error("chain has not started")] NotStarted,
        /// wait_for gave up; `last` tells "drand stalled" (NotYet kinds) from "relays unreachable".
        #[error("round {round} unavailable before the timeout")] Timeout { round: u64, last: Vec<RelayError> },
    }
    impl Error {
        /// Relays, NotStarted and Timeout → true (a due round always comes into existence
        /// eventually; a false "terminal" is worse than one extra retry).
        /// Chain, Config, Http → false.
        pub fn is_retryable(&self) -> bool;
    }

    #[non_exhaustive] #[derive(Debug)]
    pub struct RelayError { pub relay: String, pub kind: RelayErrorKind }
    #[non_exhaustive] #[derive(Debug)]
    pub enum RelayErrorKind {
        NotYet,                              // a NotYet answer (425, or 404 inside its grace window) counted after due + grace
        NotFound, Status(u16), Timeout, BodyTooLarge, Invalid(crate::Error),
        Transport(Box<dyn core::error::Error + Send + Sync>),
    }
}

// ── test chain (feature "test-chain") ────────────────────────────────────
/// Deterministic chain whose secret key comes from `seed`. Anyone with the seed
/// can forge its beacons: tests and demos only. Its hash differs from every real chain.
pub struct TestChain { /* sk, ChainInfo */ }
impl TestChain {
    pub fn new(seed: [u8; 32], scheme: Scheme, period: NonZeroU32, genesis_time: i64,
               beacon_id: impl Into<String>) -> Result<Self, Error>;
    pub fn info(&self) -> ChainInfo;
    /// Pure, no memo, so TestChain is Send + Sync (asserted at compile time).
    /// Unchained: O(1). Chained: O(round); keep genesis_time near now.
    pub fn sign(&self, round: u64) -> Beacon;
}
```

## 6. Verification details

- **Backend:** arkworks 0.6, which is pure Rust and works on wasm32 and no_std. No arkworks type appears in the public API, so a `blst` backend can come later.
- **Pairing checks:** one `multi_pairing` product equal to identity. Signature on G1: `e(sig, −g2)·e(H(m), pk) == 1`. Signature on G2: `e(−g1, sig)·e(pk, H(m)) == 1`.
- **Canonical encodings (required).** Randomness is `sha256(signature bytes)`, so two byte strings for one point would give two "verified" randomness values. A relay would then choose the outcome. Every decoder therefore enforces all of the following:
  - **Exact lengths:** signatures and keys must be 48/96 bytes (BLS12-381) or 64/128 (BN254). arkworks ignores trailing bytes (checked), so we compare lengths before decoding. For chained schemes only, `previous_signature` must be 96 bytes, except at round 1, where it must be 32 bytes and equal to `genesis_seed`. Unchained schemes ignore it and drop it.
  - **BLS12-381:** ZCash compressed flags must be canonical (arkworks enforces this), and x must be < p (arkworks rejects x+p, checked). `Validate::Yes` ensures the point is on the curve and in the subgroup.
  - **BN254:** every coordinate must be < p. Use `from_bigint`, which returns None at or above p, never `from_be_bytes_mod_order`; kyber also rejects ≥ p. Check the point is on the curve, and check the G2 subgroup explicitly (kyber doesn't).
  - **Infinity** is rejected for keys and signatures. arkworks accepts the canonical infinity encoding, so we check for it explicitly.
  - We reject non-canonical input rather than canonicalize it, so `Beacon.signature` is always exactly what was verified and hashed.
- **Hash-to-curve:**
  - BLS12-381: arkworks `MapToCurveBasedHasher<_, DefaultFieldHasher<Sha256, 128>, WBMap<_>>` with the §2 tags.
  - BN254 (0.2): our own `expand_message_xmd<H: BlockSizeUser>` with `s_in = H::BlockSize`. It is tested against RFC 9380 K.1 (SHA-256) and K.2 (SHA-512, which exercises `s_in ≠ 64`). Then come our hash_to_field (L = 48, count = 2), SVDW (Z = 1) and Q0+Q1. These are checked against live evmnet beacons and kyber/energon. The doc comment says why `DefaultFieldHasher<Keccak256>` is wrong (§2).
- **Trust model:**
  - **Where a Verifier comes from.** Only from a `ChainInfo`, and a `ChainInfo` is either built in, pinned through `from_json(body, &expected)`, or hand-built with `new` (the caller vouches for it). It cannot be deserialized.
  - **Scheme mislabeling** (the hash doesn't cover the scheme). The verifier accepts σ only if σ = sk·H_label(m_label(r, prev)), while the network publishes only sk·H_real(m_real(r′)). Equality needs the same group and tag plus `m_label(r, prev) = m_real(r′)`. That forces r = r′ with an identical message, which chained schemes rule out by requiring a non-empty prev, or a SHA-256 collision. So a mislabel yields either the true randomness or a failure. The key-length check in `Verifier::new` is defence in depth.
  - **Round binding:** the round is in the signed message. `verify_round` and the client also compare it with the requested round. Round 0 is rejected everywhere, because `/public/0` means latest.
- **What a malicious relay can still do:**
  - It can withhold, delay, long-poll, or answer 404/425/500 for a due round, or stream a huge body. Body size is capped at 64 KiB and requests time out.
  - All of that affects liveness only, **provided the application never moves to a different round after a failure**. Otherwise a relay that dislikes a published round can force a redraw (selective abort). The README says this, and Rafi fixes it (§10.6).
  - If t of n League of Entropy nodes collude, they can learn rounds early. That limit holds for any drand user.

## 7. Client behavior

- **Relays:** the defaults per built-in chain are api.drand.sh, api2, api3 (v2) and drand.cloudflare.com (v1). Other chains need explicit relays.
- **`fetch(round)`** rejects round 0, then follows the `Strategy`:
  - Each answer goes through `http::classify` and then `Verifier::verify_round`.
  - An invalid beacon counts as that relay's failure (`RelayErrorKind::Invalid`) and is logged at `warn`; another relay can still win.
  - Valid encodings are unique and BLS signatures are deterministic, so the first verified beacon *is* the answer. Relays never need to agree.
  - If there is no beacon and `now < due + grace`, the result is `Fetch::NotYet`.
  - Otherwise it is `client::Error::Relays`. A NotYet answer after due + grace is recorded as `RelayErrorKind::NotYet`; this is what a drand stall looks like, and it stays retryable.
- **Retry semantics:** every relay-level failure is retryable (`Relays`, `NotStarted`, `Timeout`), because a round that is due always comes into existence eventually. Only `Chain`, `Config` and `Http` (build-time) errors are terminal.
- **Durable executors:**
  - Call `fetch` inside the retried step.
  - Map `NotYet` and `is_retryable()` errors to retryable failures, and everything else to terminal ones.
  - Keep one attempt's worst case (`relays × request_timeout`, minus the hedging overlap) well under the step's retry budget.
- **Clock:** the `Clock` (unix ms) can be injected; `ClientBuilder::test_transport` with a test clock and `tokio::time::pause` makes client tests deterministic.

## 8. Testing

- **Vectors (checked-in JSON):** several rounds per live chain (default r1/r2, quicknet r1000…, evmnet r1000…) plus info, from Cloudflare v1, drand.sh v1/v2 and `drand-cli get public`. `pedersen-bls-unchained` vectors come from drand's Go test fixtures, or are generated with `drand-cli` against a local test network.
- **Negative vectors:**
  - Bit flips, wrong round, wrong chain, swapped schemes.
  - Signature or key with trailing bytes.
  - BLS12-381 x+p; BN254 x+p and y+p.
  - Canonical infinity; all-zero BN254 infinity.
  - Non-subgroup G2 points.
  - Chained schemes: wrong `previous_signature` length; round-1 prev ≠ genesis seed.
  - Unchained schemes: a bogus `previous_signature` is dropped.
  - `randomness` mismatch; round 0.
- **Property tests:**
  - `Schedule`: `round_at(round_time(r)) == r`, `round_after(t) == round_at(t) + 1` after genesis, `round_time(round_after(t)) > t`.
  - TestChain sign → verify for every supported scheme; any tampering fails.
  - Serde round trips: JSON (v1, v2, the old Rafi shape) and postcard.
- **Client tests:** an in-process axum relay covering 200, 425 (before and after due + grace), 404 before, during and after grace, 500, 429, 403 (`Unexpected`), invalid beacon, wrong round, oversized body, slow relay (hedging), every relay failing, round 0, a missing TLS feature at `build()`, and `is_retryable` for each error. Live tests are `#[ignore]`d.
- **Builds:**
  - `wasm32-unknown-unknown` (core + http, no default features plus `bls12-381`, and once more with `client` on to show it is cfg'd out), a no_std target, MSRV, clippy and docs.
  - A CI check that `cargo tree -e features --target wasm32-unknown-unknown --no-default-features -F bls12-381` contains neither `ark-std/std` nor `getrandom`.
  - After 0.1: `cargo-semver-checks`, cargo-fuzz on the parsers, and a nightly comparison job with `drand-cli`.

## 9. Rafi integration constraints

- **Unchained only.** Rafi supports only unchained schemes, because `Event::Draw` has no `previous_signature`. Config rejects chained chains.
- **Commit margin.** For a real chain, config rejects `lead_secs < period`; a lead of at least 2 × period gives a full period of margin. The default of 6s on quicknet satisfies this. `lead_secs = 0` is allowed only with the test chain, because committing to an already-published round would let the operator pick the outcome.
- **Wasm verify page.** The verify page (Dioxus wasm) uses `rafi-core` → `drand-beacon` with `default-features = false, features = ["bls12-381"]`. The `cargo tree` check (§8) keeps `ark-std/std` and getrandom out. Measure the bundle size before and after in M2.

## 10. Rafi migration

1. **Chain context.**
   - `CommitDoc::new` takes the chain hash as a parameter (commit.rs:44 hard-codes quicknet); the app sets it from its configured chain and is the only writer of `drand_chain`.
   - `DrawState::freeze` records `drand_chain`. The field is `#[serde(default)]` (= quicknet) for state already in the Restate K/V store and the SQLite `draw_state` column.
   - `verify(commit_json, list_hash, events, resolve: impl Fn(&ChainHash) -> Option<ChainInfo>)` builds one `Verifier` from `CommitDoc.drand_chain`.
   - The app and the verify page resolve with `chains::by_hash`; tests resolve with their TestChain.
   - When the resolver returns None, `Report` has a distinct `chain_unknown` status and `ok()` is false, so a test can't pass by accident. The verify page renders it as "test chain, not publicly verifiable", visibly distinct from both PASS and FAIL and never shown as a pass. A dishonest operator could commit to a test chain, and this label is the signal.
2. **Verified completion.**
   - `DrawState::complete(&VerifiedBeacon)` checks the round against pending and `VerifiedBeacon::chain()` against the recorded `drand_chain`.
   - Verification runs in two places: right after the journaled `ctx.run("beacon")` result (`client.verifier().verify_round`, then `complete`), and in `verify()`, which rebuilds each `Event::Draw` with `Beacon::from_hex` and verifies it again. Verification is deterministic, so Restate's code replay sees the same result.
3. **Freeze-time chain check.**
   - `Raffle::freeze` (restate-rafi) rejects, with a terminal 409, a commit whose `drand_chain` differs from `client.chain().hash()`.
   - The local engine performs the same check on freeze and when switching engines.
4. **Round arithmetic stays identical.**
   - `round = schedule.round_at(now + lead_secs)`, using the chain's `Schedule` (not quicknet constants), so TestChain's genesis and period drive the sleep and the UI countdown.
   - This keeps `lead_secs = 0` for test-chain tests and keeps replays deterministic.
   - An equivalence test compares it with the old `target_round` for lead ∈ {0, 3, 6, 9}.
5. **`rafi-drand` stays as thin Rafi glue:**
   - It keeps `DrandConfig` (`deny_unknown_fields`, figment `RESTATE_RAFI_DRAND__*`, `RAFI_DRAND_*`).
   - `mock` stays the **only** chain switch: `mock = true` means the test chain (§10.8), and `false` means quicknet. A later `chain` field can replace it, but there are never two knobs.
   - Relays parse with `v1:`/`v2:` prefixes, with **unprefixed URLs meaning v1**, so existing configs and README defaults keep working. The same parsing applies to the app env vars and the endpoint TOML/env.
   - Config validation follows §9 (unchained only, the `lead_secs` rule).
   - It builds the `Verifier` once at startup, so a bad scheme or key fails at boot.
   - **M2:** it keeps its HTTP fetching, but drops the agreement check, verifies each answer (the first relay that verifies wins) and returns the plain `Beacon` (`into_beacon`) as the journaled `ctx.run` value. That beacon is verified again before `complete` (§10.2). It also serves mock beacons from `TestChain::sign`.
   - **M4:** it builds a `drand_beacon::client::Client` (with `test_transport` in mock mode) and deletes its own fetching. The local engine (app engine.rs:101-112) uses `wait_for`.
6. **Selective-abort fix: a beacon failure never clears the pending draw.**
   - **The gap today.** `fail` clears pending (state.rs:365) and doesn't bump `next_k`. After round R is public, the organizer or a withholding relay can then draw a different unit with the same k, void the unit, finish the raffle, or switch engines (`switch_to_local` calls `fail`, app server/service.rs:548-563). Any of these changes the outcome.
   - **Fix.** The Restate `draw` handler (service.rs:184-190) and the local engine (engine.rs:121-126) stop calling `fail` on fetch or verify errors. They append a non-terminal `Event::Stall { k, round, reason }` (version bump, projected) and return, leaving `pending` in place. In Restate, retryable errors are retried until the budget runs out and then recorded as a Stall; non-retryable ones are recorded as a Stall immediately.
   - **Retrying.** Pressing Draw again re-enters `begin`, which is already idempotent for the pending unit and rejects others (state.rs:270-274). The wait saturates to 0 (service.rs:155), so it fetches the **same round** immediately, which is replay-safe. `void` and `finish` already refuse while pending.
   - **Switching engines.** `switch_to_local` keeps the pending draw and the local engine resumes it with `wait_for(pending.round)`.
   - **The only exits** are an offline draw on the same pending (`complete_offline`; the log records "offline draw replaced committed round R") or an explicit, logged "abandon" that calls `fail`. The verify page flags both prominently.
   - **What verify reports.** `verify.rs` accepts `Stall` as a no-op that must match the pending k and round; `verify.py` already ignores unknown event types. For abandoned or offline-replaced draws, `verify` outputs `(k, round, pass, pool)`. The browser page, or `verify.py --fetch`, fetches R and shows the winner it would have picked.
   - **Admin UI.** The alert at admin.rs:750 ("Press Draw again to commit to a new round") changes to "retry round R". The retry and offline buttons target `pending.unit_id` instead of `next_pending` (admin.rs:712-716, 752).
   - **Schema.** No new `Unit` or `DrawState` field beyond the event variant and `drand_chain` (§10.1).
   - **Pool freeze while pending.**
     - The pass-2 pool is computed at settle time from pass-1 outcomes (state.rs:234-245, 328), and `resolve` has no pending check (state.rs:381).
     - So during a Stall, after R is public, resolving a pass-1 winner as NotHere or Declined would change the pending draw's winner.
     - Fix: `resolve` returns `DrawError::DrawInProgress` for NotHere or Declined while `pending.is_some()`. `Claimed` stays allowed, because Drawn and Claimed are both in the pass-2 pool. The admin UI disables those buttons during a pending draw.
     - General invariant: while a draw is pending, no accepted command or event may change any input of the draw function (`list_hash`, `k`, `pass`, `pool`) or the recorded `drand_chain`.
       - A property test applies random commands and events (including `Stall`, `Command::Abandon`, and engine switches) to random pending states.
       - It asserts that those inputs are unchanged, and that Abandon is the only way to clear pending without a Draw event.
       - It also covers `begin` for another unit, `void` and `finish`, which already refuse.
       - `verify()` replays every Resolve through `state.resolve`, so a log that breaks the rule fails public verification.
     - UX consequence: while the next prize's draw is pending (countdown or stall), an earlier undecided winner can be marked Claimed but not NotHere or Declined. The admin tooltip says so, and the flow table in PLAN.md §5 (PLAN.md:124) is updated.
     - Old dev logs with such a resolve now fail `verify()`. They are reset (§10.9), and the vectors.rs header notes the rule.
   - **Abandon plumbing.** A new `Command::Abandon { unit_id }` in core engine.rs, routed through the Restate `resolve` handler (service.rs:203-208) and the local engine, with an audit-log entry.
   - **UI.**
     - `last_failure` (view.rs:157-158) is set on `Stall` too, and kept while the same pending draw is in place.
     - The projector and phone countdowns (screen.rs:203, 235; admin.rs:726) show "retrying round R" instead of sitting at 0.
     - The admin copy says "retrying…": a retry or an offline exit queues behind the in-flight `draw` invocation on the virtual object, which can take up to lead plus the retry budget.
     - An offline draw on a pending draw whose round is already due needs an in-page confirmation step, because it is the one remaining reroll lever and is flagged publicly.
     - (Since `7b8721f` there are no offline draws, so this lever is gone; abandon, logged with the round it gave up, is the only exit.)
   - **Tests.** Tests that relied on `switch_to_local` failing the draw are updated. A new test switches engines with a stalled draw and checks that the local engine resumes the same round and the store ignores late Restate writes (store lib.rs:43).
7. **Fixtures.**
   - `testdata/draw-vectors.json` is regenerated from real quicknet beacons (rounds 1000, 1005, 1010, 1012, 1020) checked in alongside it. `rafi-draw-v1` is unchanged (same algorithm), and the vectors.rs header says so.
   - The core unit and property tests (state.rs:488, verify.rs:155) use TestChain through a `test-chain` dev-dependency.
8. **Mock mode.**
   - The mock chain's parameters are pinned: a fixed public seed, `bls-unchained-g1-rfc9380`, quicknet's genesis time, a 3s period, and `beacon_id = "rafi-test"`. Its hash is therefore the same in the app, the endpoint and across restarts, and the freeze check (§10.3) passes.
   - Messages change from "fake" to "signed by a public test key, not verifiable as drand".
   - The `test-chain` feature is enabled only through a Rafi `mock-drand` cargo feature (on in dev and test builds). A binary built without it **fails at boot** when mock is requested, rather than silently using quicknet.
9. **Compatibility:**
   - SQLite rows, `verify.json` and `verify.py` hold `Event::Draw` hex strings, which don't change. The new `Stall` event is additive.
   - `DrawState` gains `drand_chain`, which defaults to quicknet (§10.1).
   - `Beacon` appears only in the Restate journal (the `ctx.run("beacon")` result). The new `Beacon` deserializes the old `{round, signature, randomness}` shape, and a test pins this.
   - Deploy with no draw in flight, or register a new deployment.
   - Rafi is pre-production (PLAN.md), so existing dev databases and dev Restate state holding fake-signature mock draws are reset.
10. **`tools/verify.py`:**
    - It reads `doc["drand_chain"]` instead of hard-coded quicknet (verify.py:29, :65).
    - It stays a sha256 + relay comparison check.
    - Its doc and the README state that the browser verifier is the one that checks BLS.
    - ui/verify.rs:113 builds its drand link from the commit's chain.

## 11. Milestones

1. **M1, core verification:** chain, schedule, beacon, strict BLS12-381 verification, built-in chains, vectors (including negatives), TestChain; wasm32 and no_std builds plus the `cargo tree` check.
2. **M2, Rafi security fix:** §10.1–10.4, 10.6, 10.7, 10.8, 10.10, the config validation in §9, and the M2 half of §10.5 (existing fetching, verified beacons, TestChain mock); bundle-size check. All of Rafi's tests, including mock mode with `lead_secs = 0`, stay green. **This removes the trust-the-relay gap and the selective-abort levers (redraw on a new round, pool changes while pending) first. The one remaining lever, an offline draw after R is due, is confirmed in the UI and flagged publicly.** (Offline draws were dropped in `7b8721f`, so that lever no longer exists.)
3. **M3, http + client:** the `http` module, `Client::fetch`/`latest`/`wait_for`, hedging, `Clock`, `test_transport`, axum relay tests.
4. **M4, Rafi switch:** the M4 half of §10.5 and §10.9; Rafi's full test suite plus a live dev-server run.
5. **M5, publish 0.1.0:** README (comparison, the security notes from §6, "not audited"), examples, CI matrix, extraction to its own repo. File the `drand_core` off-by-one and v2-parsing issues upstream, and ask drand to list the crate on its client-libraries page.
6. **0.2:** `bn254`, `watch` (a named `Watch` type implementing `futures_core::Stream`, `Send + 'static`), health and chain discovery, `ClientBuilder::chain_hash`, `cargo-semver-checks`, fuzzing, a nightly `drand-cli` cross-check.

## 12. Risks and open questions

- **BN254 correctness (0.2):** the parameters are pinned and were proven in a scratch build. The remaining risk is our own SVDW/XMD code, which the vectors in §8 cover.
- **Not audited:** stated in the README along with the exact checks performed.
- **Unprobed relay:** `api.drand.secureweb3.com:6875` stays out of the defaults.

## 13. Implementation notes

- **`restate-drand` (added 2026-10-07).** A second reusable crate, `crates/restate-drand`, holds the Restate side.
  It is a `Drand` kept in the service, with `commit_round(&ctx)` and `await_beacon(&ctx, round)`: a journaled
  clock read, a durable sleep, a journaled fetch with a retry budget, and verification. `AwaitError::Unavailable`
  (stall) is kept apart from cancellation. On cancellation, `Raffle` records a `draw cancelled` stall on the same round (so the admin controls show at once) and then fails the invocation; an e2e test cancels a sleeping draw to check it. The `Raffle` object uses it;
  its journal steps are `drand: now` and `drand: beacon`.

- **Order.** M2 and M4 landed together: Rafi went straight to `drand_beacon::client::Client` instead of
  first patching the old fetcher. The end state is the plan's.
- **Verified against live data.** Fixtures in `crates/drand-beacon/tests/data` were fetched from Cloudflare (v1)
  and api.drand.sh (v2) on 2026-10-07: quicknet r1, r1000…r1020, r30000000; default r1, r2, r1000000;
  evmnet r1000 (parsed only). The `pedersen-bls-unchained` vector is the retired testnet beacon from
  `drand_core`'s test suite. The ignored live tests (`tests/live.rs`: each relay and API version on its own, plus a hedged run) and `examples/fetch.rs` were run against the
  public relays.
- **Additions.** `http::Relay` implements `FromStr`/`Display` with the `v1:`/`v2:` prefixes (Rafi's config
  uses it). `ClientBuilder::test_transport` requires the builder's
  chain to equal `TestChain::info()`. `rafi_core::verify::verify_public` resolves only public chains (the verify
  page); `rafi_drand::resolve_chain` adds the mock chain (tests, server).
- **The "would have picked" check.** `rafi_core::verify::Report::replaced` lists abandoned and
  offline-replaced committed rounds with their pool. The verify page lists them, and `tools/verify.py --fetch`
  computes the winner each round would have picked. The page does not fetch drand itself.
- **Bundle size** (`cargo build -p rafi --features web --release`, before wasm-opt): 5,246,582 → 5,450,954
  bytes (+3.9%), gzip 1,340,534 → 1,406,771 (+4.9%). The `cargo tree` check keeps `std` and `getrandom` out of the
  wasm core.
- **MSRV.** CI copies the crate out of the workspace, resolves a fresh lockfile with Rust 1.85 and runs the
  tests, which also exercises the extraction.
- **Hardening from the implementation review.**
  - `ChainInfo::from_json` fixes the genesis seed (32 bytes) and key lengths, and requires a built-in chain
    to match exactly: the chain hash concatenates key ‖ seed ‖ beacon ID without lengths, so a relay could
    otherwise move bytes between fields.
  - `Verifier::new` refuses chained chains without a 32-byte seed.
  - `Relay` rejects credentials, control characters and non-numeric ports.
  - `build()` validates relay URLs and the user agent, so configuration errors are never retryable relay errors.
  - `wait_for` bounds each attempt by its deadline and accepts `Duration::MAX`.
  - `latest()` falls back only on "not yet".
  - On the Rafi side:
    - Both verifiers reject a draw that does not settle the announced draw on its round (a silent redraw).
    - `verify.py` also rejects pool-changing resolutions while a draw is pending.
    - Pending local draws resume at startup.
    - The admin shows the retry, offline and abandon controls once a draw is 90 s overdue.
    - Both engines refuse to draw for a raffle on another chain.
    - The local engine's timeout starts when the round is due.
- **Restate stall test.** The `draw` handler always sleeps until the round is due, so the end-to-end stall
  test uses quicknet behind an unreachable relay (a 3 s lead and a 1 s timeout) rather than a long lead.

