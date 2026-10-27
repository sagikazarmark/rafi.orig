# Draw algorithm: `rafi-draw-v1`

This is the specification of how Rafi picks winners, written so that you can build an independent verifier from it.
For *why* the algorithm looks like this, see [Why the draw is verifiable](../explanation/verifiability.md).

The implementation is [`crates/raffle`](../../crates/raffle) (`algorithm.rs`, `manifest.rs`, `raffle.rs`,
`verify.rs`). [`tools/verify.py`](../../tools/verify.py) implements it a second time with only the Python standard
library. CI checks both against the same vectors.

## Notation

- `SHA256(x)` is SHA-256, and `‖` is byte concatenation.
- `BE_u64(n)` is `n` as 8 bytes, big-endian.
- Hashes and signatures are written as lowercase hex in JSON, and are hashed as raw bytes.

## Inputs

### Manifest

The manifest holds everything that decides the outcome except the randomness. It is written when the organizer
starts the draw, before any draw can be announced.

```json
{"algo":"rafi-draw-v1","raffle":"<raffle id>","beacon":{"drand":"<chain hash>"},"entrants":["<id>",…],"items":[{"id":"<item id>","name":"<name>"},…]}
```

| Field | Meaning |
|---|---|
| `algo` | Always `"rafi-draw-v1"`. It also serves as the domain-separation tag. |
| `raffle` | The raffle's ID. This ties the manifest to one raffle. |
| `beacon` | The randomness source: `{"drand": "<chain hash>"}`. Only a public drand chain is publicly verifiable. |
| `entrants` | The entrant IDs: random 128-bit values in lowercase hex, **sorted, deduplicated and lowercase**. Names and emails are not included. |
| `items` | One item per copy of a prize, each with a unique `id` and a `name`. |

The fields always appear in this order, and the manifest has no maps or floats. That makes its serialization
canonical, and the canonical form is pinned so that any JSON library produces the same bytes:

- no whitespace anywhere outside strings; object keys in the order shown; arrays in the order given;
- integers as plain digits (the manifest has none today);
- strings in UTF-8, escaping only `"` as `\"`, `\` as `\\`, and the control characters U+0000–U+001F: U+0008 as
  `\b`, U+0009 as `\t`, U+000A as `\n`, U+000C as `\f`, U+000D as `\r`, every other one as `\u00xx` with
  lowercase hex. Nothing else is escaped: not `/`, not U+007F, not U+2028 or U+2029, and no non-ASCII character.

These are the rules of `serde_json` and of Python's `json.dumps(…, separators=(",", ":"), ensure_ascii=False)`,
which is why both verifiers reproduce the string.

The exact published string is what gets hashed: verifiers hash it **as published**, never a re-serialization. A
verifier does also check that the published string *is* canonical, by parsing it and re-serializing it under the
rules above: a string that isn't (a duplicate key, say, which parsers resolve differently) could be read two ways,
and a manifest that can be read two ways isn't a commitment.

A manifest is valid when:

- `algo` is an algorithm the reader knows (today only `rafi-draw-v1`; a reader *opening a new raffle* requires
  the current one)
- there is at least one entrant and at least one item
- the entrants are sorted, unique and lowercase
- the item IDs are unique

```
manifest_hash = SHA256(manifest)                 the published string's UTF-8 bytes
```

### Beacon

Every draw commits to one round `R` of the manifest's drand chain. The engine commits to the first round published
at least the lead (6 s by default) after the draw was requested. A round's beacon carries a BLS `signature`, which
must verify against the chain's public key:

```
randomness = SHA256(signature)                   the signature's raw bytes
```

The default chain is quicknet:

| | |
|---|---|
| Chain hash | `52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971` |
| Genesis | 1692803367 (Unix time) |
| Period | 3 s |
| Scheme | `bls-unchained-g1-rfc9380` |
| Relays | `api.drand.sh`, `api2.drand.sh`, `api3.drand.sh`, `drand.cloudflare.com` |

## Selection

`k` is the draw number, starting at 1. It increases only when a draw completes, so no-show redraws count. Retrying
a stalled draw keeps its `k`. After an abandon, the next draw reuses the abandoned draw's `k` with a new round.

```
seed_k      = SHA256("rafi-draw-v1" ‖ randomness ‖ manifest_hash ‖ BE_u64(k))
rand_u64(c) = BE_u64(SHA256(seed_k ‖ BE_u64(c))[0..8])        c = 0, 1, 2, …
uniform(n)  : limit = 2^64 − (2^64 mod n)
              take rand_u64(c) for c = 0, 1, … until one is below limit; return it mod n
winner_k    = pool_k[uniform(|pool_k|)]
```

Rejection sampling means there is no modulo bias. The algorithm needs nothing but SHA-256. It deliberately avoids
general-purpose random number generators, whose output isn't guaranteed to stay the same across versions and
languages.

## Pools

`pool_k` is a list of entrant IDs in manifest order (sorted). It depends only on the completed draws before draw `k`:

```mermaid
flowchart TD
    A["entrants not yet drawn in pass 1<br/>(whatever their outcome)"] -->|not empty| P1["pass 1 pool"]
    A -->|empty| B["pass-1 winners who did not forfeit<br/>and are not yet drawn in pass 2"]
    B -->|not empty| P2["pass 2 pool"]
    B -->|empty| X["no eligible entrants:<br/>the draw is refused"]
```

- **Pass 1:** every entrant not yet drawn in pass 1, whatever the outcome of their draw. This is the
  one-win-per-person rule: being drawn takes you out of pass 1, even if you then forfeit.
- **Pass 2:** only once pass 1 is exhausted, which means there are more items than people. It holds the pass-1
  winners who didn't forfeit and haven't been drawn in pass 2 yet.
- A **forfeit** is a draw resolved `not_here` or `declined`.

## The event log

A raffle is its manifest plus an append-only event log. The verifier replays the log through the raffle's own
transitions. Each event is a JSON object with a `type` tag:

| `type` | Fields | Meaning |
|---|---|---|
| `announced` | `k`, `item`, `round`, `round_time` | Draw `k` committed to drand round `round`, which is published at Unix time `round_time` |
| `drawn` | `k`, `item`, `round`, `signature`, `randomness`, `pass`, `pool_size`, `index`, `entrant` | The round's beacon picked `pool[index]` |
| `stalled` | `k`, `item`, `round`, `reason` | The round's beacon couldn't be fetched or verified (yet). The draw stays pending on the same round. |
| `abandoned` | `k`, `item`, `round`, `reason` | The organizer abandoned the pending draw, and the item can be drawn again |
| `resolved` | `k`, `resolution` | `claimed`, `not_here` or `declined` |
| `voided` | `item` | The item was withdrawn |
| `finished` | | The raffle ended |

These rules hold, and the replay enforces them:

1. At most one draw is pending at a time. A `drawn` event must settle the pending draw **on its announced round**.
2. While a draw is pending, nothing may change its inputs. Other draws, forfeits (`not_here`, `declined`), voiding
   its item and finishing are all refused. A claim (`claimed`) is allowed, because it can't change the pool.
3. A pending draw leaves only through `drawn` (its own round) or an explicit `abandoned`. A `stalled` event leaves it
   pending.
4. Only an item that is available can be drawn or voided. Only a drawn item can be resolved.

## `verify.json`

`GET /api/r/<slug>/verify.json` publishes everything a verifier needs once the list is frozen:

```json
{
  "format": 1,
  "v": 42,
  "title": "…",
  "state": "drawing",
  "test_mode": false,
  "manifest": "<the canonical manifest string, exactly as hashed>",
  "manifest_hash": "<hex>",
  "events": [ … ],
  "names": [["<entrant id>", "<display name>"], …]
}
```

`format` versions this envelope (the manifest and the events are versioned by `algo`): fields may be added under
the same number, and a reader refuses a number it doesn't know. `v` is the raffle's version in the database, which
the response also sends as its `ETag`; the same `v` is the same content, and anything that changes the content
(a draw, a name hidden or erased) moves it. The response is `Cache-Control: no-cache`: readers revalidate with
`If-None-Match` and get a `304` while nothing changed, and an erased name is never served from a cache.

`names` lets people check that they are in the list: hidden names show as `★ ★ ★`. An entrant whose personal data
is erased is no longer in it (their ID stays in the manifest), so once the organizer erases everyone's, it is
empty. `test_mode` is `true` for a rehearsal, whose entrants may be fakes.

## Verification

A log verifies when all of these hold:

1. `SHA256(manifest)` equals the published `manifest_hash`, hashing the published string as is.
2. The manifest is valid and canonical (re-serializing what it parses to, under the rules above, gives the
   published string back), and its chain is a public drand chain the verifier knows.
3. Every `drawn` beacon's BLS signature verifies against that chain for its round, and
   `randomness = SHA256(signature)`.
4. Replaying every event through the rules reproduces it exactly: the same pass, pool size, index and winner.
5. Every event has a `type` the verifier knows. **A verifier never skips what it can't interpret**: an unknown
   event could have changed the pool, and skipping it would vouch for a log that wasn't checked. An unknown
   `algo` or `type` is a verification failure that says the verifier is older than the log.

For each `abandoned` draw, a verifier can show whom the committed round would have picked: fetch the round, then
apply the selection to the pool as it stood.

## Versions and compatibility

The manifest and the event log are published, stored in the database (`raffle_events.body`) and held by the
raffle engine while a raffle runs, all as the JSON above, and they stay readable for as long as anyone may want to
verify a draw. So their shape is a contract, and `algo` is its version:

- **A reader dispatches on `algo`** and keeps every algorithm it ever shipped. Rebuilding a stored raffle, showing
  it, and verifying it work for every version; only opening a *new* raffle requires the current one.
- **Within a version, changes are additive only.** A new field on an event carries a default, so a log written
  before it still reads; a field or an event type is never renamed or removed; a field's meaning never changes.
- **A new event type, or any change to how winners are picked or pools are formed, is a new `algo`**
  (`rafi-draw-v2`), with its own section in this document and its own vectors. Logs under the old version keep
  verifying under the old rules.
- **A reader older than the log fails closed** (rule 5 above) rather than guessing. Operators avoid the case by
  never running a build older than the one that wrote the database (see [Upgrades](../operations/upgrades.md)).

## Test vectors

[`testdata/draw-vectors.json`](../../testdata/draw-vectors.json) holds `select` vectors (`randomness`,
`manifest_hash`, `k`, `pool_size` → `index`) and a whole `scenario` (`manifest`, `manifest_hash`, `events`) built
from real quicknet beacons ([`testdata/quicknet-beacons.json`](../../testdata/quicknet-beacons.json)). An
independent implementation should reproduce them all:

```sh
python3 tools/verify.py --vectors testdata/draw-vectors.json
```
