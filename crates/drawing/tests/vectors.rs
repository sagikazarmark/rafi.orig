//! Shared test vectors for `rafi-draw-v1`.
//!
//! `testdata/draw-vectors.json` is checked by this test, by `tools/verify.py`
//! and (through `rafi-drawing` compiled to WASM) by the browser verifier. Run with
//! `UPDATE_VECTORS=1` to regenerate after an intentional algorithm change,
//! which must also bump the algorithm identifier.
//!
//! The scenario draws from real quicknet beacons (`testdata/quicknet-beacons.json`),
//! so the browser verifier checks their BLS signatures too. It was regenerated
//! when that replaced fake signatures; `rafi-draw-v1` itself did not change.
//! It follows the state machine's fairness rules: a stalled draw keeps its
//! round, and "not here"/"declined" are never resolved while a draw is pending
//! (a log doing that fails verification).

use rafi_drawing::{
    commit::{CommitDoc, UnitRef},
    drand::{Beacon, VerifiedBeacon, Verifier, chains},
    draw,
    state::{DrawState, Outcome},
    verify::verify_public,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct SelectVector {
    randomness: String,
    list_hash: String,
    k: u64,
    pool_size: usize,
    index: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Scenario {
    commit_doc: String,
    list_hash: String,
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
        let list_hash = h(&format!("list-{i}"));
        let k = i as u64 + 1;
        select.push(SelectVector {
            randomness: hex::encode(randomness),
            list_hash: hex::encode(list_hash),
            k,
            pool_size: *pool_size,
            index: draw::select(&randomness, &list_hash, k, *pool_size),
        });
    }

    let entrants = (0..12)
        .map(|i| hex::encode(&h(&format!("entrant-{i}"))[..16]))
        .collect();
    let units = vec![
        UnitRef {
            unit_id: "u-ticket-1".into(),
            item_id: "ticket".into(),
            item_name: "KCD Budapest ticket".into(),
        },
        UnitRef {
            unit_id: "u-ticket-2".into(),
            item_id: "ticket".into(),
            item_name: "KCD Budapest ticket".into(),
        },
        UnitRef {
            unit_id: "u-voucher-1".into(),
            item_id: "voucher".into(),
            item_name: "CNCF exam voucher".into(),
        },
        UnitRef {
            unit_id: "u-gift-1".into(),
            item_id: "gift".into(),
            item_name: "Foo gift".into(),
        },
    ];
    let doc = CommitDoc::new("raffle-vectors", &chains::QUICKNET_HASH, entrants, units);
    let mut s = DrawState::freeze(&doc);
    s.begin("u-voucher-1", 1000, 0).unwrap();
    s.complete(&beacon(1000)).unwrap();
    s.resolve("u-voucher-1", Outcome::NotHere).unwrap();
    s.begin("u-voucher-1", 1003, 0).unwrap();
    s.abandon("abandoned by the organizer").unwrap();
    s.begin("u-voucher-1", 1005, 0).unwrap();
    s.stall("drand unavailable").unwrap();
    s.complete(&beacon(1005)).unwrap();
    s.resolve("u-voucher-1", Outcome::Claimed).unwrap();
    s.begin("u-ticket-1", 1010, 0).unwrap();
    s.complete(&beacon(1010)).unwrap();
    s.resolve("u-ticket-1", Outcome::Declined).unwrap();
    s.begin("u-ticket-1", 1012, 0).unwrap();
    s.complete(&beacon(1012)).unwrap();
    s.resolve("u-ticket-1", Outcome::Claimed).unwrap();
    s.void("u-gift-1").unwrap();
    s.begin("u-ticket-2", 1020, 0).unwrap();
    s.complete(&beacon(1020)).unwrap();
    s.resolve("u-ticket-2", Outcome::Claimed).unwrap();
    s.finish().unwrap();

    Vectors {
        algo: draw::ALGO.into(),
        select,
        scenario: Scenario {
            commit_doc: doc.canonical_json(),
            list_hash: s.list_hash.clone(),
            events: serde_json::to_value(&s.events).unwrap(),
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
        let r = rafi_drawing::hash_from_hex(&v.randomness).unwrap();
        let l = rafi_drawing::hash_from_hex(&v.list_hash).unwrap();
        assert_eq!(draw::select(&r, &l, v.k, v.pool_size), v.index);
    }
    let events = serde_json::from_value::<Vec<_>>(stored.scenario.events.clone()).unwrap();
    let report = verify_public(
        &stored.scenario.commit_doc,
        &stored.scenario.list_hash,
        &events,
    );
    assert!(report.ok(), "{report:#?}");
}
