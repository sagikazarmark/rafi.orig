# Privacy and retention

> **Historical document.** The spec the privacy and retention features were built from, kept for the reasoning behind its decisions. What the software does is in [docs/explanation/privacy-and-abuse.md](../explanation/privacy-and-abuse.md), which is the source of truth; its "Follow-ups" are tracked there and in the pre-release review.

**Status: implemented (2026-10-21), amended (2026-10-26), moved to history (2026-10-27).** The privacy review's main spec: automatic erasure comes
back, redesigned; the legal surfaces the hosted instance needs are added; the marketing opt-in is removed and gets
its own plan ([docs/plans/marketing-opt-in.md](../plans/marketing-opt-in.md)). What the software does is in
[docs/explanation/privacy-and-abuse.md](../explanation/privacy-and-abuse.md); this document is the reasoning.

> **Amended before the first release** ([docs/plans/pre-release-review.md](../plans/pre-release-review.md), SCH-3): the recorded
> acceptance of the terms (`legal_acceptances`, the gate on creating a raffle, the admin's "I accept") was removed.
> `/terms` and `/privacy` stay, with their versions; acceptance is by use. The reasoning and the consequences are
> in that document's §Accepted risks. §Terms and the processing agreement below describes the removed design.

[retention.md](retention.md) describes the automatic erasure Rafi had until 2026-10-20.
Where this spec differs from it, this spec wins; where it is silent, that design and its §8 "decisions worth
keeping" still hold.

> Not legal advice. The plan is engineering and product design; §Further notes lists what a lawyer must check before
> the hosted instance launches.

## Problem statement

An organizer runs a raffle at a meetup. Everyone in the room types their name and email into their phone, the draw
happens, prizes are handed out, and the evening ends. From then on, the personal data of a hundred people sits in
the database until the organizer remembers to press "Erase personal data now". Most never will, and nothing happens
when they don't. The privacy notice entrants saw promises erasure "after the event" without saying when. A raffle
the organizer never finishes keeps its entrants forever, and so does one the organizer created, opened, and walked
away from.

Nobody tells the entrant who is responsible for their data in a way that holds up: the organizer's name may be
blank, there is no privacy contact, and the only notice mixes the organizer's role with the operator's. The hosted
operator, in turn, processes personal data on organizers' behalf with no processing agreement, and organizers
accept nothing when they start using the service. Backups and D1 Time Travel can bring erased data back and there
is no tool to erase it again. An organizer who stops using Rafi has no way to take their own name and contact off
the raffles that still show them.

## Solution

Every raffle has an **event date** that is fixed once anyone has joined, and a **retention period** that is fixed
with it. From the two, the raffle has one **erasure date** that the entrant sees on the join form and in the
privacy notice as a calendar date, and that no later action of the organizer can move. On that date, housekeeping
erases the personal data of everyone who did not win, whatever state the raffle is in. Winners' data stays a while
longer, so prizes can still be delivered, until the organizer marks them delivered or a hard ceiling passes. The
organizer can erase earlier at any time, as now. A raffle that was never frozen loses its entrants outright on the
erasure date: there is no draw to keep verifiable.

The installation sets the periods and their ceilings; an organizer may shorten the non-winner period for their
raffles; a raffle inherits the organizer's choice when it is created and locks it with the rules. The organizer
names themselves and a privacy contact before registration can open, and may add their own privacy text or a link
to it; the generated notice stays, with the operator named as processor and a link to the operator's own notice.
Organizers accept the operator's terms, with the processing agreement as part of them, before they create their
first raffle; the acceptance and its version are recorded. Automatic erasure writes the audit log, a tool re-applies
recorded erasures after a restore, and an organizer can remove their own data from Rafi.

The marketing opt-in goes. It needs an export that doesn't exist and a retention clock of its own; it comes back as
one piece in its own plan.

## User stories

### Entrants

1. As an entrant, I want the join form to tell me the exact date my name and email are erased, so that I know what
   I'm agreeing to before I enter.
2. As an entrant, I want that date to hold whatever the organizer does afterwards, so that the promise means
   something.
3. As an entrant, I want to know who the controller is and how to reach them about my data, so that I can exercise
   my rights.
4. As an entrant, I want to know who operates the service and what they do with my data, so that I understand the
   whole chain.
5. As an entrant, I want to accept the raffle rules and to be *shown* the privacy notice, not asked to "accept" it,
   so that the form doesn't pretend a notice is consent.
6. As an entrant, I want to see that there is a minimum age and what it is, so that I know whether I may enter.
7. As an entrant, I want my display name off the public verification page and the projector once the erasure date
   passes, so that my presence at the event stops being public.
8. As an entrant who won, I want my contact details kept long enough for the prize to reach me, so that winning
   isn't undone by the erasure.
9. As an entrant, I want to be able to ask the organizer to erase me earlier, and to have that be possible, so that
   erasure on request works.
10. As an entrant, I want no marketing checkbox on the form until the service can actually honour one properly, so
    that I'm not asked for a consent that can't be managed.

### Organizers

11. As an organizer, I want a sensible erasure period by default, so that I'm compliant without configuring anything.
12. As an organizer, I want to shorten the non-winner period for all my future raffles, so that my meetup's policy
    is applied without repeating it.
13. As an organizer, I want to shorten it for one raffle, so that a special event can have its own rule.
14. As an organizer, I want the event date and the period to lock once someone has joined, so that I can't break
    the promise by accident.
15. As an organizer, I want to be told why I can't change them, so that the lock isn't a mystery.
16. As an organizer, I want to erase everyone now, as today, so that I can be done before the deadline.
17. As an organizer, I want to erase one entrant on request even after the automatic deadline passed for others,
    so that requests are always possible.
18. As an organizer, I want winners' data held until I mark their prizes delivered, so that a coupon I send by
    email next week still has an address to go to.
19. As an organizer, I want to mark all prizes delivered at once, so that a raffle where everything was handed over
    on the night is closed in one tap.
20. As an organizer, I want to see, on the raffle, how many winners' data is still held and until when, so that I
    know what's outstanding without being emailed.
21. As an organizer, I want to be warned on a raffle I never finished that its entrants are erased on the date
    regardless, so that I'm not surprised.
22. As an organizer, I want to be warned if automatic erasure hasn't run, so that a broken scheduler doesn't make
    me break my promise silently.
23. As an organizer, I want to give my privacy contact once in my defaults, so that every raffle names it.
24. As an organizer, I want to add my own privacy text or link to my organisation's notice, so that entrants get my
    full policy without me rewriting Rafi's.
25. As an organizer, I want to be told what I must fill in before registration can open, so that a notice is never
    published with a blank controller.
26. As an organizer, I want to read and accept the operator's terms and processing agreement once, so that the
    legal basis for the operator holding my entrants' data is in place.
27. As an organizer, I want to be asked again when the terms change materially, so that I know what I'm agreeing to.
28. As an organizer, I want to export the winners of a finished raffle as before, and to know the export is logged,
    so that I can deliver prizes and account for the data I took.
29. As an organizer, I want to remove my name, contact and defaults from Rafi when I stop using it, so that
    finished raffles don't show my details indefinitely.
30. As an organizer, I want my finished raffles to stay verifiable after that, so that past winners can still check
    the draw.

### Operators

31. As the operator of the hosted instance, I want the periods, ceilings, minimum age, operator identity and
    document versions to come from the deployment's environment, so that no settings page is needed.
32. As the operator, I want the server to refuse to start without its legal documents, so that a misconfigured
    production instance can't go live with placeholders.
33. As the operator, I want automatic erasure to write the audit log, so that I can show what was erased when.
34. As the operator, I want a tool that re-applies recorded erasures after a database restore, so that a restore
    doesn't bring personal data back.
35. As the operator, I want erasure to be set-based and idempotent, so that it fits D1's limits and a failed run can
    simply run again.
36. As the operator, I want to know which organizers accepted which version of the terms, so that I can prove it.
37. As the operator, I want to query raffles whose winners' data is older than the ceiling allows, so that nothing
    is held past it even if the scheduler was down.
38. As a self-hoster, I want to supply my own documents, operator name and contact, so that my instance speaks for
    me and not for Rafi's authors.
39. As a self-hoster, I want erasure on by default, so that the safe behaviour needs no flag.

## Implementation decisions

### Vocabulary

New terms, to go into the glossary:

- **Event date**: the calendar day the raffle is drawn. Required; fixed once a real entrant has joined.
- **Retention period**: days after the event date before non-winners are erased.
- **Erasure date**: event date + retention period. Shown to entrants; the day non-winners are erased.
- **Winner hold**: days after the erasure date a winner's data is kept, unless their prize is marked delivered.
- **Expire**: what happens to a raffle that was never frozen when its erasure date passes: its entrants are deleted
  and it becomes **Data erased** (`Purged`).
- **Claimed**: the resolution formerly labelled "Handed over": the winner was present and their code was checked.
  It does not mean the prize was delivered; **Mark delivered** does.

### The three clocks

| Clock | Counts from | Default | Who can shorten it | How |
|---|---|---|---|---|
| Non-winners | end of the event date | 7 days | installation (`RAFI_RETENTION_DAYS`, ≤ `RAFI_RETENTION_MAX_DAYS`), organizer defaults, the raffle | a setting, until the lock; then "Erase personal data now" |
| Winners | the erasure date | 90 days (`RAFI_RETENTION_WINNERS_DAYS`) | nobody configures it | "Mark delivered" / "All prizes delivered" moves a winner onto the non-winner clock |
| Everyone | the erasure date + winner hold | — | — | the hard ceiling: the raffle becomes `Purged` |

A winner is an entrant of a draw whose outcome is `claimed`, exactly as the removed design had it. Drawn entrants
who forfeited are non-winners. Winners are identified from the draws, not from the raffle's state, so an unfinished
raffle has winners too.

Installation defaults and ceilings are validated at startup: default ≤ max, all ≥ 0, max ≤ 365. `RAFI_RETENTION`
as an on/off flag is gone: erasure is always on. A deployment that wants no erasure doesn't exist.

### The event date and the erasure date

- `event_date` becomes required and typed. The admin sends the calendar date and the instant that day ends in the
  organizer's timezone (as the browser reports it). The store keeps both: the date for display, the instant for
  arithmetic. Past dates are allowed (the organizer may set the raffle up on the day), with a warning in the admin
  that erasure is then immediate or imminent.
- `create` takes an event date (the form defaults to today). `duplicate` gives the copy today's date, not the
  original's.
- `retention_days` is copied into the raffle at creation from the owner's defaults, or from the installation default
  if the owner has none, and must be ≤ the installation maximum.
- The erasure date is a stored column, written by the same settings write that writes the event date and the
  retention period: `erase_at = event_ends_at + retention_days × 86 400`. It is never written anywhere else.
- The lock: the existing `rules_editable` rule (no real entrant has accepted the rules) covers the event date, the
  retention period and therefore the erasure date, exactly as it covers the additional rules. The store's guarded
  settings write refuses the change the same way; the refusal names what is locked.
- Owner defaults gain `retention_days` (nullable: unset means the installation default), validated against the
  installation maximum when saved.

### What housekeeping does

`housekeeping::run` gains the erasure duty back, with a `now` the tests can pass:

1. **Expire** every raffle that is not frozen or later (`Draft`, `Ready`, `Open`, `Closed`), whose `erase_at` has
   passed: delete its entrants outright, set `purged_at`, move it to `Purged`. Nothing is verifiable in such a
   raffle, so nothing is kept. This is a new pure lifecycle rule, `expire`, alongside `unfreeze`: not a `Command`,
   because no organizer asks for it.
2. **Erase non-winners** of every raffle whose `erase_at` has passed, in any state: the `ERASED` write, one
   `erasures` row per entrant, one audit row per raffle naming the count (actor `housekeeping`, action
   `auto_erase`). Delivered winners are on this clock too.
3. **Erase everyone** of every raffle whose `erase_at + winner hold` has passed, the same way; raffles in `Finished`
   become `Purged` (as today's manual purge does) and their live channel is ended.
4. **Frozen or Drawing raffles are erased but not moved.** The engine owns their state. Their admin shows the
   raffle as drawing with erased entrants; the organizer can still finish and then purge. Best effort, after the
   ceiling, housekeeping asks the engine to finish such a raffle; erasure never waits for that.
5. Proof-of-work cleanup and ending live channels stay as they are.
6. Each successful run records `last_retention_ok_at`; `whoami` reports `retention_stale` (no success in 2 hours)
   and the admin shows the banner the removed design had.

All erasure is set-based SQL in one batch per duty, for D1. The run is idempotent: a raffle already purged, an
entrant already erased, match nothing.

### Erasure rules that change

- `personal_data_erasable` (one entrant, on request) becomes true once the raffle is over **or** its erasure date
  has passed. From the freeze until then, nobody leaves the list, as now.
- "Erase personal data now" (`purge`) stays as it is: finished or purged raffles only. It remains the organizer's way
  to shorten any clock after the lock.
- `ERASED` is unchanged except that it no longer mentions `consent_marketing`, which is gone.

### Winners

- The resolution `claimed` is labelled **Claimed** in the admin, the README and the guides; "Handed over" goes.
- `delivered_at` keeps its meaning. A new use case, **All prizes delivered**, sets it for every claimed winner of the
  raffle that hasn't got it, with an audit row. Available once the raffle is finished.
- The admin's People tab shows, per claimed winner without `delivered_at`, the date their data is erased, and a
  summary line on the raffle: how many winners are held and until when.
- There are no reminders. The summary line is the whole mechanism until a mail subsystem exists.

### The join form and the notices

- The checkbox says "I accept the raffle rules" and nothing else. Under it, plain text: the privacy notice link,
  the erasure date in words ("Your name and email are erased on 28 October 2026; winners' once their prize is
  delivered, at the latest on 26 January 2027"), and the minimum age ("By entering you confirm you are 16 or
  older"). `CONSENT_VERSION` is bumped.
- The minimum age is `RAFI_MIN_AGE` (default 16). It is rendered in the standard rules and on the join form. An
  organizer raises it in the additional rules, as today; there is no organizer field for it in this spec.
- The per-raffle privacy notice:
  - names the controller from `organizer_name`, and the privacy contact from `privacy_email`, falling back to
    `contact_email`;
  - states the three clocks with their concrete dates;
  - names the operator (`RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL`) as the processor running the service, and links
    to the installation's own privacy notice for the operator's subprocessors (hosting, Clerk, Restate, drand). The
    hard-coded Clerk and hosting sentences leave the code;
  - renders the organizer's `privacy_text` and/or `privacy_url` in a section of its own, **after** the generated
    sections, never instead of them;
  - loses the marketing paragraph.
- The installation's privacy notice and its terms are served at `/privacy` and `/terms` from documents the
  deployment supplies.

### Organizer identity and privacy fields

- `owner_defaults` and `raffles` gain `privacy_email`, `privacy_text` and `privacy_url`, copied at creation like the
  other defaults. Text and URL lock with the rules; the email stays editable (a contact must be current).
- `privacy_url` must be an absolute `https` URL; `privacy_text` has the rules' length limit.
- `Command::Open` is refused unless `organizer_name` is set and at least one of `contact_email` or `privacy_email`
  is. The refusal names the missing fields; the admin shows them before the organizer gets that far.

### Terms and the processing agreement

- One document, "Terms", with the processing agreement as part of it. The deployment supplies it as Markdown with a
  version string (`RAFI_TERMS_VERSION`); the installation's privacy notice likewise (`RAFI_PRIVACY_VERSION`).
  Natively the documents are files in `RAFI_DOCS_DIR`; on Workers they are bundled assets. Without them the server
  fails to start, except under dev auth, where built-in placeholders marked as such are used.
- A new table, `legal_acceptances`: owner, accepting user, document, version, accepted at. Primary key (owner,
  document, version).
- `create` and `duplicate` refuse with `TermsNotAccepted` unless the owner has accepted the current version. The
  admin shows the document and an **I accept** button; acceptance is an admin endpoint that records the row. Owner
  and accepting user come from the session (the auth layer must expose the user ID alongside the owner it already
  resolves).
- A version bump re-gates creation; existing raffles keep running. Material changes to the privacy notice are the
  operator's to communicate; the version makes "what was current when" answerable.

### Exports

- `export_csv` loses the `marketing_consent` column and writes an audit row (action `export_csv`, detail: the row
  count). It stays generated on request, never stored.

### Organizer offboarding

A new use case, **Remove my data**, with a confirmation, for the signed-in owner:

- refused while any of the owner's raffles is frozen or drawing (the event is live);
- pre-freeze raffles are deleted outright;
- every other raffle: all entrants erased (logged as a purge), `organizer_name`, `contact_email` and the three
  privacy fields blanked, the raffle `Purged`;
- `owner_defaults` deleted;
- `legal_acceptances` kept: they hold the owner's opaque IDs only, and are the operator's evidence;
- the Clerk account is deleted in Clerk; the docs say so.

The privacy notice of such a raffle names "the raffle organizer" as controller with no contact, and says the
organizer has left the service. There is nobody left to exercise rights against through Rafi, and no personal data
left to exercise them on.

### Restore tooling

A `rafi reapply-erasures` subcommand applies `ERASED` to every entrant that `erasures` names, and `Purged` to every
raffle whose `purged_at` is set, and reports the counts. It is idempotent. The self-hosting and Cloudflare docs make
it the step after any restore.

### Schema

Nothing is deployed, so `0001_init.sql` is edited in both dialects:

- `raffles`: `event_date` stays text (display), plus `event_ends_at INTEGER NOT NULL`, `retention_days INTEGER NOT
  NULL`, `erase_at INTEGER NOT NULL`, `privacy_email`, `privacy_text`, `privacy_url` (text, default empty).
- `entrants`: `consent_marketing` dropped.
- `owner_defaults`: `retention_days INTEGER` (nullable), `privacy_email`, `privacy_text`, `privacy_url`.
- `meta` is back, for `last_retention_ok_at`.
- `legal_acceptances` as above.
- An index on `raffles (erase_at)` where `purged_at IS NULL`.

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `RAFI_RETENTION_DAYS` | 7 | Non-winner period for owners who set none |
| `RAFI_RETENTION_MAX_DAYS` | 30 | The longest an owner or raffle may choose |
| `RAFI_RETENTION_WINNERS_DAYS` | 90 | Winner hold after the erasure date; the hard ceiling |
| `RAFI_MIN_AGE` | 16 | Rendered in the rules and the join form |
| `RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL` | required (placeholders under dev auth) | Named as processor in every notice |
| `RAFI_DOCS_DIR` | required natively | Where `terms.md` and `privacy.md` live |
| `RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION` | required | Versions recorded and displayed |

The settings reach the use cases through `Policy`, as `max_entrants` does today.

### Docs

`privacy-and-abuse.md` (Erasure section rewritten around the three clocks), `configuration.md`,
`self-hosting.md` and `cloudflare.md` (documents, operator identity, restore step), `running-a-raffle.md` (event
date, lock, Claimed vs. delivered, All prizes delivered), `glossary.md` (the new terms, `Purged` widened to
expiry), `routes.md` (`/terms`, `/privacy`, the acceptance and offboarding endpoints). `retention.md` stays history
with a line pointing here.

## Testing decisions

A good test here drives the system through its real entry points and checks what an entrant, an organizer or an
operator could observe: what the join page says, what `verify.json` lists, what the CSV contains, what state the
admin API reports, what the `erasures` and `audit` tables hold. No test asserts on SQL or on private functions.

**Seams**, from the top down; the existing ones are enough, and the HTTP seam carries most of the weight:

1. **The HTTP server tests** (`rafi-server`'s test module, in-memory SQLite, mock drand, the test engine): the same
   seam as `full_raffle_flow`, `public_view_has_no_personal_data` and `erasures_are_logged_without_personal_data`.
   Time is controlled by setting an event date in the past through the settings API before anyone joins, and by
   calling `housekeeping::run` with an explicit `now`. Scenarios:
   - the join page and privacy notice show the erasure date, the operator, the privacy contact, the minimum age, and
     no marketing box;
   - the event date and retention period are refused once a real entrant joined, with the right refusal;
   - non-winners are erased on the erasure date in `Finished`, in `Drawing`, and in `Open` (where the raffle
     expires and its entrants are gone, not erased); winners are kept; a delivered winner is not; the ceiling erases
     the rest and purges; the draw still verifies afterwards; a second run changes nothing; each erasure is logged
     once and the audit row names the count;
   - erasing one entrant on request is refused while frozen before the date and allowed after it;
   - "All prizes delivered" and the held-winners summary;
   - `Open` is refused with a blank organizer; allowed once named;
   - `create` is refused before the terms are accepted and allowed after; a version bump re-gates;
   - `export_csv` is audited and has no marketing column;
   - "Remove my data" blanks the organizer, purges, deletes drafts, is refused mid-draw, and leaves `verify.json`
     intact;
   - `whoami.retention_stale` after two hours without a run.
2. **Store conformance scenarios** (SQLite, Postgres, D1 alike), as `domain.erasure_log` and `domain.rules_fixed`
   are today: the erasure batch per duty, expiry, the guarded settings write refusing locked fields, owner defaults
   with the new columns, `legal_acceptances`, `meta`. These exist because D1 is reachable only here; the HTTP
   tests run on SQLite and, in CI, Postgres.
3. **Pure rules** in `rafi-lifecycle`, unit tests next to `removing_an_entrant`: `expire`, the widened
   `personal_data_erasable`, the open-gate, erasure-date arithmetic, config validation.
4. **The restore tool**: an HTTP-level test erases, re-inserts the personal data by hand, runs the tool, and checks
   the data is gone again.

Not tested by code: the rendered Markdown of the operator's documents; the Worker's `scheduled` handler (the
Workers CI job starts it under `wrangler dev`; a cron smoke test there is welcome but not required).

## Out of scope

- The marketing opt-in and everything it needs: its own plan.
- Any email from Rafi: reminders, expiry warnings, export notifications.
- A privacy-request portal, per-entrant export, lookup by email across raffles. Requests are handled by the
  organizer with the tools the admin has (People tab, erase one, export).
- An operator UI for reviewing held winners; the operational query and a documented review suffice.
- A per-organizer minimum age field.
- Separate entrant terms: the rules are the entrant terms.
- Re-acceptance flows beyond "a version bump re-gates creation".
- Changing what is public on the verification page before the erasure date.
- Encryption at rest, backup schedules, breach procedures: operational documentation, not this spec.

## Further notes

- **Why the event date and not the finish.** The requirement is that no ordinary organizer action can extend the
  deadline, and "Finish" is an ordinary organizer action. Entrants join at the event, so the event date is today
  when it locks and never needs to move. A backstop on creation or last-activity dates would have to be long and
  would promise entrants a date they can't see.
- **Why winners are held by default.** `claimed` means present with a valid code; a coupon may still go out by
  email next week. The wrong default harms the winner. The hold is bounded, which is the one thing the originating
  plan's §7 lacked.
- **Why one Terms document.** The processing agreement binds the same two parties as the terms; one acceptance,
  one version, one table.
- **Why erasure is always on.** The removed design was off by default natively and on by default on Workers, and
  its notice promised erasure either way. A notice that may be false is worse than a flag that can't be turned off.
- **Residue after erasure**, to be stated in the notice and checked in legal review: `created_at`,
  `consent_rules_at`, `delivered_at`, `source`, `hidden`, the entrant ID in the manifest, `draws`, `erasures` and
  `audit`, and D1 Time Travel for up to 30 days. None names a person; together they say that *someone* joined at a
  given second and won a given prize. The organizer's exported CSV is outside Rafi's reach, and the notice says so.
- **Legal review before the hosted launch** should cover: the operator's role (processor, or joint controller given
  that the operator fixes the data model, the public verification page and the retention ceilings); the Terms and
  the processing agreement against Art. 28; the generated notice against Art. 13; the winner hold; the residue
  above; the minimum age as rendered; the self-hosting documentation's allocation of responsibilities; UK GDPR and
  US prize-promotion rules for a worldwide instance.
- **Order of work.** Schema and the erasure duties first (restoring from `9448be9` where it still fits), then the
  event date and the lock, the join form and notices, the organizer fields and the open-gate, terms and acceptance,
  the restore tool, offboarding, docs. The marketing removal is a small first commit on its own.

## Follow-ups, after the review of the implementation

Recorded here so they aren't lost; none blocks the hosted launch.

- **Audit actor.** Done before the release ([docs/plans/pre-release-review.md](../plans/pre-release-review.md), SCH-1): every audit
  row carries `actor` (the owner key) and `user_id` (the signed-in user acting for it), so members of one
  organization are told apart.
- **Account deletion.** Deleting a Clerk account or organization doesn't remove the organizer's data here; the
  admin's "Remove my data" does, and the terms say to do that first. A Clerk webhook (`user.deleted`,
  `organization.deleted`) calling the same use case would close the gap.
- **Access tiers** come from Clerk's claims (admin, the custom `org:manager` role or `org:raffles:manage`
  permission, member). Clerk's free plan has only admins and members, which the tiers degrade to.
- **Restore on Workers** has no command: housekeeping re-erases what is past its date on its own, and the SQL for
  the rest is in the Cloudflare operations doc.
