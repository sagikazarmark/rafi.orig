# altcha-demo

dioxus-altcha end to end, as a Dioxus fullstack app:

```sh
dx serve            # then http://127.0.0.1:8080
```

## Pages

- **`/`:** a form protected by the proof-of-work.
  - Server functions (`src/server.rs`) issue PBKDF2/SHA-256 challenges and verify the solutions, accepting each
    challenge once by its nonce.
  - The page has a difficulty slider (the server's `cost`).
  - It can solve on load (`Auto::OnMount`) or on submit (`Auto::Off` and `solved().await`).
  - After a join, attack buttons replay or change the accepted payload, and show why the server refuses each.
- **`/strict`:** the same page, served with `Content-Security-Policy: worker-src 'self'`.
  - The embedded `blob:` worker is blocked, so the workers load from `/altcha-worker.js`, which the server serves from
    `dioxus_altcha::WORKER_JS` (`SolveOptions::worker_url`).
  - A checkbox uses the embedded worker anyway, to show the fallback to solving on the page.
- **`/bench`:** measurements for whatever device opens it (the plan's M4).
  - Use `?costs=1000,5000&workers=1,2,4&runs=30`.
  - It shows the median and p95 solve times, the same challenges solved by `altcha`'s Rust in wasm, and one
    verification's time.
  - Use `dx serve --release`: the Rust-wasm comparison is several times slower in a debug build.

## Configuration

- **The server's HMAC key:** `ALTCHA_DEMO_KEY`, or random per start.
- **The replay set** is in memory: a real app keeps redeemed nonces in its database until they expire.
- **Phones:** WebCrypto needs HTTPS or localhost, so put a tunnel in front (e.g.
  `cloudflared tunnel --url http://localhost:8080`).
