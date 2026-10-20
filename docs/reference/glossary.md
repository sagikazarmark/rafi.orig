# Glossary

The words Rafi uses, in the docs, the UI and the code. Where the UI uses a different label, it is given in quotes.

## The event

| Term | Meaning |
|---|---|
| **Raffle** | One event's prize draw, from setup to finish. It belongs to a Clerk organization, or else to a user. |
| **Organizer** | Whoever runs the raffle in the admin. The rules and the privacy notice name them as the data controller. |
| **Prize** | What is given away: a name, a quantity, and optionally a sponsor. |
| **Item** | One copy of a prize. Each item is drawn on its own, in any order the organizer picks live. |
| **Entrant** | Someone in the raffle. They joined on their phone, were added by the organizer ("added by you"), or are a **fake** entrant in test mode. |
| **Display name** | The name an entrant shows on the projector ("Name shown on the screen"). The organizer can hide or rename it. |
| **Entrant ID** | A random 128-bit ID in lowercase hex. It is public: the manifest lists entrants by it. |
| **Claim code** | A private code shown on the entrant's phone and in the admin. A winner shows it to collect their prize. |
| **Join code** | The short code in the join URL `/j/<CODE>`, which is also on the projector |
| **Slug** | The raffle's public name in `/s/<slug>`, `/v/<slug>`, `/rules/<slug>` and `/privacy/<slug>` |
| **Test mode** | Rehearsal: the organizer can add fake entrants, and a TEST badge shows on every screen |

## The lifecycle

| Term | Meaning |
|---|---|
| **Draft** | Being set up. Not public. |
| **Published** (`Ready`) | The projector shows the intro screen, but nobody can join yet |
| **Registration open** / **closed** (`Open` / `Closed`) | Whether phones can join. Registration can be reopened until the freeze. |
| **Freeze**, "Freeze list & start the draw" | Closes registration for good, fixes the entrant list, writes the manifest and opens the raffle in the engine |
| **Ready to draw** (`Frozen`) | Frozen, before the first draw |
| **Drawing** | At least one draw has started |
| **Finished** | "Finish the raffle": nothing more can be drawn |
| **Data erased** (`Purged`) | "Erase personal data now": names, emails and claim codes are gone. The anonymous raffle (manifest and log) stays. |
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
| **Resolution** | What happened to a drawn winner: **"Handed over"** (`claimed`), **"Not here"** (`not_here`) or **"Declined"** (`declined`) |
| **Forfeit** | "Not here" or "Declined": the person is out, and the item is drawn again |
| **Withdraw** (`void`) | Taking an available item out of the raffle |
| **Event log** | The raffle's append-only record: `announced`, `drawn`, `stalled`, `abandoned`, `resolved`, `voided`, `finished` |
| **Verification page** | `/v/<slug>`: re-runs every draw from the manifest and the log, in the browser |

## The system

| Term | Meaning |
|---|---|
| **App** | The server and the web app: `rafi` natively, `rafi-worker` on Cloudflare Workers |
| **Raffle engine** | The component that owns a raffle from opening to finish and runs its draws. It sees only anonymous IDs. |
| **Restate** | The durable execution engine the raffle engine runs on. Each raffle is one `Raffle` **virtual object**. |
| **Projection** | The raffle as stored in the app's database. The engine writes it after every change, and everything reads it. |
| **Live updates** | Changes pushed to phones and the projector. On Workers they are pushed; natively, clients poll. |
| **Participant view** | The small public state a phone renders |
| **Full view** | What the projector and the admin show |
| **Raffle room** | On Workers, the Durable Object (`RaffleRoom`) that hosts one raffle's live channel |
| **Housekeeping** | An hourly run that forgets expired proof-of-work challenges and ends live updates a day after a raffle finishes |
| **Proof-of-work** | ALTCHA v2: a small computation the phone solves before it can join, to slow down bots |
