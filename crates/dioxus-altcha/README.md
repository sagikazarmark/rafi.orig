# dioxus-altcha

[ALTCHA](https://altcha.org) v2 proof-of-work for [Dioxus](https://dioxuslabs.com): the browser solves the
challenge in Web Workers on WebCrypto, off the main thread, and a hook puts it in a form. The server issues and
verifies with the official [`altcha`](https://crates.io/crates/altcha) crate (re-exported here). The form value is the
one the official widget posts, so any ALTCHA v2 server accepts it.

```rust
use dioxus::prelude::*;
use dioxus_altcha::{AltchaField, fetch_challenge, use_altcha};

#[component]
fn Join() -> Element {
    let altcha = use_altcha(|| fetch_challenge("/api/altcha"));
    rsx! {
        form { method: "post", action: "/join",
            input { name: "email" }
            AltchaField { altcha }
            button { disabled: altcha.payload().is_none(), "Join" }
        }
    }
}
```

On the server, [`altcha-server`](../altcha-server) issues signed challenges and redeems each solution once, over
a replay store you provide (a database table; `MemoryStore` for one process):

```rust
let verifier = altcha_server::Verifier::new(key, 5_000)?; // PBKDF2 iterations per attempt
// GET /api/altcha (or `altcha_server::axum::challenge`, with the `axum` feature):
let challenge = verifier.challenge()?;
// ... and for the posted `altcha` field, after your cheaper checks:
let payload = verifier.redeem(&store, &form.altcha).await?;
```

Or by hand with `altcha`: `create_challenge`, then `decode_payload` and `verify_solution`, then record
`payload.challenge.parameters.nonce` until it expires, and refuse it if it's already there.

- **Algorithms:** `PBKDF2/SHA-256|384|512`, and iterated `SHA-*`, which is slow above `cost` 1. Not `SCRYPT` or `ARGON2ID`.
- **Workers:** up to 4. Chromium runs every worker's PBKDF2 on one thread, so on Android more workers don't help;
  Safari and Firefox scale.
- **No workers?** If none can start (a CSP, an odd webview), it solves on the main thread, yielding to the page. It
  loads the solver there as a `<script>` from the workers' URL: under a CSP that also restricts scripts, that needs
  `worker_url` (same-origin), since a `blob:` script is blocked too.
- **Secure context:** WebCrypto needs HTTPS or localhost.
- **CSP:** the workers start from a `blob:` URL (`worker-src blob:`). Otherwise serve `WORKER_JS` as a file and set
  `SolveOptions::worker_url`. Without the default `embed-worker` feature, that's required, and the solver isn't in
  your wasm.
- **The challenge passes through untouched** (`RawChallenge`), so a server on another version still verifies it.
- **The hook** (`use_altcha_with_options`):
  - solves on mount (`Auto::OnMount`) or on demand (`Auto::Off`, with `start()` or `solved().await` on submit);
  - solves again when the challenge expires, after `refresh()`, and when the page comes back from the back/forward
    cache;
  - its first render is `Idle` everywhere, so SSR and hydration match.
- **Desktop and mobile:** the `native` feature solves on threads. It's off by default, so a liveview server never
  does the work.
- **On Workers:** `altcha`'s `solve_challenge` and `verify_solution` read std clocks, which panic on
  `wasm32-unknown-unknown`. To verify on Cloudflare Workers, patch in
  [altcha-org/altcha-lib-rs#1](https://github.com/altcha-org/altcha-lib-rs/pull/1) until it's released.

`examples/altcha-demo` is the whole thing as a Dioxus fullstack app (`dx serve` there). Server functions issue and
verify, and the page has a difficulty slider, solve-on-load or solve-on-submit, attacks the server refuses, a strict-CSP
page and a bench.

## Tests

- `cargo test`: the payload, the hook in a VirtualDom (solving with `native`), and the drift test, which needs node
  18+. The drift test solves challenges from `altcha` with `worker.js` under node and checks them with
  `verify_solution`.
- In browsers:

  ```sh
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner GECKODRIVER=geckodriver \
    cargo test --target wasm32-unknown-unknown --test wasm_solve
  ```

  (or `CHROMEDRIVER`, `SAFARIDRIVER`)
