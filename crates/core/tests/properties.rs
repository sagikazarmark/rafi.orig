//! Property tests for the draw state machine.

use std::collections::HashSet;

use proptest::prelude::*;
use rafi_core::{
    commit::{CommitDoc, UnitRef},
    drand::{Beacon, randomness_from_signature},
    state::{DrawError, DrawState, Event, Outcome, UnitStatus},
    verify::verify,
};

fn beacon(round: u64) -> Beacon {
    let sig: Vec<u8> = round
        .to_be_bytes()
        .iter()
        .cycle()
        .take(48)
        .copied()
        .collect();
    Beacon {
        round,
        randomness: hex::encode(randomness_from_signature(&sig)),
        signature: hex::encode(sig),
    }
}

#[derive(Debug, Clone)]
enum Op {
    Draw(usize),
    Resolve(usize, u8),
    Fail(usize),
    Void(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..6usize).prop_map(Op::Draw),
        4 => (0..6usize, 0..3u8).prop_map(|(u, o)| Op::Resolve(u, o)),
        1 => (0..6usize).prop_map(Op::Fail),
        1 => (0..6usize).prop_map(Op::Void),
    ]
}

proptest! {
    #[test]
    fn invariants_hold_and_log_verifies(n in 0usize..8, units in 1usize..6, ops in proptest::collection::vec(op(), 0..60)) {
        let doc = CommitDoc::new(
            "prop",
            (0..n).map(|i| format!("{i:032x}")).collect(),
            (0..units).map(|i| UnitRef { unit_id: format!("u{i}"), item_id: "i".into(), item_name: "P".into() }).collect(),
        );
        let mut s = DrawState::freeze(&doc);
        let mut round = 1;
        let mut last_version = s.version;
        for op in ops {
            match op {
                Op::Draw(u) => {
                    let unit = format!("u{}", u % units);
                    round += 1;
                    if s.begin(&unit, round, 0).is_ok() {
                        let _ = s.complete(&beacon(round));
                    }
                }
                Op::Resolve(u, o) => {
                    let outcome = [Outcome::Claimed, Outcome::NotHere, Outcome::Declined][o as usize];
                    let _ = s.resolve(&format!("u{}", u % units), outcome);
                }
                Op::Fail(u) => {
                    round += 1;
                    if s.begin(&format!("u{}", u % units), round, 0).is_ok() {
                        s.fail("test").unwrap();
                    }
                }
                Op::Void(u) => { let _ = s.void(&format!("u{}", u % units)); }
            }
            prop_assert!(s.version >= last_version);
            last_version = s.version;
            prop_assert!(s.pending.is_none());

            // A claimed unit always has a winner; nobody holds two pass-1 claims.
            let mut holders = HashSet::new();
            for unit in &s.units {
                match unit.status {
                    UnitStatus::Drawn | UnitStatus::Claimed => {
                        let e = unit.entrant_id.clone().unwrap();
                        prop_assert!(doc.entrant_ids.contains(&e));
                        holders.insert(e);
                    }
                    _ => prop_assert!(unit.entrant_id.is_none()),
                }
            }

            // Pass-1 draws never repeat an entrant.
            let mut seen = HashSet::new();
            for e in &s.events {
                if let Event::Draw { pass: 1, entrant_id, .. } = e {
                    prop_assert!(seen.insert(entrant_id.clone()), "entrant drawn twice in pass 1");
                }
            }
        }
        let report = verify(&doc.canonical_json(), &s.list_hash, &s.events);
        prop_assert!(report.ok(), "{report:#?}");
    }

    #[test]
    fn empty_raffle_cannot_draw(units in 1usize..4) {
        let doc = CommitDoc::new("e", vec![], (0..units).map(|i| UnitRef { unit_id: format!("u{i}"), item_id: "i".into(), item_name: "P".into() }).collect());
        let mut s = DrawState::freeze(&doc);
        prop_assert_eq!(s.begin("u0", 1, 0), Err(DrawError::NoEligibleEntrants));
    }
}
