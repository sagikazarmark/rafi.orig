# Entrants from a list

**Status: proposed, revision 5 (2026-11-01).** Issue #38: "Allow creating raffles where participants don't enter
themselves, but are entered from the list. Example: import registered users for an event from the registration
system."

Phase 1 extends the organizer's "add someone" to adding many people at once: paste a table, or pick a CSV file.
Phase 2, sketched at the end, lets other systems (a registration or check-in tool) add people through an API. Phase
1 is shaped so that phase 2 adds a transport and an authentication, not a second way of entering people.

The first draft was reviewed by a privacy expert, an architecture review against the code, an organizer-UX review
and a security review. Revisions 2 to 5 record the owner's decisions on top of that (§Review log).

> Not legal advice. §Further notes lists what a lawyer must check before the feature goes live on the hosted
> instance.

## Problem statement

Rafi assumes most entrants enter themselves: they scan the QR code, type an email and a display name, accept the
rules and see the privacy notice, and their phone keeps their claim code. The organizer can add a person without a
smartphone, one at a time.

Many events already know who takes part: a conference's registration system, a meetup's list on Meetup or Luma,
the guests checked in at the door, a company's invitees. Asking hundreds of people who already registered to scan
a code and type their email again is friction that buys nothing, and leaves out whoever missed the announcement or
has a flat phone. Some organizers want the draw to be among the people on their list, full stop, with no QR code at
all. Today the only way is "Add someone without a smartphone", once per person.

Doing it properly has needs a phone join doesn't:

- **Nobody accepted anything.** The rules, the age limit and the notice have to reach people another way, and the
  organizer is responsible for being allowed to share the list at all.
- **Nobody chose their display name.** The name comes from a registration form, and it is shown on the projector
  and the verification page, both public pages.
- **The data came from somewhere else.** GDPR Art. 14, not Art. 13: the notice must name the source and the
  categories, state the legitimate interest and the right to object, and reach people who never visited Rafi.
- **Being on a list can itself say something.** A support group's or a political meetup's attendee list reveals
  more than a name (Art. 9, indirectly).
- **The list is big and messy.** Hundreds or thousands of rows, columns Rafi doesn't need, CSV dialects that differ
  by country (`;` in much of Europe, Windows code pages from Excel), one order email for a group of tickets, no
  emails at all (Meetup), names longer than a display name may be.
- **No phone knows the entry.** No claim code on a phone, no "YOU WON!": the winner recognizes their name on the
  projector, and the organizer checks it is really them. This is how people added by hand work today.
- **A registrant isn't an attendee.** Winners must be present, and that stays (§Presence). Free events lose a third
  to a half of registrants, so a registration list means many "Not here" redraws; a **check-in list** means few.
  The main case for phase 1 is the check-in list.

## Solution

There is one kind of raffle. Two things change in it:

1. **Adding people** is one dialog, **Add people**, whatever else the raffle does. Its first tab is today's form for
   one person; its second, **Paste or upload a list**, adds many. Everyone added either way is **added by the
   organizer** (today's `manual` source): the same badge, the same check at the stage, the same paragraph in the
   privacy notice.
2. **Registration** (joining with the QR code) becomes a setting, **"Let people join with the QR code"**, on by
   default. Off, the raffle has no registration: no Open or Close, no QR code on the projector, no join form; it
   goes from Published to "Freeze list & start the draw", and its entrants are the people the organizer added.

Any member adds people, one by one or from a list, as any member adds one person today: no special access, no
further confirmation.

The list is pasted (copied from a spreadsheet) or a CSV/TSV file, **parsed in the browser**. The organizer maps
columns (the name, as one column or first and last; the email, optional; a registration ID, optional) and picks how
names appear: the **full name** (the default) or **first name and initial**. A preview shows counts (**312 to add ·
41 already in · 6 to check**), every row that needs a decision, and the first rows to be added. Only the mapped
fields of the rows to add leave the browser.

People added from a list are treated like everyone else: their names are on the projector (the recent names
included, so the room sees real people on the list) and on the verification page, the organizer can rename or hide
them, and they are erased on the same dates. Full names are stored as they are, up to 64 characters; the projector
fits long ones on screen. **Undo last add** removes one batch before the freeze. Someone removed at their request
stays out when the list is added again.

## User stories

### Organizers

1. As an organizer, I want to enter everyone on my guest list in one go, so that nobody has to scan a code.
2. As an organizer, I want adding one person and adding a list to be the same thing in the same place, whether or
   not people also join with the QR code.
3. As an organizer, I want a raffle with no QR code at all when only my list takes part, and otherwise the same
   raffle as always.
4. As an organizer, I want to paste rows straight from a spreadsheet or my registration tool's export, so that I
   don't have to clean a file first.
5. As an organizer, I want Rafi to find the name and email columns, in my tool's or my language's headers, and let
   me correct it, so that any export works.
6. As an organizer, I want to see before anything is stored what will be added and what needs my decision.
7. As an organizer, I want full names on the screen, as people wrote them, so that winners recognize themselves and
   the room sees real people, and the option of "Anna K." when the list is large or the event calls for it.
8. As an organizer, I want to add an updated list (the check-ins at the door) and have only the new people added,
   and a retried add never to add anyone twice.
9. As an organizer, I want a group order with one email for five tickets to give five entrants, not one.
10. As an organizer, I want to undo the last add before the freeze, so that a wrong file costs nothing.
11. As an organizer, I want a ready-made paragraph with the rules and privacy links, available before the event, so
    that I can put it on my event page or in my confirmation email.
12. As an organizer, I want to add my registrants and still let walk-ins join with the QR code.
13. As an organizer, I want the draw console to tell me to check a winner I added, with their email, so that nobody
    else walks up and takes the prize.

### People on the list

14. As someone on a list, I want to be told I'm in a raffle, by whom, under which rules, where my data came from,
    and that my name will be shown, before the draw.
15. As someone on a list, I want to recognize my name when I win.
16. As someone on a list, I want to be able to say I don't want to take part, and stay out even if the organizer
    adds the list again; after the freeze, my name hidden and my data erased when the raffle ends.
17. As someone on a list, I want my other registration details (phone, company, ticket type) never to reach the
    raffle service.

### Operators

18. As the operator, I want every add audited with who did it and the counts, without personal data, so that the
    organizer's instructions are on record.
19. As the operator, I want every add bounded by the entrant cap in the database and all or nothing.
20. As the operator, I want the terms and processing agreement to cover data organizers bring.

## Implementation decisions

### Vocabulary

| Term | Meaning |
|---|---|
| **Added by the organizer** (`Source::Manual`, `manual`) | Put in by the organizer, one by one or from a list. "added by you" in the admin. One source for both: no phone, no rules accepted, the same check at the stage. |
| **Add**, **batch** | One use of Add people: one person or a list. Each has a random batch ID (`entrants.batch_id`), the unit of retry and undo. |
| **Import**, **adding from a list** | An add from the list tab. |
| **Registration** (setting) | Whether people may join with the QR code. On by default. Off: no registration states, no join form. |
| **Name format** | How a list's name becomes a display name: `full` (default) or `first_initial`. |
| **Registration ID** (`list_ref`) | The source system's ID for a row, optional, admin only: matches people on a later add, especially without emails. |
| **Removed at their request** | An entrant taken off on request before the freeze (anyone real, a joiner too), remembered so a later add skips them (`add_removals`) unless lifted for a row. |
| **Took themselves off** | An added entrant who objects after the freeze (`objected_at`): hidden, declined if drawn, erased at the finish. |

### Feasibility

**Feasible, without touching the draw.** The manifest holds anonymous entrant IDs whatever their source, so
`rafi-raffle`, the engine, the algorithm and the verifier are untouched. The work is in `rafi-lifecycle` (the
registration setting and the freeze from Published, name formats and lengths), `rafi-store` (an all-or-nothing batch
insert, two small tables), `rafi-app` (the add use case replaces `add_manual`), the admin API, the web app (the
dialog, the console, the projector), the server's pages and texts, and docs.

What it builds on: `insert_fakes`'s chunked multi-row insert in one `db.batch`; the `INSERT … SELECT … WHERE …
ON CONFLICT DO NOTHING` the draws table already runs on all three backends; the display-name and email rules shared
by browser and server; the settings model with state-guarded SQL (`test_mode`); the claim-code generator; the audit
log; Dioxus 0.7's `FormEvent::files()` / `FileData::read_bytes()` for files.

Findings from the code that the plan accounts for:

- Change counts are only zero/non-zero, and on D1 include trigger writes (every entrant insert bumps the raffle's
  version), so a batch can't report how many rows it inserted: the use case reads back.
- A cap guard per statement isn't all or nothing: one decision per batch is needed (§Store).
- Rows with one `created_at` sort by random ID: a batch keeps the list's order with a row number.
- `unfreeze` and `start::next` assume a freeze from Open or Closed.
- The rules and privacy pages are 404 until the raffle is published.
- The UI picks lifecycle buttons from the state; the new rules must come from `AdminActions`.
- `insert_manual` and `insert_fakes` don't enforce the cap in SQL, and `insert_fakes` doesn't check claim codes for
  uniqueness: both fixed with the same tools.
- `insert_manual` records the join form's consent for someone who saw no form: an inaccurate record.
- `Email::parse` rejects whitespace only: control and bidirectional characters pass.
- The join page tells someone whose email is already in that "the device you entered on has your claim code",
  which is wrong for anyone the organizer added, today already.
- Display names are at most 24 characters (`MAX_NAME_CHARS`), checked on every join, add and rename; full names are
  often longer (§Names).

| Risk | Answer |
|---|---|
| D1 queries per batch | 4 shared + 7 per row parameters (the ID, the name, the email as given and its key, the claim code, the row number, the registration ID): 13 rows per statement, 156 statements for 2000 rows with the guard's two, one round trip, under the paid plan's 1000 per invocation (the hosted instance is on it). One JSON parameter (`json_each` / `jsonb_to_recordset`) stays a measured fallback: it would be the first SQL that differs by dialect. |
| Body size | At most `MAX_ADD_ROWS` (2000) rows per request, ~250 KB; the add route's limit is 512 KB. Larger lists go in parts, each part all or nothing: the list tab takes at most 2000 people to add, and the organizer adds the rest next (the people already in are skipped), so each add is one undo. |
| CPU on Workers | Validation of 2000 rows and the version trigger's 2000 bumps: measure in the store spike. |
| Bundle size | A hand-written RFC 4180 reader instead of `csv-core`, and `unicode-normalization` already in the build: the list tab took the web app's WebAssembly from 1035 to 1094 KB gzipped (2.96 to 3.11 MB raw). |
| Long names on screen | The projector's winner reveal, the recent names and the thanks screen must fit 64 characters: checked at projector size, and in the end-to-end evening. |

Rough size: two weeks, in the slices of §Slices.

### Presence

Unchanged: winners must be present, for every raffle, and "Not here" forfeits. The guide says to add the
**check-in list**, or to remove the no-shows before the freeze. Check-in before the freeze and a "winners must be
present" setting are later options (§Out of scope).

### One source, one way of adding

- No new source. `Source::Manual` is "added by the organizer", whether one by one or from a list; the admin says
  "added by you" as today. `Source::accepted_rules()` becomes `locks_settings()` (exhaustive: `SelfService` and
  `Manual` lock the rules, the event date and the retention; `Fake` doesn't). Callers: `settings.rs:362`,
  `raffles.rs:307`, `reads.rs:46`.
- Added entrants store `consent_version = NULL` and `consent_rules_at = NULL`: they confirmed nothing on a form.
  Today's manual entrants are fixed with them.
- New nullable columns on `entrants`: `batch_id`, `batch_row` (the row's position, for the list's order),
  `list_ref` (`UNIQUE (raffle_id, list_ref)`, admin only, erased with the entrant), `objected_at` (§Objections).
- **Add people** in the People tab replaces "Someone without a smartphone? Add them here.", available whenever
  entrants are editable, with or without registration:
  - **One person**: today's form (name, optional email). Any member, from Draft, as today.
  - **Paste or upload a list**: the import (§Parsing). Any member, as for one person; a published raffle (so the
    paragraph's links work), not in test mode (a rehearsal uses fake entrants; its draws are public).
- Both go through the same use case and the same batch insert: the one-person add is a batch of one, and gains the
  cap in SQL and retry safety.
- Size: at most `MAX_ADD_ROWS` per request, and the room left under `RAFI_MAX_ENTRANTS` for the whole add; an add
  that doesn't fit is refused whole, with the room left in the message. Rate: 10 adds or undos per minute per
  owner (the one-person form is far below that).

### Registration as a setting

- `raffles.registration INTEGER NOT NULL DEFAULT 1` and the same in `owner_defaults`; in `RaffleSettings`
  (serde default on), in `RaffleView` (for the projector), saved with the settings, copied by `duplicate` and from
  the defaults. The settings form: a switch, "Let people join with the QR code", with the hint "You can always add
  people yourself, one by one or from a list."
- **Editable before the freeze, except while registration is open** (Draft, Published, Closed): switching it off
  while phones are joining would cut them off mid-form; close registration first. SQL-guarded like `test_mode`.
- **Not fixed with the rules**, because the rules don't depend on it: they always describe both ways in (§Rules and
  notice). An organizer can add registrants, then switch registration on for walk-ins, or the other way round.
- The lifecycle takes it: `apply(self, cmd, registration)`: off refuses `Open`; `(Ready, Freeze)` is allowed only
  when off. `start::next(state, registration, …)` gives `unfreeze_to` = the state it froze from when that was
  Published, Closed otherwise as today; `unfreeze` accepts Ready. SQL guards: `AND registration = 1` on opening,
  `AND registration = 0` on a freeze from Ready.
- A freeze from Published runs the checks opening would have: a named controller and contact
  (`settings::missing_for_open`), an erasure date not passed.
- A raffle in Closed with registration switched off freezes from Closed as today; "Reopen registration" isn't
  offered.
- `AdminActions` gains the allowed lifecycle commands (the header stops deciding from the state), `add`, `import`,
  `undo_add`; `start` is computed with the setting. Steps and the empty People tab get wording for raffles without
  registration ("Add the people taking part, then freeze the list").
- **Decided in slice 7:** every lifecycle write is guarded on the setting the lifecycle decided with
  (`raffles::transition` and `freeze` take it: `AND registration = ?`), which gives both guards above and refuses a
  move whose setting changed since the read (`changed`). A raffle found frozen without a manifest doesn't record
  where it froze from: with registration it unfreezes to Closed as before, without it to Published. The view's and
  the settings' `registration` default to on when absent; `AdminActions` gains `lifecycle` (the commands
  `RaffleState::apply` allows with the setting) and `edit_registration` (`registration_editable`); `add_list` and
  `undo` were already there. Refusals: `no_registration` (opening without it), `registration_locked` (switching while
  open or after the freeze), and `organizer_missing` naming the freeze rather than opening when it is off. The
  admin header hides the join link and code without registration; the projector's intro says "The draw is coming
  up" where it would say registration opens or has closed.

### Telling people

There is no confirmation before adding a list: any member adds it. What the organizer is responsible for (a basis to
share the list, telling its people, the age limit, no lists that reveal sensitive data) is in the terms they accept
and in the guide, and the list tab says it in one line under the button:

> You're responsible for being allowed to share this list, and for telling the people on it about the draw before
> it, with the paragraph below.

**A reminder at the freeze**, when the raffle has entrants added from a list: one line in the freeze dialog, no
checkbox, nothing recorded:

> People added from a list will be shown on the screen and the verification page. Make sure they were told about
> the draw.

### Parsing, mapping and the preview (browser)

All in a pure module of the web app (`apps/web/src/import.rs`, no Dioxus, natively tested); the rules the server
re-checks are in `rafi_lifecycle::entrant`.

- **Input.** A textarea (paste: the main path; spreadsheets copy as tab-separated UTF-8) and a file input for
  `.csv`, `.tsv`, `.txt`, at most 5 MB (the textarea too). Files are read as bytes and must be UTF-8 (BOM or not):
  otherwise "This file isn't UTF-8. Open it in your spreadsheet, select the cells, copy, and paste them here." XLSX
  isn't read: copy and paste. A preview showing mojibake patterns (`Ã¡`, `Å‘`) warns the same way. The parsed data
  lives in memory only: no storage, no logs.
- **Limits.** 10 000 rows, 200 columns, 1 KB per field; an unclosed quote that swallows the rest is an error; the
  delimiter (tab, `,`, `;`) is sniffed from the first 20 lines (or Excel's `sep=` line); RFC 4180 quoting. A list
  is read a moment after it is pasted, behind a spinner, and the preview made again only when what it depends on
  changes (10 000 rows: well under a second).
- **Columns.** Detected by header, case- and accent-insensitive, with aliases for the common exports: `name`,
  `full name`, `first name`, `given name`, `last name`, `surname`, `family name`, `email`, `e-mail`, `attendee
  email`, `ticket first name`, `ticket last name`, `ticket email`, `order email` (an attendee or ticket email beats
  an order email), `api_id`, `order #`, `attendee #`, and Hungarian `név`, `vezetéknév`, `keresztnév`, `e-mail
  cím`. Otherwise the mostly email-shaped column is the email. The organizer corrects any of it, and says whether
  the first row is a header. Fixtures from real Luma, Eventbrite, Pretix, Tito, Meetup and Google Forms exports are
  collected before the detector is written.
- **Checks per row.** The shared name rules (§Names) and `Email::parse`. A name the blocklist refuses (its
  substring matching catches real names: Nazir, Buzinkay, Shitara) can be approved row by row by the organizer;
  approval lifts the blocklist only, never the invisible-character or legibility rules, and is counted in the audit.
  Other failures are listed with their reason, to fix in the source or add without.
- **Duplicates.**
  - Same email key and same name: one person, the first row kept.
  - Same email key, different names (a group order): all kept, the email on the first only; the preview groups
    them ("Same email, different names (7)") with **Keep all** (default) or **Keep first only**.
  - An email or registration ID already in the raffle: "already in", skipped. When the one already in joined by
    phone under another name, the preview says so ("joined by phone as *Batman*"): someone may have used the
    registrant's email first.
  - An email or registration ID **removed at their request** earlier: skipped; the organizer can lift it per row.
  - Rows with neither email nor registration ID can't be matched again: the preview says so ("Without emails or
    IDs, adding this list again may add people twice").
- **Sent.** For each row to add: the display name, the email, the registration ID, approvals, lifted removals
  (`lift_removal`); plus the batch ID
  (random, made for the rows sent: the same rows sent again, a retry, send the same one; while an add's reply is
  unknown, its entrants are left out of the preview so it stays the same). Nothing else.

### Names

- **Stored as written.** Names the organizer adds (one person or a list) may be up to **64 characters**
  (`MAX_ADDED_NAME_CHARS`); joiners keep 24 (`MAX_NAME_CHARS`), whose point is to keep trolls from filling the
  projector, which doesn't apply to the organizer's own list. A rename by the organizer allows 64 for any entrant.
  Nothing is shortened in the database: the stored name is what the person and the organizer recognize.
- **Fitted on screen.** The projector's winner reveal scales the font down for long names, and the recent names and
  the thanks screen cut with "…" where they must; the verification page, the admin, the phones and the CSV show
  them in full.
- NFC-normalized: every display name, joins included (`display_name`, `added_name`), so lengths count what a reader
  sees and the uniqueness check compares one spelling. One column ("Names are written family name first (Kovács
  Anna → Anna Kovács)", shown only then) or first and last name columns.
- **`full`** (default): the full name. **`first_initial`**: "Anna K.", for large or more private events.
- **Collisions**: display names are unique within the raffle, joined entrants included. Real namesakes get a
  number ("Anna Kovács (2)"); with `first_initial`, more letters of the family name first ("Anna Ko.", "Anna Ki."),
  at most three, then a number. Letters are added only where they tell people apart (or move them off a name in
  the raffle): two Anna Kovács are "Anna K." and "Anna K. (2)". The draw console's email tells namesakes apart.
- The server re-checks the length, the format and the uniqueness, and refuses a batch that collides.

### Store

- **All or nothing with a guard row.** A table `batch_guards (key TEXT PRIMARY KEY)`, empty outside a batch. One
  `db.batch`:
  1. `INSERT INTO batch_guards (key) SELECT ?key WHERE <raffle editable, erase_at > now, and for lists: published,
     not test mode> AND (SELECT COUNT(*) FROM entrants WHERE raffle_id = ?r) + ?n <= ?cap AND NOT EXISTS (SELECT 1
     FROM entrants WHERE raffle_id = ?r AND batch_id = ?batch)`: the one decision. The guard table has no trigger,
     so its change count is reliable.
  2. The rows, 13 per statement: `INSERT INTO entrants (…) SELECT * FROM (VALUES …) AS v WHERE EXISTS (SELECT 1
     FROM batch_guards WHERE key = ?key) ON CONFLICT (raffle_id, email_norm) DO NOTHING`. (`list_ref` conflicts are
     filtered by the app first; an ID collision still errors.)
  3. `DELETE FROM batch_guards WHERE key = ?key`.

  The key is `add:<raffle_id>`, so two adds into one raffle serialize. SQLite and D1 serialize batches; Postgres
  runs them serializable with retries. Afterwards the use case reads back the batch's IDs: the missing ones lost an
  email race with a join and are reported as skipped. A refused guard leaves nothing; the use case re-reads the
  raffle to say why. A retried batch (same ID) is refused by the guard and answered with the first one's result.
- `insert_manual` goes: the one-person add is this batch. The same guard gives `insert_fakes` the cap.
  `ids::code_besides(&mut HashSet)` gives unique claim codes to adds and fakes.
- `delete_batch(raffle_id, batch_id)` while editable; `remove_entrant` "at their request" (any real entrant, a
  joiner who asks too: the next list may have them) also writes `add_removals (raffle_id, email_norm, list_ref,
  removed_at)`, in the delete's batch and under its condition: one row with both keys (`UNIQUE` per raffle on
  each), and a row of its own for a key the first couldn't take because the other was remembered already. An
  add's rows go in only where no removal matches (`NOT EXISTS` on the `VALUES`' `column5` and `column11`, the same
  on every backend), and the removals the organizer lifts are deleted under the guard first, so lifting is all or
  nothing with the add. **Decided in slice 5:** the rows are deleted when the manifest is written (with the list
  fixed for good, no add can come and the email keys serve nothing), not on the erasure date; also in
  `expire_due`'s batch, with an erasure of everyone (a purge), with the owner leaving, and by the raffle's
  deletion (cascade). Housekeeping's `forget_removals` deletes any left for a raffle that is frozen, purged or
  past its erasure date, which is what `reapply-erasures` (through housekeeping) needs after a restore. They hold
  an email key in clear, as `entrants` does: a hash without a secret would hide nothing.
- Conformance on SQLite, Postgres and D1: several chunks; the cap refusing the whole batch (no rows, no guard row
  left); a frozen raffle; an email clash with a joined entrant; a race with joins; a race with the freeze (every
  inserted entrant is in the manifest, or the add is refused); a retried batch ID; undo; removals; a 64-character
  name.

### Use cases (`rafi-app`)

- `add_entrants(caller, raffle, AddRequest { batch_id, origin: One | List, rows, name_format })`, replacing
  `add_manual`, any member: `One` takes exactly one row; `List` needs what §One source lists.
  Every row re-validated (names, emails, approvals, duplicates against current entrants and removals, display-name
  uniqueness), claim codes, one batch, one audit row `add`: `batch=<id> origin=<one|list> added=<n> skipped=<m>
  approved=<k> format=<f>` (replacing `add_manual`). Invalid rows or a name collision refuse the whole request
  (400, with row indexes and reasons, so the browser can resolve and resend); emails already in are skipped and
  reported. No live notification: nothing on a phone changes.
- `undo_add(caller, raffle, batch_id)`: while editable, audited `undo_add` with the count. The UI offers the last
  batch; the API takes any.
- `remove_entrant` gains "at their request" (`at_request`), audited as the entrant's ID and `at_request`; an add
  reports a row left out for it as `removed_at_request`, and counts lifts (`lifted=<k>`).
- These use cases are what phase 2 calls.

### Objections

- **Before the freeze:** remove the entrant, "at their request". They stay out of later adds.
- **After the freeze**, for entrants added by the organizer (they accepted no rules): "Took themselves off" in the
  People tab hides the name at once and sets `objected_at`; if they are drawn, the console says so and offers
  **Declined** only; they are erased when the raffle finishes, not on the erasure date. The anonymous ID stays in
  the manifest: removing it would break the verification. Audited `object`.
- **Decided in slice 5:** the rule is `entrant::may_object` (`RaffleState::objections_allowed`, Frozen or Drawing;
  `Source::may_object`, manual only; not a winner who claimed a prize: the claim is the draw's result and stands,
  and their data is erased on request at the finish), and the same in the write's SQL. The server refuses
  "Claimed" and "Not here" for an objected winner before the engine is asked (`objected_may_be`): they claim
  nothing, and "Not here" would record an absence. An objected name can't be shown again (`patch_entrant`
  refuses, in SQL too). `unobject_entrant` undoes a mistake until the finish, audited `unobject`; the name stays
  hidden until the organizer shows it. Housekeeping erases objected entrants of a raffle that is over within the
  hour (`erase_objected`: logged, audited `auto_erase` with `<n> objected`), before `erase_due`.
- `privacy-and-abuse.md`'s "sharing their entry is part of the rules entrants accept" is reworded: it holds for
  people who joined, not for people the organizer added.

### The evening

| Place | Entrants added by the organizer |
|---|---|
| **Projector, before the freeze** | In the recent names like everyone (a batch's rows are stamped with `created_at = now − NAME_DELAY_SECS`, in list order, as fakes are), and in the entrant count. Without registration: the prizes, "Everyone on *organizer*'s list is in the draw. Rather not? Tell the organizer before the draw.", the privacy URL; no QR code, no "No smartphone?" badge. With registration and a list added: "Registered? You're already in. Not registered? Scan to join." |
| **Projector, a draw** | The display name, fitted to the screen, and "Come to the front!" for a winner with no phone to tell them. |
| **Draw console** | The name, the email (masked until tapped: consoles get mirrored), "added by you", and "Check it's them: ask for the email they registered with, or their badge." The button reads **"Confirmed: it's them"** for an entrant with no phone; the resolution is `claimed`. |
| **Verification page and `verify.json`** | Names listed like everyone's. Counts by source are published: "320 entrants: 8 joined on their phones, 312 added by the organizer". |
| **`/j/<CODE>`, no registration** | No form: "This raffle has no registration: the organizer adds the people taking part", links to the rules, the privacy notice and, once frozen, the verification page. |
| **`/j/<CODE>`, an email already in** | One wording for every source: "This email is already entered. Your entry is safe. If you win, go to the organizer." Typing an email never attaches a device to an existing entry: a rule, with a test. |
| **Winners CSV** | `source = manual`, as today. Tab and carriage return join the formula-injection guard's prefixes. |

`/v/` and `verify.json` get `X-Robots-Tag: noindex` (for every raffle). Names are rendered with bidi isolation.

### Rules and notice

- **Rules**, for every raffle (so registration can be switched without changing what anyone accepted): "Entrants
  are the people who join with the QR code while registration is open, and the people the organizer adds (for
  example, from the event's guest list). If the organizer added you, you can ask them to take you off." The age
  rule gains: "the organizer is responsible for the people it adds meeting the age limit". Winners present, free
  entry and no cash are unchanged.
- **Privacy notice**, a section **"If the organizer added you"**, always present: the data (name, and email if
  given; nothing else); the source (from you, at the event, or from the organizer's records about the event, such
  as its registration or guest list); the purpose (this draw); the basis, Art. 6(1)(f), with the interest named
  ("rewarding the attendees of *event* with a prize draw"); then, in a separate paragraph, the **right to object**
  and what it does before and after the freeze; that the name is shown on the screen and the public verification
  page as for everyone; the same erasure dates. The paragraphs written for phone entrants ("Your email address is
  required to enter") say they are about joining with the QR code.
- **The paragraph to send**, in the list tab and in the raffle's settings (so it can go on the event page in
  advance): plain text, for the organizer to edit, the organizer's `privacy_url` only if `https:`, with a concrete
  cutoff:

  > You're in the prize draw of *\<title\>* on *\<date\>*, run by *\<organizer\>* with Rafi, as someone on our
  > guest list. Your name is shown on the event screen and the public results page as your full name, like “Anna
  > Kovács”. Rules: *\<rules URL\>* · Privacy: *\<privacy URL\>*. Rather not take part? Tell *\<contact\>*
  > before the draw on *\<date\>*.

### Erasure and retention

Unchanged clocks. Added entrants and their registration IDs are erased like everyone's data; objected ones at the
finish; `add_removals` when the manifest is written (or the raffle expires, is purged or deleted). Erasures are
recorded and re-applied after a restore as today; `reapply-erasures` runs housekeeping, which erases objected
entrants of raffles that are over and forgets removals no add can need.

### Terms, the processing agreement and the operator's records

- The terms template (§4) adds data the organizer provides (adds, lists, later the API) and the people on
  organizers' lists as data subjects; adding is an instruction; the organizer warrants a basis to share the list,
  that its people are told, that it reveals no special-category data, and the age limit; the service receives only
  name, email and registration ID. A new terms version, accepted as terms are today, which the operator notifies
  organizers of directly (the processing agreement changes).
- The operator's record of processing (Art. 30(2)) and a short assessment of the feature (not a full DPIA for
  ordinary meetups).

### Admin API

- `add_entrant` is replaced by `fn add_entrants(id: String, add: AddRequest) -> AddResult;`
- `fn undo_add(id: String, batch_id: String) -> u32;`
- `fn object_entrant(id: String, entrant_id: String) -> ();` and `fn unobject_entrant(id: String, entrant_id:
  String) -> ();`, and `remove_entrant` gains `at_request: bool` (`#[serde(default)]`: the macro takes `serde`
  attributes on arguments).
- `registration` in `RaffleSettings`, `RaffleDefaults` and `RaffleView`; `list_ref`, `batch_id`, `objected` and
  per-entrant `actions` (`object`, `unobject`) in `AdminEntrant`; `removed` (the removals at their request, email
  key and registration ID) in `RaffleAdmin`; `lift_removal` in `AddRow`; the source counts in `VerifyData`; the lifecycle commands and the new actions in `AdminActions`.
- No tier gate: like `add_entrant` today, open to any member.

### Docs

`glossary.md` (Added by the organizer, Batch, Registration), `running-a-raffle.md` (Add
people; "From a guest list": export the checked-in guests, tell people in advance; raffles without registration;
the checklist), `the-draw.md` (checking a winner you added), `privacy-and-abuse.md` (where added data comes from,
what is sent, what is public, the organizer's responsibilities, objections, the audit
actions `add`, `undo_add`, `object`), `for-participants.md` (being added by the organizer), `verifiability.md`
(added entrants are the organizer's word, shown as source counts; no grinding: entrant IDs are server-random and the
round is unknown until after the manifest is fixed; a threat-model row "Organizer stacks the list: not prevented,
visible"), `routes.md`, `configuration.md`, the terms template. Guidance: don't add lists of sensitive events or of
minors; use QR joins there.

## Slices

Each is shippable and tested on its own.

0. **Groundwork.** `Email::parse` refuses control and invisible characters; the CSV guard's prefixes; added
   entrants stop recording a consent; the join page's one wording; `noindex` on `/v/`.
1. **One way of adding.** The guard table and the batch insert with its conformance (the cap in SQL, for fakes
   too), `code_besides`, the new columns, `add_entrants` replacing `add_manual`, `undo_add`, the Add people dialog
   with its one-person tab, `MAX_ADD_ROWS`.
2. **Long names.** `MAX_ADDED_NAME_CHARS`, the projector fitting long names (reveal, recent names, thanks screen),
   checked at projector size.
3. **Lists, the server's side and the texts.** Name formats, the name key and the collision resolver in
   `rafi_lifecycle::names` (pure, for the browser); the server's uniqueness check of added names (the one-person
   form too) and blocklist approvals, with `approved` and `format` in the `add` audit row; the rules' sentences and
   the join form's new version; the privacy notice's section; the paragraph (`rafi_api::paragraph_to_send`) in the
   settings.
4. **The list tab.** Parser, column detection with fixtures, mapping, files, the preview with groups and decisions,
   approvals per row, retries by batch ID, names made with `names::unique_names`, the paragraph and the
   responsibility line.
5. **Removals and objections.** Removal at their request with `add_removals` and its erasure; objections after the
   freeze. Done: removals forgotten with the manifest (§Store), objections refused for a claimed winner and
   undoable until the finish (§Objections).
6. **The evening.** The console's prompt and "Confirmed: it's them", "Come to the front!", the reminder at the
   freeze, source counts on `/v/`, the projector's lines. Done: whom no phone tells is a rule of the source
   (`Source::no_phone`: added by the organizer; a fake stands in for a joiner), sent as `no_phone` on the view's
   draws, the participant view's latest winner (the projector takes the reveal from there when the live channel is
   ahead) and the admin's entrants; the console shows no claim code for them (they never saw one) and masks every
   winner's email until **Show**. "Added from a list" is an entrant the organizer added with a row number
   (`batch_row`), kept through erasure: the view's `list_added`, which the projector's "Registered? You're already
   in." and the freeze's reminder go by. The view carries the organizer's name and the privacy notice's address
   for the intro without registration. `VerifyData.sources` counts the manifest's entrants by their rows' source at
   read time (left out when they don't add up to the manifest); not in the manifest. The header's missing fields
   come from `AdminActions.missing` (`RaffleState::opening_ahead`). The confirmations isolate names (`<bdi>`).
7. **Registration as a setting.** Schema, settings, defaults, duplicate, the SQL guards, `apply` and `next` with
   the setting, the freeze from Published and unfreezing to it, the lifecycle buttons from `AdminActions`, the
   projector and `/j/` without registration. Done: every lifecycle write guarded on the setting (§Registration as
   a setting).
8. **Docs and terms.**

## Testing decisions

HTTP-level, observing what the organizer, the entrant or the operator can see:

- adding one person works as today, for any member, through the batch: no consent recorded, a unique claim code,
  one `add` audit row; a retry with the same batch ID adds nothing and returns the same result;
- a member's list add is stored with `source = manual`, the list's order, and one audit row with counts, the
  member, and no personal data;
- a second add of an updated list adds only new emails and registration IDs; a group order gives every name, the
  email on the first;
- an add past the cap is refused whole and adds nothing, for one person too; a list add in test mode, before
  publishing, past the erasure date or after the freeze is refused;
- the server refuses rows the browser should have caught (invalid email, control characters, a blocked name
  without approval, two equal display names, a name over 64 characters) with their indexes; a joiner's name over
  24 is still refused;
- an added entrant locks the settings; undoing the batch unlocks them;
- someone removed at their request is skipped by the next add until lifted; the marker is gone after erasure;
- after the freeze, an objection hides the name, the console offers Declined only, and the entrant is erased at
  the finish;
- added entrants are in the recent names and in `verify.json`'s names in full; the source counts are published;
- the notice has the "If the organizer added you" section; the admin's view says whether the raffle has entrants
  from a list (for the freeze dialog's reminder), and the freeze needs no tick;
- with registration off: `Open` is refused, the raffle freezes from Published only with a controller and contact
  and unfreezes to Published, the view says there is no registration, and `/j/` has no form and takes no join;
  switching it is refused while registration is open and after the freeze;
- typing an added entrant's email on the join page gives the common wording and no cookie.

Store conformance as in §Store. Unit tests in `rafi-lifecycle` for `locks_settings`, the registration rules, the
freeze from Published, name lengths, formats and collisions. The parser natively, with real export fixtures,
mojibake, `;` CSVs, BOMs and pathological quoting. The end-to-end evening draws a 64-character name.

## Out of scope for phase 1

- **Presence options.** Winners must be present in every raffle. Later, if asked for: a check-in step before the
  freeze (the organizer marks who came, by tap or by adding the check-in list, and the freeze offers to remove the
  rest), or a "winners must be present" setting for online events (winners contacted afterwards, "Can't reach"
  instead of "Not here", every prize sent later).
- **An organizer's confirmation before adding a list** (where it comes from, a basis, no sensitive data, the age
  limit), recorded per raffle and feeding the notice's source. The terms carry those responsibilities for now.
- A private projector. `/s/<slug>` and the full view are public to anyone with the slug, so the names on the
  projector are public online until erasure, as the verification page's are. Revisit with a secret projector link
  (`/s/<slug>#k=…`) if organizers ask for it.
- Replacing a list (removing whoever is no longer on it); the organizer removes cancellations by hand.
- XLSX and ODS; non-UTF-8 files (paste instead).
- Sending anything to entrants from Rafi.
- Personal links (phase 2).
- The projector cycling through the whole list (it shows the latest 20 names, as today).
- A call-up countdown on the projector.
- A filter on a status column in the list tab ("only checked-in rows"); remembering the column mapping.
- Confusable-character checks (UTS #39) across display names.
- Committing the source counts in the manifest (a new manifest version and a verifier change).

## Phase 2: an API for other systems

Sketched to keep phase 1's seams right; it gets its own plan.

- **What matters most is check-in**: an entrant added when they check in is present by definition. A generic
  endpoint that Zapier, Make and n8n can call covers Luma, Eventbrite, Tito and Google Forms; a Pretix plugin is the
  natural first native integration.
- **Endpoints**, versioned and documented as a public contract: `PUT /api/v1/raffles/{id}/entrants/{ref}`
  (idempotent upsert), `DELETE` of the same (a cancellation, before the freeze: required of integrations, for
  accuracy), `GET /api/v1/raffles/{id}` (state, count, whether entrants are still accepted). They run the phase 1
  use cases as list adds.
- **API keys**: created in the admin (who may create them is phase 2's question); `rafi_<id>_<256-bit secret>`, the secret's SHA-256 stored, constant-time
  compared; owned by the organization with the creating user recorded; scoped to `entrants:write` on named raffles;
  expiry, last use, a second key for rotation, revoked by "Remove my data"; every call audited with the key ID and
  rate-limited per key. A key can never read emails or claim codes. Evaluate Clerk's API keys first.
- **Origin**: `/api/v1` outside Clerk's layer, bearer only, cookies ignored, no CORS; `/api/admin` never accepts a
  key.
- **References**: limited to 64 characters of `[A-Za-z0-9._-]`, never email-shaped; stored as HMAC-SHA256 under a
  server secret (`RAFI_REF_KEY`, versioned) over the raffle and the reference: pseudonymous, personal data, erased
  with the entrant. Phase 1's `list_ref` moves to the same hash then.
- **Personal links** (`/j/<CODE>#e=<token>`): the token in the fragment, never sent to the server by a link
  scanner or in a `Referer`; the page posts it to get the entry cookie and clears it. 128-bit, stored hashed,
  rotatable, expiring at erasure, not single-use. They give an added entrant the claim code, "YOU WON!" and the
  notice on their phone, which fixes the stage check properly. Returned once, at creation.

## Further notes

- **Why parse in the browser.** It is the minimization: a registration export has a dozen columns and the service
  never sees the rest. It also keeps the server from parsing spreadsheets; it re-checks what it receives.
- **Why full names, stored as written.** Winners must recognize themselves without a phone telling them, and the
  room should see the list is made of real people. A name shortened in the database would be less recognizable and
  lost for good; fitting it on screen is a layout job. The cost is that the names are public online, on the
  projector's public view and the verification page, until erasure; the paragraph and the notice say so,
  `first_initial` is one click away, and the organizer can hide any name.
- **Why no access control on lists.** Adding a list is adding people, which any member does today; the audit row
  records who added what. A tier (lists for Managers) can come later with the confirmation.
- **Why no confirmation, before adding or at the freeze.** The organizer owes the notice to the people on the list,
  and the terms say so. A checkbox Rafi can't check proves little (the audit row already shows who added the list
  and when), and a required one at the freeze blocks the organizer on stage. What helps is the paragraph to send,
  available before the event, the line in the list tab and the reminder at the freeze. A recorded statement per
  raffle (where the list comes from, a basis, no sensitive data, that people were told) would be stronger evidence
  and give the notice a concrete source; it can come later without changing the data model.
- **Why one source.** Adding one person by hand and adding a list are the same act with the same consequences: no
  phone, no rules accepted, the organizer's word, a check at the stage. Two sources would differ only in the UI.
- **Why registration isn't fixed with the rules.** The rules describe both ways in for every raffle, so switching
  registration changes nothing anyone accepted.
- **The list is the organizer's word.** As with people added by hand today, nothing stops an organizer from
  inventing entrants. The draw stays fair over the list it was given; the presence rule makes invented entrants
  forfeit, the source counts make the list's provenance visible, and the stage check keeps a stranger from claiming
  as someone else.
- **Legal review before launch** should cover: Art. 6(1)(f) and the interest as stated; publishing full names of
  people who didn't choose their display name, and its description in the notice; the Art. 14 notice with a
  generic source, and the organizer's duty to deliver it (Rafi provides it and reminds at the freeze); whether the
  terms alone carry the organizer's warranties well enough, with no recorded statement per raffle; the
  processing agreement's coverage of data organizers bring; the objection mechanics after the freeze (Art. 21: the
  anonymous ID kept for verifiability); sensitive lists and minors, now only in the terms and the guide.

## Review log

- **Privacy:** a notice confirmation at the freeze (a reminder since revision 5), the projector's opt-out line, a
  concrete cutoff in the paragraph; Art. 6(1)(f) with the interest named; objections after the freeze; one "already
  entered" wording; the terms and the record of processing; the age wording; manual consent fixed.
- **Architecture:** the guard-row batch instead of per-statement cap guards; read-back instead of change counts;
  no live notification; the lifecycle functions taking the new setting; the opening checks on a freeze from
  Published; lists only once published; `AdminActions` for the new rules; `MAX_ADD_ROWS`; unique claim codes; byte
  reads for UTF-8; the slice order.
- **UX:** the check-in list as the main case; group orders; batch IDs for retries and registration IDs for
  email-less lists; the stage check; "Confirmed: it's them" and "Come to the front!"; paste first, no XLSX; export
  aliases; a preview of counts and decisions; undo of the last add; the paragraph in the settings.
- **Security:** all or nothing; removals that stick; the stage check; no device linking by email, as a tested rule;
  the phone-joined-first warning; NFC, `Email::parse`, bidi isolation, CSV guard; blocklist approvals per row; source
  counts and the no-grinding argument; browser parsing limits; body limit and rate limit; noindex; no list adds in
  test mode; phase 2's keys, origin, references and fragment tokens.
- **Revision 2, the owner's decisions:**
  - Full names by default, shown like everyone's: on the projector (recent names included) and the verification
    page. This reverses the privacy review's "imported names off the verification page by default", its
    "collisions never widen" (moot with full names) and its "forfeits without names"; the admin-only list name
    goes. Kept as mitigations: the paragraph and the notice say names are public, the `first_initial` format,
    hiding per entrant.
  - Registration is a setting ("Let people join with the QR code") instead of a raffle mode; the rules cover both
    ways in, so it isn't fixed with them.
  - One way of adding: the one-person form and lists share a dialog, a use case, a batch insert and the `manual`
    source; `Source::Imported` goes.
  - The projector stays public; a private projector link is deferred.
- **Revision 3, the owner's decisions:**
  - Presence stays required for every raffle; check-in and a presence setting are deferred.
  - Names are stored as written, up to 64 characters for names the organizer adds, and fitted on screen; nothing is
    shortened in the database.
  - No confirmation before adding a list, and the terms carry the organizer's responsibilities. This drops the
    privacy review's per-raffle statement (the source for Art. 14(2)(f), the opt-in variant, the special-category
    and age statements), deferred; the notice names the source generically.
- **Revision 4, the owner's decision:** no special access control: any member adds one person or a list.
- **Revision 5, the owner's decision:** the required tick at the freeze goes; a reminder line without a checkbox
  takes its place. Nothing records that people on a list were told.
