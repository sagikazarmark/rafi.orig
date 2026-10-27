# Configuration

Every setting of every Rafi deployable. To see how the deployables fit together, read
[Operations: overview](../operations/overview.md).

- [The app](#the-app-rafi-and-rafi-worker) (`rafi`, `rafi-worker`)
- [On Workers](#workers-only): the Worker's own variables, the engine's included (`rafi-worker`)
- [The raffle engine, native](#the-raffle-engine-rafi-restate) (`rafi-restate`)
- [Build time](#build-time)

Empty values count as unset everywhere. Flags take `1`/`true`/`yes`/`on`.

## The app (`rafi` and `rafi-worker`)

Natively these are environment variables. On Workers they are vars and secrets: a secret wins over a var with the
same name.

| Variable | Default | |
|---|---|---|
| `IP`, `PORT` | `127.0.0.1`, `8080` | Listen address (native only) |
| `RAFI_PUBLIC_DIR` | `target/dx/rafi-web/release/web/public` | The bundled web app, relative to the working directory (native only) |
| `RAFI_PARTICIPANT_DIR` | `apps/participant/dist` | The participant page's WebAssembly and fallback, from `./scripts/participant.sh` (native only) |
| `RAFI_PUBLIC_URL` | `http://localhost:8080` | External base URL, used for QR codes and links |
| `RAFI_COOKIE_SECURE` | on when `RAFI_PUBLIC_URL` is `https://` | Marks the participant cookie `Secure` (`1`/`true` or `0`/`false`). Set it when TLS ends at a proxy in front of a plain `http://` URL. |
| `RAFI_DATABASE_URL` | `sqlite://rafi.db` | The database, shared with `rafi-restate` (native only; Workers use D1). Either SQLite (`sqlite://…`, created if missing) or PostgreSQL (`postgres://user:password@host/db`; the database must exist). The app applies the migrations on start. |
| `RAFI_DEV_AUTH` | off | **Development only:** no login, and every admin is `dev`. Workers refuse it unless built with the `dev` feature. |
| `CLERK_SECRET_KEY` | | Clerk backend key. Required unless `RAFI_DEV_AUTH` is set. See also `CLERK_PUBLISHABLE_KEY` [at build time](#build-time). In a Clerk organization, the session's role decides what a member may do: `org:admin` everything; the custom role `org:manager` or permission `org:raffles:manage` (made in the Clerk dashboard) creates, configures and deletes raffles; any member runs them. |
| `RESTATE_INGRESS_URL` | | The raffle engine's Restate ingress. Without it, draws can't start (required on Workers). |
| `RESTATE_API_KEY` | | Restate Cloud: an API key with ingress access |
| `RESTATE_TIMEOUT_SECS` | `90` | How long a call to the raffle engine waits. A command queued behind a draw that is waiting for its beacon may take the engine's drand lead plus its fetch timeout. Past this timeout the command is accepted and applies later. Keep it above the engine's `lead_secs + timeout_secs`. |
| `RAFI_DRAND_CHAIN` | quicknet | The drand chain new raffles commit to, as a chain hash. Release builds accept only public drand chains. `mock` (a public test chain, **not** verifiable) needs a `mock-drand` build. The raffle engine must draw from the same chain: the app checks this and refuses to start a draw otherwise. |
| `RAFI_DRAND_MOCK` | off | Development alias for `RAFI_DRAND_CHAIN=mock` |
| `RAFI_ALTCHA_KEY` | random per start | HMAC key for the proof-of-work challenges. Required on Workers. Without it natively, challenges die with the process. Never rotate it during an event. |
| `RAFI_ALTCHA_COST` | `1000` | Proof-of-work difficulty: PBKDF2 iterations per attempt, about 256 attempts per solve. At least 1. Provisional. |
| `RAFI_MAX_ENTRANTS` | `2000` | The most entrants a raffle takes. This is the hard limit behind the soft rate limits. |
| `RAFI_RETENTION_DAYS` | `7` | Days after the event date before non-winners' personal data is erased, for organizers who choose nothing. At most `RAFI_RETENTION_DAYS_MAX`. |
| `RAFI_RETENTION_DAYS_MAX` | `30` | The longest period an organizer or a raffle may choose. At most 365. |
| `RAFI_RETENTION_WINNERS_DAYS` | `90` | How long a winner's data is held past the erasure date unless their prize is marked delivered: the hard ceiling, after which everyone is erased and a finished raffle is purged. |
| `RAFI_MIN_AGE` | `16` | The youngest an entrant may be. Rendered in the rules and on the join form; entrants confirm it by entering. Organizers raise it in their additional rules. |
| `RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL` | required | Who runs this installation: named as the processor in every raffle's privacy notice. Under `RAFI_DEV_AUTH`, placeholders. |
| `RAFI_DOCS_DIR` | required (native) | The directory with the installation's `terms.md` (with the data processing agreement) and `privacy.md`, as Markdown. On Workers they are bundled from `apps/worker/legal/`. Under `RAFI_DEV_AUTH`, placeholders marked as such. |
| `RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION` | required | The documents' versions, e.g. `2026-10`, shown with them at `/terms` and `/privacy` and named in every raffle's notice. Under `RAFI_DEV_AUTH`, `dev`. |
| `RUST_LOG` | `info` | Log level, in [`tracing-subscriber` filter syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) (native only) |

The app refuses to start (natively) or every request (on Workers) when a numeric variable doesn't parse, the
retention settings contradict each other, or the operator, the versions or the documents are missing (or still the
unreviewed templates) without dev auth.

`rafi reapply-erasures` (native only) is the command for after a database restore: it takes the same environment,
re-applies the erasure log, the purges and the organizers who left, runs housekeeping once, and exits.

### Workers only

The Worker hosts the app and the raffle engine (under `/restate`), so it takes the app's variables above and
these. Vars and secrets live in [`apps/worker/wrangler.toml`](../../apps/worker/wrangler.toml) and
`wrangler secret put`; the Worker reads nothing else.

| Variable | Default | |
|---|---|---|
| `RESTATE_IDENTITY_KEYS` | required | Restate's request signing keys (`publickeyv1_…`, comma separated). Without them the Worker refuses every request, except in a `dev` build. |
| `RAFI_DO_JURISDICTION` | `eu` | The Durable Objects' jurisdiction. Empty means none, which is what local development needs because workerd has no jurisdictions. |
| `RAFI_DRAND_RELAYS` | as `drand.relays` below | Relays the engine asks, in order, comma separated |
| `RAFI_DRAND_LEAD_SECS` | `6` | As `drand.lead_secs` below |
| `RAFI_DRAND_TIMEOUT_SECS` | `60` | As `drand.timeout_secs` below |
| `RAFI_DRAND_MOCK` | off | The mock chain, for the app and the engine alike (development only; the app reads it as `RAFI_DRAND_CHAIN=mock`) |

The bindings:

| Binding | |
|---|---|
| `DB` | The D1 database |
| `RAFFLE_ROOM` | The per-raffle live channel (`RaffleRoom` Durable Object) |
| `JOIN_PER_RAFFLE_LIMITER` | 600 joins per minute per raffle (soft, per location) |
| `JOIN_PER_EMAIL_LIMITER` | 5 joins per minute per raffle and email |
| `CONNECT_LIMITER` | 600 live-channel connections per minute |
| Cron `0 * * * *` | The hourly housekeeping run: erasing personal data on the dates, expired proof-of-work challenges, ending stale live channels |

## The raffle engine (`rafi-restate`)

Settings come from an optional configuration file (`--config <FILE>` or `CONFIG_FILE`; JSON, YAML or TOML),
overridden by `RAFI_RESTATE_` environment variables, with `__` separating levels. Unknown settings are errors.

| Setting | Variable | Default | |
|---|---|---|---|
| `database.url` | `RAFI_RESTATE_DATABASE__URL` | `sqlite://rafi.db` | The app's database (its `RAFI_DATABASE_URL`) |
| `endpoint.listener` | `RAFI_RESTATE_ENDPOINT__LISTENER` | `0.0.0.0:9080` | Listen address |
| `endpoint.identity_keys` | `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` | | Restate's request signing keys (`publickeyv1_…`, comma separated) |
| `drand.relays` | `RAFI_RESTATE_DRAND__RELAYS` | `api.drand.sh`, `api2`, `api3` (v2), `drand.cloudflare.com` (v1) | Relays to ask, in order. A list (`[a, b]`) of URLs with a `v1:` or `v2:` prefix; a bare URL means v1. Every beacon is BLS-verified, whichever relay serves it. |
| `drand.lead_secs` | `RAFI_RESTATE_DRAND__LEAD_SECS` | `6` | Seconds from a draw request to its committed round. Must be at least the chain period (3 s for quicknet). |
| `drand.timeout_secs` | `RAFI_RESTATE_DRAND__TIMEOUT_SECS` | `60` | Stop fetching after this many seconds. The draw then stays pending (stalled) on its round. |
| `drand.mock` | `RAFI_RESTATE_DRAND__MOCK` | `false` | The public test chain, for offline development. Draws are **not** verifiable, and the binary must be built with `--features mock-drand`. Must match the app's `RAFI_DRAND_CHAIN`. |
| `services.raffle.*` | | Restate's defaults | Restate policy for the `Raffle` object: timeouts, retention, retry policy and per-handler overrides |
| | `RUST_LOG` | `info` | Log level |

```toml
[database]
url = "sqlite:///data/rafi.db"

[endpoint]
identity_keys = ["publickeyv1_..."]

[drand]
lead_secs = 6

# Restate policy for the Raffle object: timeouts, retention, retry policy, per-handler overrides.
[services.raffle]
inactivity_timeout = "5m"
```

## Build time

| Variable | | |
|---|---|---|
| `CLERK_PUBLISHABLE_KEY` | `rafi-web` | Clerk's frontend key, compiled into the web app |
| Feature `mock-drand` | `rafi`, `rafi-server`, `rafi-restate` | Allows the mock chain. Never ship it. |
| Feature `dev` | `rafi-worker` | Local development only: allows `RAFI_DEV_AUTH`, the mock chain and Restate requests without identity keys. `[env.dev]` in `wrangler.toml` builds with it. |
