# The draw

What happens from **Freeze list & start the draw** to **Finish the raffle**, from the organizer's side. For why this
makes the result trustworthy, see [Why the draw is verifiable](../explanation/verifiability.md).

## In short

1. **Freezing** fixes who is in and what can be won, and publishes it.
2. **Each Draw** commits to a public drand randomness round a few seconds in the future, waits for it, and picks one
   winner from it with a public formula.
3. **You decide** what happened to each winner: claimed, not here, or declined. A no-show's prize is drawn again.
4. **Anyone can re-run** every draw on the verification page.

## 1. Freezing the list

**Freeze list & start the draw**, offered while registration is open or closed (or, in a raffle [without
registration](running-a-raffle.md#raffles-without-registration), once it is published), turns the raffle's
entrants and prizes into a **manifest**:

- every entrant, as a random anonymous ID (no names or emails)
- one **item** per copy of each prize ("3 × T-shirt" becomes three items)
- the draw algorithm (`rafi-draw-v1`) and the randomness source (drand quicknet)

The manifest and its hash are published on the verification page. From this moment nothing about who can win
changes, except through the public decisions below.

## 2. Drawing an item

Pick any item that is still **to be drawn** and press **Draw**. You choose the order live, and it doesn't matter to
the result: nobody knows what any future draw will pick.

```mermaid
sequenceDiagram
    actor You as Organizer
    participant S as Projector
    participant R as Rafi
    participant D as drand

    You->>R: Draw "T-shirt"
    R->>R: commit to round R (about 6 s ahead)
    R->>S: "Locking in public randomness · drand round R" + countdown
    Note over D: round R does not exist yet
    D-->>R: round R is published
    R->>R: verify it, compute the winner
    R->>S: the winner's name
    R->>You: the winner, with their claim code
```

1. **Commit.** Rafi picks the drand round that will be published about 6 seconds from now and records it. This
   happens *before* the round exists.
2. **Countdown.** The projector, every phone and your admin show "drand round R" with a countdown. During it, the
   **Draw** buttons are disabled: only one draw runs at a time.
3. **Reveal.** When round R is published, Rafi fetches it, checks its signature and computes the winner:
   `pool[uniform(SHA256(…randomness, manifest hash, draw number…))]`. The details are in the
   [reference](../reference/draw-algorithm.md).
4. **Winner.** The projector shows the winner's display name. The winner's phone shows **YOU WON!** with their claim
   code, and everyone else sees "Not this time". Your admin shows the winner's name, email (masked until you press
   **Show**) and claim code. A winner [you added](#checking-a-winner-you-added) has no phone: the projector says
   **"Come to the front!"** under their name.

### Who can be drawn

- **One win per person.** Once someone is drawn, they're out of the pool, whatever happens next.
- **Pass 2.** If there are more items than people, the pool runs dry. A second pass then draws from earlier winners
  who didn't forfeit, each at most once more. A winner drawn again sees every prize they won on their phone, with
  their claim code once. The People tab lists every prize on their row, and one **Mark delivered** covers them all.
- If nobody at all is left, Rafi refuses the draw, and the item stays to be drawn.

## 3. Deciding what happened

Each drawn item waits for your decision:

```mermaid
stateDiagram-v2
    direction LR
    state "To be drawn" as Available
    state "Counting down to round R" as Pending
    state "Stalled on round R" as Stalled
    state "Drawn: waiting for the winner" as Drawn
    state "Claimed" as Claimed
    state "Withdrawn" as Voided

    [*] --> Available
    Available --> Pending: Draw
    Pending --> Drawn: round R arrives, winner picked
    Pending --> Stalled: round R can't be fetched
    Stalled --> Pending: Retry round R
    Stalled --> Available: Abandon this draw (logged)
    Drawn --> Claimed: Claimed
    Drawn --> Available: Not here / Declined (winner is out)
    Available --> Voided: Withdraw
```

| Button | When | Effect |
|---|---|---|
| **Claimed** (**Confirmed: it's them** for someone [you added](#checking-a-winner-you-added)) | The winner came up and showed the matching claim code, or you checked it is them | The item is theirs. Works even while another draw is counting down. A prize delivered **on the spot** is marked delivered with it; one **sent later** is **Mark delivered** in the People tab, once it reached them. The prize's delivery is set in the Prizes tab, and the console says which it is. |
| **Not here** | Nobody answers to the name | The winner is out of the raffle, and the item goes back to be drawn again with a new round |
| **Declined** | The winner doesn't want it, or [took themselves off](#someone-who-took-themselves-off) | Same as Not here |
| **Withdraw** | An item that won't be given away (it broke, the sponsor didn't show) | The item is withdrawn. Only an item that is still to be drawn can be withdrawn. |

**Not here** and **Declined** ask for confirmation, can't be undone, and are recorded in the public log. They're
disabled while another draw is counting down, because they would change who that draw can pick.

### Checking a winner you added

Someone you added, one by one or from a list ("added by you" in the console), has no phone: nothing tells them
they won, and they never saw a claim code. So:

- The projector shows **"Come to the front!"** under their name: they recognize it and come up. Not when their
  name is hidden (you hid it, or they took themselves off): nobody would recognize ★ ★ ★, so call them yourself.
- The console shows their name, their email masked ("a•••@example.com") with a **Show** button, "added by you",
  and **"Check it's them: ask for the email they registered with, or their badge."** No claim code: there is
  nothing to compare it with. The email is masked because consoles get mirrored onto projectors, and asking for it
  is the check: shown in full, it would give the answer away. It also tells namesakes apart.
- When it is them, press **Confirmed: it's them**. It is recorded as **claimed**, as for a joiner who showed their
  code, and a prize delivered on the spot is marked delivered with it. If nobody comes, **Not here**; if it's
  somebody else, don't confirm.

Winners who joined with the QR code show their code as always, and their button reads **Claimed**.

### Someone who took themselves off

Someone you added who asked, after the freeze, not to take part ("Take off at their request" in the People tab) is
still in the draw by their anonymous ID: removing it would break the verification. If they are drawn, the console
says **"Took themselves off: decline"** and offers **Declined** only: they claim nothing, and "Not here" would
record an absence that isn't what happened (the server refuses both). The prize goes back to be drawn again, as
for any decline, and the projector shows their name hidden.

## When drand can't be reached

Rafi waits up to 60 seconds past the round for its beacon, trying several public relays. If it still can't get it,
the draw **stalls**. Stalling is not an error that resets anything: the draw stays committed to round R.

The admin shows "Round R could not be fetched yet" with two options:

- **Retry round R.** This fetches the *same* round again, so retrying can never change the result. Use it when the
  venue's or the server's connection hiccuped.
- **Abandon this draw.** Use it only if drand stays unreachable. The item goes back to be drawn, and the next draw
  commits to a new round. The abandon is public: the verification page lists it, and anyone can see whom the
  abandoned round would have picked.

The projector shows "Still fetching drand round R" meanwhile.

> Why not just draw again with a new round? Because then a stall would be a way to reroll a result you didn't like.
> Committing to a round means sticking to it.

## Finishing

**Finish the raffle** ends the drawing. You can finish with prizes left, and they simply aren't given away. It isn't
available while a draw is counting down. Everyone sees **Thanks for playing!**, the winners, and a link to check the
draw.

## What everyone sees

| Moment | Projector | Winner's phone | Everyone else's phone |
|---|---|---|---|
| Ready to draw | "N prizes left", entrant count | "The draw is starting!" | "The draw is starting!" |
| Countdown | Prize, "drand round R", countdown | "Locking in public randomness" | same |
| Drawn | The winner's display name, and "Come to the front!" for someone you added | **YOU WON!**, claim code, chime (and vibration on Android) | "Not this time", prizes left |
| Finished | **Thanks for playing!**, winners, verification link | Their prize | "Thanks for playing!" |

## See also

- [Verifying a draw](verifying-a-draw.md)
- [Why the draw is verifiable](../explanation/verifiability.md)
- [Troubleshooting](../operations/troubleshooting.md)
