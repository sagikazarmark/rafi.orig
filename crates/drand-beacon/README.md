# drand-beacon

**Fetch and strictly verify [drand](https://drand.love) randomness beacons in Rust.**

- A `no_std` + `alloc` core that never reads the clock and builds for `wasm32-unknown-unknown`: chain
  parameters, round arithmetic, v1/v2 beacon parsing and BLS verification.
- An async client (`client` feature, native targets) that asks several relays with hedged requests and
  returns the first beacon that verifies. Its single-attempt `fetch` fits durable executors (Restate,
  Temporal): "not yet" and "retryable" are explicit.
- A deterministic `TestChain` (`test-chain` feature) that signs real beacons, so tests go through the
  real verification path.

```rust
use drand_beacon::{Beacon, Verifier, chains};

let verifier = Verifier::new(&chains::quicknet())?;
let beacon = Beacon::from_json(br#"{"round":1000,"signature":"b44679b9a59af2ec876b1a6b1ad52ea9b1615fc3982b19576350f93447cb1125e342b73a8dd2bacbe47e4b6b63ed5e39"}"#)?;
let verified = verifier.verify_round(1000, &beacon)?;
println!("{}", hex::encode(verified.randomness()));
```

```rust
use std::time::Duration;
use drand_beacon::{chains, client::{Client, Fetch}};

let client = Client::builder(chains::quicknet()).build()?; // features: client + rustls
let latest = client.latest().await?;
let next = client.wait_for(latest.round() + 1, Duration::from_secs(30)).await?;
match client.fetch(next.round() + 1).await? {
    Fetch::Ready(beacon) => println!("{}", hex::encode(beacon.randomness())),
    Fetch::NotYet { due_at_ms } => println!("due at {due_at_ms}"),
    _ => {}
}
```

## Features

| Feature | Default | |
|---|---|---|
| `bls12-381` | yes | Verification of the BLS12-381 schemes (`default`, `quicknet`) via arkworks |
| `client` | no | Async `reqwest` client (cfg'd out on `wasm32-unknown-unknown`) |
| `rustls` / `native-tls` | no | TLS for the client's built-in `reqwest::Client` (or pass your own with `.http(..)`) |
| `test-chain` | no | `TestChain`: signs beacons with a key derived from a public seed |

The built-in HTTP client is `reqwest` without default features, so it ignores `HTTPS_PROXY`; pass your own
client with `.http(..)` if you need a proxy.

Supported schemes: `pedersen-bls-chained`, `pedersen-bls-unchained`, `bls-unchained-g1-rfc9380`.
`bls-bn254-unchained-on-g1` (evmnet) is parsed but not verified yet; the deprecated `bls-unchained-on-g1`
is reported as unsupported. MSRV: 1.85.

## Tests

`cargo test --features client,test-chain` runs everything offline: real beacons from the live relays are checked
in under `tests/data`, and client behavior is tested against in-process mock relays. The live tests talk to
every public relay and API version separately (chain info, quicknet and `default` rounds, a future round) and
are opt-in:

```sh
cargo test --features client,rustls --test live -- --ignored
```

## Security

- **Trust root.** A `Verifier` is built from a `ChainInfo` that is built in (`chains::*`), parsed against an
  expected chain hash (`ChainInfo::from_json`), or constructed explicitly. Relay-supplied `/info` is never
  trusted by itself. drand's chain hash does not cover the scheme, but a mislabeled scheme can only make
  verification fail: the network signs one message per round, and no other scheme's message equals it
  without a SHA-256 collision.
- **Checks.** Every beacon is verified with a pairing check against the chain key. Points must use the one
  canonical encoding (exact length, canonical flags, coordinates below the field modulus, not infinity, in
  the prime-order subgroup), because the randomness is `sha256(signature bytes)`: two encodings of one point
  would let a relay choose between two "verified" values. Chained beacons must carry the right previous
  signature (the genesis seed at round 1); unchained beacons drop any previous signature. The round is part
  of the signed message, and `verify_round` also checks it against the requested round. Round 0 is rejected
  (relays serve the latest beacon for it).
- **What a relay can still do.** Withhold, delay, or answer "not found" for a round that is due. That only
  affects liveness, *if your application never moves to a different round after a failure*. Otherwise a
  relay that dislikes a published round can force a redraw. `fetch` reports such failures as retryable.
- **drand's own assumption.** A threshold of colluding League of Entropy nodes can learn rounds early. When
  committing to a future round (`Schedule::round_after`), leave at least one period of margin.
- **Not audited.**

## Compared with other crates

| | drand-beacon | [`drand_core`](https://crates.io/crates/drand_core) | [`drand-verify`](https://crates.io/crates/drand-verify) |
|---|---|---|---|
| Verification | strict (canonical encodings) | yes | yes |
| Networking | async, several relays, hedged; or bring your own via `http` | blocking `ureq`, one relay | none |
| HTTP API | v1 and v2 | v1 | — |
| `no_std` / wasm core | yes / yes | no / yes | no / yes |
| Test chain that signs | yes | no | no |
| evmnet (BN254) | planned | no | no |

## License

MIT or Apache-2.0.
