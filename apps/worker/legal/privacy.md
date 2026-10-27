# Privacy notice

> **Template.** Delete this note once a lawyer has reviewed the document: the server refuses to start while a
> document still carries it. This is the engineering draft the Rafi repository ships with. Before this installation
> goes live, its operator reviews it with a lawyer, fills in the bracketed parts, and sets
> `RAFI_PRIVACY_VERSION`. Every raffle's own privacy notice links here for the parts that are the
> service's rather than the organizer's.

**[Operator legal name, address]** ("we") runs this raffle service at **[public URL]**. Privacy questions:
**[operator email]**.

## Two roles

- **For the entrants of a raffle**, the organizer who runs it is the data controller, and we are their
  processor: we store and process entries on their instructions, under a data processing agreement that is
  part of our terms (`/terms`). The raffle's own privacy notice (`/privacy/<raffle>`) names the organizer,
  their contact, and the dates on which personal data is erased.
- **For organizers' accounts and the operation of the service**, we are the controller. That is what the rest
  of this notice is about, along with the providers we rely on for both roles.

## Organizers' accounts

Organizers sign in through **Clerk, Inc.** (USA; EU–US Data Privacy Framework), which holds their account
details (name, email, sign-in method, organization). We keep the account's identifier with each raffle and the
audit log of what was done to a raffle (actions and times, no entrants' personal data). Basis: performing our
contract with the organizer (Art. 6(1)(b)) and our legitimate interest in running a secure service
(Art. 6(1)(f)).

An organizer can remove their own data from the admin ("Remove my data"): their defaults go, their name and
contacts are taken off their raffles, and their raffles' personal data is erased. The anonymous draws stay
verifiable; the account's identifier stays with them and in the audit log. The account itself is deleted in
Clerk.

## Providers we rely on (subprocessors)

| Provider | What for | What it sees |
|---|---|---|
| **[Hosting: e.g. Cloudflare, Inc. (USA; DPF, SCCs); the database (D1) and the live channels (Durable Objects) in the EU jurisdiction, the application code running wherever the user is]** | Running the application and storing its database | Everything the service stores. D1's point-in-time recovery keeps erased data for up to 30 more days. |
| **Restate** ([Restate Cloud, EU] or self-hosted) | The raffle engine: it runs each draw against the public randomness round it committed to | Anonymous entrant IDs and prize names only; its journal is kept for about an hour and a raffle is forgotten when it finishes |
| **Clerk, Inc.** (USA; DPF) | Organizer sign-in | Organizers' account details; never entrants' data |
| **drand / League of Entropy** | Public randomness | Nothing: we only download public beacons |

Providers outside the EU/EEA are covered by **[the EU–US Data Privacy Framework, where certified, and otherwise
the European Commission's standard contractual clauses]**. A copy of the clauses in force is available from
**[the operator's email]** on request.

## Logs, security and backups

The service writes no personal data to its logs and sets only strictly necessary cookies: the entry, on the
entrant's device, and the organizer's sign-in session. No analytics, no tracking. Backups and the hosting
provider's point-in-time recovery may hold erased data for up to **[30]** more days; a restore re-applies
every recorded erasure.

## Retention

Entrants' data is erased on the dates each raffle's notice shows: by default 7 days after the event for
non-winners, and for winners once their prize is marked delivered, at the latest 90 days after that.
Organizers' audit logs are kept for **[period]** after the account is removed.

## Your rights

Entrants exercise their rights (access, correction, erasure, restriction, objection) against the raffle's
organizer, whom its notice names; we help the organizer where the admin can't. Organizers exercise theirs
against us at **[operator email]**. Everyone may complain to their supervisory authority: **[authority]**.
