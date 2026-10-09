//! Property tests for the draw state machine.

use std::{collections::HashSet, num::NonZeroU32, sync::OnceLock};

use drand::{Scheme, TestChain};
use proptest::prelude::*;
use rafi_core::{
    commit::{CommitDoc, UnitRef},
    drand::{ChainHash, ChainInfo, VerifiedBeacon, Verifier},
    state::{DrawError, DrawState, Event, Outcome, UnitStatus},
    verify::verify,
};

fn chain() -> &'static (TestChain, Verifier) {
    static CHAIN: OnceLock<(TestChain, Verifier)> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let tc = TestChain::new(
            [5; 32],
            Scheme::BlsUnchainedG1Rfc9380,
            NonZeroU32::new(3).unwrap(),
            0,
            "props",
        )
        .unwrap();
        let v = Verifier::new(tc.chain()).unwrap();
        (tc, v)
    })
}

fn beacon(round: u64) -> VerifiedBeacon {
    let (tc, v) = chain();
    v.verify_round(round, &tc.sign(round)).unwrap()
}

fn resolve(h: &ChainHash) -> Option<ChainInfo> {
    let info = chain().0.chain().clone();
    (info.hash() == *h).then_some(info)
}

#[derive(Debug, Clone)]
enum Op {
    Begin(usize),
    Complete,
    Stall,
    Abandon,
    Offline,
    Resolve(usize, u8),
    Void(usize),
    Finish,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..6usize).prop_map(Op::Begin),
        3 => Just(Op::Complete),
        2 => Just(Op::Stall),
        1 => Just(Op::Abandon),
        1 => Just(Op::Offline),
        4 => (0..6usize, 0..3u8).prop_map(|(u, o)| Op::Resolve(u, o)),
        1 => (0..6usize).prop_map(Op::Void),
        1 => Just(Op::Finish),
    ]
}

/// Everything the pending draw's outcome depends on.
fn draw_inputs(s: &DrawState) -> (u64, (u8, Vec<String>), String, String) {
    (
        s.next_k,
        s.pool(),
        s.drand_chain.clone(),
        s.list_hash.clone(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn invariants_hold_and_log_verifies(n in 0usize..8, units in 1usize..6, ops in proptest::collection::vec(op(), 0..40)) {
        let doc = CommitDoc::new(
            "prop",
            &chain().0.chain().hash(),
            (0..n).map(|i| format!("{i:032x}")).collect(),
            (0..units).map(|i| UnitRef { unit_id: format!("u{i}"), item_id: "i".into(), item_name: "P".into() }).collect(),
        );
        let mut s = DrawState::freeze(&doc);
        let mut round = 1;
        let mut last_version = s.version;
        for op in ops {
            let pending_before = s.pending.clone();
            let inputs_before = draw_inputs(&s);
            let events_before = s.events.len();
            match op {
                Op::Begin(u) => {
                    round += 1;
                    let _ = s.begin(&format!("u{}", u % units), round, 0);
                }
                Op::Complete => {
                    if let Some(p) = s.pending.clone() {
                        let _ = s.complete(&beacon(p.round));
                    }
                }
                Op::Stall => { let _ = s.stall("unavailable"); }
                Op::Abandon => { let _ = s.abandon("abandoned"); }
                Op::Offline => { let _ = s.complete_offline([round as u8; 32]); }
                Op::Resolve(u, o) => {
                    let outcome = [Outcome::Claimed, Outcome::NotHere, Outcome::Declined][o as usize];
                    let _ = s.resolve(&format!("u{}", u % units), outcome);
                }
                Op::Void(u) => { let _ = s.void(&format!("u{}", u % units)); }
                Op::Finish => { let _ = s.finish(); }
            }
            prop_assert!(s.version >= last_version);
            last_version = s.version;

            // While a draw is pending, nothing may change its inputs, and it
            // only ends by a draw event or an explicit abandon.
            if let Some(p) = &pending_before {
                match &s.pending {
                    Some(now) => {
                        prop_assert_eq!(now, p);
                        prop_assert_eq!(&draw_inputs(&s), &inputs_before);
                    }
                    None => {
                        let ended = &s.events[events_before..];
                        prop_assert!(
                            matches!(ended, [Event::Draw { .. }] | [Event::Abandon { .. }]),
                            "pending draw ended by {:?}", ended
                        );
                    }
                }
            }

            // A claimed unit always has a winner; nobody holds two pass-1 claims.
            for unit in &s.units {
                match unit.status {
                    UnitStatus::Drawn | UnitStatus::Claimed => {
                        let e = unit.entrant_id.clone().unwrap();
                        prop_assert!(doc.entrant_ids.contains(&e));
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
        let report = verify(&doc.canonical_json(), &s.list_hash, &s.events, resolve);
        prop_assert!(report.ok(), "{report:#?}");
    }

    #[test]
    fn empty_raffle_cannot_draw(units in 1usize..4) {
        let doc = CommitDoc::new("e", &chain().0.chain().hash(), vec![], (0..units).map(|i| UnitRef { unit_id: format!("u{i}"), item_id: "i".into(), item_name: "P".into() }).collect());
        let mut s = DrawState::freeze(&doc);
        prop_assert_eq!(s.begin("u0", 1, 0), Err(DrawError::NoEligibleEntrants));
    }
}
