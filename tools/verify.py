#!/usr/bin/env python3
"""Independent verifier for Rafi draws (rafi-draw-v1).

Usage:
    verify.py verify.json              # file downloaded from /api/r/<slug>/verify.json
    verify.py https://host/api/r/<slug>/verify.json
    verify.py --fetch verify.json      # also re-fetch every beacon from drand
    verify.py --vectors testdata/draw-vectors.json

Standard library only. The algorithm:

    manifest_hash = SHA256(manifest)                     (published canonical JSON string)
    seed_k       = SHA256("rafi-draw-v1" || randomness || manifest_hash || BE_u64(k))
    rand_u64(c)  = BE_u64(SHA256(seed_k || BE_u64(c))[0:8])
    uniform(n)   = first rand_u64(c) below 2^64 - (2^64 mod n), taken mod n
    winner_k     = pool_k[uniform(len(pool_k))]

    pool (pass 1) = entrants never drawn before (whatever the outcome)
    pool (pass 2) = pass-1 winners not forfeited/declined, not yet drawn in pass 2
    randomness    = SHA256(drand signature)

This script checks the algorithm and, with --fetch, that each signature is the
one the public drand relays serve for that round. It does not check the BLS
signature itself (that needs pairing-friendly curve code); the browser
verifier on the raffle's verify page does. With --fetch it also shows whom
the committed round of an abandoned draw would have picked.
"""

import hashlib
import json
import sys
import urllib.request

ALGO = b"rafi-draw-v1"
RELAYS = ["https://api.drand.sh", "https://drand.cloudflare.com"]
# Public drand chains; anything else (e.g. a test chain) is not publicly verifiable.
PUBLIC_CHAINS = {
    "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971": "quicknet",
}


def sha256(*parts: bytes) -> bytes:
    h = hashlib.sha256()
    for p in parts:
        h.update(p)
    return h.digest()


def select(randomness: bytes, manifest_hash: bytes, k: int, n: int) -> int:
    seed = sha256(ALGO, randomness, manifest_hash, k.to_bytes(8, "big"))
    limit = 2**64 - (2**64 % n)
    c = 0
    while True:
        x = int.from_bytes(sha256(seed, c.to_bytes(8, "big"))[:8], "big")
        c += 1
        if x < limit:
            return x % n


def pool(entrants, draws):
    """draws: list of dicts with pass, entrant, outcome (in draw order)."""
    drawn1 = {d["entrant"] for d in draws if d["pass"] == 1}
    p1 = [e for e in entrants if e not in drawn1]
    if p1:
        return 1, p1
    winners = {d["entrant"] for d in draws if d["pass"] == 1 and d["outcome"] in ("drawn", "claimed")}
    drawn2 = {d["entrant"] for d in draws if d["pass"] == 2}
    return 2, [e for e in entrants if e in winners and e not in drawn2]


def fetch_beacon(chain, round_):
    for relay in RELAYS:
        try:
            with urllib.request.urlopen(f"{relay}/{chain}/public/{round_}", timeout=10) as r:
                return json.load(r)
        except Exception:  # noqa: BLE001 - try the next relay
            continue
    return None


def verify(manifest: str, expected_manifest_hash: str, events, fetch=False) -> bool:
    ok = True
    manifest_hash = sha256(manifest.encode())
    if manifest_hash.hex() != expected_manifest_hash.lower():
        print(f"FAIL manifest hash: computed {manifest_hash.hex()}, published {expected_manifest_hash}")
        ok = False
    else:
        print(f"ok   manifest hash {manifest_hash.hex()}")
    doc = json.loads(manifest)
    entrants = doc["entrants"]
    # The beacon source is tagged by kind: {"drand": "<chain hash>"}.
    chain = doc["beacon"].get("drand")
    if chain is None:
        print(f"FAIL unknown beacon source {doc['beacon']}")
        return False
    name = PUBLIC_CHAINS.get(chain)
    print(f"     {len(entrants)} entrants, {len(doc['items'])} items, drand chain {chain[:12]}… ({name or 'NOT a public drand chain'})")
    if name is None:
        print("FAIL draws use a test chain: their randomness is not publicly verifiable")
        ok = False

    def would_have_picked(k, round_, what):
        pass_, p = pool(entrants, draws)
        print(f"     draw {k}: committed round {round_} was {what} (pass {pass_}, pool {len(p)})")
        if fetch and name and p:
            b = fetch_beacon(chain, round_)
            if b is None:
                print(f"WARN draw {k}: could not fetch drand round {round_}")
            else:
                idx = select(sha256(bytes.fromhex(b["signature"])), manifest_hash, k, len(p))
                print(f"     draw {k}: round {round_} would have picked index {idx} -> {p[idx]}")

    draws = []  # in order
    by_k = {}
    pending = None  # (k, round) of the announced draw
    for ev in events:
        t = ev["type"]
        if t == "announce":
            if pending is not None:
                print(f"FAIL draw {ev['k']}: announced while draw {pending[0]} is pending")
                ok = False
            pending = (ev["k"], ev["round"])
        elif t == "draw":
            k = ev["k"]
            # A draw must settle the announced draw on its committed round
            # (a beacon from another round would be a silent redraw).
            if pending is None or pending != (k, ev["round"]):
                print(f"FAIL draw {k}: round {ev['round']} is not the announced round"
                      + ("" if pending is None else f" {pending[1]}"))
                ok = False
            pending = None
            pass_, p = pool(entrants, draws)
            if not p:
                print(f"FAIL draw {k}: no eligible entrants, but a winner was published")
                ok = False
                continue
            sig = bytes.fromhex(ev["signature"])
            randomness = bytes.fromhex(ev["randomness"])
            if sha256(sig) != randomness:
                print(f"FAIL draw {k}: randomness != SHA256(signature)")
                ok = False
            if fetch and name:
                b = fetch_beacon(chain, ev["round"])
                if b is None:
                    print(f"WARN draw {k}: could not fetch drand round {ev['round']}")
                elif b["signature"].lower() != ev["signature"].lower():
                    print(f"FAIL draw {k}: signature differs from drand round {ev['round']}")
                    ok = False
            idx = select(randomness, manifest_hash, k, len(p))
            expected = p[idx]
            good = expected == ev["entrant"] and pass_ == ev["pass"] and len(p) == ev["pool_size"]
            ok &= good
            print(f"{'ok  ' if good else 'FAIL'} draw {k}: round {ev['round']}, pass {pass_}, pool {len(p)}, "
                  f"index {idx} -> {expected}" + ("" if good else f" (published {ev['entrant']})"))
            d = {"pass": pass_, "entrant": expected, "outcome": "drawn"}
            draws.append(d)
            by_k[k] = d
        elif t == "resolve":
            if pending is not None and ev["outcome"] in ("not_here", "declined"):
                print(f"FAIL draw {ev['k']}: resolved {ev['outcome']} while draw {pending[0]} was pending")
                ok = False
            if ev["k"] in by_k:
                by_k[ev["k"]]["outcome"] = ev["outcome"]
                print(f"     draw {ev['k']}: {ev['outcome'].replace('_', ' ')}")
        elif t == "stall":
            print(f"     draw {ev['k']}: round {ev['round']} not available yet ({ev['reason']}); still pending")
        elif t == "fail":
            if pending != (ev["k"], ev["round"]):
                print(f"FAIL draw {ev['k']}: abandon does not match the pending draw")
                ok = False
            would_have_picked(ev["k"], ev["round"], f"ABANDONED ({ev['reason']})")
            pending = None
        elif t == "void":
            print(f"     item {ev['item']} voided")
    return ok


def load(src):
    if src.startswith("http://") or src.startswith("https://"):
        with urllib.request.urlopen(src, timeout=15) as r:
            return json.load(r)
    with open(src) as f:
        return json.load(f)


def main(argv):
    fetch = "--fetch" in argv
    args = [a for a in argv if not a.startswith("--")]
    if "--vectors" in argv:
        data = load(args[0])
        ok = True
        for v in data["select"]:
            got = select(bytes.fromhex(v["randomness"]), bytes.fromhex(v["manifest_hash"]), v["k"], v["pool_size"])
            if got != v["index"]:
                print(f"FAIL select vector k={v['k']} n={v['pool_size']}: {got} != {v['index']}")
                ok = False
        print(f"{'ok  ' if ok else 'FAIL'} {len(data['select'])} selection vectors")
        s = data["scenario"]
        ok &= verify(s["manifest"], s["manifest_hash"], s["events"])
    else:
        if not args:
            print(__doc__)
            return 2
        data = load(args[0])
        ok = verify(data["manifest"], data["manifest_hash"], data["events"], fetch=fetch)
    print("VERIFIED" if ok else "VERIFICATION FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
