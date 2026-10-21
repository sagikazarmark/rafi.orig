# Privacy and abuse

What Rafi stores, what it publishes, and how it keeps a raffle free of spam and remote entries. The privacy notice
each raffle shows its entrants is rendered from the raffle's settings at `/privacy/<slug>`.

> This page describes the software, not your legal obligations. As the organizer, you are the data controller.

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

There is no automatic erasure. The organizer erases personal data by hand once the raffle is finished:

- **"Erase personal data now"** (the raffle's menu, once it is finished) erases every entrant's name, email and claim
  code, and ends live updates. The raffle's state becomes "Data erased".
- **Erasing one entrant** (People tab, once it is finished) works on its own, for example on request.
- **Before the freeze**, removing an entrant deletes them outright.
- **From the freeze until the finish**, nobody leaves the list: sharing their entry with the event is part of the
  rules entrants accept. The organizer can hide or rename a name meanwhile, and a request to erase is carried out
  once the raffle is finished.

Entrant IDs and the event log stay, so the draw remains verifiable. Erasures are recorded (entrant ID and time
only), so they can be re-applied after a database restore.

On Cloudflare D1, Time Travel keeps erased data for up to 30 more days. Mention this in your privacy notice if you
deploy on Workers.

## Coming back without an account

An entry has three parts:

- a public **entrant ID**
- a private **claim code**
- a secret token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` over HTTPS) that lasts 3 days. Only the token's
  SHA-256 is stored.

| Situation | What happens |
|---|---|
| The phone locks, the tab is closed, the QR code is scanned again | The cookie restores the entry, claim code included |
| Another browser, private mode, cleared cookies | The same email finds the entry, and the page says it is safe. It is **not** moved to the new device: otherwise anyone could take over an entry by typing someone else's email. |
| No phone | The organizer adds the person in the admin and confirms them by name at the stage |

The projector is the source of truth. A faked "you won" screen lacks the winner's private claim code.

## Abuse

| Defense | |
|---|---|
| **One entry per email** | Per raffle, on a normalized key: lowercased, `+tag` dropped, and for Gmail, dots dropped. No device fingerprinting. |
| **Proof-of-work** | Joining requires solving an [ALTCHA v2](https://altcha.org) challenge (PBKDF2/SHA-256) in the browser. Challenges are HMAC-signed and each is redeemed once. The difficulty is `RAFI_ALTCHA_COST`. |
| **Rate limits** | 600 joins per minute per raffle, and 5 per minute per raffle and email. **Never per IP**, because a venue shares one. On Workers they are soft (per location). |
| **Entrant cap** | `RAFI_MAX_ENTRANTS` (2000), enforced in the database: the hard limit |
| **Registration window** | The join code works only while registration is open |
| **Presence** | The rules require winners to be present to claim. Remote entries forfeit ("Not here"), and the prize is drawn again. |
| **Display names** | A blocklist, a 5 s delay before a name reaches the projector, and one-tap hide or rename |
| **Before the freeze** | The organizer can remove duplicates and trolls |

Edge protection by IP (DDoS and similar) is the platform's job: Cloudflare, or your reverse proxy.

## Cookies and tracking

There is one strictly necessary cookie: the entry. There are no analytics and no tracking, so no cookie banner is
needed. On Workers, invocation logs are off because they would record client IPs and cookies.

## Rules and legal notes

The standard rules (`/rules/<slug>`) say:

- entry is free and needs no purchase or donation
- there is one entry per person, and one prize per person (unless there are more prizes than people)
- winners must be present
- prizes can't be exchanged for cash
- how the draw works

The organizer appends their own **additional rules**, which is where any age limit goes. Additional rules lock once
a real entrant has accepted them.

Free entry matters: in many jurisdictions, selling entries or tying them to donations turns a prize draw into a
regulated lottery. Prize tax treatment also varies. Check both for where you run your event.
