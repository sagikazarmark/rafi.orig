# Your legal documents

An installation can serve two documents of its operator's: **terms** (`/terms`), with the data processing agreement
organizers accept by using the service, and a **privacy notice** (`/privacy`). Rafi ships neither: you write your
own, with a lawyer, for how you run the service. Each is optional. This page is what the software does, for writing
them; it isn't legal advice.

## Where they go

| | Terms | Privacy notice |
|---|---|---|
| On Cloudflare Workers | `legal/terms.md` in your installation's repository | `legal/privacy.md` |
| Natively | `terms.md` in `RAFI_DOCS_DIR` | `privacy.md` in `RAFI_DOCS_DIR` |

Each is Markdown, starting with front matter that gives its version ([the
format](../reference/configuration.md#the-legal-documents-versions)):

```markdown
---
version: 2026-11
---
# Privacy notice
```

- A document without its version is refused: the app serves nothing until it is fixed, naming the file.
- A changed document is a new version: change its `version:` with its text. The page shows it, and every raffle's
  notice names the privacy notice's.
- Nothing records who read which version, so a change to the data processing agreement is for you to notify
  organizers of directly.
- On Workers, a push of `legal/` deploys them. Raw HTML in the Markdown is shown as text, and a link or image goes
  only to a web page, a mail address or a place on this site; any other is shown as its text.

## Without them

| Missing | What changes |
|---|---|
| Terms | `/terms` says the operator hasn't provided terms. The admin doesn't say organizers accept terms, and raffles' notices don't claim a data processing agreement. The home page doesn't link `/terms`. |
| Privacy notice | `/privacy` says the operator hasn't provided a privacy notice. Raffles' notices say so and link none, and say nothing of how long backups are kept. The home page doesn't link `/privacy`. |

Everything else works the same.

## What every raffle's notice already says

Each raffle has its own privacy notice, built in (`/privacy/<slug>`), whatever documents you provide. Write yours so
they agree with it. It says:

- **The controller:** the raffle's organizer, with the contact they give.
- **What is collected and why:** a joiner's display name and email, to run the draw under its rules (Art. 6(1)(b));
  the duplicate and abuse checks (one entry per email, rate limits, a proof-of-work in the browser) on the
  legitimate interest in a fair draw (Art. 6(1)(f)); for people the organizer adds, their name and, if given, their
  email, from the organizer's records about the event, on the organizer's legitimate interest (Art. 6(1)(f)), with
  their right to object. The draw makes no decision with legal effects.
- **What is public:** display names on the event screen and the verification page until erased; the draw uses
  anonymous IDs only; emails never.
- **Cookies:** one strictly necessary cookie for a joiner's entry; no analytics or tracking.
- **Retention:** the raffle's erasure dates (non-winners, and winners once their prize is delivered, at the latest
  on a later date), what stays after erasure (the anonymous entry and the draw log), and that winners' details an
  organizer exported are theirs to erase.
- **After erasure:** on Workers, that erased data can stay in D1's point-in-time recovery (Time Travel) for up to
  30 days, and in any backup you keep; with a privacy notice, that it says how long.
- **Who processes it:** you, by `RAFI_OPERATOR_NAME` and `RAFI_OPERATOR_EMAIL`, as the organizer's processor (under
  the data processing agreement in your terms, when there are terms); how the software works (the hosting, below,
  holds the raffle's data on your behalf; Restate, Clerk and drand receive none of an entrant's personal data); and
  your privacy notice, with its version, or that you haven't published one.
- **Rights:** access, correction, erasure, restriction, objection and portability, against the controller, and a
  complaint to a supervisory authority.
- Then the organizer's own privacy text and link, if any.

## What the software does with personal data

The facts your documents may need, by provider. Everything else, from what each field is for to when it is erased, is
in [Privacy and abuse](../explanation/privacy-and-abuse.md).

| Provider | What for | What it receives | Where |
|---|---|---|---|
| **Cloudflare, Inc.** (US) | Hosting on Workers: the app, its database (D1), the live channels (Durable Objects) | Everything the service stores. D1's point-in-time recovery keeps erased data for up to 30 days | The database and the live channels in the EU if you chose so before the first deploy ([Data residency](cloudflare.md#data-residency)), otherwise where Cloudflare places them; the Workers run near each visitor |
| **Restate** (Restate Cloud, or self-hosted) | The raffle engine: runs each draw against the randomness round it committed to | Anonymous entrant IDs and prize names. Its journal is kept for an hour, and it forgets a raffle when it finishes | Restate Cloud's region you created the environment in |
| **Clerk, Inc.** (US) | Organizer sign-in | Organizers' accounts (name, email, sign-in method, organization); nothing of entrants | Clerk's |
| **drand** (League of Entropy) | Public randomness | Nothing: the engine only downloads public beacons | |

And on your side:

- **Organizers' accounts:** you hold each raffle's owner (the Clerk user or organization) and the audit log of what
  was done (actions and times, no entrants' data). When an organizer removes their data ("Remove my data"), their
  account's identifiers stay on their raffles and in the audit log as your record. Rafi doesn't delete them by
  itself: decide how long you keep them, and run the runbook's [Organizers who
  left](runbook.md#organizers-who-left-their-account-ids) on that schedule.
- **Logs:** the application's own warnings and errors, with raffle IDs and no personal data. On Workers the
  per-request logs are off: they would hold IPs and cookies.
- **Cookies:** the entry's, on an entrant's phone, and the organizer's sign-in session (Clerk's).
- **Retention settings:** `RAFI_RETENTION_DAYS`, `RAFI_RETENTION_MAX_DAYS` and `RAFI_RETENTION_WINNERS_DAYS` set the
  erasure dates every raffle's notice shows ([configuration](../reference/configuration.md)).
- **The organizer's tools:** the admin shows them what is held and until when, lets them correct a name, remove
  someone at their request (and keep them out of a later add), erase one entrant or everyone, export winners, and
  remove their own data. Your terms can point at these as how you help organizers answer entrants' requests.
- **People organizers add:** a list is read in the organizer's browser; the service receives only each person's name,
  email and registration ID. What organizers should warrant about the people they add (a lawful basis, telling them
  before the draw, no special-category data, the age limit) is in [Privacy and
  abuse](../explanation/privacy-and-abuse.md#people-the-organizer-adds).

Your record of processing: [the operator's records](overview.md#the-operators-records).
