//! Property tests: the lifecycle's commands and the drawing it follows agree.

use std::{num::NonZeroU32, sync::OnceLock};

use drand::{Scheme, TestChain};
use proptest::prelude::*;
use rafi_drawing::{
    BeaconFetch, BeaconSource, Decision, DrawRequest, Drawing, Resolution,
    drand::{VerifiedBeacon, Verifier},
};
use rafi_lifecycle::{
    Command, RaffleState,
    drawing::{deliverable, items, lifecycle, manifest},
};

fn chain() -> &'static (TestChain, Verifier) {
    static CHAIN: OnceLock<(TestChain, Verifier)> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let tc = TestChain::new(
            [7; 32],
            Scheme::BlsUnchainedG1Rfc9380,
            NonZeroU32::new(3).unwrap(),
            0,
            "lifecycle",
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

fn source() -> BeaconSource {
    BeaconSource::Drand(chain().0.chain().hash())
}

fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        Just(Command::Publish),
        Just(Command::Unpublish),
        Just(Command::Open),
        Just(Command::Close),
        Just(Command::Freeze),
        Just(Command::Purge),
    ]
}

/// An engine's calls on the drawing.
#[derive(Debug, Clone)]
enum Op {
    Announce(usize),
    Complete,
    Stall,
    Abandon,
    Resolve(usize, u8),
    Void(usize),
    Finish,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..4usize).prop_map(Op::Announce),
        3 => Just(Op::Complete),
        1 => Just(Op::Stall),
        1 => Just(Op::Abandon),
        3 => (0..4usize, 0..3u8).prop_map(|(i, o)| Op::Resolve(i, o)),
        1 => (0..4usize).prop_map(Op::Void),
        1 => Just(Op::Finish),
    ]
}

fn apply(d: &mut Drawing, op: Op, round: &mut u64, item: impl Fn(usize) -> String) {
    match op {
        Op::Announce(i) => {
            *round += 1;
            let request = DrawRequest {
                item: item(i).into(),
            };
            let _ = d.announce(&request, &source(), *round, 0);
        }
        Op::Complete => {
            if let Some(p) = d.pending().cloned() {
                let _ = d.settle(BeaconFetch::Verified(beacon(p.round)));
            }
        }
        Op::Stall => {
            let _ = d.settle(BeaconFetch::Unavailable("relays down".into()));
        }
        Op::Abandon => {
            if let Some(p) = d.pending().cloned() {
                let _ = d.decide(&Decision::Abandon { item: p.item });
            }
        }
        Op::Resolve(i, o) => {
            let resolution = [
                Resolution::Claimed,
                Resolution::NotHere,
                Resolution::Declined,
            ][o as usize];
            let _ = d.decide(&Decision::Resolve {
                item: item(i).into(),
                resolution,
            });
        }
        Op::Void(i) => {
            let _ = d.decide(&Decision::Void {
                item: item(i).into(),
            });
        }
        Op::Finish => {
            let _ = d.decide(&Decision::Finish);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Commands never move a raffle into the states its drawing decides,
    /// and nothing is editable once the list is frozen.
    #[test]
    fn commands_leave_drawing_and_finishing_to_the_drawing(commands in proptest::collection::vec(command(), 0..30)) {
        let mut state = RaffleState::Draft;
        for command in commands {
            let before = state;
            if let Some(next) = state.apply(command) {
                state = next;
            }
            prop_assert!(
                !matches!(state, RaffleState::Drawing | RaffleState::Finished) || state == before,
                "{before:?} -{command:?}-> {state:?}"
            );
            if state == RaffleState::Frozen && before != RaffleState::Frozen {
                prop_assert_eq!(RaffleState::Frozen.unfreeze(before), Some(before));
            }
        }
        for s in RaffleState::ALL {
            if s.follows_drawing() || s.is_over() {
                prop_assert!(!s.entrants_editable() && !s.prizes_editable() && !s.test_mode_editable());
            }
        }
    }

    /// Wherever the drawing stands, the raffle following it is in a state
    /// that follows it until it finishes, can draw exactly until then, and
    /// never goes back.
    #[test]
    fn the_raffle_follows_its_drawing(entrants in 1usize..6, prizes in 1u64..4, ops in proptest::collection::vec(op(), 0..40)) {
        let manifest = manifest(
            "r1",
            source(),
            (0..entrants).map(|i| format!("{i:032x}")),
            items([("p", "Prize", prizes)]),
        );
        let mut d = Drawing::open(&manifest).unwrap();
        let item = |i: usize| format!("p-{}", i as u64 % prizes + 1);
        let mut round = 1;
        let mut previous = lifecycle(&d);
        prop_assert_eq!(previous, RaffleState::Frozen);
        for op in ops {
            apply(&mut d, op, &mut round, item);
            let state = lifecycle(&d);
            prop_assert!(state.follows_drawing() || state == RaffleState::Finished);
            prop_assert_eq!(state.drawable(), !d.is_finished());
            prop_assert!(RaffleState::ALL.iter().position(|s| *s == state) >= RaffleState::ALL.iter().position(|s| *s == previous), "{previous:?} -> {state:?}");
            prop_assert!(d.winners().is_subset(&d.drawn_entrants()));
            for e in d.entrants() {
                prop_assert_eq!(
                    deliverable(Some(&d), e.as_str()),
                    d.winners().contains(e),
                    "only a winner holding a claimed item is delivered to"
                );
            }
            previous = state;
        }
    }
}
