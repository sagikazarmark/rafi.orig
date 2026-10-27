# Troubleshooting

## Phones

**Phones can't join: the button stays disabled, or the join fails.**
- The page must be served over **HTTPS**. Phones solve the proof-of-work with WebCrypto, which browsers only offer
  on HTTPS and `localhost`. On a LAN, use a tunnel or a certificate (see
  [Getting started](../development/getting-started.md#testing-with-phones)).
- Registration must be **open**: the join code works only then.
- Natively, check that the participant scripts are where the app expects (`RAFI_PARTICIPANT_DIR`). The app logs a
  warning at startup if they're missing: "phones can't join or follow the draw".

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

Natively, set `RUST_LOG` (for example `RUST_LOG=info,rafi_server=debug`). Both binaries log to stdout. On Workers,
invocation logs are deliberately off. Use `wrangler tail` while debugging, keeping in mind that it shows request
details.
