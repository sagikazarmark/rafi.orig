# Marketing opt-in

**Status: proposed, deferred (2026-10-21).** Builds on [privacy-and-retention.md](privacy-and-retention.md), which
removes the half-built opt-in Rafi has today. This plan brings it back whole: the consent, the export it was always
missing, a retention clock of its own, and withdrawal. Nothing here is started until that spec has shipped.

> Not legal advice. §Further notes lists what a lawyer must check before the feature goes live on the hosted
> instance.

## Problem statement

Meetup organizers run raffles partly to grow their meetup. They would like to ask, at the moment someone joins,
whether that person wants to hear about the next one. Rafi used to have the checkbox, stored the answer, and then
gave the organizer no way to get the list out: the only export was of drawn entrants. An opted-in non-winner was
collected and never usable, which is the worst of both worlds: the data existed and the purpose it was collected
for was never served.

Done properly, the opt-in has needs the raffle's own data doesn't: it is consent, not contract, so it must be
optional, separate, unticked, withdrawable and provable; it has a recipient (the organizer, never the operator);
it has a purpose that outlives the draw, so it can't die on the non-winner clock without giving the organizer time
to export; and it must not become the back door through which emails are kept indefinitely.

## Solution

An organizer switches the opt-in on for a raffle. The join form then shows a second, unticked checkbox that names
the organizer as the recipient and future meetups as the purpose. Consent is recorded with its time and the version
of the text. The organizer gets a **marketing export**: the consenting emails with their consent evidence, logged
like every export, with an **export and clear** option that drops the list from Rafi on the spot. If they don't
clear it, the list lives until the **marketing deadline**, a fixed number of days after the raffle's erasure date,
and is then erased by housekeeping. When an entrant's other personal data is erased before that, their email alone
is held for the list, visibly, until the deadline or the clear.

Withdrawal is one tap for the organizer, and self-service for the entrant from their own phone while their entry
cookie lasts. After the export, the list lives in the organizer's own system and withdrawal is against that; the
notice says so.

## User stories

### Entrants

1. As an entrant, I want the marketing box to be separate, optional and unticked, so that entering the raffle
   never signs me up for anything.
2. As an entrant, I want the box to say who will email me and about what, so that my consent is informed.
3. As an entrant, I want to be sure the service operator never uses my address for its own purposes, so that the
   consent goes to one party only.
4. As an entrant, I want to know how long Rafi keeps my address for this and where it goes afterwards, so that I
   know whom to ask to stop.
5. As an entrant, I want to withdraw from my own phone right after joining if I change my mind, so that I don't
   have to write an email to undo a tap.
6. As an entrant, I want withdrawal through the organizer to work at any time, so that the right is real.
7. As an entrant who was erased from the raffle, I want my name off every screen and page even if my email is
   still held for the list, so that the hold doesn't keep me public.

### Organizers

8. As an organizer, I want to switch the opt-in on per raffle, and to have my default for new raffles, so that I
   decide whether to ask.
9. As an organizer, I want to export the consenting emails with their consent evidence, so that I can import them
   into my own tool and show, if asked, when and to what each person consented.
10. As an organizer, I want to export and clear in one go, so that Rafi holds nothing I've already taken.
11. As an organizer, I want to export more than once before the deadline, so that a failed import doesn't lose the
    list.
12. As an organizer, I want to see how many consents a raffle has and the date they're erased, so that I don't
    miss the window.
13. As an organizer, I want to withdraw one entrant's consent in one tap, so that a request is handled in seconds.
14. As an organizer, I want to clear the whole list without exporting, so that I can drop it if plans change.
15. As an organizer, I want "Erase personal data now" to tell me what happens to the list, so that I'm not
    surprised either way.
16. As an organizer, I want the opt-in to be impossible to switch on until I've named a privacy contact, so that
    nobody consents to an unnamed recipient.

### Operators

17. As the operator, I want the marketing period to be an installation ceiling, so that no organizer can keep a
    list in Rafi past it.
18. As the operator, I want a switch to turn the feature off for the whole installation, so that a self-hoster who
    doesn't want it can refuse it.
19. As the operator, I want every export and every clear in the audit log, so that the data's movements are
    accounted for.
20. As the operator, I want housekeeping to erase held emails at the deadline the same way it erases everything
    else, so that there is one mechanism.
21. As the operator, I want held emails to be gone after a restore and the erasure tool, so that the hold never
    survives a backup it shouldn't.

## Implementation decisions

### Vocabulary

- **Marketing opt-in**: the organizer's option to ask, and the entrant's consent when given.
- **Marketing deadline**: the raffle's erasure date + the marketing period. When held emails are erased.
- **Held email**: an erased entrant's address kept only for the marketing list, until the deadline or a clear.
- **Clear the list**: drop every consent of a raffle from Rafi, and every held email with it.

### Consent

- `entrants.marketing_consent_at INTEGER` (nullable) replaces the old boolean: `NULL` is no consent; a value is
  consent and when. The text's version is the entry's `consent_version`; the join form's texts are versioned
  together.
- The join form shows the box only when the raffle has the opt-in on. The label names the organizer: "Yes,
  *Budapest Rust Meetup* may email me about future meetups. Optional." Unticked. Manual and fake entrants never
  consent.
- The participant page, while the entry cookie is valid, shows the consent and a one-tap withdrawal. The cookie is
  the entrant's proof of identity, as it is for seeing the claim code.
- Withdrawal by the organizer: a one-tap action on the entrant in the People tab, with an audit row. Withdrawal
  sets `marketing_consent_at` to `NULL`; if the email was held, it is erased in the same write.

### Turning it on

- `raffles.marketing_opt_in INTEGER` and the same in `owner_defaults`, default off, copied at creation like the
  other defaults. Locked with the rules: the join form's text can't change once someone accepted it.
- Switching it on requires `privacy_email` or `contact_email` to be set, as `Open` does: consent needs a named
  recipient.
- Installation switch `RAFI_MARKETING` (default on). Off, the setting is hidden, the box never shows, and existing
  consents are left to their deadline.

### The marketing clock

- `RAFI_RETENTION_MARKETING_DAYS` (default 30, ceiling 90, validated at startup). Not configurable per organizer
  or raffle: the organizer's way to shorten it is to export and clear, or to clear.
- The marketing deadline is `erase_at + marketing period`, derived; nothing new is stamped.
- The privacy notice states it with the concrete date, and that the list then lives in the organizer's own system.

### Two-step erasure

When an entrant who consented is erased, by any path (housekeeping's non-winner or ceiling duty, "Erase personal
data now", erase one on request), and the marketing deadline hasn't passed:

1. The `ERASED` write runs as it does for everyone, **except** that `email_raw` is kept; `email_norm`, the name, the
   claim code and the token go. The entrant is `erased`, off the verification page and the projector, and marked
   held (`email_held_until` = the marketing deadline).
2. At the deadline, housekeeping erases held emails (`email_raw = NULL`, `email_held_until = NULL`), set-based, one
   audit row per raffle with the count.

If the deadline has already passed when step 1 runs, the email goes with the rest. Erasing one entrant on request
is the one path that **does not hold**: a request to erase is a withdrawal too.

"Erase personal data now" says, when a list exists: "*N* marketing consents are kept for export until *date*", with
a checkbox to clear them as well.

The `erasures` row is written at step 1. The restore tool re-applies `ERASED` in full, so a restore drops held
emails: the conservative outcome, and the docs say so.

### The export

- A new use case, **marketing export**: CSV with `email,consented_at,consent_version,raffle,event_date`, for every
  entrant of the raffle with a consent whose email is present (held or not). Generated on request, never stored.
  Available from the freeze on (before it, the list is still growing).
- An audit row per export: action `export_marketing`, detail the row count.
- **Export and clear**: a checkbox on the export dialog. The admin downloads, then calls **clear the list**; the
  clear is its own endpoint and its own audit row, so a failed download clears nothing.
- **Clear the list** on its own, from the raffle's menu: every `marketing_consent_at` of the raffle to `NULL`,
  every held email erased, audit row `clear_marketing` with the count. Idempotent.

### The admin

- Raffle settings: the opt-in switch, with the recipient line it will show and the requirement it has.
- People tab: a consent marker per entrant and the withdrawal tap; held entrants show as erased with "email held
  for the list until *date*".
- The raffle's summary: "*N* consents · export by *date*", the export button, clear.

### The notices

- The per-raffle privacy notice regains the consent paragraph: purpose, recipient (the organizer, named), lawful
  basis (Art. 6(1)(a)), that the operator never uses the address, the marketing deadline as a date, that the
  export moves the list into the organizer's system, and how to withdraw before and after.
- The join form's text under the box repeats the recipient and the withdrawal route in one line.
- The operator's Terms cover the marketing list as processed on the organizer's instructions, with the organizer
  responsible for honouring withdrawals in their own system.

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `RAFI_MARKETING` | on | Whether organizers may switch the opt-in on |
| `RAFI_RETENTION_MARKETING_DAYS` | 30 | Days after the erasure date before held emails are erased; ceiling 90 |

### Schema

`entrants.marketing_consent_at`, `entrants.email_held_until`; `raffles.marketing_opt_in`;
`owner_defaults.marketing_opt_in`. By then databases may exist, so this is a numbered migration, not an edit of
`0001_init.sql`.

### Docs

`privacy-and-abuse.md` (a Marketing section: what is collected, who sees it, the clock, withdrawal),
`running-a-raffle.md` (switching it on, exporting, clearing), `configuration.md`, `glossary.md`, `routes.md`.

## Testing decisions

The same seams as the retention spec: the HTTP server tests carry the behaviour, store conformance covers the
set-based writes on all three backends, lifecycle unit tests cover the pure rules. A good test observes what the
entrant, organizer or operator could: the join page, the participant page, the CSV, `verify.json`, the audit and
`erasures` tables.

HTTP-level scenarios:

- the box is absent with the opt-in off, present and unticked with it on, and names the organizer;
- switching it on is refused without a contact, and locked once someone joined;
- a consent is recorded with its time and version; a manual entrant has none;
- the entrant withdraws from the participant page while the cookie lasts; afterwards the page offers nothing;
- the organizer withdraws one; a held email is erased with it;
- the marketing export lists exactly the consenting emails with their evidence, from the freeze on, and is audited
  each time; a second export still works;
- export and clear: the clear is separate, audited, idempotent, and empties the next export;
- on the erasure date a consenting non-winner is erased but their email is held, they are off `verify.json` and the
  projector, and the export still has them; a non-consenting one is gone entirely;
- at the marketing deadline the held email is erased and audited; a run after that changes nothing;
- erase on request never holds;
- "Erase personal data now" holds by default and clears when asked;
- after a restore and the erasure tool, held emails are gone;
- the privacy notice shows the paragraph and the date only when the opt-in is on;
- with `RAFI_MARKETING` off, the switch is refused and existing consents still expire.

Conformance: the hold-aware erasure batch, the deadline batch, the clear, on SQLite, Postgres and D1.

## Out of scope

- Sending any email from Rafi, including expiry warnings to organizers before the deadline. When a mail subsystem
  exists, a warning at 7, 3 and 1 days is the natural addition; until then the admin's "export by" line is it.
- Double opt-in. The organizer's own tool does that if they want it.
- Unsubscribe handling after export: the organizer's system's job, and the notice says so.
- Per-organizer or per-raffle marketing periods.
- Any use of the list by the operator, ever.

## Further notes

- **Why a third clock and not the non-winner one.** The non-winner period answers "how long after the draw might
  anyone need to know who entered" (almost never); the marketing period answers "how long does a volunteer have to
  sit down and export" (a few weekends). Different question, different basis, different number; 30 days is the
  honest answer, and the ceiling keeps it from becoming a workaround.
- **Why hold the email rather than the whole entrant.** The name is public on the verification page until erasure;
  the hold must not keep anyone public longer. The consent covers an address and nothing else.
- **Why the entrant can withdraw from the participant page.** It is the one self-service right Rafi can offer
  without accounts, because the cookie already authenticates the entrant to their own entry. It costs a button.
- **Age.** The consent is the one processing where Art. 8 bites. The retention spec's "you confirm you are 16 or
  older" line on the join form is what makes the consent one an adult gave, by the entrant's own confirmation; 16
  is the highest threshold any member state uses, so no per-country logic is needed.
- **Legal review before launch** should cover: the consent text against Art. 7 (separate, specific, informed,
  withdrawable as easily as given); whether a held email on an otherwise erased row is adequately described in the
  notice; the organizer's duties after export in the Terms; and ePrivacy rules on the organizer's side for the
  emails they then send, which Rafi can only point to.
