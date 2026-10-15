# dioxus-altcha (plan)

Status: implemented, 2026-10-15, except the M4 measurements. Reviewed in four passes before that: fit with the code, protocol and security, browser mechanics, and the Dioxus API. The findings are folded in below.

**Where it stands (2026-10-15).** M1, M2, M3 and M5 are done on the `dioxus-altcha` branch.

Verified:

- **The crate's tests** pass natively, including the hook in a VirtualDom with the `native` solver and the drift test against `worker.js` under node.
- **`wasm_solve`** passes in headless Firefox: workers, drop, timeout, the main-thread fallback, a missing script.
- **The demo** works in Firefox: it solves on mount, the solution verifies, refresh works, and unmounting cancels.
- **`scripts/workers-check.sh`** passes. The evening joins through the WebAssembly and, newly, through `fallback.js` with WebAssembly removed, so `verify_solution` runs on Workers with the patched `altcha`.

Not yet:

- **Safari:** not run; safaridriver needs `sudo safaridriver --enable` once.
- **M4:** the benchmark (the demo's `?bench`) is in, with desktop results (M4 below). Still needed: old phones over HTTPS, and a deployed Worker. Until then `RAFI_ALTCHA_COST` defaults to the provisional 1,000.

What changed from the text below while implementing it:

- **Rafi's key prefix is a constant**, `"00"` (`KEY_PREFIX`). `ProofOfWork::new(key, cost)` validates the key and the cost at startup; there is no `key_prefix` setting, and tests use `cost: 1` with the same prefix (about 256 one-round tries).
- **`embed-worker`:** embedding `WORKER_JS` is a default feature. Rafi turns it off and loads its workers from `/participant/altcha-worker.js`, the file `fallback.js` uses.
- **The phone bundle:** after dropping the embedded copy, the untagged-enum reply and `to_uppercase`, it is 276,553 B gzipped against `main`'s 267,656. That leaves about 10 KB of the 280 KB budget.
- **`RawChallenge::usable_for()`** is the deadline rule of §3.3, shared by the hook and rafi's `extras.rs`. `AltchaError` gained `Cancelled`. There's no `build.rs`: plain `cfg(target_arch)` and the `native` feature.
- **The demo** is `crates/dioxus-altcha/examples/altcha-demo`, with its own `[workspace]`. CI checks it builds and runs `wasm_solve` in Chrome and Firefox (the Workers job).
- **`testbeds/cloudflare` was broken on `main`:** it still called `CardStrings::from_lookup`, removed with the language switch. It now builds its `CardStrings` directly.

## 1. Goal

A reusable crate, `dioxus-altcha`, that solves [ALTCHA](https://altcha.org) v2 proof-of-work challenges in the browser, off the main thread, with a Dioxus hook and a form field on top. Rafi moves from its own SHA-256 scheme (`rafi-altcha`) to the v2 protocol of the official [`altcha`](https://crates.io/crates/altcha) crate, on the server and on the phone.

The crate lives in this workspace (`crates/dioxus-altcha`, `publish = false`) with no rafi dependencies, so it can move out unchanged.

## 2. Decisions

### 2.1 Solving runs in JavaScript Web Workers, on WebCrypto

- **`worker.js`** mirrors `altcha::solve_challenge` with the default `CounterMode::Uint32`. Byte for byte:
  - **Password:** `nonce || counter`, where the counter is 4 bytes, big-endian (`DataView.setUint32`). Nonce and salt are hex-decoded.
  - **`PBKDF2/SHA-256|384|512`:** `importKey("raw", password, "PBKDF2")`, then `deriveBits({ salt, iterations: cost, hash }, keyLength * 8)`.
  - **`SHA-256|384|512`:** `max(1, cost)` rounds of `digest`. Round 1 hashes `salt || password`, and each later round hashes the full previous digest. The result is truncated to `keyLength` only at the end.
  - **The key** is reported as full-length lowercase hex. The server compares strings.
  - **Prefix match:** an even-length prefix is compared as bytes. An odd-length prefix is compared as a lowercase hex string.
  - **Counter limit:** past `0xffffffff` the worker stops with an error, rather than wrapping like Rust's `wrapping_add`.
  - **`SCRYPT`, `ARGON2ID`:** rejected before any worker starts.
- **PBKDF2 is the supported path.** The iterated-SHA variant awaits one promise per round, which measured 82× slower than PBKDF2 at `cost` 1000 under node. It works, but only makes sense with `cost` around 1.
- **Structure:** `worker.js` defines `solve(challenge, start, step)`. It wires `self.onmessage` only when `typeof WorkerGlobalScope !== "undefined"`, so the main thread and node can call the same function.
  - The language level is ES2017: `async`/`await` but no `?.`, `??`, BigInt, class fields or modules.
- **Loading the worker:**
  - A classic worker (module workers need Safari 15+ and Firefox 114+).
  - From a Blob of `WORKER_JS` with `type: "text/javascript"`, or from `SolveOptions::worker_url` for apps whose CSP doesn't allow `worker-src blob:`.
  - It works whatever builds the app: dx (`rafi-web`), plain wasm-bindgen (`rafi-participant`), trunk.
- **Parallel:**
  - Worker `i` of `n` tries counters `i, i + n, …`, the split that `counter_start`/`counter_step` exist for, as in the official widget.
  - The default is `n = min(hardwareConcurrency, 4)`, or 2 if unknown.
  - **Chromium serializes PBKDF2:** it runs on one process-wide WebCrypto thread for every worker (`components/webcrypto/webcrypto_impl.cc`, crbug 623700). On Android Chrome, Android WebView and in-app browsers, more workers don't make it faster; two keep that thread busy.
  - Safari (a queue per global) and Firefox (a thread pool) do scale.
  - So Android Chrome is the slow population, and M4 sets the difficulty for it.
- **Main-thread fallback:** if creating the workers throws, or every worker reports `error` before its first message (CSP, odd webviews), `solve` runs `solve()` on the main thread instead. It yields with `setTimeout(0)` about every 150 ms, as the widget does. PBKDF2 runs off the calling thread in all three engines, so the page stays responsive.
- **Secure context:** `crypto.subtle` exists only on HTTPS and localhost. `solve` checks `isSecureContext && crypto.subtle` first and fails at once with `InsecureContext`. Testing on a phone at `http://192.168.x.x` therefore no longer works, which today's Rust `sha2` solver allowed. Use a tunnel, mkcert, or a deployed preview.
- **Why not Rust in the worker:**
  - The app's own wasm can't be reused: it would run `main`, which launches Dioxus with no `document`.
  - A dedicated solver wasm means a committed build artifact with a drift check.
  - Rafi's `fallback.js` would still need a JS solver.

  On Chromium, one native thread (with SHA instructions) against several scalar-wasm workers is not obviously a win either way. M4 measures it. If wasm wins on Android, or scrypt or Argon2id matter, a wasm backend can be added behind a feature without changing `solve`'s signature.

### 2.2 The challenge passes through untouched

- **The client keeps the challenge as raw JSON** (`RawChallenge`, a `Box<RawValue>`) and builds the payload from it.
- **Why:** deserializing into `altcha::Challenge` drops fields it doesn't know. A server on a newer `altcha`, or a JS server, would then see a signature mismatch. The widget doesn't parse it either.
- **This also keeps rafi's phone wasm small,** because it never deserializes `parameters.data`.
- **The client reads only** `algorithm` and `expiresAt` from it, through a small private struct.

### 2.3 The crate is client-only

- **Verification stays in the app**, which uses `altcha` directly.
- **The form value is the widget's standard:** base64 (standard alphabet, padded) of the JSON `{challenge, solution}`.
- **The crate offers `decode_payload`** for servers. It has no Dioxus or wasm dependency, but servers that can't depend on Dioxus (`rafi-app-core`) decode with `base64` + `serde_json` themselves.
- **No server helper:** a `verify` helper would run `verify_solution` on the server. On Cloudflare Workers that panics without the `web-time` patch (§2.5).

### 2.4 Random-prefix challenges, not deterministic

- **v2 has two modes:**
  - **random prefix:** the client searches for a key starting with `keyPrefix`;
  - **deterministic:** the server picks the counter, derives the key and signs it.
- **Random prefix:**
  - Issuing costs nothing (two RNG draws and one HMAC), which matters because `GET /api/altcha` is unauthenticated.
  - Verifying re-derives one key, on a join, after the per-raffle rate limit (`entrants.rs:86-99`).
- **The tail:**
  - The number of tries is geometric. With a one-byte prefix (`"00"`), 5% of solves take more than 3× the mean, and 0.7% more than 5×.
  - More workers divide the mean and the tail alike, so the ratio stays the same.
  - The median is 0.69× the mean. So a median target of 1 s means a mean of about 1.44 s and a p95 of about 4.3 s.
- **Deterministic:** the counter is uniform on a range, so the worst case is about twice the mean. But every challenge request costs the server a key derivation. It also needs its own `hmac_key_signature_secret`, derived from the main key (as the JS library's `deriveHmacKeySecret` does), never `RAFI_ALTCHA_KEY` itself. Verifying is then one HMAC.
- **Prefix granularity** is one hex digit (16× steps). `cost` tunes linearly in between, so rafi fixes the prefix at `"00"` and tunes `cost`.

### 2.5 The `altcha` patch

- **What it fixes:** without [altcha-lib-rs#1](https://github.com/altcha-org/altcha-lib-rs/pull/1), `solve_challenge` and `verify_solution` panic on `wasm32-unknown-unknown`: `Instant::now()` always, and `SystemTime::now()` when the challenge expires. `create_challenge` doesn't read the clock.
- **How it's pinned:** the workspace extends its existing `[patch.crates-io]` with `altcha = { git = "https://github.com/sagikazarmark/altcha-lib-rs", rev = "<sha>" }`. Use a `rev`, not the branch, so `cargo update` doesn't follow the fork.
- **cargo-deny:** `deny.toml` has `unknown-git = "deny"`, so the fork goes in `allow-git`.
- **Who needs it:**
  - **`rafi-app-worker`** verifies on Workers, so it needs the patch until the PR is merged.
  - **The browser doesn't:** it never calls either function. So publishing `dioxus-altcha` doesn't wait on the PR.
- **`getrandom` 0.2's `js` feature** is needed to build `altcha` for wasm (its `rand` 0.8 uses `thread_rng`). Each crate built alone for wasm declares it, target-specific:
  - **`dioxus-altcha`;**
  - **`rafi-app-core`,** as a renamed dependency (`getrandom02 = { package = "getrandom", version = "0.2", features = ["js"] }`), since `getrandom` already names 0.3;
  - **`testbeds/cloudflare`,** which `worker-build` builds alone, so it gets no feature unification.

  In the `rafi-app-worker` build, `js` is already enabled through restate-sdk, restate-drand and superboring.
- **Workers' clock** stands still while code runs: it only advances across I/O. For `verify_solution` that's harmless, because the expiry check gets the request's time. But `solve_challenge` would never time out there, so no test solves inside the Workers runtime. Rafi computes `expires_at` with `ids::now()`, never `SystemTime`.

## 3. The crate

```
crates/dioxus-altcha/
  build.rs           cfg alias: `web` = wasm32 + not(feature "native")
  src/lib.rs         re-exports, RawChallenge, payload encoding and decoding
  src/solve.rs       solve(): workers, fallback, timeout, cancellation
  src/hook.rs        use_altcha, UseAltcha, AltchaState, AltchaField
  src/worker.js      the solver
  tests/worker.rs    the drift test (§3.4), via tests/solve.mjs under node
  tests/wasm_solve.rs  wasm-bindgen-test, headless Chrome and Firefox
examples/altcha-demo/  its own package (§3.4)
```

### 3.1 Core (no Dioxus types)

```rust
pub use altcha; // apps call `dioxus_altcha::altcha::create_challenge` in lockstep

/// A challenge as the server sent it, passed through untouched (§2.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawChallenge(Box<RawValue>);
impl RawChallenge {
    pub fn from_challenge(c: &altcha::Challenge) -> Self;
    pub fn expires_at(&self) -> Option<i64>; // None if absent or 0
}

/// The solver, for apps that serve it themselves (CSP, rafi's fallback.js).
/// Its message protocol (§3.2) is part of the public contract.
pub const WORKER_JS: &str = include_str!("worker.js");

/// The form value: base64 of the JSON `{challenge, solution}`.
pub fn payload(challenge: &RawChallenge, solution: &altcha::Solution) -> String;
pub fn decode_payload(value: &str) -> Result<altcha::Payload, DecodeError>; // caps the length at 8 KB

#[non_exhaustive]
pub struct SolveOptions { /* private */ }
impl SolveOptions {
    pub fn new() -> Self;
    pub fn workers(self, n: usize) -> Self;              // default: min(hardwareConcurrency, 4), else 2
    pub fn timeout(self, t: Option<Duration>) -> Self;   // default: 90 s of visible time
    pub fn worker_url(self, url: impl Into<String>) -> Self; // default: a Blob of WORKER_JS
}

/// On every target. Without `web` (or `native`, §5 M5): Err(UnsupportedTarget).
pub async fn solve(challenge: &RawChallenge, opts: &SolveOptions) -> Result<altcha::Solution, SolveError>;

/// `GET` the URL, read the body as a RawChallenge. On every target, like `solve`.
pub fn fetch_challenge(url: impl Into<String>) -> impl Future<Output = Result<RawChallenge, FetchError>> + 'static;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SolveError {
    UnsupportedAlgorithm(String), InvalidChallenge(String),
    InsecureContext, UnsupportedTarget, Timeout, Worker(String),
}
```

- **Errors are flattened to owned strings,** so they can live in a signal, as dioxus-clerk does.
- **The timeout counts only visible time.** It pauses on `visibilitychange`, because iOS suspends hidden pages, workers included. Without that, a phone locked for more than 90 s would come back to a spurious `Timeout` with no work lost. `gloo-timers` futures clear their timer on drop.
- **Dependencies:**
  - `altcha`, `serde`, `serde_json` (with `raw_value`), `base64`, `thiserror`;
  - for wasm32: `web-sys`, `js-sys`, `wasm-bindgen-futures`, `futures-channel`, `gloo-timers`, `getrandom` 0.2 with `js`.
- **Versions:** explicit (`altcha = "1"`, `dioxus = "0.7"`), not `workspace = true`, so the crate moves out unchanged.

### 3.2 Worker protocol and cleanup

- **In:** a JSON string, `{"challenge": <raw>, "start": i, "step": n}`.
- **Out:** a JSON string, either `{"counter", "derivedKey", "time"}`, which deserializes into `altcha::Solution`, or `{"error": "<message>"}`.
- **No objects or `Error`s cross the boundary:** cloning an `Error` isn't universal on old engines, and a `DataCloneError` in the worker fails silently. Rust reads replies with `as_string()` and `serde_json`, with no `serde-wasm-bindgen`.
- **A drop guard owns the workers, their closures and the Blob URL.** On a solution, an error, the timeout, or the future being dropped, it does this for each worker:
  1. `set_onmessage(None)` and `set_onerror(None)`;
  2. `terminate()`;
  3. then drops the closures (never `forget`).

  Then it revokes the URL.
- **Cancelling by drop works in Dioxus 0.7:** a scope's tasks are dropped when it unmounts (`runtime.rs:187`, `tasks.rs:310`). It doesn't for `spawn_local` or `spawn_forever`. Rafi's page-lifetime `spawn_local` is never cancelled, which is fine.
- **`solution.time` is chosen by the client.** Servers must not trust it.

### 3.3 Dioxus layer

```rust
pub fn use_altcha<F, Fut, E>(fetch: F) -> UseAltcha;
pub fn use_altcha_with_options<F, Fut, E>(fetch: F, opts: AltchaOptions) -> UseAltcha
where
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<RawChallenge, E>> + 'static,
    E: Display + 'static;

#[non_exhaustive]
pub struct AltchaOptions { /* private */ }
impl AltchaOptions {
    pub fn new() -> Self;
    pub fn auto(self, auto: Auto) -> Self; // default: Auto::OnMount
    pub fn solve(self, opts: SolveOptions) -> Self;
}
pub enum Auto { OnMount, Off } // on focus: call `start()` from `onfocusin`

#[derive(Clone, Copy)]
pub struct UseAltcha {
    state: Signal<AltchaState>,
    task: Signal<Option<Task>>,
    run: Callback<()>,
    done: UseWaker<Result<String, AltchaError>>,
}
impl PartialEq for UseAltcha { /* compares `state` (signals compare by pointer) */ }
impl Debug for UseAltcha { /* peeks */ }
impl UseAltcha {
    pub fn state(&self) -> AltchaState;            // cloned, subscribes
    pub fn signal(&self) -> ReadSignal<AltchaState>;
    pub fn payload(&self) -> Option<String>;       // None once past the deadline
    pub fn start(&self);
    pub fn refresh(&self);
    pub async fn solved(&self) -> Result<String, AltchaError>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum AltchaState {
    Idle, Fetching, Solving,
    Solved { payload: String },
    Expired, Failed(AltchaError),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AltchaError { Fetch(String), Solve(SolveError) }

#[component]
pub fn AltchaField(
    altcha: UseAltcha,
    #[props(default = "altcha".into())] name: String,
    #[props(extends = input)] attributes: Vec<Attribute>,
) -> Element;
```

- **The runner is a `use_callback`,** like `use_future` and `use_resource`. A `Callback` runs in the scope that created it, so a `start()` from a child's handler doesn't make the solve the child's. That matters because `spawn` uses the current scope.
- **One task at a time:** the runner cancels the stored `Task` before spawning a new one, so two solves never race and no stale `state.set` lands.
- **`start()`** does nothing while `Fetching`, `Solving`, or `Solved` before the deadline. **`refresh()`** cancels, sets `Idle`, and starts again if `Auto::OnMount`. After a submit the challenge is spent, so the app calls `refresh()`.
- **Solving on submit:** `solved()` starts the solve if idle. It returns the payload if solved and before the deadline, and otherwise awaits `done`.

  ```rust
  onsubmit: move |e| async move { e.prevent_default(); let p = altcha.solved().await?; submit(p).await }
  ```
- **Expiry:**
  - The deadline is relative to receipt: receipt time (`performance.now()`) plus `expiresAt − Date.now()`, minus a margin of 30 s.
  - If that's not in (0, 24 h], the phone's clock is too far off to judge, and the hook leaves expiry to the server instead of looping fetch, solve, expired. The widget has the same skew but only shows "expired".
  - The expiry sleep runs inside the same task, so cancelling or unmounting kills it.
  - With `Auto::OnMount` it fetches and solves again, at most once per deadline. `payload()` and `solved()` check the deadline themselves, since background tabs throttle timers.
- **Back/forward cache:** on `pageshow` with `persisted`, the hook calls `refresh()`. A restored form would otherwise post a spent challenge.
- **SSR and hydration:**
  - The first render is `Idle` on server and client. `Fetching` is set inside the spawned task, never during render, and the hidden input is `""` on both sides.
  - `spawn` makes a client-only task, which SSR never polls, so the server side needs no `cfg`.
  - Liveview does poll it, on the server, which is why the native backend is an opt-in feature.
- **Options are read on the first render only.**
- **Dioxus features:** `macro`, `html`, `signals`, `hooks`. The app picks the renderer.
- **Progress:** none. The total is unknown, and the widget shows a spinner.
- **The hook's state machine** (start, refresh, expiry, bfcache) is a pure reducer, tested natively.

### 3.4 Tests

- **Drift test** (`tests/worker.rs`, native, needs node; CI's test job has node 24):
  1. Create challenges with `altcha::create_challenge` for:
     - `PBKDF2/SHA-256|384|512`, and `SHA-256|384|512` at `cost` 1;
     - one- and odd-length prefixes, and an uppercase prefix passed to the solver.
  2. Solve them with `solve()` from `worker.js`, loaded with `readFileSync` + `new Function` in the main realm (`globalThis.self = globalThis`).
  3. Check each solution with `verify_solution`.

  Also check that `SCRYPT` comes back as `{error}`, and that the file parses at ES2017.
- **wasm-bindgen-test** (`tests/wasm_solve.rs`, headless Chrome and Firefox; Safari by hand with safaridriver):
  - `solve` on a cheap challenge;
  - dropping it mid-solve terminates the workers;
  - the timeout;
  - a broken `worker_url` falls back to the main thread.
- **Payload tests:**
  - `payload` and then `decode_payload` round-trip;
  - unknown challenge fields survive the round trip;
  - a decoded payload verifies.
- **Demo:** `examples/altcha-demo` is its own package, excluded from the workspace. Native `cargo clippy --workspace --all-targets` would otherwise compile a dx web app, which is why `rafi-web` is excluded.
  - It issues, solves and verifies in the browser, and shows the states, `refresh()` and cancellation on unmount.
  - It needs the patch for `verify_solution`. When the crate moves out, the demo carries its own patch until upstream merges.

## 4. Rafi's migration

The server and the client switch protocols in one change: a page loaded before the deploy posts a v1 payload, which is refused, and the page asks the phone to retry. Don't deploy during an event.

### 4.1 Server

- **`app-core/src/altcha.rs`** (refer to the crate as `::altcha`; the module shadows it):
  - **`challenge()`** calls `::altcha::create_challenge` with:
    - `algorithm: "PBKDF2/SHA-256"`;
    - `cost` from `ProofOfWork`;
    - `key_prefix: "00"`;
    - `expires_at = ids::now() + 20 min`;
    - the key as `hmac_signature_secret`.

    It returns `Result`, because `create_challenge` does. The settings are checked at startup, so in practice it can't fail. `public.rs:79-85` maps an error to 500.
  - **`redeem()` refuses** (`Ok(false)`) a payload that:
    - fails to decode (it's longer than 8 KB, or isn't base64 JSON of a `Payload`);
    - has no `expires_at`, or a non-positive one (a challenge that never expires could be replayed after the purge);
    - fails `verify_solution`, or makes it return `Err`.

    Otherwise it records `parameters.nonce` in `altcha_used` until `expires_at`. A unique violation is `Ok(false)`, a replay. Any other store error stays `Err`, as `JoinError::Failed`.

    The nonce is the right replay key: every variant of one challenge (another valid counter, extra fields) keeps the same signed nonce, and the JS library uses it too.
  - **Helpers:** it exposes `encode`/`decode` helpers for both test suites.
- **`ProofOfWork::new(key: String, cost: u32)`,** with the key prefix fixed at `"00"` (`KEY_PREFIX`), built in `server/src/lib.rs`.
- **`server/src/config.rs`:**
  - `RAFI_ALTCHA_MAX` becomes `RAFI_ALTCHA_COST`, which must be ≥ 1, because WebCrypto rejects 0 iterations.
  - `RAFI_ALTCHA_KEY` stays, and must not be empty: `altcha` treats `""` as no secret.
  - The random default key becomes `hex::encode(random_bytes::<32>())`; a `String` can't hold raw bytes, and `from_utf8_lossy` would lose entropy. A key set in the environment gives the same HMAC bytes as before.
  - `for_tests` uses `cost: 1`.
- **No changes in** `app-worker/src/lib.rs`, `wrangler.toml` or `deploy/.env.example`: Workers use the same `Config::from_lookup`, and none of them sets `RAFI_ALTCHA_MAX`. Optionally set a low `RAFI_ALTCHA_COST` in `wrangler.toml`'s `[env.dev.vars]`.
- **The `altcha_used` table is unchanged.** Its `challenge` column holds the nonce, and the conformance tests assume nothing about the value. Old rows go at the next hourly purge, and a 32-character nonce can't collide with a 64-character v1 challenge.
- **Tests:**
  - `server/src/tests.rs` (`solved_altcha`, the replay test) and `app-core/tests/use_cases.rs` (`solved`, its `Policy` literal) solve with `::altcha::solve_challenge` natively, at `cost: 1`, because they run in debug builds, some in loops.
  - The replay test's row check becomes `(parameters.nonce, parameters.expires_at)`.
  - Deleting `crates/altcha` drops the only expiry and forgery tests, so app-level cases replace them:
    - an expired challenge;
    - a challenge signed with another key;
    - a forged signature;
    - a changed parameter;
    - a tampered counter;
    - a correct counter with the wrong `derivedKey`;
    - a second valid counter for the same challenge, which is a replay;
    - no `expiresAt`.
- **`testbeds/cloudflare`:** `altcha_challenge()` calls `create_challenge`, and the testbed gets `getrandom` 0.2 with `js` (§2.5).
- **Dependencies:**
  - `altcha = "1"` goes in `[workspace.dependencies]`, used by `app-core` (plus `base64`), `server` (dev) and the testbed.
  - `dioxus-altcha` goes in as a workspace member and dependency, used by `participant`.
  - `crates/altcha` (`rafi-altcha`) is removed: its member entry, its dependency entry, and the CI `-p` lists. `ci.yaml:77-79`'s wasm clippy gets `-p dioxus-altcha`.
- **Docs:**
  - **README:** the crate list (line 79), the environment table (`RAFI_ALTCHA_MAX`, lines 252-253), the rebuild note (line 126), and phone testing needing HTTPS (§2.1).
  - **`docs/PLAN.md`** (lines 57, 196, 212, 304, 418).
  - **`docs/workers-plan.md`**'s "upstream later" (lines 255, 277, 406) now points here.

### 4.2 Phone

- **`participant/src/extras.rs`:**
  - `fetch_and_solve` becomes `dioxus_altcha::fetch_challenge` and `solve`, and the input gets `payload(..)`. The form stays server-rendered and enhanced by element ID, so rafi doesn't use the hook.
  - On submit, if the payload's deadline has passed (a phone left on the join page for 20 minutes), it fetches and solves again first.
  - On `pageshow` with `persisted` (back/forward cache), it clears the input and solves again.
  - A wasm panic starts `fallback.js` while the Rust workers keep running, because Drop doesn't run after an abort. That costs about a second of duplicate work and is harmless.
- **`fallback.js`:**
  - `solveAltcha` loads `/participant/altcha-worker.js` and starts the same workers, with the same counter split. If they fail, it loads the script into the page and calls `solve()` on the main thread.
  - It encodes with `btoa('{"challenge":' + raw + ',"solution":' + JSON.stringify(s) + '}')`, where `raw` is the challenge response's text, so the challenge passes through untouched.
  - A worker `error` event counts as failure, and Join is enabled, as today. On Workers, a missing file returns `index.html` with status 200 (`not_found_handling = "single-page-application"`).
  - It also solves again on a restored `pageshow`, which it already listens for.
- **Form encoding:** base64 contains `+` and `/`. The form posts natively, which URL-encodes it. Keep it that way, and don't hand-build the body.
- **`scripts/participant.sh`** copies `crates/dioxus-altcha/src/worker.js` to `dist/altcha-worker.js`. All three deployments serve the dist directory as it is.
- **Size:**
  - Today the build is 267,656 B gzipped against a budget of 286,720, about 19 KB of room.
  - `rafi-altcha`'s `sha2` goes away. `altcha`'s key-derivation code is stripped because nothing reaches it, and the raw challenge avoids its serde code. `participant.sh` checks the result.

## 5. Milestones

1. **M1. The core.**
   - `worker.js`, `solve` (workers, fallback, timeout, drop guard), `RawChallenge`, `payload`/`decode_payload`, `fetch_challenge`.
   - The drift, wasm-bindgen and payload tests.
   - The workspace member, the `[patch.crates-io]` `rev`, the `deny.toml` `allow-git` entry.
2. **M2. The Dioxus layer.** `use_altcha`, `UseAltcha`, `AltchaField`, the reducer tests, `examples/altcha-demo`.
3. **M3. Rafi on v2, in one change.**
   - §4.1 and §4.2.
   - A provisional `RAFI_ALTCHA_COST` default of 1,000, so that `workers-check.sh`'s evening test (Join within 20 s in headless Chromium, `evening.mjs:82-83`) passes.
   - `cargo test`, `cargo deny`, `workers-check.sh` and `participant.sh` (within budget) pass.
4. **M4. Measure over HTTPS, then set the defaults.**
   - **Solve time:** median and p95 over about 30 solves, at a few values of `cost` with prefix `"00"`, on:
     - an old Android phone (Chrome), at 1, 2 and 4 workers;
     - an old iPhone (Safari).
   - **Android only:** compare against a wasm solver in workers, to settle §2.1.
   - **Server:** one verification's CPU time on a deployed Worker.
   - **Then:** set `RAFI_ALTCHA_COST`'s default for Android Chrome, and decide on deterministic mode (§2.4).

   **How:** build the demo (`cd crates/dioxus-altcha/examples/altcha-demo && dx build --platform web --release`), serve its `public` directory over HTTPS (e.g. `cloudflared tunnel --url http://localhost:8766` in front of `python3 -m http.server 8766`), and open `/?bench&costs=1000,3000,5000&workers=1,2,4&runs=30` on each phone, screen on, until it says `done`. Each run prints, per cost:
   - the median and p95 to solve at each worker count;
   - the same challenges solved by `altcha`'s Rust in wasm on one thread (the §2.1 comparison);
   - one `verify_solution` in wasm, a proxy for a Worker's CPU (Workers run V8, as Chrome does). The deployed Worker's own figure still needs the deploy.

   **Desktop results** (2026-10-15, an Apple M-series Mac with 14 cores, headless; 30 runs, 10 for Rust-wasm, ms). These are a baseline only; old phones are several times slower:

   | Browser | cost | 1 worker | 2 workers | 4 workers | Rust-wasm, 1 thread | verify |
   |---|---|---|---|---|---|---|
   | Chromium 137 | 1,000 | 25 / 53 | 12 / 64 | 16 / 65 | 111 / 256 | 0 / 1 |
   | Chromium 137 | 5,000 | 56 / 320 | 53 / 301 | 49 / 300 | 527 / 3,751 | 3 / 4 |
   | Firefox | 1,000 | 26 / 120 | 13 / 79 | 10 / 44 | 67 / 230 | 1 / 2 |
   | Firefox | 5,000 | 94 / 567 | 63 / 143 | 47 / 158 | 385 / 1,998 | 3 / 4 |

   (median / p95). What they settle so far:
   - **Chromium doesn't scale** with workers at cost 5,000 (56 → 49 ms), as §2.1 found in its source. Firefox does (94 → 47 ms).
   - **WebCrypto beats Rust-wasm** by about 10× on Chromium and 4–8× on Firefox, so no wasm backend for PBKDF2.
   - **Verification is cheap:** about 3 ms at cost 5,000 in wasm.
   - **The phones decide the cost:** at the default 1,000, the desktop median is about 25 ms.
5. **M5, later.** A `native` feature for Dioxus desktop and mobile: `altcha::solve_challenge` on `n` threads, joined through a oneshot.
   - It's opt-in, so liveview servers never do the proof of work themselves.
   - Dropped threads keep running until the timeout, because `solve_challenge` can't be cancelled. Unless an upstream PR adds a cancel flag, document it.

## 6. Open questions

- **Difficulty and mode:** these wait for M4. If the tail from §2.4 shows up on Android, switch to deterministic mode, with its own key-signature secret.
- **Crate split:** a framework-agnostic core in a crate named `dioxus-*` pulls Dioxus into other frameworks. When publishing, either split out the core (`solve`, `payload`, `WORKER_JS`), or put the Dioxus layer behind a default feature. Rafi is fine either way.
- **Per-client rate limit:** the join limit is per raffle, so posts with no proof of work still use up a raffle's budget. That's already true today; consider a per-IP limit separately.
- **Key rotation:** accepting a previous key next to the current one costs one extra HMAC on a miss, and still no key derivation. Not needed yet.
