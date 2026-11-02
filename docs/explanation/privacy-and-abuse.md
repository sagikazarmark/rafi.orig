# Privacy and abuse

What Rafi stores, what it publishes, when it erases it, and how it keeps a raffle free of spam and remote entries.
The privacy notice each raffle shows its entrants is rendered from the raffle's settings at `/privacy/<slug>`; the
installation's own notice is at `/privacy`, and the operator's terms, with the data processing agreement, at
`/terms`.

> This page describes the software, not your legal obligations. As the organizer, you are the data controller; the
> operator of the installation is your processor. The plan behind this design is
> [docs/history/privacy-and-retention.md](../history/privacy-and-retention.md), and for the people the organizer
> adds, [docs/history/entrant-import.md](../history/entrant-import.md).

## Data minimization

| Data | Who sees it |
|---|---|
| **Display name** | The projector (after a 5 s delay, unless the organizer hides it), the verification page's list of entrants (until erasure), the admin. It is optional: an entrant who leaves it blank gets a generated one ("Brave Otter"), so nobody has to put their real name on a screen. Up to 24 characters when joining; up to 64 when the organizer adds someone or renames anyone, so a full name from a registration list is stored as written, never shortened (the projector fits it on screen, cutting it with "…" only in the recent names and on the thanks screen). A list's names are its full names, or first name and initial ("Anna K.") if the organizer chooses. The organizer can hide or rename it in one tap. Hidden names show as ★ ★ ★. |
| **Email** | The admin only. It is never on the projector, in either public view, on the verification page, or in the raffle engine. |
| **Registration ID** | For someone the organizer added with one (their ID in the organizer's registration system, to match them on a later add): the admin only, erased with their name and email |
| **Removed at their request** | The email key and registration ID of someone removed before the freeze because they asked, so that a later add leaves them out: the admin only, kept only until the list is frozen for good |
| **Claim code** | The entrant's own phone and the admin, until erasure |
| **Entrant ID** | Public. It is random, and is what the manifest and the event log contain. |

The **raffle engine** (Restate) receives only the manifest: anonymous entrant IDs and item names. It keeps its
journal for a short retention window and forgets a raffle when it finishes. **drand** receives no personal data at
all: Rafi only downloads public beacons. **Clerk** authenticates organizers only.

## What is public

Once the list is frozen, `/v/<slug>` and `/api/r/<slug>/verify.json` publish:

- the manifest: entrant IDs, item names, the algorithm, the drand chain
- the event log: every draw, its round, signature, pool size and winner ID
- a map from entrant ID to display name, so people can check that they're in: the verification page lists every
  entrant with **Find your name**, and tells an entrant who opens it from their phone that they're in. An erased
  entrant drops out of it (their ID stays in the manifest), so erasing everyone **empties it**.
- whether the raffle was a rehearsal in **test mode**, whose entrants may be fakes. Switching test mode off removes
  every fake entrant, and a raffle out of test mode never freezes with fakes in it.
- how many entrants joined on their phones and how many the organizer added (and fakes): counts only, not who is
  which. The projector also says of a winner the organizer added that they should come to the front, since no
  phone tells them.

The verification page and the projector (`/s/<slug>`, with its recent names) ask search engines not to index them,
nor the JSON behind them (`X-Robots-Tag: noindex`): they are public to whoever has the link, not to a search on
someone's name. That is a request crawlers honour, not access control.

## Erasure

Every raffle has an **event date** (at most a year ahead, so the bounded period counts from a bounded day), fixed
once a real entrant has joined, and a **retention period** fixed with it. Together they give the raffle one
**erasure date**, which the database computes from the two, which the join form and the privacy notice show
entrants as a calendar day ("at the end of 4 November 2026"), and which no later action of the organizer moves.
Registration can't open once that date has passed, and a raffle still open then stops taking joins (the form would
promise a day gone by) and expires within the hour. The hourly housekeeping run keeps three clocks:

| Clock | When | What |
|---|---|---|
| **Non-winners** | the erasure date: the end of the event day plus the period (7 days unless the organizer chose otherwise, 0 to the installation's maximum, 30) | Everyone who didn't win, and winners whose prize was marked delivered, lose their name, email, registration ID, claim code and token. In any state: a raffle nobody finished is erased all the same. |
| **Winners** | the erasure date plus the **winner hold** (90 days) | Winners (the entrants of claimed draws) are kept so a prize can still reach them, until it is marked delivered ("Mark delivered", or "All prizes delivered" from the menu, or claiming a prize the organizer delivers on the spot), which puts them on the first clock. A prize's delivery is fixed per draw when it is claimed (`draws.delivery`): switching a prize to "on the spot" afterwards changes the claims still to come, so a winner who claimed it as "sent later" stays owed it, and a later claim of theirs doesn't move them onto the first clock. At the hold, everyone left is erased, a finished raffle becomes "Data erased", and a raffle still being drawn is asked to finish. |
| **Expiry** | the erasure date | A raffle that was never frozen has no draw to keep verifiable: its entrants are deleted outright (and who was removed at their request) and it becomes "Data erased". |
| **Took themselves off** | the finish | Someone the organizer added who took themselves off after the freeze (below) is erased once the raffle is over, within the hour, not on the erasure date: what they were told is that their data goes when the raffle does. One who is also a claimed winner (the claim and "Took themselves off" pressed within a second of each other: both land) is left to the winners' clock, as a prize may still be owed to them. |

The organizer can always erase earlier, never later:

- **"Erase personal data now"** (the raffle's menu, once it is finished) erases everyone at once and ends live updates.
- **Erasing one entrant** (People tab) works once the raffle is finished, or once its erasure date has passed: for
  example on request.
- **Before the freeze**, removing an entrant deletes them outright; so does deleting the raffle, which is
  possible until its first draw is announced (from then on the record stays, and the raffle is purged instead).
  Removed **at their request** (anyone real, someone who joined too), they stay out of the organizer's later adds:
  their email key and registration ID are kept for that alone, in `add_removals`, in the same write as the
  delete, and a later add skips a row with either unless the organizer lifts it for that row ("add anyway", which
  deletes it). They are deleted when the list is frozen for good (the manifest is written: no add can come), with
  the raffle's expiry, purge or deletion, and with its owner leaving; housekeeping deletes any left for a raffle
  that is frozen, purged or past its erasure date (after a restore, say). They hold the email key in clear, as
  `entrants` does: a hash without a secret would hide nothing.
- **From the freeze until the finish or the date**, nobody who **joined** leaves the list: sharing their entry
  with the event is part of the rules they accepted. The organizer can hide or rename a name meanwhile. Someone the
  organizer **added** accepted no rules and may object (GDPR Art. 21): **Took themselves off** hides their name
  for good and records it (`entrants.objected_at`) in one write; a draw that picks them can only be declined (the
  server refuses "Claimed" and "Not here"); their data is erased once the raffle is over (the clock above). Their
  anonymous ID stays in the manifest, which removing it would break. It is refused for a winner who already
  claimed a prize: the draw's result stands, and their data can be erased on request once the raffle is over.
  The organizer can undo it, a mistake, until the finish; the name stays hidden until they show it.

Entrant IDs and the event log stay, so the draw remains verifiable. Every erasure is recorded (entrant ID and time
only) and every automatic one audited per raffle, so they can be re-applied after a database restore
(`rafi reapply-erasures`, see [Self-hosting](../operations/self-hosting.md#backups)), which then runs housekeeping:
whoever took themselves off a raffle that is over is erased again, and removals at their request that no add can
need are deleted again. What remains of an erased
entrant besides the ID: when they entered, for someone who joined on their phone when they accepted the rules and
which version of the join form and the standard rules they were shown (`consent_version`, as `<form
version>/age<minimum age>`, e.g. `2026-11-01/age16`; someone the organizer added saw no form and has neither),
whether their prize was
delivered, how they got in, whether their name was hidden, and whether they took themselves off. A winner's exported details are the organizer's
copy, outside Rafi.

If no housekeeping run has succeeded for two hours (counting from the first raffle: a fresh installation has
nothing to erase yet), every organizer's admin says so.

## The audit log

Every action on a raffle is a row: who (the owner key, and the member who acted), when, what, and a detail that
never holds personal data (a count, an ID, the names of the settings that changed). The organizer's: `create`,
`settings` (which fields), `delete`, the lifecycle moves, `freeze`, `draw`, `resolve`, `void`, `abandon`,
`finish`, `add` (one person or a list: `batch=<ID> origin=<one|list> added=<n> skipped=<m> approved=<k>
lifted=<l>`, and `format=<full|first_initial>` when the browser made a list's names: how many were added and
skipped, how many of them only because the organizer approved a name the blocklist refuses, and how many though
they were removed at their request, never who), `undo_add` (the batch ID and how many were removed), `add_fakes`, `remove_fakes`, `patch_entrant`,
`deliver` and `undeliver` (by hand, or after
a `resolve` that changed it: a claim of a prize delivered on the spot, or of one sent later to a winner already
delivered), `deliver_all`, `set_prize_delivery`, `remove_entrant` and `erase_entrant` (the entrant's ID, and
`at_request` after it when they asked), `object` and `unobject` (the entrant's ID: "Took themselves off" and its
undo), `purge`, `end_live`, `export_csv`. Housekeeping's: `expire`, `auto_erase` (the count, as `3 objected` for
the people who took themselves off, erased at the finish), `auto_purge`, `auto_finish`. Leaving: `delete` and `purge` with "owner left". The log is the operator's record
and stays after the organizer leaves.

On Cloudflare D1, Time Travel keeps erased data for up to 30 more days. The installation's notice should say so (the
template does).

## People the organizer adds

The organizer can add people instead of them joining with the QR code: one by one, or from a list such as the
event's registration or guest list (parsed in the organizer's browser; only the name, the email and the
registration ID reach the service). They accepted nothing and saw no form, so nothing of a consent is recorded for
them, and Rafi doesn't reach them: the organizer tells them, before the event.

- **What the organizer answers for.** Adding people is the organizer's instruction to the operator, and the terms
  (the processing agreement) have them warrant a lawful basis for sharing the list, that its people are told
  about the draw before it, with the rules and the notice, and can object, that it reveals no special-category
  data, and that its people meet the age limit (the rules say that too). Nothing records it per raffle: the list
  tab says it in one line, and the freeze reminds them. Being on a list can say more than a name: don't add the
  attendees of a support group or of a religious or political meeting, nor a youth club's members; let them join
  with the QR code.
- **The paragraph to send.** The raffle's settings show a paragraph to copy (and edit) onto the event page or into
  a confirmation email: that they are in the organizer's prize draw, that their name is shown on the event screen
  and the public results page and how (full name, or first name and initial), the rules and the privacy notice
  (absolute links, which work once the raffle is published, with the organizer's own notice if it is `https:`), and
  whom to tell before the draw if they'd rather not take part (the privacy contact, or else the contact email).
- **The notice** has a section **"If the organizer added you"**, in every raffle: what is held (the name, and the
  email if the organizer gave one, nothing else), where it came from (the person, at the event, or the organizer's
  records about the event, such as its registration or guest list), what for (this draw), and the basis, the
  organizer's legitimate interest in rewarding the event's attendees with a prize draw (GDPR Art. 6(1)(f)); then,
  in a paragraph of its own, the right to object: before the freeze they are removed and stay out if the organizer
  adds the list again; after it their name is hidden and their data erased when the raffle is over, the anonymous
  ID staying in the draw so it can be verified. Their name is public like everyone's, and erased on the same dates.
  The paragraphs about the email being required and the entry cookie say they are about joining with the QR code.
- **Objections**, as the notice describes them (the mechanics are under [Erasure](#erasure)): before the freeze,
  **remove** them with **They asked to be taken off**, and the list tab's preview shows them as **Removed at their
  request**, left out unless added anyway, as does the one-person form for their email; from the freeze until the
  finish, **Took themselves off**: hidden, declined if drawn, erased at the finish.
- **Names are unique.** Nobody the organizer adds is shown with a name someone in the raffle already has (compared
  composed, lowercase, with whitespace collapsed), whether that someone joined or was added, nor with another row's:
  the add is refused whole, naming the rows and whom they clash with. Two "Anna K."s on the screen couldn't tell
  which of them won. The one-person form takes a surname or a number then; a list's names are made unique in the
  browser, a number for real namesakes ("Anna Kovács (2)"), and for first names and initials more letters of the
  family name first ("Anna Ko.", "Anna Ki."), at most three. Joins and renames aren't held to it: a joiner's name is
  their own choice, and the organizer renames on purpose.
- **Approved names.** The blocklist matches parts of words, so it refuses real names ("Nazir", "Buzinkay",
  "Shitara"). The organizer approves such a name row by row (or in the one-person form); an approval lifts the
  blocklist and nothing else (the length, the invisible characters and the letter-or-digit rule hold), and the
  `add` audit row counts the approvals.
- **The list stays in the browser.** A pasted or uploaded list is read in the organizer's page and held in its
  memory only: nothing of it is stored, logged or sent but the name, the email and the registration ID of each row
  added, in the columns the organizer mapped. The other columns (phone numbers, companies, answers to a
  registration form) never reach the service. The preview flags an email someone already joined with by phone
  under another name ("joined by phone as …"): someone may have used a registrant's email first.

## Who is responsible

- **The organizer** is the controller of their entrants' data. Registration can't open until the raffle names them
  and a way to reach them (a contact email, or a separate privacy contact that can change at any time). They may add
  their own privacy text or link to their own notice; it is shown after Rafi's generated notice, never instead of it.
  In a Clerk organization the owner is the organization and every member runs its raffles; **admins** remove the
  organization's data, **managers** (the custom `org:manager` role, or the
  `org:raffles:manage` permission) also create, configure and delete raffles, **members** do everything else.
- **The operator** of the installation is the organizer's processor, named in every notice with a link to the
  installation's own, which lists its subprocessors (hosting, the raffle engine, organizer sign-in, drand).
  Organizers accept the operator's terms, with the data processing agreement, by using the service; the admin
  links them, and each version is shown with its text. The agreement covers the people organizers add as well as
  those who join ([above](#people-the-organizer-adds)). Nothing records who accepted which version, so a change to
  the processing agreement is for the operator to notify organizers of directly.
- **Leaving:** "Remove my data" in the admin deletes the organizer's raffles that were never frozen, erases their
  other raffles' personal data and takes their name and contacts off them, and deletes their defaults, all in one
  write that does nothing while a raffle of theirs is being drawn. The anonymous draws stay verifiable. What stays
  besides: the account's identifiers on its raffles and in the audit log (the owner, and the member who acted) as
  the operator's record of what was done, for the period the operator's notice names; and the raffles' titles,
  descriptions, rules and prize sponsors, which the organizer wrote and may want to blank before leaving if they
  name anyone. The account itself is deleted in Clerk.

## Coming back without an account

An entry has three parts:

- a public **entrant ID**
- a private **claim code**
- a secret token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` over HTTPS) that lasts 3 days, sent only to the
  join page (`Path=/j/`): never to the APIs or the live channel. Only the token's SHA-256 is stored.

| Situation | What happens |
|---|---|
| The phone locks, the tab is closed, the QR code is scanned again | The cookie restores the entry, claim code included |
| Another browser, private mode, cleared cookies | The same email finds the entry (one the organizer added too), and the page says, in the same words for either, that it is safe and that a winner goes to the organizer. It is **not** moved to the new device: otherwise anyone could take over an entry by typing someone else's email. This does tell whoever types the email that it is entered. That can't be hidden by wording: a join is visible in the public entrant count and the recent names anyway, so anyone with the join code who solves a proof-of-work can learn it, five tries a minute per email, while registration is open. The page says so plainly rather than pretending otherwise. |
| No phone | The organizer adds the person in the admin and confirms them by name at the stage |

The projector is the source of truth. A faked "you won" screen lacks the winner's private claim code.

## Abuse

| Defense | |
|---|---|
| **One entry per email** | Per raffle, on a normalized key: lowercased, `+tag` dropped, and for Gmail, dots dropped. No device fingerprinting. |
| **Proof-of-work** | Joining requires solving an [ALTCHA v2](https://altcha.org) challenge (PBKDF2/SHA-256) in the browser. Challenges are HMAC-signed and each is redeemed once. The difficulty is `RAFI_POW_COST`. |
| **Rate limits** | 600 joins per minute per raffle, and 5 per minute per raffle and email; 20 adds or undos per minute per organizer. **Never per IP**, because a venue shares one. On Workers they are soft (per location). |
| **Entrant cap** | `RAFI_MAX_ENTRANTS` (2000), enforced in the database (the hard limit) for joins and for everything the organizer adds, people and fakes: an add that doesn't fit is refused whole, in the same transaction as its rows. At most 1000 items (prize copies) in one raffle |
| **Registration window** | The join code lets people in only while registration is open. The code is six characters out of 31 (887 million), and there is no per-IP limit, so codes can be guessed at a cost; a hit shows the raffle's public pages. Once the raffle is purged, its code answers nothing, so guessing finds only raffles inside their hold period. |
| **Presence** | The rules require winners to be present to claim. Remote entries forfeit ("Not here"), and the prize is drawn again. |
| **Display names** | Nothing invisible (bidirectional controls, zero-width characters, soft hyphens, blank fillers) and at least one letter or digit, so no name can look like another or like nothing; composed (Unicode NFC), so two spellings of one name are one; a blocklist (which the organizer can lift for a real name they add, and only it); at most 24 characters when joining, so nobody fills the projector (64 for the organizer's own); the names the organizer adds unique in the raffle; rendered isolated from the text around it (`<bdi>`), so a right-to-left name can't reorder it; a 5 s delay before a name reaches the projector; one-tap hide or rename |
| **Before the freeze** | The organizer can remove duplicates and trolls |

Edge protection by IP (DDoS and similar) is the platform's job: Cloudflare, or your reverse proxy.

## Cookies and tracking

There is one strictly necessary cookie: the entry. There are no analytics and no tracking, so no cookie banner is
needed. On Workers, the automatic per-invocation logs are off because they would record client IPs and cookies;
what is logged is the application's own warnings and errors, which carry raffle IDs and error text, no personal
data (and `wrangler tail`, when an operator runs it, shows request details live).

## Rules and legal notes

The standard rules (`/rules/<slug>`) say:

- the raffle is for the people attending the event in person; its entrants are the people who join with the QR code
  while registration is open and the people the organizer adds (for example, from the event's guest list), who can
  ask the organizer to take them off (both ways in, in every raffle's rules)
- entry is free and needs no purchase or donation
- there is one entry per person, and one prize per person (unless there are more prizes than people)
- entrants must be at least the installation's minimum age (16), which they confirm by joining; the organizer is
  responsible for the people it adds meeting the age limit
- winners must be present
- prizes can't be exchanged for cash
- how the draw works

The organizer appends their own **additional rules**, which is where a higher age limit goes. The additional rules,
the event date, the retention period and the organizer's privacy text and link lock once a real entrant is in under
them: someone who joined accepted them, and someone the organizer added was entered under them (no acceptance is
recorded for them: they saw no form). Fake entrants lock nothing. The standard rules are part of the join form's
version (`consent_version`): a change to them is a new version.

Free entry matters: in many jurisdictions, selling entries or tying them to donations turns a prize draw into a
regulated lottery. Prize tax treatment also varies. Check both for where you run your event.
