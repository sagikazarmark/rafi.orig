//! Shared test vectors for `rafi-draw-v1`.
//!
//! `testdata/draw-vectors.json` is checked by this test, by `tools/verify.py`
//! and (through `rafi-drawing` compiled to WASM) by the browser verifier. Run with
//! `UPDATE_VECTORS=1` to regenerate after an intentional change: an algorithm
//! change must also bump the algorithm identifier; a manifest format change
//! changes the manifest hash, and so which entrants the scenario picks.
//!
//! The scenario draws from real quicknet beacons (`testdata/quicknet-beacons.json`),
//! so the browser verifier checks their BLS signatures too. It was regenerated
//! when that replaced fake signatures; `rafi-draw-v1` itself did not change.
//! It follows the state machine's fairness rules: a stalled draw keeps its
//! round, and "not here"/"declined" are never resolved while a draw is pending
//! (a log doing that fails verification).

use rafi_drawing::{
    BeaconFetch, BeaconSource, Decision, DrawRequest, Drawing, Manifest, ManifestItem, Resolution,
    algorithm,
    drand::{Beacon, VerifiedBeacon, Verifier, chains},
    verify_public,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct SelectVector {
    randomness: String,
    manifest_hash: String,
    k: u64,
    pool_size: usize,
    index: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Scenario {
    manifest: String,
    manifest_hash: String,
    events: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Vectors {
    algo: String,
    select: Vec<SelectVector>,
    scenario: Scenario,
}

fn h(label: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(label.as_bytes()).into()
}

/// A real quicknet beacon from `testdata/quicknet-beacons.json`, verified.
fn beacon(round: u64) -> VerifiedBeacon {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/quicknet-beacons.json"
    );
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("beacons file")).unwrap();
    let b = data["beacons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["round"] == round)
        .expect("beacon in testdata");
    let beacon: Beacon = serde_json::from_value(b.clone()).unwrap();
    Verifier::new(&chains::quicknet())
        .unwrap()
        .verify_round(round, &beacon)
        .unwrap()
}

fn generate() -> Vectors {
    let mut select = Vec::new();
    for (i, pool_size) in [1usize, 2, 3, 7, 10, 31, 100, 257, 1000, 4_000_000_000]
        .iter()
        .enumerate()
    {
        let randomness = h(&format!("randomness-{i}"));
        let manifest_hash = h(&format!("manifest-{i}"));
        let k = i as u64 + 1;
        select.push(SelectVector {
            randomness: hex::encode(randomness),
            manifest_hash: hex::encode(manifest_hash),
            k,
            pool_size: *pool_size,
            index: algorithm::select(&randomness, &manifest_hash, k, *pool_size),
        });
    }

    let entrants = (0..12)
        .map(|i| hex::encode(&h(&format!("entrant-{i}"))[..16]).into())
        .collect();
    let items = vec![
        ManifestItem {
            id: "book-1".into(),
            name: "Book".into(),
        },
        ManifestItem {
            id: "book-2".into(),
            name: "Book".into(),
        },
        ManifestItem {
            id: "voucher-1".into(),
            name: "Voucher".into(),
        },
        ManifestItem {
            id: "mug-1".into(),
            name: "Mug".into(),
        },
    ];
    let doc = Manifest::new(
        "drawing-vectors",
        BeaconSource::Drand(chains::QUICKNET_HASH),
        entrants,
        items,
    );
    let mut s = Drawing::open(&doc).unwrap();
    let source = BeaconSource::QUICKNET;
    let draw = |s: &mut Drawing, item: &str, round: u64| {
        let request = DrawRequest { item: item.into() };
        s.announce(&request, &source, round, 0).unwrap();
        s.settle(BeaconFetch::Verified(beacon(round)))
            .unwrap()
            .unwrap();
    };
    let decide = |s: &mut Drawing, decision: Decision| s.decide(&decision).unwrap();
    let resolve = |item: &str, resolution| Decision::Resolve {
        item: item.into(),
        resolution,
    };
    draw(&mut s, "voucher-1", 1000);
    decide(&mut s, resolve("voucher-1", Resolution::NotHere));
    let voucher = DrawRequest {
        item: "voucher-1".into(),
    };
    s.announce(&voucher, &source, 1003, 0).unwrap();
    decide(
        &mut s,
        Decision::Abandon {
            item: "voucher-1".into(),
        },
    );
    s.announce(&voucher, &source, 1005, 0).unwrap();
    s.settle(BeaconFetch::Interrupted("drand unavailable".into()))
        .unwrap();
    s.settle(BeaconFetch::Verified(beacon(1005)))
        .unwrap()
        .unwrap();
    decide(&mut s, resolve("voucher-1", Resolution::Claimed));
    draw(&mut s, "book-1", 1010);
    decide(&mut s, resolve("book-1", Resolution::Declined));
    draw(&mut s, "book-1", 1012);
    decide(&mut s, resolve("book-1", Resolution::Claimed));
    decide(
        &mut s,
        Decision::Void {
            item: "mug-1".into(),
        },
    );
    draw(&mut s, "book-2", 1020);
    decide(&mut s, resolve("book-2", Resolution::Claimed));
    decide(&mut s, Decision::Finish);

    Vectors {
        algo: algorithm::ALGO.into(),
        select,
        scenario: Scenario {
            manifest: doc.canonical_json(),
            manifest_hash: s.manifest_hash().to_string(),
            events: serde_json::to_value(s.events()).unwrap(),
        },
    }
}

#[test]
fn vectors_match() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/draw-vectors.json"
    );
    let generated = generate();
    if std::env::var_os("UPDATE_VECTORS").is_some() {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&generated).unwrap() + "\n",
        )
        .unwrap();
    }
    let stored: Vectors =
        serde_json::from_str(&std::fs::read_to_string(path).expect("vectors file")).unwrap();
    assert_eq!(
        stored, generated,
        "test vectors changed; see the module docs"
    );

    for v in &stored.select {
        let r = hex::decode(&v.randomness).unwrap().try_into().unwrap();
        let l = hex::decode(&v.manifest_hash).unwrap().try_into().unwrap();
        assert_eq!(algorithm::select(&r, &l, v.k, v.pool_size), v.index);
    }
    let events = serde_json::from_value::<Vec<_>>(stored.scenario.events.clone()).unwrap();
    let report = verify_public(
        &stored.scenario.manifest,
        &stored.scenario.manifest_hash,
        &events,
    );
    assert!(report.ok(), "{report:#?}");
}
