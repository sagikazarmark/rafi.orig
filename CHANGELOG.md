# Changelog

What changes between releases, for operators and organizers. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project is pre-1.0, so a minor version may change
what an operator configures or what a client reads, and says so here.

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
- The Worker's `wrangler.toml` is the production shape; an installation's values are secrets (`[secrets]
  required`), its D1 ID goes into a generated `wrangler.<site>.toml` (`scripts/worker-config.sh`), and its legal
  documents are bundled from `RAFI_LEGAL_DIR`. The first deploy is in
  [docs/operations/cloudflare.md](docs/operations/cloudflare.md).
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
