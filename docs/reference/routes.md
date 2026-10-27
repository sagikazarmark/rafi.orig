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
| `GET /api/altcha` | A proof-of-work challenge for the join form |
| `GET /healthz` | `ok` |

## Admin API

| Path | What |
|---|---|
| `POST /api/admin/<function>` | One function of the admin API. Requires a Clerk session, and every call checks that the raffle belongs to the caller (the active organization, or else the user). |

Both sides come from one declaration, `admin_functions!` in [`crates/api/src/admin.rs`](../../crates/api/src/admin.rs).
Among them: `whoami` (who is signed in and what they may do, whether housekeeping looks stopped), `deliver_all`
("All prizes delivered"), `remove_my_data` (the organizer leaving), and `export_winners`, which is audited.

## Static files

| Path | What |
|---|---|
| `/participant/` | The participant page's WebAssembly and its JavaScript fallback (`RAFI_PARTICIPANT_DIR` natively) |
| `/participant/altcha-worker.js` | The proof-of-work solver, served from the `dioxus-altcha` version built into the server |

## Workers only

| Path | What |
|---|---|
| `/partyline/raffle/<id>` | The raffle's live channel (WebSocket, through the `RaffleRoom` Durable Object). Available while the raffle is public and not over. |

On Workers, the paths the Worker answers before static assets are listed in `run_worker_first` in
[`apps/worker/wrangler.toml`](../../apps/worker/wrangler.toml). A reverse proxy in front of a native server can pass
everything through to `rafi`.
