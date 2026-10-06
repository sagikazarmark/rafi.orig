#!/usr/bin/env python3
"""Independent verifier for Rafi draws (rafi-draw-v1).

Usage:
    verify.py verify.json              # file downloaded from /api/r/<slug>/verify.json
    verify.py https://host/api/r/<slug>/verify.json
    verify.py --fetch verify.json      # also re-fetch every beacon from drand
    verify.py --vectors testdata/draw-vectors.json

Standard library only. The algorithm:

    list_hash    = SHA256(commit_doc)                    (published canonical JSON string)
    seed_k       = SHA256("rafi-draw-v1" || randomness || list_hash || BE_u64(k))
    rand_u64(c)  = BE_u64(SHA256(seed_k || BE_u64(c))[0:8])
    uniform(n)   = first rand_u64(c) below 2^64 - (2^64 mod n), taken mod n
    winner_k     = pool_k[uniform(len(pool_k))]

    pool (pass 1) = entrants never drawn before (whatever the outcome)
    pool (pass 2) = pass-1 winners not forfeited/declined, not yet drawn in pass 2
    randomness    = SHA256(drand signature)
"""

import hashlib
import json
import sys
import urllib.request

ALGO = b"rafi-draw-v1"
QUICKNET = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971"
RELAYS = ["https://api.drand.sh", "https://drand.cloudflare.com"]


def sha256(*parts: bytes) -> bytes:
    h = hashlib.sha256()
    for p in parts:
        h.update(p)
    return h.digest()


def select(randomness: bytes, list_hash: bytes, k: int, n: int) -> int:
    seed = sha256(ALGO, randomness, list_hash, k.to_bytes(8, "big"))
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


def fetch_beacon(round_):
    for relay in RELAYS:
        try:
            with urllib.request.urlopen(f"{relay}/{QUICKNET}/public/{round_}", timeout=10) as r:
                return json.load(r)
        except Exception:  # noqa: BLE001 - try the next relay
            continue
    return None


def verify(commit_doc: str, expected_list_hash: str, events, fetch=False) -> bool:
    ok = True
    list_hash = sha256(commit_doc.encode())
    if list_hash.hex() != expected_list_hash.lower():
        print(f"FAIL list hash: computed {list_hash.hex()}, published {expected_list_hash}")
        ok = False
    else:
        print(f"ok   list hash {list_hash.hex()}")
    doc = json.loads(commit_doc)
    entrants = doc["entrant_ids"]
    print(f"     {len(entrants)} entrants, {len(doc['units'])} prize units, chain {doc['drand_chain'][:12]}…")

    draws = []  # in order
    by_k = {}
    for ev in events:
        t = ev["type"]
        if t == "draw":
            k = ev["k"]
            pass_, p = pool(entrants, draws)
            if not p:
                print(f"FAIL draw {k}: no eligible entrants, but a winner was published")
                ok = False
                continue
            sig = bytes.fromhex(ev["signature"])
            randomness = bytes.fromhex(ev["randomness"])
            if ev["mode"] == "beacon":
                if sha256(sig) != randomness:
                    print(f"FAIL draw {k}: randomness != SHA256(signature)")
                    ok = False
                if fetch:
                    b = fetch_beacon(ev["round"])
                    if b is None:
                        print(f"WARN draw {k}: could not fetch drand round {ev['round']}")
                    elif b["signature"].lower() != ev["signature"].lower():
                        print(f"FAIL draw {k}: signature differs from drand round {ev['round']}")
                        ok = False
            else:
                print(f"WARN draw {k}: OFFLINE mode, randomness is not publicly verifiable")
            idx = select(randomness, list_hash, k, len(p))
            expected = p[idx]
            good = expected == ev["entrant_id"] and pass_ == ev["pass"] and len(p) == ev["pool_size"]
            ok &= good
            print(f"{'ok  ' if good else 'FAIL'} draw {k}: round {ev['round']}, pass {pass_}, pool {len(p)}, "
                  f"index {idx} -> {expected}" + ("" if good else f" (published {ev['entrant_id']})"))
            d = {"pass": pass_, "entrant": expected, "outcome": "drawn"}
            draws.append(d)
            by_k[k] = d
        elif t == "resolve":
            if ev["k"] in by_k:
                by_k[ev["k"]]["outcome"] = ev["outcome"]
                print(f"     draw {ev['k']}: {ev['outcome'].replace('_', ' ')}")
        elif t == "fail":
            print(f"     draw {ev['k']} on round {ev['round']} failed: {ev['reason']}")
        elif t == "void":
            print(f"     unit {ev['unit_id']} voided")
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
            got = select(bytes.fromhex(v["randomness"]), bytes.fromhex(v["list_hash"]), v["k"], v["pool_size"])
            if got != v["index"]:
                print(f"FAIL select vector k={v['k']} n={v['pool_size']}: {got} != {v['index']}")
                ok = False
        print(f"{'ok  ' if ok else 'FAIL'} {len(data['select'])} selection vectors")
        s = data["scenario"]
        ok &= verify(s["commit_doc"], s["list_hash"], s["events"])
    else:
        if not args:
            print(__doc__)
            return 2
        data = load(args[0])
        ok = verify(data["commit_doc"], data["list_hash"], data["events"], fetch=fetch)
    print("VERIFIED" if ok else "VERIFICATION FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
