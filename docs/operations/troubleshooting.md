# Troubleshooting

## Phones

**Phones can't join: the button stays disabled, or the join fails.**
- The page must be served over **HTTPS**. Phones solve the proof-of-work with WebCrypto, which browsers only offer
  on HTTPS and `localhost`. On a LAN, use a tunnel or a certificate (see
  [Getting started](../development/getting-started.md#testing-with-phones)).
- Registration must be **open**: the join code works only then.
- Natively, the participant build is where the app expects (`RAFI_PARTICIPANT_DIR`), and is the build the binary
  was compiled against: the app doesn't start otherwise, and says so.

**"This email is already entered."** That's working as intended: one entry per email. The entry is safe, but it
stays on the device it was made on.

**Phones don't update.** Natively they poll, so allow a few seconds. They also re-fetch when brought back to the
foreground. On Workers, check that `RAFFLE_ROOM` is bound, and `wrangler tail` for "nudging … failed".

## Starting the draw

**"Freeze list & start the draw" fails, or "Retry handing the list to the raffle engine" appears.**
The list froze, but the raffle couldn't be opened in the engine. Check that:
- `RAFI_ENGINE_INGRESS_URL` (and `RAFI_ENGINE_API_KEY` for Restate Cloud) is set on the app. Without it, draws can't start,
  and the app logs this at startup.
- Restate is up, and the engine is registered with it (`curl localhost:9070/deployments` on a self-hosted Restate).
- The engine is up and can write to the database.

Then press **Retry handing the list to the raffle engine**. It picks up where it stopped.

**The engine is on a different drand chain.** The app's `RAFI_DRAND_CHAIN` and the engine's chain (`drand.mock` /
`RAFI_DRAND_MOCK`) must match. The app refuses to start a draw otherwise, and the engine refuses raffles committed
to another chain.

## During the draw

**A draw stalls: "Round R could not be fetched yet".**
The engine couldn't get the round's beacon within its fetch timeout (60 s). The draw stays committed to round R, so
nothing is lost. Check that the engine has outbound HTTPS to the drand relays (`api.drand.sh`, `api2.drand.sh`,
`api3.drand.sh`, `drand.cloudflare.com`). Then **Retry round R**. Only if drand stays unreachable should the
organizer **Abandon this draw**, which is public. See [The draw](../guide/the-draw.md#when-drand-cant-be-reached).

**A decision takes long, or answers "applies after the current draw".**
Commands for a raffle apply one at a time. A decision sent during a draw queues behind it. Past
`RAFI_ENGINE_TIMEOUT_SECS` the app reports it as accepted, and it applies once the draw settles. Retrying is safe:
every command has an ID and applies once.

**"Not here" / "Declined" are disabled.** A draw is counting down. Those would change who it can pick, so they wait.

## Verification

**The verification page says "Test chain: not publicly verifiable."** The raffle committed to the mock chain: the
app ran with `RAFI_DRAND_CHAIN=mock` in a `mock-drand` (or Workers `dev`) build. Never use such a build for a real
event. The admin shows a "mock drand" badge on such raffles.

**"This raffle hasn't been frozen yet, or it doesn't exist."** The verification data is published at the freeze.

## Logs

Natively, set `RUST_LOG` (for example `RUST_LOG=info,rafi_server=debug`). Both binaries log to stdout.

On Workers, the application's warnings and errors (an internal error behind a 500, a housekeeping step that failed,
the hourly housekeeping report) go to the console, and Workers Logs keeps them. The automatic per-invocation
records are deliberately off (`invocation_logs = false` in `wrangler.toml`): they would hold client IPs and
cookies. `wrangler tail` shows the console lines live, and shows each request's details too while it runs,
whatever the setting; use it while debugging, knowing that.

### Finding an organizer's error

When a call fails on the server's side, the admin shows the error with a reference, such as "internal error
(reference: 3f9a1c2b7d004e51)": the request's ID, which every response carries in `x-request-id`
([Request IDs](../reference/routes.md#request-ids)). Every line logged while that request ran carries it, with the
method and the route's pattern, and a request that ended in a server error leaves a warning of its own with its
status and latency:

```text
WARN request{id=3f9a1c2b7d004e51 method=POST route=/api/admin/{name}}: rafi_server::trace: request failed status=500 latency_ms=12
```

Natively, search the log for `id=<reference>`; the ID is on the lines at `info` and above (the default), so not under
`RUST_LOG=warn`. Every other request leaves a line at `debug` (`RUST_LOG=info,rafi_server::trace=debug`). On
Workers the reference is the request's Ray ID: search Workers Logs, or `wrangler tail --search <reference>`, for it.
