//! Property tests for the raffle's state machine.

use std::{collections::HashSet, num::NonZeroU32, sync::OnceLock};

use drand::{Scheme, TestChain};
use proptest::prelude::*;
use rafi_raffle::{
    BeaconFetch, BeaconSource, Command, Decision, DrawRequest, EntrantId, Error, Event, ItemState,
    Manifest, ManifestHash, ManifestItem, Raffle, Resolution,
    drand::{ChainHash, ChainInfo, VerifiedBeacon, Verifier},
    verify,
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

fn source() -> BeaconSource {
    BeaconSource::Drand(chain().0.chain().hash())
}

fn item(i: usize) -> rafi_raffle::ItemId {
    format!("item-{i}").into()
}

/// The engine protocol's calls, as an engine makes them.
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
        4 => (0..6usize).prop_map(Op::Announce),
        3 => Just(Op::Complete),
        2 => Just(Op::Stall),
        1 => Just(Op::Abandon),
        4 => (0..6usize, 0..3u8).prop_map(|(u, o)| Op::Resolve(u, o)),
        1 => (0..6usize).prop_map(Op::Void),
        1 => Just(Op::Finish),
    ]
}

/// Every command an organizer could send for a raffle with `items` items.
fn candidates(items: usize, pending: Option<rafi_raffle::ItemId>) -> Vec<Command> {
    let mut out = Vec::new();
    for i in 0..items {
        out.push(Command::Draw(DrawRequest { item: item(i) }));
        for resolution in [
            Resolution::Claimed,
            Resolution::NotHere,
            Resolution::Declined,
        ] {
            out.push(Command::Decide(Decision::Resolve {
                item: item(i),
                resolution,
            }));
        }
        out.push(Command::Decide(Decision::Void { item: item(i) }));
    }
    if let Some(item) = pending {
        out.push(Command::Decide(Decision::Abandon { item }));
    }
    out.push(Command::Decide(Decision::Finish));
    out
}

/// Everything the pending draw's outcome depends on.
fn draw_inputs(
    s: &Raffle,
) -> (
    Option<u64>,
    (u8, Vec<EntrantId>),
    BeaconSource,
    ManifestHash,
) {
    (
        s.pending().map(|p| p.k),
        s.pool(),
        *s.beacon(),
        *s.manifest_hash(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn invariants_hold_and_log_verifies(n in 1usize..8, items in 1usize..6, ops in proptest::collection::vec(op(), 0..40)) {
        let manifest = Manifest::new(
            "prop",
            source(),
            (0..n).map(|i| format!("{i:032x}").into()).collect(),
            (0..items).map(|i| ManifestItem { id: item(i), name: format!("Item {i}") }).collect(),
        );
        let mut s = Raffle::open(&manifest).unwrap();
        let mut round = 1;
        for op in ops {
            // What the raffle admits, its engine can apply: a command that
            // passes the app's check never fails at the engine.
            for command in candidates(items, s.pending().map(|p| p.item.clone())) {
                if s.admit(&command).is_ok() {
                    let mut real = s.clone();
                    let applied = match &command {
                        Command::Draw(request) => real.announce(request, &source(), round + 1, 0).map(|_| ()),
                        Command::Decide(decision) => real.decide(decision),
                    };
                    prop_assert!(applied.is_ok(), "{:?} admitted, but applying it failed: {:?}", command, applied);
                }
            }
            let pending_before = s.pending().cloned();
            let inputs_before = draw_inputs(&s);
            let events_before = s.events().len();
            let version_before = s.version();
            match op {
                Op::Announce(i) => {
                    round += 1;
                    let request = DrawRequest { item: item(i % items) };
                    let _ = s.announce(&request, &source(), round, 0);
                }
                Op::Complete => {
                    if let Some(p) = s.pending().cloned() {
                        let _ = s.settle(BeaconFetch::Verified(beacon(p.round)));
                    }
                }
                Op::Stall => { let _ = s.settle(BeaconFetch::Unavailable("relays down".into())); }
                Op::Abandon => {
                    if let Some(p) = s.pending().cloned() {
                        let _ = s.decide(&Decision::Abandon { item: p.item });
                    }
                }
                Op::Resolve(i, o) => {
                    let resolution = [Resolution::Claimed, Resolution::NotHere, Resolution::Declined][o as usize];
                    let _ = s.decide(&Decision::Resolve { item: item(i % items), resolution });
                }
                Op::Void(i) => { let _ = s.decide(&Decision::Void { item: item(i % items) }); }
                Op::Finish => { let _ = s.decide(&Decision::Finish); }
            }
            prop_assert_eq!(s.version() - version_before, (s.events().len() - events_before) as u64);

            // While a draw is pending, nothing may change its inputs, and it
            // only ends by a draw event or an explicit abandon.
            if let Some(p) = &pending_before {
                match s.pending() {
                    Some(now) => {
                        prop_assert_eq!(now, p);
                        prop_assert_eq!(&draw_inputs(&s), &inputs_before);
                    }
                    None => {
                        let ended = &s.events()[events_before..];
                        prop_assert!(
                            matches!(ended, [Event::Drawn { .. }] | [Event::Abandoned { .. }]),
                            "pending draw ended by {:?}", ended
                        );
                    }
                }
            }

            // A drawn or claimed item's winner is an entrant.
            for item in s.items() {
                if let ItemState::Drawn { winner, .. } | ItemState::Claimed { winner, .. } = &item.state {
                    prop_assert!(manifest.entrants.contains(winner));
                }
            }

            // Pass-1 draws never repeat an entrant.
            let mut seen = HashSet::new();
            for d in s.draws() {
                if d.pass == 1 {
                    prop_assert!(seen.insert(d.entrant.clone()), "entrant drawn twice in pass 1");
                }
            }
        }
        let report = verify(&manifest.canonical_json(), &manifest.hash().to_string(), s.events(), resolve);
        prop_assert!(report.ok(), "{report:#?}");
    }

    #[test]
    fn a_raffle_without_entrants_does_not_open(items in 1usize..4) {
        let manifest = Manifest::new(
            "e",
            source(),
            vec![],
            (0..items).map(|i| ManifestItem { id: item(i), name: format!("Item {i}") }).collect(),
        );
        prop_assert!(matches!(Raffle::open(&manifest), Err(Error::InvalidManifest(_))));
    }
}
