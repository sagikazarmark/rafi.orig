# Routes

Every URL a Rafi app serves. All of them are on one origin.

## Pages

| Path | What | Who |
|---|---|---|
| `/admin`, `/admin/r/<id>` | The organizer's admin: raffles, prizes, people, the draw console | Organizers (Clerk login) |
| `/s/<slug>` | The projector screen | Public |
| `/v/<slug>` | The verification page. It re-runs every draw in the browser. | Public, once the list is frozen |
| `/j/<CODE>` | The participant page: the join form, then the entrant's own card. Server-rendered. `POST` joins. Without registration, no form and no join: the organizer adds the people taking part, and the page links the rules, the notice and, once frozen, the verification page. | Public (the entry lives in a cookie) |
| `/rules/<slug>` | The raffle's rules | Public |
| `/privacy/<slug>` | The raffle's privacy notice: the organizer as controller, the erasure dates, the operator as processor, the organizer's own text | Public |
| `/privacy` | The installation's own privacy notice (the operator's; from `privacy.md`) | Public |
| `/terms` | The operator's terms, with the data processing agreement (from `terms.md`), which organizers accept by using the service | Public |

The web app (`rafi-web`) serves `/admin`, `/s/…` and `/v/…` as a single-page app: the server answers every path it
doesn't own with the web app's `index.html` (`GET` only; a miss under `/assets/`, `/participant/`, `/api/` or `/legal/`
is a 404). The server (`rafi-server`) renders `/j/…`, `/rules/…`, `/privacy/…`, `/privacy` and `/terms`.

## Security headers

Every response carries `X-Content-Type-Options: nosniff` and `Referrer-Policy: strict-origin-when-cross-origin`, and
every page a `Content-Security-Policy` of its kind (`crates/server/src/csp.rs`):

| Pages | Scripts | Also |
|---|---|---|
| `/j/…`, `/rules/…`, `/privacy/…`, a 404 | This origin's, WebAssembly, and the two inline ones (the head script, the participant build's loader) by their SHA-256 hashes | Styles from this origin only; images from this origin and `data:` |
| `/privacy`, `/terms` | As above | Images from any `https:` origin, which the operator's Markdown may link (its tables' columns aren't aligned: that takes `style` attributes) |
| The web app | This origin's, WebAssembly, `'unsafe-eval'` (Dioxus runs `document::eval` through `new Function`), and Clerk's: the Frontend API host the publishable key names, Cloudflare Turnstile, Clerk's fraud protection | Inline styles (Dioxus's `style` attributes, Clerk's CSS-in-JS), Clerk's images, frames and workers; none of Clerk's under dev auth |

All of them connect to this origin and the live channel's socket at `RAFI_PUBLIC_URL`'s host (`wss://` for an
`https://` URL, `ws://` for `http://`), post forms to this origin only, and allow no plugins and no `<base>`. None may be
framed (`frame-ancestors 'none'`, and `X-Frame-Options: DENY`) but the projector (`/s/…`) and the verification page
(`/v/…`), which venues and meetup sites embed. A response that isn't a page carries `frame-ancestors 'none'` alone.
A browser checks `frame-ancestors` only when a page loads, and the web app moves between its pages without loading
one, so the admin (`/admin…`) also refuses itself: inside a frame it shows only a link that opens it at the top
(`target="_top"`), where its own `frame-ancestors 'none'` applies.
The web app's page is revalidated on every load (`no-cache`, an `ETag`), and its `304` carries the same policy: a
browser keeps a `304`'s headers in place of the cached page's. `RAFI_CSP=report-only` sends the policies as
`Content-Security-Policy-Report-Only`, framing still denied ([configuration](configuration.md)).

The projector (`/s/…`) and the verification page (`/v/…`), for every raffle, and the JSON with the names behind
them (`/api/r/<slug>/view`, `/api/r/<slug>/verify.json`), carry `X-Robots-Tag: noindex`: they list entrants'
names until erasure, for the room and for whoever follows a link to check a draw, not for a search on someone's
name. A header, not a `<meta>` tag (the shell is one `index.html` for every path, and JSON has no place for one),
and not `robots.txt`, which would keep crawlers from ever seeing it while a page linked from elsewhere could still
be listed by its URL.

## Public JSON

These never contain emails, tokens or claim codes.

| Path | What |
|---|---|
| `GET /api/r/<slug>/view` | The full view, which the projector and the admin poll: state, count, recent names, prizes, every draw (each with `no_phone`: the winner was added by the organizer and no phone tells them, so the projector says "Come to the front!"), the server's time, whether people join with the QR code (`registration`; without it the projector shows no QR code), the organizer's name (`organizer`), the raffle's privacy notice (`privacy_url`, absolute), its verification page (`verify_url`, absolute), and whether anyone on the list was added from a list (`list_added`). Unversioned: it is for the web app of the same deployment and may change without notice in any release; a program reading a raffle reads `verify.json` |
| `GET /api/r/<slug>/participant` | The participant view, which phones render: state, the current draw, the latest winner's name and prize (and `no_phone`, for the projector, which takes the reveal from here), winners by entrant ID (whether they claimed, and whether the prize is sent later), prizes left |
| `GET /api/r/<slug>/verify.json` | The manifest, its hash, the event log, display names and the entrants counted by source ([format](draw-algorithm.md#verifyjson)) |
| `GET /api/challenge` | A proof-of-work challenge for the join form |
| `GET /config.js` | The web app's configuration, as a script `index.html` loads first (`window.__RAFI_CONFIG__`): Clerk's publishable key, or `null` under dev auth. The server alone decides how organizers sign in; nothing of an installation's is in the web app bundle. It also applies the saved colour scheme before first paint, which `index.html` can't do inline under its policy. |
| `GET /healthz` | `ok` |

## Admin API

| Path | What |
|---|---|
| `POST /api/admin/<function>` | One function of the admin API: the arguments as a JSON object in, the result as JSON out. Requires a Clerk session, and every call checks that the raffle belongs to the caller (the active organization, or else the user). |

Every call must say `Content-Type: application/json`, a function without arguments too (415 otherwise): a form on
another page can't send that type, and `fetch` from another origin can't without CORS, which the admin API doesn't
allow. A browser that sends `Sec-Fetch-Site` must say `same-origin` (or `none`): a page on a sibling subdomain is the
same *site*, so Clerk's `SameSite=Lax` cookie goes along with its request, which is refused with 403 `cross_site`.

Both sides come from one declaration, `admin_functions!` in [`crates/api/src/admin.rs`](../../crates/api/src/admin.rs),
which also says how the functions are named. Among them: `whoami` (who is signed in and what they may do, whether
housekeeping looks stopped), `set_lifecycle` (`publish`, `unpublish`, `open`, `close`: the ones the admin's view
allows are its `actions.lifecycle`; `open` without registration is 409 `no_registration`), `start_draw` (the freeze,
from Registration open or closed, or from Published without registration, `actions.start`), `save_settings` (with
`registration`, whether people join with the QR code, `true` when left out: switchable before the freeze but not
while registration is open, 409 `registration_locked`, `actions.edit_registration`; `save_defaults` takes it too),
`set_prize_delivery` (on the spot or sent
later, the one part of a prize that changes after the freeze), `deliver_all_prizes` ("All prizes delivered"),
`remove_my_data` (the organizer leaving), and `export_winners_csv`, which is audited.

`add_entrants` adds people the organizer vouches for, one person (`"origin": "one"`) or a list (`"list"`, at most
2000 rows, once the raffle is published and not in test mode), all or nothing under the entrant cap. Each add has a
`batch_id` the client makes at random (1 to 64 of `A-Z a-z 0-9 - _`): sent again, it answers the first add's result
and adds nothing, after `undo_add` too (the result is kept, without personal data, until the list is frozen). A row is `name`, optional `email` and `list_ref` (the registration ID), and `approved` (the
organizer approved a name the blocklist refuses: it lifts the blocklist only); a list's add may say how the browser
made its names (`"name_format": "full"` or `"first_initial"`), for the audit log. Every name added must be nobody
else's in the raffle (compared composed, lowercase, whitespace collapsed), joined or added, nor another row's. The
result names each row by its index: `added` with its entrant ID, or `skipped` (`already_in`, an email or
registration ID already in the raffle; `removed_at_request`, one of someone removed at their request, unless the
row says `"lift_removal": true`, which deletes that removal with the add). `undo_add` takes an add's entrants off the list again before
the freeze and returns how many; the admin's view offers the latest add (`actions.undo`), and says whether a list
can be added now (`actions.add_list`) and why not (`actions.add_list_refusal`: `unpublished`, `test_mode`, `frozen`,
`erasure_date_passed`), so the list tab derives nothing. `actions.missing` names what the settings lack for the
step that lets entrants in (opening registration, or without it the freeze from Published) while that step is
ahead, in Draft and Published: the header says so and keeps the step's button disabled. Each entrant says whether
they have `no_phone` (added by the organizer): drawn, the console has the organizer check it is them and confirm
("Confirmed: it's them"), which `resolve_item` records as `claimed`.

`remove_entrant` takes `"at_request": true` (optional, `false` when left out) for someone who asked: before the
freeze, a later add leaves them out by their email key and registration ID, which the admin's view lists
(`removed`, until the freeze) so the list tab and the one-person form can say so. `object_entrant` is "Took
themselves off", for someone the organizer added, from the freeze until the raffle is over (not for a winner who
claimed a prize), and `unobject_entrant` its undo; each entrant of the admin's view says whether it `objected` and
which of the two its `actions` allow. `resolve_item` refuses `claimed` and `not_here` for a winner who took
themselves off (409 `objected_winner`). It takes the draw the console shows, `k` (the view's `draws[].k`), and
refuses once another console forfeited that draw and drew the item again (409 `draw_changed`); without `k` it
resolves whichever draw holds the item.

`export_winners_csv` returns the drawn entrants as CSV text: UTF-8 with a byte order mark, a header line, every
field double-quoted (`"` doubled), a field starting with `=`, `+`, `-`, `@`, a tab or a carriage return prefixed
with `'` so a spreadsheet doesn't run it as a formula, columns
`entrant_id,display_name,email,source,claim_code,outcome,prize,delivered`
(`outcome` is `drawn`, `claimed`, `not_here` or `declined`; `delivered` is `yes` or `no`). Every export is
audited with its row count.

A failed call answers with a non-2xx status and `{"code": "…", "message": "…"}`: the `code` is a stable
`snake_case` name a program can act on (a refusal's kind, such as `settings_fixed` or `forbidden`, or `not_found`,
`invalid_arguments`, `unauthorized`, `unavailable`, `internal`), the `message` is for the organizer. The status says
what kind of failure: 400 for a request that can never succeed, 401 without a session, 403 for the caller's tier,
404 for what isn't theirs, 409 for what the raffle's state refuses, 413 for arguments over 512 KB (`too_large`), 429
over the organizer's rate limit on adds (`limited`), 503 for an engine out of reach. An add refused for its rows
(`rows`, 400) also carries `"rows": [{"row": 1, "problem": "email"}, …]`, each invalid row by its index (`name`,
`blocked_name`, `email`, `list_ref`, `duplicate_email`, `duplicate_list_ref`, and for a display name someone else
has, `name_taken` or `duplicate_name` with `"shown_as"`, the name as the other has it). On a server error (5xx)
the admin shows the organizer the request's ID as a reference (below).

## Request IDs

Every response from the server's routes carries an `x-request-id` header: natively 16 random hex characters, on
Workers Cloudflare's Ray ID (`cf-ray`). Every line the server logs while handling the request carries the same ID,
with the method and the route's pattern (`/j/{code}`, never the path itself), so an organizer's reference finds the
lines ([Logs](../operations/troubleshooting.md#logs)). Natively the static files carry one too; on Workers, files
Workers Assets serves, the live channel and the engine's endpoint don't pass the server's routes and have none.

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

On Workers, Workers Assets answers the paths that are files (`/assets/…`, `/participant/assets/…`,
`/participant/fallback.js`, `/legal/…`) and the Worker every other one, the web app's `index.html` included: it is
compiled into the Worker, not among the files, so that it is served with its policy
([`apps/worker/wrangler.toml`](../../apps/worker/wrangler.toml)). A reverse proxy in front of a native server can pass
everything through to `rafi`, and shouldn't add a `Content-Security-Policy` of its own: a browser enforces both.
