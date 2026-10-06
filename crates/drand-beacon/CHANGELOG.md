# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Chain model: `ChainInfo` (no `Deserialize`; built in, pinned by hash with `from_json`, or constructed
  explicitly), `ChainHash`, `Scheme`, and the built-in `quicknet`, `default` and `evmnet` chains.
- `Schedule`: round arithmetic (`round_at`, `round_time`, `round_after`, millisecond variants).
- `Beacon` parsing for the v1 and v2 HTTP APIs, with format-aware serde (v1 JSON shape, compact binary).
- `Verifier`: strict BLS verification of `pedersen-bls-chained`, `pedersen-bls-unchained` and
  `bls-unchained-g1-rfc9380` (quicknet), with canonical-encoding checks, producing a `VerifiedBeacon`.
- `http`: relay URLs (v1 and v2) and response classification without I/O.
- `client` feature (native targets): an async `reqwest` client with hedged requests across relays,
  `fetch` (one attempt, for durable executors), `latest` and `wait_for`.
- `test-chain` feature: `TestChain`, a deterministic chain that signs real, verifiable beacons.
