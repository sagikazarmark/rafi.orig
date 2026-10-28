# Changelog

What changes between releases, for operators and organizers. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project is pre-1.0, so a minor version may change
what an operator configures or what a client reads, and says so here. A change to the shape of
[`deploy/cloudflare`](deploy/cloudflare) (its `wrangler.toml` or `.dev.vars.example`) is called out under
Operators, so an installation knows to compare its copy with the release's.

## Unreleased

The first release. What it is, in short: live raffles for meetups, verifiable by anyone, on Cloudflare Workers
(production) or self-hosted ([README](README.md)). The sections below record what the pre-release review changed
in what was already there ([docs/plans/pre-release-review.md](docs/plans/pre-release-review.md)), for anyone who
ran a development build before.

### Operators

- Environment variables are all `RAFI_*`: the app reaches the engine through `RAFI_ENGINE_INGRESS_URL`,
  `RAFI_ENGINE_API_KEY` and `RAFI_ENGINE_TIMEOUT_SECS` (were `RESTATE_*`); the Worker's identity keys are
  `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`; the native engine's config file is `RAFI_RESTATE_CONFIG`;
  `RAFI_RETENTION_MAX_DAYS` (was `RAFI_RETENTION_DAYS_MAX`); `RAFI_DO_JURISDICTION` takes `none` instead of an
  empty string. Every numeric variable refuses a typo.
- Nothing of an installation's is compiled in, so one build serves every installation. Clerk's publishable key
  is the server's (`CLERK_PUBLISHABLE_KEY`, required with `CLERK_SECRET_KEY`), handed to the web app at
  `/config.js`; the Worker reads the legal documents from `legal/` among its static files (`RAFI_LEGAL_DIR` and
  `build.rs` are gone), the way the native server reads `RAFI_DOCS_DIR`. A release ships
  `rafi-cloudflare-<version>.tar.gz` (the Worker, the static files, the D1 migrations) with `SHA256SUMS`, and
  [`deploy/cloudflare`](deploy/cloudflare) deploys it to an installation's account with its `wrangler.toml`,
  secrets and documents, by a Deploy to Cloudflare button or by hand; `scripts/worker-config.sh` and
  `wrangler.<site>.toml` are gone. The first deploy is in
  [docs/operations/cloudflare.md](docs/operations/cloudflare.md).
- An installation pins the release it deploys by content: `RAFI_SHA256` in `deploy.sh`, next to `RAFI_VERSION`,
  the tarball's SHA256 as the release page shows it. Pinned, `deploy.sh fetch` checks the download against it, so
  the installation's repository says which bytes it deploys and a release asset that changed under its version is
  refused; empty (as the template ships it), the download is checked against the release's `SHA256SUMS` as before.
  The release workflow checks the published tarball's digest, as GitHub computed it on upload, against the build's.
- Fail closed: `RAFI_ENGINE_API_KEY` is required on Workers; `RAFI_DEV_AUTH` needs a build with `--features dev`
  (native too); the native engine listens on `127.0.0.1:9080` and needs `identity_keys` or
  `endpoint.insecure = true`; `RAFI_ALTCHA_KEY` is at least 32 characters.
- The Worker logs its warnings and errors (Workers Logs keeps them; the per-invocation records stay off).
- Register the engine with Restate **without `force`**; a changed engine needs a new URI
  ([docs/operations/upgrades.md](docs/operations/upgrades.md)).
- The recorded acceptance of the terms is gone: acceptance is by use, and a change to the processing agreement is
  notified to organizers directly.
- `rafi reapply-erasures` (and the D1 statements) erase a purged raffle's entrants whether or not the erasure log
  came back with the backup.
- The audit log carries the acting user next to the owner, and names settings changes, deliveries and automatic
  finishes.
- The participant page's WebAssembly is a quarter smaller (200 KB gzipped, was 265: built as a binary by `dx`,
  not as a `cdylib`), and its module and glue are named by their hash and served `Cache-Control: immutable`, so a
  phone fetches them once, not five revalidations per page load. The server compiles the build's loader in:
  `just apps/participant/ build` runs before `cargo build`, `RAFI_PARTICIPANT_DIR` is the build's `dist/public`,
  and `rafi` refuses to start without it, or on a build other than the one it was compiled against. `dx` builds
  the web app with the release profile too (`opt-level = "z"`, as the binaries).

### Organizers

- A raffle can be deleted until its first draw is announced; after that its record stays and "Erase personal
  data now" is the way. Raffles whose data is erased are hidden from the list behind a switch.
- Nobody joins past the erasure date; the admin warns a day ahead when erasure is at midnight.
- Join codes are six characters, shown as `K7Q-2X3`; a purged raffle's code no longer answers.
- Display names, prize names and the organizer's name can't contain invisible characters and need a letter or
  digit.
- At most 1000 items (prize copies) per raffle; the entrant cap holds for manual and fake additions too.
- The winners CSV opens correctly in Excel (UTF-8 with a byte order mark).

### Clients of the public JSON and the admin API

- `verify.json` carries `format` and `v`, and is served with an `ETag`; `tools/verify.py` fails on an event type
  it doesn't know instead of skipping it, and checks the manifest's canonical form.
- Admin API errors carry a stable `code`; `lifecycle` is `set_lifecycle`, `deliver_all` is `deliver_all_prizes`,
  `export_winners` is `export_winners_csv`; arguments must be `application/json`.
- A phone module reading a state or a follow kind it doesn't know keeps following instead of freezing.
- The entry cookie is scoped to `/j/`.
