# Glossary

The words Rafi uses, in the docs, the UI and the code. Where the UI uses a different label, it is given in quotes.

## The event

| Term | Meaning |
|---|---|
| **Raffle** | One event's prize draw, from setup to finish. It belongs to a Clerk organization, or else to a user. |
| **Organizer** | Whoever runs the raffle in the admin. The rules and the privacy notice name them as the data controller. |
| **Owner** | Whom a raffle belongs to in the admin: a Clerk organization (shared by its members) or a user. The audit log's `actor`; the member who acted is its `user_id`. |
| **Admin**, **Manager**, **Member** | An organization member's tier, from Clerk: admins remove the organization's data; managers (the custom `org:manager` role or `org:raffles:manage` permission) also create, configure and delete raffles; members run them |
| **Leave**, "Remove my data" | The owner's data goes: never-frozen raffles deleted, the rest erased and purged with the organizer's name and contacts, the defaults deleted |
| **Operator** | Whoever runs the installation: the organizer's processor, named in every privacy notice, whose terms (with the data processing agreement) organizers accept by using the service |
| **Event date** | The calendar day the raffle is drawn. Required; fixed once a real entrant has joined. |
| **Retention period** | Days after the event date before non-winners are erased: the installation's default (7), the organizer's, or the raffle's, at most the installation's maximum (30). Fixed with the event date. |
| **Erasure date** | The end of the event day plus the retention period, computed by the database. Shown to entrants ("at the end of 4 November"); the day non-winners are erased. |
| **Winner hold** | How long a winner's data is kept past the erasure date (90 days) unless their prize is marked delivered: the ceiling |
| **Prize** | What is given away: a name, a quantity, optionally a sponsor, and its **delivery**. |
| **Delivery** (of a prize) | How it reaches its winner, chosen per prize: **"On the spot"** (`on_the_spot`: handed over when the winner claims it, so "Claimed" also marks it delivered) or **"Sent later"** (`later`, the default: a voucher by email, say, marked delivered once it reached them). Not in the manifest: it can change until the raffle is over, and applies to the claims made from then on (each claim keeps the delivery it was made with). A winner of more than one prize (more items than people) counts as delivered while every prize they claimed is: claiming one sent later marks them not delivered again. |
| **Item** | One copy of a prize. Each item is drawn on its own, in any order the organizer picks live. |
| **Entrant** | Someone in the raffle. They joined on their phone, were added by the organizer ("added by you"), or are a **fake** entrant in test mode. |
| **Add**, "Add people" | The organizer putting people in (source `manual`, "added by you"): one person, or a list, as one **batch**, all of them or none, under the entrant cap. A batch has a random ID: an add retried with it adds nobody twice, and **"Undo last add"** takes the latest batch off the list until the freeze. |
| **Display name** | The name an entrant shows on the projector ("Name shown on the screen"). Optional when joining: left blank, it is a **generated name**. Up to 24 characters when joining, up to 64 when the organizer adds someone or renames anyone (`MAX_NAME_CHARS`, `MAX_ADDED_NAME_CHARS`). The names the organizer adds are unique in the raffle (a namesake gets a number, "Anna Kovács (2)"). The organizer can hide or rename it. |
| **Name format** | How a list's names are shown: `full` (the default, "Anna Kovács") or `first_initial` ("Anna K.", with more letters of the family name to tell two people apart, at most three, then a number). The browser makes the names; the add's audit row records the format. |
| **Registration ID** (`list_ref`) | A row's ID in the organizer's registration system (Luma's `api_id`, Eventbrite's attendee number, Meetup's member ID), optional, admin only: it recognizes someone on a later add of the list, especially one without emails. Erased with the entrant. |
| **Group order** | Rows of a list with one email and different names: one buyer's email on several tickets. All of them are added by default, the email with the first; the organizer can keep the first only. |
| **Generated name** | An adjective and an animal, like "Brave Otter": for entrants who leave their name blank (or press 🎲), and for fake entrants |
| **Entrant ID** | A random 128-bit ID in lowercase hex. It is public: the manifest lists entrants by it. |
| **Claim code** | A private code shown on the entrant's phone and in the admin. A winner shows it to collect their prize. Erasure clears it. Someone added by the organizer has no phone and never sees one: the organizer checks it is them instead ("Confirmed: it's them"). |
| **Join code** | The six-character code in the join URL `/j/<CODE>`, also on the projector for typing (shown as `K7Q-2X3`; case and dashes don't matter). Letters and digits without look-alikes. A purged raffle keeps its code but no longer answers to it |
| **Slug** | The raffle's public name in `/s/<slug>`, `/v/<slug>`, `/rules/<slug>` and `/privacy/<slug>` |
| **Registration** (setting), "Let people join with the QR code" | Whether people may join with the QR code (`registration`, on by default; in the settings, the owner's defaults and copies). Off, the raffle has no registration: no Open or Close, no QR code on the projector, no form on `/j/<CODE>`; it freezes from Published, and its entrants are the people the organizer added. Switchable before the freeze, but not while registration is open; not fixed with the rules, which describe both ways in. |
| **Test mode** | Rehearsal: the organizer can add fake entrants, and a TEST badge shows on every screen, the verification page included. Switching it off removes the fakes; it is fixed once the list is frozen. |

## The lifecycle

| Term | Meaning |
|---|---|
| **Draft** | Being set up. Not public. |
| **Published** (`Ready`) | The projector shows the intro screen, but nobody can join yet. Without registration, the list is frozen from here. |
| **Registration open** / **closed** (`Open` / `Closed`) | Whether phones can join. Registration can be reopened until the freeze. |
| **Freeze**, "Freeze list & start the draw" | Fixes the entrant list (closing registration for good, if the raffle has it), writes the manifest and opens the raffle in the engine. From Registration open or closed; without registration, from Published, after the checks opening would make (an organizer and a contact named, the erasure date not passed). A freeze that finds nobody goes back to Registration closed, or without registration to Published. |
| **Ready to draw** (`Frozen`) | Frozen, before the first draw |
| **Drawing** | At least one draw has started |
| **Finished** | "Finish the raffle": nothing more can be drawn |
| **Data erased** (`Purged`) | Names, emails and claim codes are gone: "Erase personal data now", the winner hold passing, the organizer leaving, or an **expired** raffle. The anonymous raffle (manifest and log) stays. |
| **Expire** | What happens to a raffle that was never frozen when its erasure date passes: its entrants are deleted outright and it becomes Data erased |
| **Erase** (one entrant) | Once the raffle is finished or its erasure date has passed, clears one entrant's personal data, on request. Until then nobody leaves the list (someone the organizer added can **take themselves off**). |
| **Removed at their request** | Someone removed before the freeze because they asked (`remove_entrant` with `at_request`; anyone real, a joiner too): their email key and registration ID are remembered (`add_removals`), so a later add of the organizer's leaves them out ("Removed at their request" in the list tab's preview) unless the organizer **adds them anyway** for that row (`lift_removal`), which deletes the removal. Kept only while the raffle can take an add: forgotten when the manifest is written, and with the raffle's expiry, purge or deletion. |
| **Took themselves off** | Someone the organizer added who asks, from the freeze until the raffle is over, not to take part (they accepted no rules, so they may object; `object_entrant`, `entrants.objected_at`): their name is hidden for good, a draw that picks them can only be **Declined**, and their personal data is erased once the raffle is over rather than on the erasure date. Their anonymous ID stays in the manifest. Not for a winner who claimed a prize. Undone ("Undo", `unobject_entrant`) until the raffle is over. |
| **Mark delivered**, "All prizes delivered" | The prize reached its winner: their data goes on the non-winner clock instead of being held. Claiming a prize delivered on the spot does it too. |
| **Housekeeping** | The hourly run that erases personal data on the dates (and of whoever took themselves off, once the raffle is over), expires never-frozen raffles, forgets removals at their request no add can need any more and expired proof-of-work challenges, and ends live updates a day after a raffle finishes |
| **End live updates** | Phones and the projector stop following the raffle live. This happens automatically a day after the finish. |

## The draw

| Term | Meaning |
|---|---|
| **Manifest** | Everything that decides the outcome except the randomness: the entrant IDs, the items, the algorithm and the beacon source. Published when the list is frozen. |
| **Manifest hash** | `SHA256(manifest)`. Every draw's seed includes it. |
| **drand** | A public randomness network run by the League of Entropy. Every period it publishes a **round**: a number and a **beacon**. |
| **Chain** | A drand network, identified by its chain hash. Rafi uses **quicknet** (a round every 3 s). |
| **Beacon** | A round's BLS **signature** by the network. It verifies against the chain's public key. **Randomness** is `SHA256(signature)`. |
| **Relay** | An HTTP server that serves drand beacons. Relays are untrusted: every beacon is verified. |
| **Mock chain** | A test chain with a public key, for offline development. Its draws are **not** verifiable. |
| **Draw** | Picking the winner of one item. It is numbered **`k`**. |
| **Announce**, **commit** | A draw fixes its round `R`, a few seconds in the future (the **lead**), before `R` exists |
| **Pending draw** | Announced, waiting for its round. Nothing may change its pool meanwhile. |
| **Stalled** | The round's beacon couldn't be fetched in time. The draw stays pending on the same round. |
| **Retry**, "Retry round R" | Drawing a stalled item again fetches the same round |
| **Abandon** | The organizer's explicit, logged way out of a stalled draw. The item goes back, and the next draw commits to a new round. |
| **Pool** | Who can be drawn: in **pass 1**, everyone not yet drawn. In **pass 2** (more items than people), earlier winners who didn't forfeit. |
| **Resolution** | What happened to a drawn winner: **"Claimed"** (`claimed`: present, their code checked, or for someone the organizer added, "Confirmed: it's them"; delivered too only if the prize is delivered on the spot), **"Not here"** (`not_here`) or **"Declined"** (`declined`) |
| **Forfeit** | "Not here" or "Declined": the person is out, and the item is drawn again |
| **Withdraw** (`void`) | Taking an available item out of the raffle |
| **Event log** | The raffle's append-only record: `announced`, `drawn`, `stalled`, `abandoned`, `resolved`, `voided`, `finished` |
| **Verification page** | `/v/<slug>`: re-runs every draw from the manifest and the log, in the browser |

## The system

| Term | Meaning |
|---|---|
| **App** | The server and the web app: `rafi` natively, `rafi-worker` on Cloudflare Workers (the crate; the deployed Worker is named `rafi`) |
| **Raffle engine** | The component that owns a raffle from opening to finish and runs its draws. It sees only anonymous IDs. |
| **Restate** | The durable execution engine the raffle engine runs on. Each raffle is one `Raffle` **virtual object**. |
| **Projection** | The raffle as stored in the app's database. The engine writes it after every change, and everything reads it. |
| **Live updates** | Changes pushed to phones and the projector. On Workers they are pushed; natively, clients poll. |
| **Participant view** | The small public state a phone renders |
| **Full view** | What the projector and the admin show |
| **Raffle room** | On Workers, the Durable Object (`RaffleRoom`) that hosts one raffle's live channel |
| **Proof-of-work** | ALTCHA v2: a small computation the phone solves before it can join, to slow down bots |
