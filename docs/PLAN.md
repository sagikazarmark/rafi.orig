# Rafi: meetup raffle plan

Status: draft plan, 2026-10-05. Versions and platform facts were checked by research agents on that date.

**Revision 3.** Cloudflare is no longer a requirement. The app is one Rust server (Dioxus 0.7 fullstack on axum) backed by SQLite. The draw runs in a Restate virtual object on Restate Cloud (EU). Each gift gets its own draw, using a public drand round.

**Revision 4 (2026-10-06).** The Restate service is a separate component: `restate-rafi` (library `crates/restate-rafi`, binary `crates/restate-rafi-endpoint`), following the layout of the other `restate-*` repos. It writes the draw state straight into the app's SQLite database (`crates/store`, shared with the app), and the app watches for new versions and pushes them to the screens. Litestream is dropped. Production is one VM running `deploy/compose.yaml` (Caddy, the app, `restate-rafi` and a self-hosted Restate, built from the checkout); development runs Restate in Docker (`compose.yaml`) with the app and `restate-rafi` on the host. Where §8 and §11 still describe the in-process SDK tunnel, Caddy or Litestream, this revision supersedes them.

**Revision 5 (2026-10-09).** The draw engine is a seam: the app talks to it through `DrawEngine` (`crates/platform`), and the engine's rules (freeze, settling a draw with its beacon, every other command, how refusals are classified) live in `rafi-core`, so every engine only does I/O. Production's engine is still the Restate `Raffle` object, reached through `restate-rafi-client`; `rafi-engine-local` is an in-process engine for tests and development, never production. An engine conformance suite runs on both. The app decides the drand chain a raffle commits to (`RAFI_DRAND_CHAIN`); the engine reports its own (`RaffleEngine/info`) only for a check before freezing, and still refuses a mismatch. **The "local draw" fallback (§6.4) is dropped:** two engines owning one raffle could announce one draw on two rounds. When Restate is down, the organizer waits, or completes a stalled draw offline (dropped in revision 6). See [engine-plan.md](engine-plan.md).

**Revision 6 (2026-10-09).** The drawing is its own, raffle-free domain ([drawing-plan.md](drawing-plan.md)): `rafi-drawing` holds the drawing (opened from a published **manifest** naming its entrants, items, algorithm and beacon source), the draw algorithm and the verifier; `rafi-core` keeps the raffle's lifecycle. A raffle's prizes (with quantities) become the drawing's items, one per copy. The engine is `DrawingEngine` (`open`, `draw`, `decide`); the Restate object is `Drawing` with handlers `open`, `draw`, `decide` and `get`, next to the `DrawingEngine` service. **Offline draws are dropped** (`7b8721f`): they were the one way to settle a draw nobody can verify. When drand is down, a draw stalls on its committed round; the organizer retries it (the same round) or abandons it (§6.4). Older sections below keep the names of their time.

## Implementation status (2026-10-06)

Milestones M0–M7 are implemented (see the README for how to run them).

**Verified:**
- **Core:** `rafi-core` unit, property and vector tests pass.
- **Independent verifier:** `tools/verify.py` agrees with the vectors.
- **App tests (14, including the end-to-end test of the app, `restate-rafi` and `restate-server` sharing one database file):** cover joins, dedupe, freeze races, draws, no-shows, the claim flow, PII-free public views, drafts staying private, Clerk auth (via `dioxus-clerk` `TestClerk`), and retention.
- **Restate:** `restate-rafi` passes an end-to-end test against **`restate-server` 1.7.13** (spawned by `restate-e2e-harness`). It covers freeze idempotency, idempotency-key dedupe of draws, terminal errors reaching the caller, and `clear_all` on finish.
- **Live run against real drand quicknet:** a full dev-server run using `verify.py --fetch` re-checked every beacon against the public relays.
- **Browser check:** the projector, admin, phone and verify pages were checked in headless Chromium, both in dev and with a `dx bundle --release` build.
- **`restate-rafi` image:** builds (xx + cargo-chef, debian slim, non-root), serves discovery; the dev compose Restate registers and invokes it on the host.

**Deviations from the plan:**
- **One app crate:** shared UI lives in `crates/app`, with no separate `rafi-ui` crate.
- **Queries:** sqlx queries are checked at runtime, not compile-time `query!`, so there is no `.sqlx/` cache to maintain.
- **Proof of work:** ALTCHA-compatible, but implemented in-tree (about 100 lines of Rust plus a JS solver) instead of a crate or widget.
- **CI:** GitHub Actions instead of a Dagger module.
- **Draw engine as a separate service:** `restate-rafi` writes the draw state into SQLite itself (`rafi-store::save_draw_state`, newest version wins, only while the raffle is still drawn by Restate); the app polls recently updated Restate raffles every 250 ms. Both processes need the same database file, so they run on one host.
- **Restate ingress:** called over plain HTTP with `reqwest`, using the classic `/Raffle/{key}/{handler}[/send]` paths, instead of the generated ingress client. This avoids pulling in reqwest 0.13 twice.
- **Emergency switch:** "Switch to local engine" was implemented as a separate action (any pending Restate draw logged as failed). Removed with the stateless server, and dropped for good in revision 5.
- **UI (redesign, 2026-10-06):** Tailwind CSS v4 + daisyUI 5 with custom light/dark themes (system default, toggle everywhere), self-hosted display font (no Google Fonts, for GDPR), dioform for the organizer forms.
- **UI components:** the admin uses the dioxus-daisyui-components registry (copied into `crates/app/src/components/`, backed by `dioxus-primitives`, a pinned git dependency allowed in `deny.toml`): dioform-bound fields via `dioxus-field`, `AlertDialog` confirmations for freeze/finish/delete/erase, `DropdownMenu`, `Tabs`, and toasts. The phone page stays server-rendered without WASM, so it keeps plain daisyUI classes.

**Not yet verified (needs real credentials):**
- Restate Cloud. Connecting `restate-rafi` would use the standalone tunnel client container (`ghcr.io/restatedev/restate-cloud-tunnel-client`) and `RESTATE_RAFI_ENDPOINT__IDENTITY_KEYS`; not set up yet.
- Clerk with real keys. The SignIn, UserButton and OrganizationSwitcher widgets are wired; server-side verification is covered by tests.
- A deployment on a real VM with a domain and Clerk keys (the full compose stack was run locally: a real drand draw through Caddy, Restate and `restate-rafi`).

## 1. Summary

A live raffle for meetups.

- **Before the event:** the organizer sets up a raffle and its prizes in the admin UI (Clerk login).
- **During the meetup:** a **projector screen** shows the intro, a QR code and a live count. Attendees join on their phones with a **display name, email and consent to the rules**. There is no account. The organizer can **add people who have no phone**.
- **Freezing the list:** when the draw starts, the entrant list is frozen and published. The list of entrant IDs (no personal data) goes to a Restate virtual object.
- **Drawing each gift:**
  - The organizer picks the next gift and presses **Draw**.
  - The virtual object commits to a **drand round about 6 s in the future** and announces it. That gap is the suspense countdown.
  - It sleeps durably until that round exists, fetches the beacon, and picks a winner from the remaining pool.
  - The projector and the winner's phone update instantly over SSE. Everyone else sees "not this time".
- **No-show:** the organizer presses "Not here". That person is out, and the same gift is drawn again with a new round.
- **The end:** everyone sees "thanks for playing", and a public page lets anyone re-run every draw.

**Stack:**
- **One Rust binary:** Dioxus **0.7.10** fullstack (axum) serves the admin and projector UI and the server functions. Plain axum + `dioxus::ssr` serves a no-WASM attendee page. Live updates use SSE from per-raffle `tokio::sync::watch` channels.
- **SQLite** (sqlx 0.9), replicated to EU object storage with **Litestream**.
- **Restate:** `restate-sdk` **0.12** runs inside the same binary and connects *out* to **Restate Cloud (EU)** through the SDK's tunnel.
- **Auth:** **Clerk** via `dioxus-clerk` 0.6 (`ClerkAuthLayer` + `current_auth()`).
- **Hosting:** **Hetzner Cloud** (Germany) with Caddy, deployed with docker compose from Dagger CI.

## 2. What the original idea was missing

These are the gaps the review found. Each one is resolved later in the plan.

1. **Coming back without an account:** a closed tab, a locked phone, a switched browser (§7).
2. **Registration only at the end:** people who leave early can't enter, the wifi gets hammered all at once, and there's dead air on stage (§4).
3. **No-shows:** the most common live failure. Resolved: draw again, and the no-show is out.
4. **Multiple wins:** resolved. One win per person.
5. **Proving the winner at the stage:** someone shouts "That's me!" from the back. Resolved with a **private** claim code. Only the winner's phone and the organizer's admin view show it, never the projector.
6. **Perceived rigging:** resolved with a verifiable draw for each gift (§6).
7. **Items with quantity > 1, and draw order:** each unit is drawn separately. The order can be chosen live, because no outcome can be known in advance.
8. **Remote entries:** a photo of the QR code gets posted in Slack.
9. **Spam and bots** on the public join endpoint.
10. **GDPR:** lawful basis, a privacy notice, separate unticked consents, retention and deletion. The projector shows names only, never emails.
11. **Hungarian gambling and tax rules:** a free draw is fine, but some details need confirming (§10).
12. **Phones sleeping:** locked phones drop their connection, so the state has to come back from the server.
13. **Projector vs control:** the projector must never show emails or admin buttons. Control runs on the organizer's phone.
14. **Undo and live changes:** mis-clicks, a sponsor bringing 3 vouchers instead of 5, a winner declining.
15. **Rehearsal:** a test mode, so practice runs don't pollute real data.
16. **After the event:** a winners export for sponsors, voucher delivery, purging the data.
17. **Hungarian name order:** "Kovács Bence" has the family name first, so "first name + last initial" can't be derived. Ask for a **display name** instead. That also keeps the data minimal.
18. **Language:** HU and EN.

## 3. Decisions

### Decided

| Topic | Decision |
|---|---|
| Multiple wins | **One win per person.** Once someone is drawn, they leave the pool, whatever the outcome. If the pool runs dry, the next pass draws from previous winners (§6.4). |
| No-show | **Draw again; the no-show is out.** "Not here" or "Declined" removes them, and the gift is drawn again with a new round. Every case is logged publicly. |
| Fairness | **Verifiable, one draw per gift.** The entrant list is frozen and published. Each draw commits to a future drand round before that round exists. Selection is deterministic and anyone can re-run it. |
| Draw engine | **Restate virtual object `Raffle`**, keyed by raffle ID. It receives **opaque entrant IDs only**, never emails or device IDs. |
| Restate hosting | **Restate Cloud, EU region.** The SDK's `tunnel` feature opens an outbound HTTP/2 connection, so no inbound port is needed. |
| App server | **One long-running Rust binary:** Dioxus 0.7.10 fullstack on axum, a single instance. |
| Live updates | **SSE** from a `tokio::sync::watch` channel per raffle. A new connection gets the latest snapshot immediately. Phones use the browser's native `EventSource`. |
| Attendee page | Plain axum + `dioxus::ssr` HTML with a small inline script. **No WASM on phones.** Dioxus can't skip hydration per route, so `/j/*` is mounted as a separate axum route. |
| Database | **SQLite** (sqlx 0.9, WAL mode, embedded migrations) + **Litestream** to an EU S3-compatible bucket. |
| Hosting | **Hetzner Cloud CX23** (about €6/month, Germany) + Caddy (TLS, HTTP/2) + docker compose. Fallback: Fly.io `ams`; see §8.7 for its iOS SSE caveat. |
| Auth | Clerk via **`dioxus-clerk` 0.6**. `ClerkAuthLayer` doesn't reject requests on its own, so every admin server function calls `require_owner()`. |

### Your call (recommendation first)

| Topic | Recommendation |
|---|---|
| When registration opens | **At the start of the meetup**; freeze when the draw starts. It's a button, not code. §7 explains why closing a tab doesn't hurt. |
| Shown on the projector | The display name as typed (max 24 characters). One tap to hide or rename an entrant. |
| Late arrivals | Can join until "Start the draw". After that the list is frozen, which is the price of verifiability. |
| Gift order | The organizer picks live. The admin view suggests "smallest prize first". |
| Eligibility | Organizers and volunteers are excluded. Sponsor staff can't win their own sponsor's prize. Speakers are eligible. All of this is in the rules. |
| Minimum age | 16+, the Hungarian digital consent age. Voucher terms may require 18+. |
| Retention | Personal data of non-winners is purged 30 days after the event. Winners' data is purged when marked delivered, or after 90 days. The anonymous draw log is kept forever. Restate data is cleared at the end of the raffle (§8.4). |
| Bot protection | **ALTCHA** (self-hosted proof of work, no third party) plus unique emails, a rate limit and a join window. Turnstile would add Cloudflare as a US data processor. |
| Winner emails | **Not in v1.** Phase 2, using Scaleway TEM, which keeps data in the EU. |
| Co-hosts | A raffle is owned by the active Clerk organization if there is one, otherwise by the user. Clerk Hobby includes organizations for free. |

## 4. The event, end to end

| When | Organizer (phone or laptop, `/admin`) | Projector (`/s/{slug}`, public) | Attendee phone (`/j/{CODE}`) |
|---|---|---|---|
| Before | Create the raffle, add items (quantity, sponsor). Set the title, rules and date. Optionally rehearse in **test mode**. | | |
| Meetup starts | Publish, then **Open registration**. | **Intro:** the prizes, the rules in short, "no phone? come to me", a big QR code plus a short URL and code (`rafi.app/j/K7Q2`). A live count and recent names. | Join form: display name, email, ☐ rules & privacy (required), ☐ emails about future meetups (optional). Then **"You're in!"** with a private claim code. |
| During | **Add entrants without a phone** (name, optional email). Hide or rename trolls; remove duplicates. | Same intro; the count grows. | The page can be closed. Reopening it restores the state (§7). |
| Start | **Start the draw:** closes registration, freezes and publishes the list, sends the IDs to the virtual object. | "N entrants, M prizes". | "Draw starting!" (asks to keep the screen on). |
| Each gift | Pick the gift, then **Draw**. The winner appears; the admin view shows their **claim code**. Hand over the gift, then **Confirm**, or **Not here** / **Declined**, which puts the gift back for a new draw (not while another draw is pending: that could change its pool, so the organizer waits for it to settle; **Confirm** always works). | Gift card, then **"Locking in drand round R…"** with a ~6 s countdown and animation (a plain fade if reduced motion is on), then the **winner's name**. | During the countdown: "Drawing <gift>…". Winner: full-screen **YOU WON <gift>** with the claim code, a sound, and vibration on Android. Everyone else: "Not this time — 4 prizes left". |
| End | **Finish** once nothing is left. **Export** the winners CSV. | **Thanks for playing!** Winners, sponsors, and a link to the verification page. | Thanks screen. Winners still see their prize. |
| After | Mark prizes delivered. The purge runs on schedule. | | `/v/{slug}` is the public verification page. |

The admin view is designed for the organizer's phone. The projector page is public and read-only, with a fullscreen button that also keeps the screen awake. It reconnects SSE by itself after the laptop sleeps.

## 5. State machines

### Raffle (app, SQLite)

```
Draft ──publish──▶ Ready ──open──▶ Open ◀──reopen── Closed
                     ▲               │ close ─────────▶ │
                     └──unpublish────┘                  │
        Open|Closed ──start_draw──▶ Frozen   (list published; VO.freeze called)
        Frozen ──first draw──▶ Drawing ──finish (or nothing left)──▶ Finished ──retention──▶ Purged
```

- Items and quantities can be edited until Frozen. From Drawing on, units can only be **voided** (logged), for example when a sponsor didn't bring them.
- Joins and manual adds are allowed until Frozen.
- `start_draw` runs as one SQLite transaction: set `state='frozen'`, which stops joins; read the entrant IDs; write the commit document. Only after that is the virtual object called.

### Draw state (Restate virtual object `Raffle`, key = raffle ID)

```
              freeze(ids, units, list_hash)            (once; same input again → no-op, different → TerminalError)
                         ▼
 unit: Pending ──draw──▶ AwaitingBeacon(round R) ──beacon──▶ Drawn(entrant)
                                     │ beacon unavailable after 60 s
                                     └──▶ Pending (draw failed, logged; organizer presses Draw again → new round)
       Drawn ──confirm──▶ Claimed
       Drawn ──not_here | declined──▶ Pending (entrant excluded; UI immediately calls draw again)
       Pending ──void──▶ Voided
 finish ──▶ hands the final log to the app, then clear_all()
```

**Entrant (app view):**
- Active
- Hidden (name masked, still eligible)
- Removed (before Frozen only)
- Drawn, which then becomes one of Claimed, Forfeited or Declined, and finally Delivered

**The attendee phone view** is a pure function of the public snapshot and the phone's own `entrant_id`. The views are:
- joined / waiting
- draw starting
- drawing, still in
- **YOU WON**
- not this time
- thanks
- registration closed (shown to non-entrants)

## 6. Verifiable per-gift draw (`rafi-draw-v1`)

### 6.1 Why this works

- **The list is frozen and published before any randomness exists.** Nobody can add or remove people to steer an outcome.
- **Each draw commits to a drand round R that hasn't happened yet.** R is computed, journaled in Restate and announced on the projector before the beacon is published. At commit time nobody knows the result, not the organizer and not the server.
- **Restate's durability makes the result final.** R is journaled before the sleep. If the process crashes or the deploy restarts, replay resumes with the **same** R, so the same winner comes out. Nobody can "retry until they like it".
- **The gift can be picked live.** The organizer chooses which gift to draw before knowing who will win it, so a fixed order doesn't need to be committed.
- **The remaining levers are public.** "Not here", "Declined", "Void" and failed beacon fetches all appear in the public log, and the affected person is in the room.

### 6.2 Algorithm

```
freeze:   entrants      = sorted hex IDs (random 128-bit)
          manifest      = canonical JSON {algo:"rafi-draw-v1", drawing, beacon:{drand:<chain hash>}, entrants, items:[{id,name}]}
          manifest_hash = SHA256(manifest)                         — published at /v/{slug} before the first draw

draw k:   R            = floor((now + 6s − 1692803367) / 3) + 1    — quicknet, journaled via ctx.run
          beacon       = GET {relay}/{chain}/public/{R} from api.drand.sh AND drand.cloudflare.com (425 → retry)
                         require both equal; randomness = SHA256(signature) (BLS verify: later, `drand_core`)
          pool_k       = sorted(entrant_ids − everyone drawn in draws 1..k−1)
                         (if empty and items remain → pass 2: pool = Claimed winners not yet drawn in pass 2)
          seed_k       = SHA256("rafi-draw-v1" ‖ randomness ‖ manifest_hash ‖ BE_u64(k))
          rand_u64(c)  = BE_u64(SHA256(seed_k ‖ BE_u64(c))[0..8])
          uniform(n)   : limit = 2^64 − (2^64 mod n); c = 0; loop { x = rand_u64(c++); if x < limit return x mod n }
          winner_k     = pool_k[uniform(|pool_k|)]
```

- `k` is a sequence number for the whole raffle that counts every draw attempt, including no-show draws. Failed beacon fetches are logged but don't use up a `k`.
- Don't use `ctx.rand()`. It is seeded from the invocation ID, which the SDK itself calls "predictable", and outsiders can't verify it.
- Don't use the `rand` crate. Its outputs aren't guaranteed to stay the same across versions. Only `sha2` is needed.
- **Chain:** quicknet `52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971`, genesis 1692803367, period 3 s.
- The SDK host clock must be NTP-synced. R comes from the host's wall clock inside `ctx.run`, and the sleep is `R_time − now + 0.5 s`.

### 6.3 Verification

`/v/{slug}` publishes:
- the manifest (entrant IDs, plus display names until the purge, so people can check they're in);
- every draw: `k`, item, R, signature, randomness, pool size, winner index, winner ID and outcome;
- an in-browser **re-run** (WebCrypto + BigInt).

Also ship `tools/verify.py` (about 40 lines, `hashlib` only) and `testdata/draw-vectors.json`. CI checks the Rust code and the Python verifier against the same test vectors.

### 6.4 Edge cases

- **More units than eligible entrants:** pass 2, as described in §6.2.
- **Nobody left at all:** "no eligible entrants", and the unit stays Pending.
- **drand outage:** the engine retries the beacon fetch up to its timeout; if the beacon can't be had, the draw **stalls**: it stays pending on its committed round, and retrying it fetches the same round, so nobody can force a redraw on a new one. The only way out is an explicit **abandon**: logged publicly (`fail`), the item returns to Pending, and the verifier shows whom the abandoned round would have picked. There is no "offline draw" (server randomness; dropped in revision 6).
- **Restate Cloud outage during the event:** ~~the admin view offers "local draw"~~ (dropped in revision 5: a second engine owning the raffle could announce a draw on another round). The app refuses to start a draw while the engine is unreachable; a draw that stalls stays pending on its round, and the organizer retries it or abandons it.

## 7. Attendee identity and coming back

> *"If registration is open before the draw and the user closes the tab, how do they get back? There's no user and password."*

There's no account, but there is an **entry**. When someone joins, the server creates two values:
- a **public entrant ID**, random and 128-bit, which appears in the published list and in the snapshot if they win;
- a **secret token**, also random and 128-bit, stored as an **HttpOnly, Secure, SameSite=Lax cookie** for that raffle until it finishes plus one day. Only `SHA256(token)` is stored.

The server-rendered page includes the entrant ID and the private claim code.

| Situation | What happens |
|---|---|
| The phone locks, the tab is closed and reopened, or the QR code is scanned again | Same browser, same cookie. `GET /j/{CODE}` renders the current state from the server, e.g. "You're in, claim code 7KQ2". |
| The page comes back from the background | `EventSource` reconnects by itself. The script also forces a reconnect on `visibilitychange`, `pageshow` or `online` if no event or heartbeat arrived in the last 20 s. The new connection gets the latest snapshot, so there's nothing to replay. |
| A different browser, private mode, or cleared cookies | The **same email** finds the existing entry. Emails are normalised: lowercased, trimmed, `+tag` removed, dots removed for Gmail. The page says *"You're already in as 'Bence K.' — your entry is safe; if you win, the organizer will confirm you by email."* The entry is **not** moved to the new device; otherwise anyone could take over someone's entry by typing their email. |
| Later | A magic-link email to move the entry to another device (phase 2). |
| No phone | The organizer adds them. They watch the projector and get confirmed by name in the admin view. |

The projector is the source of truth. A fake "you won" screen made with someone else's public ID won't work, because it lacks their private claim code.

## 8. Architecture

```
                       ┌────────────────────── rafi (one Rust binary, Hetzner CX23) ──────────────────────┐
 Organizer ── HTTPS ──▶│ Dioxus fullstack router (SSR + hydration)                                       │
 (admin, phone)        │   /admin/**  server fns: require_owner(current_auth())                          │
 Projector ───────────▶│   /s/{slug}, /v/{slug}                                                          │
 Phones ──────────────▶│ axum routes (no WASM):                                                          │
   via Caddy (TLS)     │   GET /j/{CODE}  dioxus::ssr HTML   POST /api/j/{CODE}/join  (ALTCHA, governor) │
                       │   GET /api/r/{id}/events  SSE ◀── Hub: watch<RaffleView> per raffle            │
                       │ Restate endpoint: VO `Raffle` ── tunnel (outbound h2) ──▶ Restate Cloud (EU)    │
                       │   ctx.run(project) → SQLite upsert + Hub publish                                │
                       │ Ingress client (reqwest) ── POST /restate/send/Raffle/{id}/draw ──▶ Restate Cloud│
                       │ SQLite (WAL) ── Litestream ──▶ EU object storage                               │
                       │ tokio interval: daily retention purge                                          │
                       └─────────────────────────────────────────────────────────────────┬───────────────┘
                                                                                         ▼ (from VO, in ctx.run)
                                                                                drand quicknet relays
```

### 8.1 Routes

| Path | Served by | Auth |
|---|---|---|
| `/`, `/admin/**` | Dioxus fullstack | Clerk. Server functions enforce owner = `sub`, or `o.id` for an organization. |
| `/s/{slug}`, `/v/{slug}` | Dioxus fullstack | public |
| `/j/{CODE}` | axum + `dioxus::ssr` | entrant cookie |
| `/rules/{slug}`, `/privacy` | axum + `dioxus::ssr` | public |
| `POST /api/j/{CODE}/join` | axum | ALTCHA + rate limit |
| `GET /api/j/{CODE}/me` | axum | cookie, `no-store` |
| `GET /api/r/{id}/events` | axum SSE | public (snapshot never holds personal data) |
| `GET /api/r/{id}/verify.json` | axum | public |

Register `/j/*` and `/api/*` before the Dioxus fallback.

### 8.2 Who owns what

| Data | Owner | Notes |
|---|---|---|
| Raffle config, items, units, entrants (personal data), consents, claim codes, cookie hashes | **SQLite** | Personal data never leaves the app. |
| Commit document and list hash | **SQLite** | Written in the `start_draw` transaction. |
| Live draw state: pool, per-unit status, draw counter `k`, pending round | **Restate VO state** | Only authoritative during Frozen/Drawing. Cleared at finish. |
| Draw log (permanent record) and live projection | **SQLite `draws`** | Written by the VO from inside `ctx.run`, upserted by `(raffle_id, k)`, so delivery is at-least-once but safe to repeat. |
| Live view | **Hub** (in memory) | Rebuilt from SQLite on startup. |

**SQLite tables:**
- `raffles`: id, owner_id, slug, join_code, title, config_json, state, test_mode, event_at, purge_at
- `items`
- `units`
- `entrants`: id, raffle_id, display_name, email_norm, email_raw, source, hidden, status, claim_code, token_hash, consent_*, created_at. `UNIQUE(raffle_id, email_norm)`.
- `commits`
- `draws`: raffle_id, k, unit_id, round, signature, randomness, pool_size, winner_idx, entrant_id, outcome, at
- `audit`: app-level admin actions

A single instance plus SQLite transactions is enough for app-side commands. A join is a guarded insert:
```sql
INSERT … SELECT … WHERE EXISTS (SELECT 1 FROM raffles WHERE id=? AND state='open')
```

### 8.3 Restate virtual object `Raffle`

```rust
struct Raffle { db: SqlitePool, hub: Hub, drand: DrandClient }

#[restate_sdk::object(journal_retention = "1h", idempotency_retention = "1h")]
impl Raffle {
    #[handler] async fn freeze(&self, ctx: ObjectContext<'_>, input: Json<Freeze>) -> HandlerResult<()>;
    #[handler] async fn draw(&self, ctx: ObjectContext<'_>, unit: String) -> HandlerResult<Json<DrawResult>>;
    #[handler] async fn resolve(&self, ctx: ObjectContext<'_>, r: Json<Resolve>) -> HandlerResult<()>;  // confirm | not_here | declined
    #[handler] async fn void(&self, ctx: ObjectContext<'_>, unit: String) -> HandlerResult<()>;
    #[handler] async fn finish(&self, ctx: ObjectContext<'_>) -> HandlerResult<()>;                     // project final log, clear_all()
    #[handler] async fn get(&self, ctx: SharedObjectContext<'_>) -> HandlerResult<Json<DrawState>>;     // recovery/debug only
}
```

**`draw(unit)` (exclusive, so draws for the same raffle queue up behind each other):**
1. Load state and validate: the unit is Pending and the pool is non-empty. Otherwise return a `TerminalError`.
2. `let (k, round, wake_in) = ctx.run(|| compute_round(now)).name("round")`. This is journaled.
3. `ctx.set("state", …AwaitingBeacon{unit, k, round})`, then `ctx.run(project)`. The projector starts its countdown, and **R is public before it exists**.
4. `ctx.sleep(wake_in)`. This is a durable timer.
5. `let beacon = ctx.run(|| drand.fetch_both(round)).name("beacon").retry_policy(… max_duration 60s)`. If it fails: log "draw failed", set the unit back to Pending, project, and return.
6. Run the pure `rafi_core::draw::select(randomness, list_hash, k, pool)`, set state, then `ctx.run(project)`. The SSE push reaches every screen within milliseconds.

**Calls from the app:**
- The app uses the generated `RaffleIngressClient` (`reqwest-client` feature) with an API key and `.idempotency_key(command_id)`.
- The admin UI generates one `command_id` per button press, so double taps are harmless.
- `draw` is called with **`send`**, which doesn't block the request for 6 s. The result arrives over SSE.

**`project`:** the closure runs in the same process. It upserts `draws` and the unit and entrant status in SQLite, then publishes a new `RaffleView` to the hub. It is idempotent by `(raffle_id, k, phase)`.

**Restate cost:** about 8 actions per draw, so a 20-gift event with a few no-shows uses **about 200–300 actions**. That fits easily in the Free tier (50k–100k per month; the sources disagree, so assume 50k).

**To check in M2:**
- Shared `get` should see state committed earlier by a still-running exclusive invocation. The docs and examples say yes; an old blog post says no.
- Tunnel maturity. The tunnel is new in 0.12, and the docs still describe the separate tunnel container. Fallbacks: (a) that container, or (b) a public endpoint mounted with `Router::nest_service("/restate", …)` via `Endpoint::handle`, behind Caddy with h2c, plus `identity_key`.

### 8.4 Restate and personal data

- **Personal data never goes to Restate.** Inputs are opaque entrant IDs, unit IDs and `list_hash`.
- Restate journals and state still contain those IDs. Retention is set to 1h at the service level (the Free plan caps history at 1 day). `finish` calls `clear_all()`.
- Restate GmbH is in Berlin. It has no public DPA, so ask dataprotection@restate.dev and list it in the notice to be safe, even though it holds no personal data.

### 8.5 Live updates (SSE)

- **Hub:** `HashMap<RaffleId, watch::Sender<RaffleView>>`.
- **`RaffleView`:**
  - `{v, phase, server_now, entrant_count, recent_names[≤20, not hidden], units[status], current_draw?{unit, round, round_at}, draws[{k, unit, item, entrant_id, display_name, outcome}], prizes_left, list_hash?}`
  - **No emails, tokens or claim codes**, ever.
- **Endpoint:** `GET /api/r/{id}/events` sends `WatchStream` events as `json_data`, with `KeepAlive` every 15 s.
- **Clients:**
  - **Phones:** native `EventSource`, which reconnects automatically, plus the forced reconnect on `visibilitychange` described in §7.
  - **Projector and admin:** a Dioxus hook around `ServerEvents`, or `EventSource` via web-sys, with a manual reconnect loop. `ServerEvents` doesn't reconnect by itself.
- **Countdown sync:** clients use `server_now` and `round_at` to estimate their clock offset, so the countdown ends together everywhere.
- **Load:** 300 SSE connections on one tokio process is trivial.
- **Proxy:** Caddy flushes SSE immediately, and tower-http compression skips `text/event-stream` by default.

### 8.6 Attendee page

- `dioxus::ssr::render_element(rsx!{…})`, using shared presentational components from `rafi-ui`.
- Inline CSS plus a ~3 KB inline script that handles:
  - the `EventSource` connection and view swaps;
  - **inside the Join tap handler**, before any `await` (Safari requires a user gesture): audio unlock and `navigator.wakeLock.request('screen')`;
  - the ALTCHA widget, which is lightweight and self-hosted.
- Target: under 30 KB on first load.

### 8.7 Hosting and operations

**Hetzner CX23**, Germany:
- 2 vCPU / 4 GB, €5.49/month, plus €0.50 for IPv4 and optional backups.
- If the shared plans are sold out, use CAX11 (ARM) at €5.99 and build an arm64 image.

**Runtime:**
- `docker compose`: a `caddy` container (automatic HTTPS, HTTP/2) next to `rafi`, which runs `ENTRYPOINT /usr/local/app/server`. A `litestream` sidecar replicates to EU S3 (Hetzner Object Storage or Scaleway).
- With the Restate tunnel there's no inbound HTTP/2 path to configure.

**Deploy:**
- Dagger builds the image with the official Dioxus Dockerfile pattern: cargo-chef, `dx` pinned to 0.7.10, `dx bundle --web --release`, and a slim runtime image.
- It pushes to GHCR, then runs `ssh … docker compose pull && docker compose up -d`.
- **Don't deploy during an event.** Add a "deploy freeze" check: refuse to deploy while any raffle is Open, Frozen or Drawing.

**Fly.io fallback** (`ams`, about $4/month):
- There's an unresolved report of SSE delays on iOS over Fly's HTTP/2. If you use Fly, set `alpn=["http/1.1"]` and turn auto-stop off.

**Secrets:** `CLERK_SECRET_KEY`, the publishable key (baked into both builds), `RESTATE_ENVIRONMENT_ID`, `RESTATE_AUTH_TOKEN`, `RESTATE_SIGNING_PUBLIC_KEY`, the Restate ingress API key, the ALTCHA HMAC key, and the S3 credentials.

**Cost:**
- About €6–7/month for Hetzner, plus a few euros for object storage.
- Restate Cloud Free, Clerk Hobby and ALTCHA cost nothing.

### 8.8 Known quirks

- **Dioxus:**
  - `ClerkAuthLayer` doesn't reject requests on its own; enforce auth in every admin server function.
  - The Clerk publishable key uses `env!` and must be identical in the wasm and server builds.
- **`dioxus::serve`** uses `into_make_service()` without `ConnectInfo`, so `tower_governor`'s default IP key extractor fails.
  - Use a custom `KeyExtractor`: raffle ID plus cookie, or the email hash.
  - Never key on IP, because the whole venue shares one NAT address.
- **restate-sdk 0.12:**
  - Needs Rust ≥ 1.90 (edition 2024).
  - Pins `restate-sdk-shared-core =7.0.3` and uses rand 0.10.
  - `reqwest-client` pulls in reqwest 0.13 next to Dioxus's 0.12. That's duplication, not a conflict.
  - With `tunnel`, rustls uses ring. If something else enables aws-lc-rs, call `CryptoProvider::install_default()` or use the SDK's `aws_lc_rs` feature.
- **sqlx:** commit the `.sqlx/` query cache, build with `SQLX_OFFLINE=true`, and run `cargo sqlx prepare --check` in CI.

## 9. Abuse and remote entries

- **One entry per normalised email per raffle**, enforced by `UNIQUE`. The cookie is a soft signal. No fingerprinting, which would need consent.
- **ALTCHA** proof of work on the join form, verified server-side with HMAC. Keep the difficulty low enough for old phones (under 1 s).
- **Rate limits** with `tower_governor` and a custom key (raffle + cookie or email hash). Use only a generous global ceiling per raffle.
- **Remote entries:**
  - The join code only works while registration is open.
  - The rules say "must be present to claim".
  - The organizer can remove entrants before the freeze.
  - The no-show rule handles the rest.
  - Rotating QR codes were rejected: they cause scan failures. Kahoot, Slido and Mentimeter all use static codes.
- **Display-name moderation:** an HU/EN blocklist, one tap to hide or rename, and a 5 s delay before a new name appears in `recent_names`.

## 10. Privacy and legal (Hungary / EU)

> Not legal advice. Items marked ⚖ need a one-time check with a Hungarian lawyer or tax adviser.

- **Gambling law (Szjtv.):**
  - Free entry means no stake, so this is **not gambling** and needs no notification to the SZTFH.
  - The old "ajándéksorsolás" (gift draw) rules were repealed on 2023-01-01.
  - **Never sell entries or tie them to donations.** That turns it into a "tombola", which must be notified.
  - Publish short terms: organizer, eligibility, prizes, draw method, claim rule.
- ⚖ **Personal income tax (SZJA):**
  - When a company gives the prize, it's usually an "egyes meghatározott juttatás" (a specific taxable benefit category). The giver pays 15% SZJA plus 13% social contribution tax on 1.18 × the value.
  - When a private person gives it, it may count as a gift, which is tax-free up to HUF 150k.
  - Confirm who legally gives each prize: you, the sponsor, CNCF/LF or KCD.
- **GDPR lawful basis:**
  - Art. 6(1)(b), taking part under the rules: running the draw and delivering prizes.
  - Art. 6(1)(f), legitimate interest backed by a short balancing test: duplicate and abuse checks.
  - **Marketing and sponsor sharing need separate, optional, unticked consent.** This follows from Planet49, Grt. 6. § and NAIH-6372/2012.
- **Checkboxes:**
  - (a) required: "I accept the rules and privacy notice";
  - (b) optional, unticked: "Email me about future meetups";
  - phase 2, per item: "Share my contact with {sponsor} if I win their prize".
  - Store the consent version and timestamp with each entry.
- **Privacy notice (Art. 13), in HU and EN.** It must cover:
  - the controller and contact details;
  - the purposes and lawful bases;
  - the legitimate interest;
  - the processors:
    - **Hetzner** (DE): hosting;
    - the **object storage provider** (EU): backups;
    - **Clerk** (US, under the EU–US Data Privacy Framework): organizers only;
    - **Restate** (DE): opaque IDs only, listed to be safe;
    - later, Scaleway TEM;
  - transfers outside the EU (Clerk only);
  - retention;
  - data subject rights;
  - the right to complain to the NAIH;
  - that the data is required to enter.
- **No Cloudflare and no Turnstile** means one less US processor. ALTCHA is self-hosted.
- **Cookie:** the entrant cookie is strictly necessary for the service the user asked for, so **no consent banner** is needed. Mention it in the notice. No analytics.
- **Data minimisation:** only a display name and email. Email is optional for entrants added by the organizer. Emails never appear on the projector, in SSE, or in Restate.
- **Retention:**
  - A daily job sets name, email, token hash and claim code to NULL once a raffle's `purge_at` passes. Entrant IDs and draws are kept.
  - Litestream backups follow the same window: set a short snapshot retention, e.g. 7 days.
  - The organizer can delete a single entrant on request, or purge a whole raffle.
- **Age:** 16+, stated in the rules.

## 11. Repository layout and tooling

```
Cargo.toml                 # workspace; release profile opt-level="z" + lto for the wasm client
crates/
  core/  rafi-core         # domain types, raffle/unit state rules, draw algorithm (sha2 only), drand round math
                           # no IO; native cargo test + proptest; test vectors
  ui/    rafi-ui           # shared presentational rsx components (used by fullstack app and SSR attendee page)
  app/   rafi              # Dioxus 0.7.10 fullstack app
         src/client/…      #   admin, projector, verify views (dioxus-clerk components, qrcode svg)
         src/server/…      #   #[cfg(feature="server")]: db (sqlx), hub (watch+SSE), attendee SSR routes,
                           #   restate/ (VO Raffle, ingress client, tunnel), drand client, retention job
migrations/                # sqlx migrations (embedded via sqlx::migrate!)
.sqlx/                     # offline query cache
tools/verify.py            # independent draw verifier
testdata/draw-vectors.json
deploy/                    # docker-compose.yml, Caddyfile, litestream.yml
Dioxus.toml
docs/PLAN.md
```

**Crates:**
- `dioxus` 0.7.10 (`fullstack`, `router`, `ssr`)
- `dioxus-clerk` 0.6 (`server`)
- `restate-sdk` 0.12 (`tunnel`, `reqwest-client`)
- `sqlx` 0.9 (sqlite, migrate)
- `tokio`, `tokio-stream`
- `tower_governor` 0.8
- `sha2`, `serde_json`
- `qrcode` 0.14 (svg)
- `reqwest` (drand)
- an ALTCHA server crate, or about 50 lines of HMAC verification

**devenv:** add `dioxus-cli` 0.7.10, `sqlx-cli`, `restate-server` + `restate` CLI (for local development), and `litestream`.

**Dev loop:**
- Run `restate-server` locally.
- Run `dx serve`, which starts the fullstack server, then register the endpoint once:
  - with the tunnel disabled in dev: `restate deployments register http://localhost:9080` (`HttpServer` on its own port in dev), or
  - via `nest_service`.
- Use a fake drand relay: a local mock serving `/public/{round}` with deterministic signatures.

**Testing:**
1. **The bulk is in `rafi-core`:** unit and property tests for the draw selection, pool rules (one win, no-show out, pass 2), round math, and the test vectors.
2. **`tools/verify.py`** runs in CI against the same vectors.
3. **The virtual object** is tested with `restate-sdk-testcontainers` (pinned `restate` image) and the mock drand relay. Cover:
   - freeze idempotency;
   - draw → replay after a forced crash gives the same winner;
   - beacon failure → Pending;
   - not_here → redraw;
   - shared `get` visibility;
   - finish → clear_all.
4. **App integration tests** use axum + in-memory SQLite. Cover:
   - join, dedupe, and a join racing the freeze;
   - auth rejection;
   - the SSE snapshot contains no personal data;
   - retention.
5. **Manual rehearsal:** a test-mode raffle, a "spawn 50 fake entrants" button, the projector and three real phones.

**CI (dagger):**
- fmt, clippy, cargo-deny;
- `cargo test`, the testcontainers suite and `verify.py`;
- `cargo sqlx prepare --check`;
- `dx bundle`;
- build the image and push it to GHCR;
- deploy from `main` over SSH, with the deploy-freeze check.

## 12. Milestones

| # | Milestone | Done when |
|---|---|---|
| M0 | **Skeleton** | The fullstack app builds and serves `/admin` behind Clerk sign-in (`require_owner`). SQLite migrations run. An `/j/test` SSR route and an SSE echo endpoint work. The Docker image deploys to Hetzner via Caddy and Litestream works. CI is green. |
| M1 | **Core** | `rafi-core` has the draw algorithm, round math, pool rules and test vectors, and `verify.py` agrees with them. |
| M2 | **Restate spike → VO** | The tunnel connects to Restate Cloud EU, with a fallback endpoint if not. The VO `Raffle` has freeze, draw, resolve, void, finish and get, plus the drand client with two relays, projection into SQLite and the hub, and the testcontainers suite. **Answer the open questions from §8.3.** |
| M3 | **Raffle admin** | CRUD for raffles and items (quantity, sponsor, duplicate), publish/open/close, entrants without a phone, hide/rename/remove, a test-mode flag and fake entrants. |
| M4 | **Attendee join** | `/j/{CODE}` SSR page, ALTCHA, email normalisation and dedupe, the guarded insert, cookie identity, consents, `/me`, HU/EN, and the rules and privacy pages. |
| M5 | **Live screens + draw** | The hub and SSE, the projector's intro/QR/count/names, the phone states with reconnects, start_draw (freeze and commit), the per-gift draw flow with countdown, Not here / Declined → redraw, void, finish, `/v/{slug}` with the in-browser verifier, and the "offline draw" fallback (dropped in revision 6; the "local draw" one in revision 5). |
| M6 | **After the event + privacy** | Winners CSV, marking prizes delivered, the retention job, per-entrant deletion, and the Litestream retention window. |
| M7 | **Polish** | Animations with reduced motion, `aria-live`, sound, Android vibration, the organizer phone layout, the deploy-freeze check, and a full rehearsal at a real meetup. |

**Phase 2:**
- Winner email via Scaleway TEM, sent from a Restate handler so retries come for free.
- A magic link to move an entry to another device.
- Voucher codes per unit, shown only to the winner, then purged.
- Sponsor-sharing consent and exports.
- BLS verification of the drand beacon (`drand_core`).
- Meetup.com RSVP cross-check.
- Templates.

## 13. Open risks

- **Restate Cloud is on the critical path during the draw.** Mitigation: a check that Restate is reachable (and on the app's drand chain) before "Start the draw", and, for a stalled draw, a retry on the same round or an abandon (§6.4; the "local draw" and "offline draw" fallbacks were dropped in revisions 5 and 6).
- **Tunnel maturity:** new in SDK 0.12, and the docs lag behind. Two fallbacks are described in §8.3. Verify in M2.
- **Shared-handler visibility of in-flight state** has mixed sources. It's only used for recovery or debugging, because the projection and SSE are the primary path. Verify in M2.
- **Restate Free tier:** the sources say 50k or 100k actions per month. About 300 per event fits either.
- **Single instance:** a deploy or crash during an event drops SSE connections. Clients reconnect, the hub is rebuilt from SQLite, Restate replays any draw in flight with the same R, and the deploy freeze prevents self-inflicted restarts.
- **Hetzner shared-plan availability.** The fallback is CAX11 ARM, or Fly `ams` with HTTP/1.1.
- **`dioxus-clerk` adoption is near zero.** You own it, so you can fix it yourself; it isn't a blocker.
- ⚖ **Prize tax treatment:** ask once, then put the answer into the rules template.
- **iOS** can't re-acquire the wake lock after the page goes to the background, and can't vibrate. This is cosmetic, since the projector stays the source of truth.

## Appendix: key sources

- Dioxus 0.7 fullstack: https://dioxuslabs.com/learn/0.7/essentials/fullstack/axum · streams/SSE: https://dioxuslabs.com/learn/0.7/essentials/fullstack/streams · deploy: https://dioxuslabs.com/learn/0.7/tutorial/deploy · wasm-split hydration issue: https://github.com/DioxusLabs/dioxus/issues/4631
- dioxus-clerk: https://github.com/sagikazarmark/dioxus-clerk
- Restate Rust SDK: https://docs.rs/restate-sdk/latest/restate_sdk/ · https://github.com/restatedev/sdk-rust (release notes v0.12.0/v0.12.1, `src/tunnel`, `src/endpoint`) · HTTP invocation: https://docs.restate.dev/services/invocation/http · retention: https://docs.restate.dev/services/configuration · managing invocations: https://docs.restate.dev/services/invocation/managing-invocations · Cloud: https://restate.dev/cloud · pricing: https://restate.dev/pricing
- drand: https://docs.drand.love/developer/API-v2/drand-http-api/ · rand reproducibility: https://rust-random.github.io/book/crate-reprod.html
- Hetzner pricing: https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/ · Fly iOS SSE/h2 report: https://community.fly.io/t/sse-events-delayed-several-seconds-for-ios-webkit-clients-over-http-2/28307
- tower_governor: https://docs.rs/tower_governor · Resend regions: https://resend.com/docs/dashboard/domains/regions · Scaleway TEM: https://www.scaleway.com/en/pricing/managed-services/
- SZTFH (gift draws, tombola): https://sztfh.hu/tevekenysegek/szerencsejatek-felugyelet/ajandeksorsolas/ · https://sztfh.hu/tevekenysegek/szerencsejatek-felugyelet/tombola/
- EDPB on Art. 6(1)(b): https://www.edpb.europa.eu/sites/default/files/files/file1/edpb_guidelines-art_6-1-b-adopted_after_public_consultation_en.pdf · NAIH-6372/2012: https://www.naih.hu/files/6372_2012_hatarozat.pdf
- Wake Lock: https://developer.mozilla.org/en-US/docs/Web/API/Screen_Wake_Lock_API · WebKit bug 254545: https://bugs.webkit.org/show_bug.cgi?id=254545
