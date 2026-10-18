# altcha-server

The server's side of [ALTCHA](https://altcha.org) v2 proof-of-work, on the official
[`altcha`](https://crates.io/crates/altcha) crate (re-exported): issuing signed challenges and redeeming each
solution once. The browser's side is [`dioxus-altcha`](../dioxus-altcha), or the official widget: the form value is
the same.

```rust
use altcha_server::{MemoryStore, Verifier};

let verifier = Verifier::new(key, 5_000)?; // PBKDF2 iterations per attempt, about 256 attempts per solve
let store = MemoryStore::default();        // one process; implement `ReplayStore` over a table for more

// GET /api/altcha: send it as JSON (or route `altcha_server::axum::challenge`, with the `axum` feature).
let challenge = verifier.challenge()?;

// The posted `altcha` field: decoded, the signature, expiry and solution checked, then its nonce recorded.
let payload = verifier.redeem(&store, &form.altcha).await?;

// Now and then, housekeeping:
verifier.forget_expired(&store).await?;
```

- **Each challenge once.** `redeem` records the challenge's signed nonce in your `ReplayStore` until it expires, and
  refuses it the second time. Every variant of one solved challenge (another counter, extra fields) keeps that nonce.
  A challenge without an expiry is refused: it would be replayable once the store forgets it.
- **`ReplayStore`** is two methods: `redeem(nonce, expires_at) -> bool` (atomic: an insert that fails on a duplicate
  key) and `forget_expired(now)`. Its futures carry no `Send` bound; with a store whose futures are `Send`, so is
  `redeem`'s.
- **Why refused:** `RedeemError::Refused(Refusal)` says which check failed (malformed, no expiry, expired, signature,
  wrong, replayed); `RedeemError::Store` is the store's error. Tell users no more than "try again".
- **Not a middleware.** Verifying costs a key derivation (`cost` iterations), and a redeemed solution is spent: redeem
  in your handler, after rate limits and other cheap checks.
- **Defaults:** `PBKDF2/SHA-256` (native WebCrypto in browsers), key prefix `00`, challenges valid 20 minutes. Change
  them with `algorithm`, `key_prefix` and `ttl_secs`. `cost` only matters when issuing: a verifier redeems its key's
  challenges whatever they asked.
- **On Workers:** `altcha`'s `verify_solution` reads std clocks, which panic on `wasm32-unknown-unknown`; patch in
  [altcha-org/altcha-lib-rs#1](https://github.com/altcha-org/altcha-lib-rs/pull/1) until it's released. This crate's
  own clock reads use `web-time`.
