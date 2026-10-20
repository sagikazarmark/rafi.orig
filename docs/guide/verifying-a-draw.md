# Verifying a draw

Anyone can check a Rafi raffle: attendees, sponsors, skeptics. You need nothing from the organizer. Everything
needed is public once the entrant list is frozen.

There are two independent ways to check, and you can use either or both:

| | Verification page | `tools/verify.py` |
|---|---|---|
| Where | `/v/<slug>`, in any browser | Your own machine, Python 3, standard library only |
| Code | Rafi's own verifier, compiled to WebAssembly | A separate implementation of the same [algorithm](../reference/draw-algorithm.md) |
| Checks drand signatures | Yes: the BLS signature, against the chain's public key | Compares each signature with the public drand relays' (`--fetch`). It does not check BLS itself. |
| Abandoned draws | Lists them | With `--fetch`, shows whom each abandoned round would have picked |

## On the verification page

Open `/v/<slug>`. It is linked from the participant page ("Check the draw yourself"), the projector's thank-you
screen and the admin. The page downloads the raffle's published data and re-runs every draw in your browser:

- **✅ All N draws check out.** The manifest matches its published hash, every beacon is a genuine drand signature
  for its round, and every draw picked exactly the entrant the algorithm dictates.
- **🧪 Test chain: not publicly verifiable.** The raffle used a development test chain. The draws replay
  consistently, but anyone could have produced their randomness. A real event should never show this.
- **❌ Verification failed.** Something doesn't match. The page lists the problems.

A raffle drawn in test mode also says **🧪 TEST MODE: a rehearsal**: its entrants may be made up.

It also shows:

- **You're in**, when you open it from your phone's "Check the draw yourself": your entry is in the frozen list.
  (The link carries your entrant ID after `#me=`, which stays in your browser.)
- **Entrants**, **Prizes** and the **Manifest hash**
- every draw: the prize, its **drand round** (linked to the beacon on `api.drand.sh`), the pool size and the winner
- any **abandoned** draws: a warning naming the round and the pool. See below.
- the **entrants**: everyone in the frozen list, by display name (hidden names masked) and the start of their ID,
  with **Find your name**. Once the organizer erases personal data, only the IDs remain.
- the event log and the raw manifest
- a link to download the published data (`verify.json`): the manifest, the log and the display names

## With `tools/verify.py`

Download [`tools/verify.py`](../../tools/verify.py). It's a single file that only needs Python 3. Point it at the
raffle's data:

```sh
python3 verify.py https://<host>/api/r/<slug>/verify.json
# or a saved copy
python3 verify.py verify.json
# also re-fetch every beacon from the public drand relays, and show whom abandoned rounds would have picked
python3 verify.py --fetch verify.json
```

Example output:

```text
ok   manifest hash 4c415cf2e358054c438ef3ccae56791cf639cec93ff3e6090db62b530f8b942c
     12 entrants, 4 items, drand chain 52db9ba70e0c… (quicknet)
ok   draw 1: round 1000, pass 1, pool 12, index 1 -> 1b10848fd312afbb7afe82eec74fc1c7
     draw 1: not here
     draw 2: committed round 1003 was ABANDONED (abandoned by the organizer) (pass 1, pool 11)
     draw 2: round 1005 not available yet (drand unavailable); still pending
ok   draw 2: round 1005, pass 1, pool 11, index 5 -> 514fd5455cdd9fd3fbb20606fa906d11
     draw 2: claimed
…
VERIFIED
```

It exits with `0` when everything verifies and `1` otherwise.

## What to look for

A green result proves the draws followed the rules from the published list and the public randomness. A few things
are worth a human look:

- **Abandoned draws.** An organizer may abandon a draw only when drand couldn't be reached. Each one is listed, and
  `verify.py --fetch` shows whom its round would have picked. One abandon during an outage is normal. An abandon
  whose round would have picked someone the organizer didn't want is a question to ask.
- **"Not here" and "Declined".** These are how a prize gets redrawn. They're in the log, and the person concerned
  was in the room.
- **The timing.** The log can't prove *when* a round was committed. That is what the live countdown on the
  projector is for. If you want to be thorough, note the manifest hash on the verification page before the first
  draw, and the round number of each draw as it's announced, then check that the log matches.
- **The list.** Check that your name is in it, and that the entrant count matches what the projector showed.

[Why the draw is verifiable](../explanation/verifiability.md) explains what this proves and where it stops.

## Writing your own verifier

The [algorithm reference](../reference/draw-algorithm.md) specifies everything, and
[`testdata/draw-vectors.json`](../../testdata/draw-vectors.json) has vectors from real quicknet beacons. A
verifier that reproduces them, checks BLS signatures against the chain's public key and replays the log rules is
fully independent of Rafi.
