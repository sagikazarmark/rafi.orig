# Routes

Every URL a Rafi app serves. All of them are on one origin.

## Pages

| Path | What | Who |
|---|---|---|
| `/admin`, `/admin/r/<id>` | The organizer's admin: raffles, prizes, people, the draw console | Organizers (Clerk login) |
| `/s/<slug>` | The projector screen | Public |
| `/v/<slug>` | The verification page. It re-runs every draw in the browser. | Public, once the list is frozen |
| `/j/<CODE>` | The participant page: the join form, then the entrant's own card. Server-rendered. `POST` joins. | Public (the entry lives in a cookie) |
| `/rules/<slug>` | The raffle's rules | Public |
| `/privacy/<slug>` | The raffle's privacy notice: the organizer as controller, the erasure dates, the operator as processor, the organizer's own text | Public |
| `/privacy` | The installation's own privacy notice (the operator's; from `privacy.md`) | Public |
| `/terms` | The operator's terms, with the data processing agreement (from `terms.md`), which organizers accept by using the service | Public |

The web app (`rafi-web`) serves `/admin`, `/s/…` and `/v/…` as a single-page app, and anything unknown falls back to
its `index.html`. The server (`rafi-server`) renders `/j/…`, `/rules/…`, `/privacy/…`, `/privacy` and `/terms`.

## Public JSON

These never contain emails, tokens or claim codes.

| Path | What |
|---|---|
| `GET /api/r/<slug>/view` | The full view, which the projector and the admin poll: state, count, recent names, prizes, every draw, the server's time |
| `GET /api/r/<slug>/participant` | The participant view, which phones render: state, the current draw, winners by entrant ID, prizes left |
| `GET /api/r/<slug>/verify.json` | The manifest, its hash, the event log and display names ([format](draw-algorithm.md#verifyjson)) |
| `GET /api/challenge` | A proof-of-work challenge for the join form |
| `GET /config.js` | The web app's configuration, as a script `index.html` loads first (`window.__RAFI_CONFIG__`): Clerk's publishable key, or `null` under dev auth. The server alone decides how organizers sign in; nothing of an installation's is in the web app bundle. |
| `GET /healthz` | `ok` |

## Admin API

| Path | What |
|---|---|
| `POST /api/admin/<function>` | One function of the admin API: the arguments as a JSON object in, the result as JSON out. Requires a Clerk session, and every call checks that the raffle belongs to the caller (the active organization, or else the user). |

Both sides come from one declaration, `admin_functions!` in [`crates/api/src/admin.rs`](../../crates/api/src/admin.rs),
which also says how the functions are named. Among them: `whoami` (who is signed in and what they may do, whether
housekeeping looks stopped), `set_lifecycle` (publish, open, close), `deliver_all_prizes` ("All prizes delivered"),
`remove_my_data` (the organizer leaving), and `export_winners_csv`, which is audited.

`export_winners_csv` returns the drawn entrants as CSV text: UTF-8 with a byte order mark, a header line, every
field double-quoted (`"` doubled), a field starting with `=`, `+`, `-` or `@` prefixed with `'` so a spreadsheet
doesn't run it as a formula, columns `entrant_id,display_name,email,source,claim_code,outcome,prize,delivered`
(`outcome` is `drawn`, `claimed`, `not_here` or `declined`; `delivered` is `yes` or `no`). Every export is
audited with its row count.

A failed call answers with a non-2xx status and `{"code": "…", "message": "…"}`: the `code` is a stable
`snake_case` name a program can act on (a refusal's kind, such as `settings_fixed` or `forbidden`, or `not_found`,
`invalid_arguments`, `unauthorized`, `unavailable`, `internal`), the `message` is for the organizer. The status says
what kind of failure: 400 for a request that can never succeed, 401 without a session, 403 for the caller's tier,
404 for what isn't theirs, 409 for what the raffle's state refuses, 503 for an engine out of reach.

## Static files

| Path | What |
|---|---|
| `/participant/assets/` | The participant page's WebAssembly and its glue, named by their hash and served `immutable` (`RAFI_PARTICIPANT_DIR` natively) |
| `/participant/fallback.js` | Its JavaScript fallback, revalidated |
| `/participant/pow-worker.js` | The proof-of-work solver, served from the `dioxus-altcha` version built into the server |

## Workers only

| Path | What |
|---|---|
| `/partyline/raffle/<id>` | The raffle's live channel (WebSocket, through the `RaffleRoom` Durable Object). Available while the raffle is public and not over. |
| `/restate`, `/restate/*` | The raffle engine's Restate endpoint (`/restate/discover`, `/restate/invoke/…`), called by Restate Cloud with signed requests, which the SDK verifies before anything else. Not for browsers. |

On Workers, the paths the Worker answers before static assets are listed in `run_worker_first` in
[`apps/worker/wrangler.toml`](../../apps/worker/wrangler.toml). A reverse proxy in front of a native server can pass
everything through to `rafi`.
