# Privacy and abuse

What Rafi stores, what it publishes, when it erases it, and how it keeps a raffle free of spam and remote entries.
The privacy notice each raffle shows its entrants is rendered from the raffle's settings at `/privacy/<slug>`; the
installation's own notice is at `/privacy`, and the operator's terms, with the data processing agreement, at
`/terms`.

> This page describes the software, not your legal obligations. As the organizer, you are the data controller; the
> operator of the installation is your processor. The plan behind this design is
> [docs/plans/privacy-and-retention.md](../plans/privacy-and-retention.md).

## Data minimization

| Data | Who sees it |
|---|---|
| **Display name** | The projector (after a 5 s delay, unless the organizer hides it), the verification page's list of entrants (until erasure), the admin. It is optional: an entrant who leaves it blank gets a generated one ("Brave Otter"), so nobody has to put their real name on a screen. The organizer can hide or rename it in one tap. Hidden names show as ★ ★ ★. |
| **Email** | The admin only. It is never on the projector, in either public view, on the verification page, or in the raffle engine. |
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

## Erasure

Every raffle has an **event date** (at most a year ahead, so the bounded period counts from a bounded day), fixed
once a real entrant has joined, and a **retention period** fixed with it. Together they give the raffle one
**erasure date**, which the database computes from the two, which the join form and the privacy notice show
entrants as a calendar day ("at the end of 4 November 2026"), and which no later action of the organizer moves.
Registration can't open once that date has passed. The hourly housekeeping run then keeps three clocks:

| Clock | When | What |
|---|---|---|
| **Non-winners** | the erasure date: the end of the event day plus the period (7 days unless the organizer chose otherwise, 0 to the installation's maximum, 30) | Everyone who didn't win, and winners whose prize was marked delivered, lose their name, email, claim code and token. In any state: a raffle nobody finished is erased all the same. |
| **Winners** | the erasure date plus the **winner hold** (90 days) | Winners (the entrants of claimed draws) are kept so a prize can still reach them, until the organizer marks it delivered ("Mark delivered", or "All prizes delivered" from the menu), which puts them on the first clock. At the hold, everyone left is erased, a finished raffle becomes "Data erased", and a raffle still being drawn is asked to finish. |
| **Expiry** | the erasure date | A raffle that was never frozen has no draw to keep verifiable: its entrants are deleted outright and it becomes "Data erased". |

The organizer can always erase earlier, never later:

- **"Erase personal data now"** (the raffle's menu, once it is finished) erases everyone at once and ends live updates.
- **Erasing one entrant** (People tab) works once the raffle is finished, or once its erasure date has passed: for
  example on request.
- **Before the freeze**, removing an entrant deletes them outright; so does deleting the raffle, which is
  possible until its first draw is announced (from then on the record stays, and the raffle is purged instead).
- **From the freeze until the finish or the date**, nobody leaves the list: sharing their entry with the event is
  part of the rules entrants accept. The organizer can hide or rename a name meanwhile.

Entrant IDs and the event log stay, so the draw remains verifiable. Every erasure is recorded (entrant ID and time
only) and every automatic one audited per raffle, so they can be re-applied after a database restore
(`rafi reapply-erasures`, see [Self-hosting](../operations/self-hosting.md#backups)). What remains of an erased
entrant besides the ID: when they joined and accepted the rules, which version of the join form they were shown
(`consent_version`, as `<form version>/age<minimum age>`, e.g. `2026-10-21.2/age16`), whether their prize was
delivered, how they got in, and whether their name was hidden. A winner's exported details are the organizer's
copy, outside Rafi.

If no housekeeping run has succeeded for two hours (counting from the first raffle: a fresh installation has
nothing to erase yet), every organizer's admin says so.

On Cloudflare D1, Time Travel keeps erased data for up to 30 more days. The installation's notice should say so (the
template does).

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
  links them, and each version is shown with its text. Nothing records who accepted which version, so a change to
  the processing agreement is for the operator to notify organizers of directly.
- **Leaving:** "Remove my data" in the admin deletes the organizer's raffles that were never frozen, erases their
  other raffles' personal data and takes their name and contacts off them, and deletes their defaults, all in one
  write that does nothing while a raffle of theirs is being drawn. The anonymous draws stay verifiable. The
  account's identifiers stay on its raffles and in the audit log (the owner, and the member who acted) as the
  operator's record of what was done, for the period the operator's notice names. The account itself is deleted
  in Clerk.

## Coming back without an account

An entry has three parts:

- a public **entrant ID**
- a private **claim code**
- a secret token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` over HTTPS) that lasts 3 days, sent only to the
  join page (`Path=/j/`): never to the APIs or the live channel. Only the token's SHA-256 is stored.

| Situation | What happens |
|---|---|
| The phone locks, the tab is closed, the QR code is scanned again | The cookie restores the entry, claim code included |
| Another browser, private mode, cleared cookies | The same email finds the entry, and the page says it is safe. It is **not** moved to the new device: otherwise anyone could take over an entry by typing someone else's email. This does tell whoever types the email that it is entered. That can't be hidden by wording: a join is visible in the public entrant count and the recent names anyway, so anyone with the join code who solves a proof-of-work can learn it, five tries a minute per email, while registration is open. The page says so plainly rather than pretending otherwise. |
| No phone | The organizer adds the person in the admin and confirms them by name at the stage |

The projector is the source of truth. A faked "you won" screen lacks the winner's private claim code.

## Abuse

| Defense | |
|---|---|
| **One entry per email** | Per raffle, on a normalized key: lowercased, `+tag` dropped, and for Gmail, dots dropped. No device fingerprinting. |
| **Proof-of-work** | Joining requires solving an [ALTCHA v2](https://altcha.org) challenge (PBKDF2/SHA-256) in the browser. Challenges are HMAC-signed and each is redeemed once. The difficulty is `RAFI_ALTCHA_COST`. |
| **Rate limits** | 600 joins per minute per raffle, and 5 per minute per raffle and email. **Never per IP**, because a venue shares one. On Workers they are soft (per location). |
| **Entrant cap** | `RAFI_MAX_ENTRANTS` (2000), enforced in the database for joins (the hard limit) and checked for the organizer's additions; at most 1000 items (prize copies) in one raffle |
| **Registration window** | The join code lets people in only while registration is open. The code is six characters out of 31 (887 million), and there is no per-IP limit, so codes can be guessed at a cost; a hit shows the raffle's public pages. Once the raffle is purged, its code answers nothing, so guessing finds only raffles inside their hold period. |
| **Presence** | The rules require winners to be present to claim. Remote entries forfeit ("Not here"), and the prize is drawn again. |
| **Display names** | Nothing invisible (bidirectional controls, zero-width characters, soft hyphens, blank fillers) and at least one letter or digit, so no name can look like another or like nothing; a blocklist; a 5 s delay before a name reaches the projector; one-tap hide or rename |
| **Before the freeze** | The organizer can remove duplicates and trolls |

Edge protection by IP (DDoS and similar) is the platform's job: Cloudflare, or your reverse proxy.

## Cookies and tracking

There is one strictly necessary cookie: the entry. There are no analytics and no tracking, so no cookie banner is
needed. On Workers, the automatic per-invocation logs are off because they would record client IPs and cookies;
what is logged is the application's own warnings and errors, which carry raffle IDs and error text, no personal
data (and `wrangler tail`, when an operator runs it, shows request details live).

## Rules and legal notes

The standard rules (`/rules/<slug>`) say:

- entry is free and needs no purchase or donation
- there is one entry per person, and one prize per person (unless there are more prizes than people)
- entrants must be at least the installation's minimum age (16), which they confirm by entering
- winners must be present
- prizes can't be exchanged for cash
- how the draw works

The organizer appends their own **additional rules**, which is where a higher age limit goes. The additional rules,
the event date, the retention period and the organizer's privacy text and link lock once a real entrant has accepted
them: they are what the entrant was shown.

Free entry matters: in many jurisdictions, selling entries or tying them to donations turns a prize draw into a
regulated lottery. Prize tax treatment also varies. Check both for where you run your event.
