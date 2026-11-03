# Configuration

Every setting of every Rafi deployable. To see how the deployables fit together, read
[Operations: overview](../operations/overview.md).

- [The app](#the-app-rafi-and-rafi-worker) (`rafi`, `rafi-worker`)
- [On Workers](#workers-only): the Workers' own variables, the app Worker's and the engine Worker's (`rafi-worker-app`, `rafi-worker-engine`; locally `rafi-worker`)
- [The raffle engine, native](#the-raffle-engine-rafi-restate) (`rafi-restate`)
- [Build time](#build-time)

Empty values count as unset everywhere. Flags take `1`/`true`/`yes`/`on`.

The names: everything of Rafi's is `RAFI_*`. `RAFI_ENGINE_*` is how the app reaches the raffle engine (Restate's
ingress); `RAFI_RESTATE_*` is the raffle engine's own configuration (the `rafi-restate` binary natively, the same
settings on the engine Worker, with `__` between levels). Nothing of Rafi's is `RESTATE_*`:
that is `restate-server`'s own namespace, and the two often share an environment. `CLERK_*` is Clerk's.

## The app (`rafi` and `rafi-worker`)

Natively these are environment variables. On Workers they are vars and secrets, read alike (wrangler refuses a
var and a secret of the same name).

| Variable | Default | |
|---|---|---|
| `IP`, `PORT` | `127.0.0.1`, `8080` | Listen address (native only) |
| `RAFI_PUBLIC_DIR` | `target/dx/rafi-web/release/web/public` | The bundled web app, relative to the working directory: its `index.html` is read at start (and refused with an inline script), its `assets/` served (native only; the Worker compiles the `index.html` in) |
| `RAFI_PARTICIPANT_DIR` | `apps/participant/dist/public` | The participant build (`just apps/participant/ build`): the one the binary was compiled against (native only) |
| `RAFI_PUBLIC_URL` | `http://localhost:8080` | External base URL, used for QR codes and links, and its host for the live channel's socket in the pages' Content-Security-Policy |
| `RAFI_COOKIE_SECURE` | on when `RAFI_PUBLIC_URL` is `https://` | Marks the participant cookie `Secure` (`1`/`true` or `0`/`false`). Set it when TLS ends at a proxy in front of a plain `http://` URL. |
| `RAFI_DATABASE_URL` | `sqlite://rafi.db` | The database, shared with `rafi-restate` (native only; Workers use D1). Either SQLite (`sqlite://…`, created if missing) or PostgreSQL (`postgres://user:password@host/db`; the database must exist). The app applies the migrations on start. |
| `RAFI_DEV_AUTH` | off | **Development only:** no login, and every admin is `dev`. Both `rafi` and the Worker refuse to start with it unless built with the `dev` feature ([below](#build-time)). |
| `CLERK_SECRET_KEY` | | Clerk backend key. Required unless `RAFI_DEV_AUTH` is set. In a Clerk organization, the session's role decides what a member may do: `org:admin` everything; the custom role `org:manager` or permission `org:raffles:manage` (made in the Clerk dashboard) creates, configures and deletes raffles; any member runs them. |
| `CLERK_PUBLISHABLE_KEY` | | Clerk's publishable key (`pk_live_…`, `pk_test_…`). Required unless `RAFI_DEV_AUTH` is set: the server hands it to the web app at `/config.js`, which signs organizers in with it, and names the Clerk host it encodes in the web app's Content-Security-Policy, so a key no host decodes from is refused. Not a secret, but the installation's: nothing of an installation's is compiled into the web app. |
| `RAFI_CSP` | `enforce` | The pages' Content-Security-Policy ([routes](routes.md#security-headers)): `enforce`, or `report-only`, which sends it as `Content-Security-Policy-Report-Only`. Browsers then log what it would refuse in their consoles and refuse nothing; framing stays denied. An escape hatch for a policy that breaks the admin (Clerk loading from a host this release doesn't know, say) until an upgrade: the app logs a warning at start while it is on. |
| `RAFI_ENGINE_INGRESS_URL` | | The raffle engine's Restate ingress. Without it, draws can't start (required on Workers). |
| `RAFI_ENGINE_API_KEY` | | Restate Cloud: an API key with ingress access. Required on Workers (a `dev` build excepted). |
| `RAFI_ENGINE_TIMEOUT_SECS` | `90` | How long a call to the raffle engine waits. A command queued behind a draw that is waiting for its beacon may take the engine's drand lead plus its fetch timeout. Past this timeout the command is accepted and applies later. Keep it above the engine's `lead_secs + timeout_secs`. |
| `RAFI_DRAND_CHAIN` | quicknet | The drand chain new raffles commit to, as a chain hash. Release builds accept only public drand chains. `mock` (a public test chain, **not** verifiable) needs a `mock-drand` build. The raffle engine must draw from the same chain: the app checks this and refuses to start a draw otherwise. |
| `RAFI_DRAND_MOCK` | off | Development alias for `RAFI_DRAND_CHAIN=mock` |
| `RAFI_POW_KEY` | random per start | HMAC key for the proof-of-work challenges: at least 32 characters outside dev auth (`openssl rand -hex 32`), since the challenges are public and a short key could be cracked offline. Required on Workers. Without it natively, challenges die with the process. Never rotate it during an event. |
| `RAFI_POW_COST` | `1000` | Proof-of-work difficulty: PBKDF2 iterations per attempt, about 256 attempts per solve. At least 1. Provisional. |
| `RAFI_MAX_ENTRANTS` | `2000` | The most entrants a raffle takes, joined, added by the organizer or fake: enforced in the database, and an add that doesn't fit is refused whole. This is the hard limit behind the soft rate limits. |
| `RAFI_RETENTION_DAYS` | `7` | Days after the event date before non-winners' personal data is erased, for organizers who choose nothing. At most `RAFI_RETENTION_MAX_DAYS`. |
| `RAFI_RETENTION_MAX_DAYS` | `30` | The longest period an organizer or a raffle may choose. At most 365. |
| `RAFI_RETENTION_WINNERS_DAYS` | `90` | How long a winner's data is held past the erasure date unless their prize is marked delivered: the hard ceiling, after which everyone is erased and a finished raffle is purged. At most 365. The hold applies to every raffle as configured now, not as it was when the raffle was frozen: changing it moves the date held winners are erased, the one the privacy notices and the People tab show, for every raffle at once. Lowering it erases them earlier than they were told, which is allowed; raising it keeps them longer than they were told, so don't raise it while any raffle holds winners. (Each raffle's own erasure date, from its retention period, never moves.) |
| `RAFI_MIN_AGE` | `16` | The youngest an entrant may be. Rendered in the rules and on the join form; entrants confirm it by entering. Organizers raise it in their additional rules. At most 30. |
| `RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL` | required | Who runs this installation: named as the processor in every raffle's privacy notice. Under `RAFI_DEV_AUTH`, placeholders. |
| `RAFI_DOCS_DIR` | required (native) | The directory with the installation's `terms.md` (with the data processing agreement) and `privacy.md`, as Markdown. On Workers they are `legal/terms.md` and `legal/privacy.md` among the static files (`deploy/cloudflare`'s `legal/`; `just assets` in `apps/worker` copies them in from this variable), read through the assets binding. Under `RAFI_DEV_AUTH`, placeholders marked as such. |
| `RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION` | required | The documents' versions, e.g. `2026-10`, shown with them at `/terms` and `/privacy` and named in every raffle's notice. Under `RAFI_DEV_AUTH`, `dev`. |
| `RUST_LOG` | `info` | Log level, in [`tracing-subscriber` filter syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) (native only) |

The app refuses to start (natively) or every request (on Workers) when a numeric variable or `RAFI_CSP` doesn't
parse, the retention settings contradict each other, or the operator, the versions, the documents or a decodable
Clerk publishable key are missing (or the documents still the unreviewed templates) without dev auth.

`rafi reapply-erasures` (native only) is the command for after a database restore: it takes the same environment,
re-applies the erasure log, the purges and the organizers who left, runs housekeeping once, and exits.

### Workers only

In production on Workers the app and the raffle engine are two Workers ([Cloudflare
Workers](../operations/cloudflare.md#the-two-workers)): the app Worker (`rafi-worker-app`) takes the app's variables
above, the engine Worker (`rafi-worker-engine`, one per release) the engine's below. Their shapes are
[`apps/worker-app/wrangler.toml`](../../apps/worker-app/wrangler.toml) and
[`apps/worker-engine/wrangler.toml`](../../apps/worker-engine/wrangler.toml), which an installation deploys from
its copy of [`deploy/cloudflare`](../../deploy/cloudflare) (`wrangler.toml` and `engine.wrangler.toml`) with its
database ID in them. The app's installation values are secrets, listed under `[secrets] required` (set once
before the first deploy with `wrangler secret bulk .prod.vars`, then `wrangler secret put`; deploys keep them); the
engine has none, only vars in `engine.wrangler.toml`, which take effect with the next deploy (it redeploys the
release's engine Worker, the same code with the vars as they are then).
A Worker reads vars and secrets alike, and nothing else. Locally, `apps/worker` (`rafi-worker`) is both in one
Worker, the engine under `/restate`, and takes all of them.

| Variable | Worker | Default | |
|---|---|---|---|
| `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` | engine | required | Restate's request signing keys (`publickeyv1_…`, comma separated), under the native engine's name for the same setting. Without them the engine refuses every request, except in a `dev` build. |
| `RAFI_DO_JURISDICTION` | both | `eu` | The Durable Objects' jurisdiction, or `none` (which local development needs: workerd has no jurisdictions). The two Workers must say the same. |
| `RAFI_DRAND_RELAYS` | engine | as `drand.relays` below | Relays the engine asks, in order, comma separated |
| `RAFI_DRAND_LEAD_SECS` | engine | `6` | As `drand.lead_secs` below |
| `RAFI_DRAND_TIMEOUT_SECS` | engine | `60` | As `drand.timeout_secs` below |
| `RAFI_DRAND_MOCK` | both | off | The mock chain, for the app and the engine alike (development only; the app reads it as `RAFI_DRAND_CHAIN=mock`) |

The bindings:

| Binding | Worker | |
|---|---|---|
| `DB` | both | The D1 database (one, the same `database_id` in both) |
| `RAFFLE_ROOM` | both | The per-raffle live channel (`RaffleRoom` Durable Object): the app Worker's class; the engine binds it by the app's name (`script_name`) |
| `ASSETS` | app | The static files, through which the app reads the legal documents |
| `JOIN_PER_RAFFLE_LIMITER` | app | 600 joins per minute per raffle (soft, per location) |
| `JOIN_PER_EMAIL_LIMITER` | app | 5 joins per minute per raffle and email |
| `ADD_LIMITER` | app | 20 adds or undos of the organizer's per minute (per owner) |
| `CONNECT_LIMITER` | app | 600 live-channel connections per minute |
| Cron `0 * * * *` | app | The hourly housekeeping run: erasing personal data on the dates, expired proof-of-work challenges, ending stale live channels |

## The raffle engine (`rafi-restate`)

Settings come from an optional configuration file (`--config <FILE>` or `RAFI_RESTATE_CONFIG`; TOML or JSON),
overridden by `RAFI_RESTATE_` environment variables, with `__` separating levels. Unknown settings are errors.

| Setting | Variable | Default | |
|---|---|---|---|
| `database.url` | `RAFI_RESTATE_DATABASE__URL` | `sqlite://rafi.db` | The app's database (its `RAFI_DATABASE_URL`) |
| `endpoint.listener` | `RAFI_RESTATE_ENDPOINT__LISTENER` | `127.0.0.1:9080` | Listen address. Whoever reaches the endpoint supplies the raffle's state, so it is for your Restate server alone: loopback unless told otherwise, and never a public address. |
| `endpoint.identity_keys` | `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` | required | Restate's request signing keys (`publickeyv1_…`, comma separated). Without them the engine refuses to start, unless `endpoint.insecure` says the network is trusted. |
| `endpoint.insecure` | `RAFI_RESTATE_ENDPOINT__INSECURE` | `false` | Run without `identity_keys`: every request is taken as Restate's. For a development machine, or a private network with Restate alone on it. |
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

Nothing of an installation's: one build serves every installation. The features are development's.

| Variable | | |
|---|---|---|
| Feature `mock-drand` | `rafi`, `rafi-server`, `rafi-restate` | Allows the mock chain. Never ship it. |
| Feature `dev` | `rafi`, `rafi-worker` | Local development only. On `rafi`, allows `RAFI_DEV_AUTH`; on the Worker, also the mock chain and Restate requests without identity keys (`[env.dev]` in `wrangler.toml` builds with it). |
| `RAFI_VERSION` | `rafi-server` (`build.rs`) | The version compiled in, which `/healthz`, `whoami` and the footers show, and whose tag the source links point at. A release sets it (`just cloudflare-tarball`); unset, a checkout at a tag `v<version>` is that version, any other build `<crate version>-dev+<commit>`. |
| `RAFI_SOURCE_URL` | `rafi-server` (`build.rs`) | The repository the pages link as the source of the version running (AGPL §13), GitHub-shaped (`…/tree/<ref>`, `…/blob/<ref>/tools/verify.py`). Default: this repository. A fork that changes Rafi sets its own. |
