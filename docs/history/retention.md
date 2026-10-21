# Retention: how it works (snapshot for re-adding it later)

> **Historical document.** A working note from Rafi's development, kept for the reasoning behind its decisions. Names, paths and details are of their time and may not match the current code, which is the source of truth.

**Status: removed (2026-10-20), redesigned and re-added (2026-10-21).** §1–§9 describe the feature as it was.
§10 was the checklist for re-adding it; the redesign that replaced it is
[docs/plans/privacy-and-retention.md](../plans/privacy-and-retention.md): the clock anchors on a locked event date
instead of the finish, winners are held past the erasure date rather than erased later than non-winners, never-frozen
raffles expire, and erasure is always on. §11 lists everything removed and what each part did.

All of this describes commit **`9448be9`**, the last commit before the feature was removed.
To see any file as it was then: `git show 9448be9:<path>`. To restore one: `git checkout 9448be9 -- <path>`. Paths
are as they were at that commit.

## 1. What retention is

The **retention run** is one pass over aging raffles. The deployment schedules it every hour: on the native server
it is a tokio task, and on Workers it is the Cron Trigger. Each run has three duties:

1. **Erasure** (only when enabled): personal data of finished raffles is erased on a schedule.
   - **Non-winners** are erased `RAFI_RETENTION_DAYS` (default 30) after the raffle finished.
   - **Winners** are erased `RAFI_RETENTION_WINNERS_DAYS` (default 90) after the finish. A winner whose prize was
     marked delivered is erased on the non-winner date instead.
   - The raffle becomes `Purged` once the winners' date passes.
2. **Proof-of-work cleanup** (always): expired ALTCHA challenges are deleted from `altcha_used`.
3. **Ending live channels** (always): a raffle that finished more than 24 h ago and still has live updates gets
   them ended. On Workers, this destroys its `RaffleRoom` Durable Object.

Each successful run records when it happened (`meta.last_retention_ok_at`). The admin shows a warning when erasure
is on and no run has succeeded for 2 hours.

Erasure is **not** the same as deleting. The entrant row stays and so does its ID; only personal fields are
cleared. The manifest, the event log, `draws` and `audit` are kept, so a published draw stays verifiable.

### Related features that are not retention

These overlap with retention but are separate, and they stay when retention is removed:

- **Manual purge:** the organizer's "Erase personal data now" (`rafi_app::purge`, `purge_raffle` API). It erases
  everyone at once, marks the raffle `Purged`, sets `live_ended_at`, writes an audit row and ends live updates.
- **Manual erasure of one entrant:** `remove_entrant` after the freeze (the entrant can't be deleted any more).
- **End live updates** (`end_live`): the manual version of duty 3.
- **Mark delivered** (`entrants.delivered_at`): this mostly existed to schedule a winner's erasure early. It is
  also exported in the CSV and shown in the UI.

## 2. Configuration

| Env var | `Config` field | Native default | Workers default | Meaning |
|---|---|---|---|---|
| `RAFI_RETENTION` | `retention_enabled` | **off** | **on** | Erase automatically. Truthy values are `1`/`true`/`yes`/`on`. |
| `RAFI_RETENTION_DAYS` | `retention_nonwinners_days` | 30 | 30 | Days after the finish before non-winners (and delivered winners) are erased. |
| `RAFI_RETENTION_WINNERS_DAYS` | `retention_winners_days` | 90 | 90 | Days after the finish before winners are erased, at the latest. |

- **Native:** parsed in `crates/server/src/config.rs` (`Config::from_lookup`). `for_tests()` uses false / 30 / 90.
- **Workers:** `apps/worker/src/lib.rs` (`app()`) turns `RAFI_RETENTION` on when it is unset or empty, on purpose
  (see `docs/workers-plan.md` §2.10):

  ```rust
  let cfg = Config::from_lookup(|name| match name {
      // Erasure is on unless switched off explicitly (docs/workers-plan.md §2.10).
      "RAFI_RETENTION" => lookup(env, name).or_else(|| Some("1".into())),
      _ => lookup(env, name),
  });
  ```

- `deploy/.env.example` documented all three variables: `# RAFI_RETENTION=1` (commented out), `RAFI_RETENTION_DAYS=30`,
  `RAFI_RETENTION_WINNERS_DAYS=90`.
- `README.md` had a row for each in its configuration table.
- **Hard-coded constants** (`crates/app/src/retention.rs`):
  - `CHANNEL_GRACE_SECS = 86_400`: a live channel survives 1 day after the finish.
  - `STALE_AFTER_SECS = 7_200`: two missed hourly runs count as stale.
- **Schedule:** the native interval is 3600 s; the Worker Cron is `"0 * * * *"` (`apps/worker/wrangler.toml`,
  `[triggers]`).

The settings reach the use cases as `Policy.retention: Retention { enabled, nonwinners_days, winners_days }`, built in
`AppState::from_parts` (`crates/server/src/lib.rs`).

## 3. Data model

These are the same in both dialects: `migrations/sqlite/0001_init.sql` and `migrations/postgres/0001_init.sql`.

| Object | Used for |
|---|---|
| `raffles.finished_at` | Set by the engine's write (`write_raffle`) when the raffle finishes. All retention dates count from it. **Not** retention-only. |
| `raffles.purge_nonwinners_at` | The date non-winners (and delivered winners) become due. Set once by the run. **Retention-only.** |
| `raffles.purge_winners_at` | The date winners become due and the raffle becomes `Purged`. Set once by the run. **Retention-only.** |
| `raffles.purged_at` | When the raffle became `Purged`. Shared with manual purge. |
| `raffles.live_ended_at` | When live updates ended. Shared with manual "End live updates". |
| `entrants.delivered_at` | A delivered winner goes on the non-winner date. Shared with the UI and CSV. |
| `entrants.erased` | 0/1 flag. Shared with manual erasure. |
| `erasures(id, raffle_id, entrant_id, at)` | The erasure log, with no personal data, so a restore (D1 Time Travel, backups) can re-apply erasures. Shared with manual erasure. |
| `altcha_used(challenge, expires_at)` and index `altcha_used_expiry` | Proof-of-work replay protection; the run deleted expired rows. |
| `meta(key PRIMARY KEY, value)` | Small operational facts. The only key ever used is `last_retention_ok_at` (unix seconds, as text). **Retention-only.** |

What `entrants::ERASED` clears (`crates/store/src/entrants.rs`, shared with manual erasure):

```rust
pub(crate) const ERASED: &str = "display_name = '#' || substr(id, 1, 6), email_raw = NULL, \
     email_norm = NULL, token_hash = NULL, consent_marketing = 0, erased = 1";
```

The claim code is **not** cleared: `claim_code` is `NOT NULL`. This contradicts PLAN.md §10 and some UI text, which
say claim codes are erased.

The `version` triggers on `raffles` and `entrants` fire on retention's writes too, which bumps the views' `v`.

## 4. How a run works

The run lives in `crates/app/src/retention.rs` (`run`). The SQL is in `crates/store/src/retention.rs`.

```text
now = ids::now()
if policy.retention.enabled:
    store::date_purges(db, PurgeAfter::days(nonwinners_days, winners_days))
    erased = store::erase_due(db, now)
altcha::purge_expired(app)          # always
channels_ended = end_channels(now)  # always; errors are logged, not returned
store::record_ok(db, now)           # meta.last_retention_ok_at = now
return Report { erased, channels_ended }
```

If any `?` step fails, the run returns an error. Channels are then not ended, `record_ok` is not written, and the
run will look stale until a later run succeeds.

### 4.1 Dating: `date_purges`

One set-based UPDATE:

```sql
UPDATE raffles SET
  purge_nonwinners_at = COALESCE(purge_nonwinners_at, finished_at + ?1),
  purge_winners_at    = COALESCE(purge_winners_at,    finished_at + ?2)
WHERE state IN (<RaffleState::is_over states: finished, purged>)
  AND finished_at IS NOT NULL
  AND (purge_nonwinners_at IS NULL OR purge_winners_at IS NULL)
```

- `PurgeAfter::days(nw, w)` gives `{ nonwinners_secs: nw * 86_400, winners_secs: w * 86_400 }`. It is defined in
  `crates/store/src/lib.rs`.
- **Once set, a date never moves.** Changing the settings later only affects raffles that haven't been dated yet.
- **Late enabling:** nothing is dated while erasure is off. When it is turned on, dates count from the original
  `finished_at`, so old raffles may be erased on that very run.
- The engine does **not** date erasure when it saves a raffle (`save_raffle`, "Nothing here is policy"). The engine
  knows no app settings. This was deliberate; see `docs/engine-plan.md` revision 8.

### 4.2 Who is due: the `due()` predicate

The predicate is built once (`LazyLock`):

```sql
entrants.erased = 0 AND EXISTS (
  SELECT 1 FROM raffles r
  WHERE r.id = entrants.raffle_id
    AND r.state IN (<states where Command::Purge applies: finished, purged>)
    AND r.finished_at IS NOT NULL AND r.purged_at IS NULL
    AND CASE WHEN entrants.id IN (
            SELECT entrant_id FROM draws WHERE raffle_id = r.id AND outcome = 'claimed')
        THEN r.purge_winners_at <= ?1
             OR (entrants.delivered_at IS NOT NULL AND r.purge_nonwinners_at <= ?1)
        ELSE r.purge_nonwinners_at <= ?1
    END)
```

- A **winner** is the entrant of a draw whose outcome is `claimed`.
- Entrants who were drawn but were "not here" or declined count as non-winners.
- An entrant who won several prizes stays on the winner schedule unless they were delivered.
- Unfinished raffles are never due.
- Once `purged_at` is set, the raffle's remaining entrants no longer match. A manual purge erases everyone anyway.

### 4.3 Erasing: `erase_due(db, now)`

1. `SELECT COUNT(*) FROM entrants WHERE <due>` gives the reported count. It runs outside the batch, so it can drift
   under concurrent writes.
2. One `db.batch` (all or nothing; this is what replaces transactions on D1):
   1. `INSERT INTO erasures (raffle_id, entrant_id, at) SELECT raffle_id, id, ?1 FROM entrants WHERE <due>`
   2. `UPDATE entrants SET <ERASED> WHERE <due>`
   3. ```sql
      UPDATE raffles SET state = 'purged', purged_at = ?1
      WHERE state IN (<purgeable>) AND finished_at IS NOT NULL
        AND purged_at IS NULL AND purge_winners_at <= ?1
      ```

Notes:

- An automatic purge does **not** set `live_ended_at`; a manual `mark_purged` does. The channel is ended separately
  by §4.5.
- An automatic purge writes **no** `audit` row; only `erasures` gets one per entrant.
- Every write is set-based, a few statements per run whatever the number of entrants. This keeps the run under D1's
  per-invocation query limit.

### 4.4 Proof-of-work cleanup

`rafi_app::altcha::purge_expired(app)` calls `rafi_store::altcha::purge_expired(db, now)`, which runs
`DELETE FROM altcha_used WHERE expires_at < ?`. The retention run was its **only** caller. Without the run, the
table grows without bound.

`ReplayStore::forget_expired` (the `altcha_server` trait impl in `crates/app/src/altcha.rs`) calls the same store
function, but the library may call it on its own schedule. That is outside rafi.

### 4.5 Ending live channels: `end_channels(app, now)`

1. `store::retention::live_channels_due(db, now - CHANNEL_GRACE_SECS)` returns the IDs of raffles with
   `finished_at <= ?` and `live_ended_at IS NULL`.
2. For each ID:
   1. `app.live_updates.end(id)` runs `Notifier::end`.
      - Natively, `Polling::end` is a no-op.
      - On Workers, `DurableObjects::end` calls `gate::destroy`, which POSTs `/destroy` to the `RaffleRoom`. The
        room publishes a final view, closes sockets with 4403, deletes its alarm and deletes its storage.
   2. `store::retention::live_ended(db, id, now)` sets `live_ended_at` only if it is still NULL.
3. Errors are `tracing::warn!`ed and the raffle is skipped, so the next run retries it.

The order is the reverse of what the `Notifier::end` docs ask: it destroys first, then records. The code's comment
explains why that is safe: over raffles refuse sockets anyway.

### 4.6 Staleness

```rust
pub async fn stale(app: &App) -> anyhow::Result<bool> {
    if !app.policy.retention.enabled { return Ok(false); }
    let last_ok = store::last_ok(&app.db).await?;      // meta.last_retention_ok_at
    Ok(last_ok.is_none_or(|at| now - at > STALE_AFTER_SECS))
}
```

It is surfaced as `Me.retention_stale` (`crates/api/src/lib.rs`), returned by the `whoami` admin endpoint
(`crates/server/src/admin.rs`). The admin's raffle list (`AdminHome`, `apps/web/src/ui/admin/mod.rs`) then shows:

> Automatic erasure of personal data hasn't run in the last two hours. Check the scheduled job (the Cron Trigger on Workers).

## 5. Scheduling

### Native

`apps/rafi/src/main.rs` calls `server::spawn_background(&app)`, which is in `crates/server/src/lib.rs`:

```rust
pub fn spawn_background(app: &AppState) {
    if !app.cfg.retention_enabled {
        tracing::info!("automatic erasure of personal data is off (RAFI_RETENTION)");
    }
    retention::spawn(app.clone());
}
```

`crates/server/src/retention.rs` defines `spawn`:

- It starts `tokio::spawn` with `tokio::time::interval(3600 s)`.
- The first tick fires immediately, so the run happens **at startup and then hourly**.
- It logs `info` when the `Report` isn't empty and `error` on failure.
- It is compiled only for non-wasm targets.

### Workers

`apps/worker/wrangler.toml`:

```toml
# Retention, expired proof-of-work challenges, ending stale live channels.
[triggers]
crons = ["0 * * * *"]
```

`apps/worker/src/lib.rs`:

```rust
#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let app = match cached(&env) {
        Ok((app, _)) => app,
        Err(e) => { worker::console_error!("rafi: retention not run: {e:#}"); return; }
    };
    match server::service::retention::run(&app).await {
        Ok(report) => worker::console_log!(
            "rafi: retention: {} erased, {} live channels ended", report.erased, report.channels_ended),
        Err(e) => worker::console_error!("rafi: retention failed: {e:#}"),
    }
}
```

## 6. User-facing surfaces

### Privacy notice

`GET /privacy/{slug}` is handled by `crates/server/src/participant.rs` (`privacy`). It passes
`retention.nonwinners_days` and `retention.winners_days` to `pages::text::privacy`, whose "How long we keep it"
section said:

> Personal data of entrants who did not win is erased {days_nw} days after the event. Winners' data is erased once
> the prize is delivered, at the latest {days_w} days after the event. Anonymous IDs and the draw log are kept so the
> draw stays verifiable.

The page stated this even when native erasure was off.

### Admin UI

- The stale banner on the raffle list (§4.6).
- The finish dialog (`apps/web/src/ui/admin/draw.rs`): "Personal data is erased automatically after the retention
  period."
- The landing page (`apps/web/src/ui/mod.rs`): "Personal data is erased automatically."

### Other surfaces

There is no CLI command and no dedicated admin endpoint besides `whoami.retention_stale`.

## 7. Tests at the time

**`crates/store/src/conformance/domain.rs`** runs on SQLite, Postgres and D1:

- `keeps_the_newest_state` (part of it):
  - the engine write leaves the `purge_*_at` dates NULL;
  - `date_purges` sets `finished_at + 30` and `finished_at + 90` (the test's `PURGE` uses seconds);
  - a second call with other settings changes nothing.
- `retention_erases_what_is_due`, registered as `"domain.retention"` in `conformance/mod.rs`, walks through a
  raffle's erasure:
  - At 129 nothing is due.
  - At 130 the non-winner and the delivered winner are erased.
  - At 189 nothing more.
  - At 190 the winner is erased, and the raffle becomes `Purged` with `purged_at = 190`.
  - At 10 000 nothing more, and the unfinished raffle is untouched.
  - The `erasures` times are correct.
  - `live_channels_due` and `live_ended` are idempotent.
  - `last_ok` / `record_ok` work.

**`crates/server/src/tests.rs`:**

- `full_raffle_flow` (with `retention_enabled = true`):
  - finishing doesn't date erasure;
  - after the dates are forced to 0, `retention::run` erases 6 entrants, the raffle is `Purged`, and the
    `erasures` log matches;
  - a second run erases 0, and the draw still verifies.
- `proof_of_work_replays_are_refused`: the run deletes the expired `altcha_used` row.
- `retention_ends_channels_and_logs_erasures`:
  - With erasure off, only the channel older than the grace period is ended, `meta.last_retention_ok_at` is
    written, and a rerun ends 0.
  - With erasure on, 3 entrants are erased with `#` names, the raffle stays `Finished` before the winners' date,
    and the `erasures` columns are exactly `at, entrant_id, id, raffle_id`.

**`crates/app/src/retention.rs`:** `stale_after_two_hours_without_a_run`.

**`crates/app/tests/use_cases.rs`:** builds `Policy` with `Retention { enabled: true, 30, 90 }`.

**Not covered:**

- `whoami.retention_stale` end to end;
- the privacy page text;
- the Worker's `scheduled` handler.

## 8. Design decisions worth keeping

- **The app owns retention, not the engine.** The engine stores `finished_at`; the app's run turns settings into
  dates. Engine hosts have no retention config (`docs/engine-plan.md` revision 8, `docs/workers-plan.md`).
- **Dates are written into the row once.** Changing a policy never retroactively shortens or extends a raffle that
  was already dated.
- **Set-based SQL in one batch:** D1 has no interactive transactions and limits queries per invocation.
- **An erasure log without personal data,** so restores can re-apply erasures. PLAN.md §8.4 says "a restore must
  re-apply them".
- **On by default on Workers (production), opt-in natively.**
- **Staleness is surfaced in the admin,** because a silent scheduler failure means personal data is kept past what
  the privacy notice promises.

## 9. Known issues at the time

- Claim codes are not erased, although PLAN.md §10 and the UI text say they are.
- The privacy notice always promised 30/90-day erasure, even when native erasure was off.
- `end_channels` destroys the channel before recording `live_ended_at`, the reverse of the `Notifier::end` contract.
- `erase_due` takes its count outside the batch.
- An automatic purge writes no `audit` row; a manual one does.

## 10. Re-adding it: checklist

1. Restore the modules from `9448be9`:
   - `crates/store/src/retention.rs` and `PurgeAfter` (`crates/store/src/lib.rs`)
   - `crates/app/src/retention.rs`
   - `crates/server/src/retention.rs`
2. Re-add the config fields and env vars (§2), `Policy.retention`, and the Workers default for `RAFI_RETENTION`.
3. Re-add the schema bits (`purge_nonwinners_at`, `purge_winners_at`, `meta`). Edit `0001_init.sql` while nothing is
   deployed; add a new migration once databases exist.
4. The schedulers (§5) already exist as the housekeeping run. Re-add the erasure branch, `record_ok`, and
   `Me.retention_stale` with its banner (§4.6).
5. Restore the privacy notice's day counts and the UI copy (§6).
6. Restore the tests (§7) and the conformance registration.
7. Update README, PLAN.md §3/§4/§5/§8/§10, workers-plan.md and `deploy/.env.example`.
8. Consider fixing the known issues in §9 at the same time.

## 11. What was removed, and what each part did

Removed on 2026-10-20 (PLAN.md revision 20). Scope decisions:

- **Only automatic erasure is gone.** The hourly run stays, renamed **housekeeping** (`rafi_app::housekeeping`,
  `rafi_store::housekeeping`, `rafi_server::housekeeping`). It still runs duties 2 and 3 from §1: proof-of-work
  cleanup and ending live channels.
- **Manual erasure stays:** `purge`, `Purged`, `Command::Purge`, erasing an entrant after the freeze, `erasures`,
  `entrants.erased`, `ERASED`, mark delivered and `deliverable`.
- **The schema change edits the squashed `0001_init.sql`** (nothing is deployed; PLAN.md revision 17).

### Removed code

| What was removed | Where it was (at `9448be9`) | What it did |
|---|---|---|
| `date_purges` | `crates/store/src/retention.rs` | Wrote `purge_*_at = finished_at + days` once per finished raffle |
| `due()` predicate, `purgeable()` | `crates/store/src/retention.rs` | SQL selecting the entrants due for erasure (winner vs. non-winner, delivered) |
| `erase_due` | `crates/store/src/retention.rs` | One batch: log to `erasures`, apply `ERASED`, mark raffles `purged` once the winners' date passed |
| `last_ok`, `record_ok` | `crates/store/src/retention.rs` | Read and write `meta.last_retention_ok_at` |
| `PurgeAfter` (+ `days()`) | `crates/store/src/lib.rs` | Retention periods in seconds, passed to `date_purges` |
| `RaffleRow.purge_nonwinners_at`, `.purge_winners_at` | `crates/store/src/raffles.rs` | Decoded the retention dates |
| `Retention { enabled, nonwinners_days, winners_days }`, `purge_after()` | `crates/app/src/retention.rs` | The deployment's retention settings |
| `rafi_app::Retention` re-export, `Policy.retention` | `crates/app/src/lib.rs` | Passed the settings to the use cases |
| The erasure branch of `run`, `record_ok` call; `Report.erased` | `crates/app/src/retention.rs` | Dated and erased each run; reported the count; recorded success |
| `STALE_AFTER_SECS`, `stale()`, `is_stale()` + its unit test | `crates/app/src/retention.rs` | "Erasure on but no success in 2 h" |
| `Config.retention_enabled`, `retention_nonwinners_days`, `retention_winners_days` | `crates/server/src/config.rs` | Parsed `RAFI_RETENTION`, `RAFI_RETENTION_DAYS`, `RAFI_RETENTION_WINNERS_DAYS` |
| `Policy.retention` wiring | `crates/server/src/lib.rs` (`from_parts`) | Mapped `Config` into `Retention` |
| "automatic erasure … is off" log | `crates/server/src/lib.rs` (`spawn_background`) | Told self-hosters erasure was off |
| `Me.retention_stale` | `crates/api/src/lib.rs` | Told the admin UI whether erasure looked stopped |
| `retention_stale: service::retention::stale(..)` | `crates/server/src/admin.rs` (`whoami`) | Filled it |
| Stale warning banner (and the `whoami` resource) | `apps/web/src/ui/admin/mod.rs` (`AdminHome`) | "Automatic erasure … hasn't run in the last two hours" |
| `RAFI_RETENTION` default `"1"` | `apps/worker/src/lib.rs` (`app()`) | Erasure on by default on Workers |
| Days parameters of `pages::privacy` | `crates/server/src/pages/text.rs`, `participant.rs` | Put the 30/90 days into the privacy notice |

### Removed schema

| What was removed | Where it was (at `9448be9`) | What it did |
|---|---|---|
| `raffles.purge_nonwinners_at`, `raffles.purge_winners_at` | `migrations/{sqlite,postgres}/0001_init.sql` | The per-raffle erasure dates |
| `meta` table | `migrations/{sqlite,postgres}/0001_init.sql` | Only held `last_retention_ok_at` |

### Removed config and docs entries

| What was removed | Where it was (at `9448be9`) | What it did |
|---|---|---|
| `RAFI_RETENTION*` lines | `deploy/.env.example` | Self-hosting config |
| `RAFI_RETENTION*` rows | `README.md` (configuration table) | Documented the variables |

### Removed and changed tests

| What changed | Where it was (at `9448be9`) | What it covered |
|---|---|---|
| Conformance scenario `domain.retention` (`retention_erases_what_is_due`) | `crates/store/src/conformance/{domain,mod}.rs` | Dating, the due rules, purging, the log, `meta` |
| The dating part of `keeps_the_newest_state` | `crates/store/src/conformance/domain.rs` | The engine's write leaves the dates NULL; `date_purges` dates once |
| The erasure section of `full_raffle_flow` and `cfg.retention_enabled = true` | `crates/server/src/tests.rs` | Erasure through the run |
| `retention_ends_channels_and_logs_erasures` | `crates/server/src/tests.rs` | Channel ending, `meta`, and erasure through the run |
| `Retention { .. }` in `Policy` | `crates/app/tests/use_cases.rs` | Test setup only |

### Renamed or rewritten (kept, not retention)

- **Store module:** `crates/store/src/retention.rs` became `crates/store/src/housekeeping.rs`. It keeps
  `live_channels_due` and `live_ended`.
- **App run:** `crates/app/src/retention.rs` became `crates/app/src/housekeeping.rs`.
  - `run` now does only `altcha::purge_expired` and `end_channels`.
  - `Report` keeps `channels_ended`.
  - `CHANNEL_GRACE_SECS` is unchanged.
- **Native scheduler:** `crates/server/src/retention.rs` became `crates/server/src/housekeeping.rs`. It is the same
  hourly tokio task, with new log messages.
- **Worker Cron:** `apps/worker/src/lib.rs` (`scheduled`) calls `housekeeping::run`. `wrangler.toml` keeps the
  `0 * * * *` Cron, with its comment updated, and the `Cargo.toml` description was updated too.
- **Conformance:** a new scenario, `domain.live_channels` (`live_channels_end_once`), keeps the channel-ending
  checks from the old retention scenario.
- **Server tests:**
  - `full_raffle_flow` now erases through the manual `purge` and still checks that the erased draw verifies and that
    each erasure is logged once.
  - `housekeeping_ends_stale_live_channels` covers the channel part, and that housekeeping erases nobody.
  - `erasures_are_logged_without_personal_data` checks the `erasures` columns, through a manual purge.
  - `proof_of_work_replays_are_refused` calls `housekeeping::run`.
- **Copy:**
  - The privacy notice's "How long we keep it" now says: "Personal data is kept until the organizer erases it after
    the event, or on request. Anonymous IDs and the draw log are kept so the draw stays verifiable."
  - The finish dialog says to erase personal data from the raffle's menu.
  - The landing page says "Personal data is erased in one click."
- **`whoami`** stays in the API (owner and dev-auth). The web client has no caller now, so the generated client
  allows dead code.
- **Comments:** comments that mentioned the retention run were updated: `crates/store/src/lib.rs`,
  `crates/app/src/altcha.rs`, `crates/lifecycle/src/raffle.rs` (`deliverable`), and `crates/api/src/admin.rs`
  (`whoami`).
- **PLAN.md:** §3, §4, §5, §8.1, §8.2 and §10 were updated, and revision 20 was added.
- **Historical docs:** workers-plan.md, engine-plan.md, events-plan.md, drawing-review.md and
  dioxus-altcha-plan.md were left as they are, as history.

### Re-adding on top of the current code

- Put the erasure branch back into `housekeeping::run`. Calling it "retention" again is optional.
- Restore the removed rows above, and the checklist in §10.
- The Cron and native task already exist, so no scheduling work is needed.
