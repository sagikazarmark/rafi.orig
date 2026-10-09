//! Independent verification of a published draw log.
//!
//! Replays the events against the manifest and checks that every draw
//! picked exactly the entrant the algorithm dictates, from a drand beacon that
//! verifies against the committed chain. This is what the public verification
//! page runs in the browser (compiled to WASM) and what `tools/verify.py`
//! reimplements (without the BLS check).

use serde::{Deserialize, Serialize};

use crate::{
    beacon::BeaconSource,
    drand::{Beacon, ChainHash, ChainInfo, Verifier, chains},
    drawing::{Drawing, Event},
    hash::ManifestHash,
    id::{EntrantId, ItemId},
    manifest::Manifest,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawCheck {
    pub k: u64,
    pub item: ItemId,
    pub round: u64,
    pub pool_size: usize,
    pub expected_entrant: EntrantId,
    pub published_entrant: EntrantId,
    pub ok: bool,
}

/// A draw the organizer abandoned before its round's beacon completed it.
/// Anyone can fetch the round and run the algorithm on `pool` to see whom it
/// would have picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbandonedDraw {
    pub k: u64,
    pub item: ItemId,
    pub round: u64,
    pub pass: u8,
    /// The pool at the time, sorted (as the algorithm indexes it).
    pub pool: Vec<EntrantId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub manifest_hash: ManifestHash,
    pub manifest_hash_ok: bool,
    /// Where the randomness came from, per the manifest (`None` if the
    /// manifest could not be read).
    pub beacon: Option<BeaconSource>,
    /// Whether every beacon was BLS-verified against a drand chain the
    /// verifier knows. False for test chains: the draws replay, but their
    /// randomness is not publicly verifiable, so the log is never reported
    /// as verified.
    pub beacons_verified: bool,
    pub draws: Vec<DrawCheck>,
    /// The draws the organizer abandoned.
    pub abandoned: Vec<AbandonedDraw>,
    /// Problems found while replaying, if any.
    pub errors: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.manifest_hash_ok
            && self.beacons_verified
            && self.errors.is_empty()
            && self.draws.iter().all(|d| d.ok)
    }
}

/// [`verify`] against the public drand chains this build knows.
pub fn verify_public(
    manifest_json: &str,
    expected_manifest_hash: &str,
    events: &[Event],
) -> Report {
    verify(
        manifest_json,
        expected_manifest_hash,
        events,
        chains::by_hash,
    )
}

/// Verifies `events` against the published canonical manifest.
///
/// `expected_manifest_hash` is the hash that was published when the drawing
/// opened; `resolve` maps the manifest's chain hash to trusted chain info.
///
/// The log is replayed through the drawing's own transitions: each published
/// event must be exactly the one the drawing records for it, so every rule
/// and every field (the winner, pass, pool size and index of a draw) is
/// checked by the code that drew.
pub fn verify(
    manifest_json: &str,
    expected_manifest_hash: &str,
    events: &[Event],
    resolve: impl Fn(&ChainHash) -> Option<ChainInfo>,
) -> Report {
    let computed = ManifestHash::of(manifest_json);
    let mut report = Report {
        manifest_hash_ok: expected_manifest_hash.parse() == Ok(computed),
        manifest_hash: computed,
        beacon: None,
        beacons_verified: false,
        draws: Vec::new(),
        abandoned: Vec::new(),
        errors: Vec::new(),
    };
    let manifest: Manifest = match serde_json::from_str(manifest_json) {
        Ok(manifest) => manifest,
        Err(e) => {
            report.errors.push(format!("manifest is not valid: {e}"));
            return report;
        }
    };
    if manifest.canonical_json() != manifest_json {
        report
            .errors
            .push("manifest is not in canonical form".into());
    }
    if let Err(e) = manifest.validate() {
        report.errors.push(e.to_string());
    }
    report.beacon = Some(manifest.beacon);
    let verifier = match manifest.beacon.drand_chain() {
        Some(&hash) => match resolve(&hash) {
            Some(info) if info.hash() == hash => match Verifier::new(&info) {
                Ok(v) => Some(v),
                Err(e) => {
                    report.errors.push(format!("drand chain {hash}: {e}"));
                    None
                }
            },
            Some(_) => {
                report
                    .errors
                    .push("resolved chain info does not match the chain hash".into());
                None
            }
            None => None,
        },
        None => None,
    };
    report.beacons_verified = verifier.is_some();

    let mut state = Drawing::open_unchecked(&manifest);
    for event in events {
        let replayed = match event {
            Event::Announced {
                item,
                round,
                round_time,
                ..
            } => state.start(item, *round, *round_time).map(drop),
            Event::Drawn {
                k,
                item,
                round,
                signature,
                entrant,
                ..
            } => {
                let beacon = match Beacon::from_hex(*round, signature, None) {
                    Ok(beacon) => beacon,
                    Err(e) => {
                        report
                            .errors
                            .push(format!("draw {k}: invalid signature: {e}"));
                        continue;
                    }
                };
                let drawn = match &verifier {
                    Some(v) => match v.verify_round(*round, &beacon) {
                        Ok(verified) => state.complete(&verified),
                        Err(e) => {
                            report.errors.push(format!(
                                "draw {k}: drand round {round} does not verify: {e}"
                            ));
                            continue;
                        }
                    },
                    None => state.complete_unverified(*round, &beacon.signature),
                };
                drawn.map(|d| {
                    report.draws.push(DrawCheck {
                        k: *k,
                        item: item.clone(),
                        round: *round,
                        pool_size: d.pool_size,
                        ok: state.events().last() == Some(event),
                        expected_entrant: d.entrant,
                        published_entrant: entrant.clone(),
                    });
                })
            }
            Event::Stalled { reason, .. } => state.stall(reason),
            Event::Abandoned { reason, .. } => {
                record_abandoned(&mut report, &state);
                state.abandon(reason)
            }
            Event::Resolved { k, resolution } => match state.resolve_draw(*k, *resolution) {
                Some(resolved) => resolved,
                None => {
                    report.errors.push(format!("resolve for unknown draw {k}"));
                    continue;
                }
            },
            Event::Voided { item } => state.void(item),
            Event::Finished => state.finish(),
        };
        match replayed {
            Err(e) => report.errors.push(format!("{}: {e}", describe(event))),
            Ok(()) if state.events().last() != Some(event) => {
                report.errors.push(format!(
                    "{}: published as {event:?}, but the replay records {:?}",
                    describe(event),
                    state.events().last()
                ));
            }
            Ok(()) => {}
        }
    }
    report
}

/// An event, briefly, for the report's errors.
fn describe(event: &Event) -> String {
    match event {
        Event::Announced { k, .. } => format!("draw {k} (announce)"),
        Event::Drawn { k, .. } => format!("draw {k}"),
        Event::Stalled { k, .. } => format!("draw {k} (stall)"),
        Event::Abandoned { k, .. } => format!("draw {k} (abandon)"),
        Event::Resolved { k, .. } => format!("draw {k} (resolve)"),
        Event::Voided { item } => format!("void {item}"),
        Event::Finished => "finish".to_owned(),
    }
}

/// Records the pending draw, with its pool, before it is abandoned.
fn record_abandoned(report: &mut Report, state: &Drawing) {
    if let Some(p) = state.pending() {
        let (pass, pool) = state.pool();
        report.abandoned.push(AbandonedDraw {
            k: p.k,
            item: p.item.clone(),
            round: p.round,
            pass,
            pool,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::Resolution,
        drawing::tests::{beacon, manifest, test_chain},
        hash,
    };

    fn resolve(h: &ChainHash) -> Option<ChainInfo> {
        let info = test_chain().chain().clone();
        (info.hash() == *h).then_some(info)
    }

    fn setup() -> (Manifest, Drawing) {
        let manifest = manifest(10, 3);
        let state = Drawing::open(&manifest).unwrap();
        (manifest, state)
    }

    fn check(manifest: &Manifest, events: &[Event]) -> Report {
        let json = manifest.canonical_json();
        verify(&json, &manifest.hash().to_string(), events, resolve)
    }

    fn draw(s: &mut Drawing, item: &str, round: u64) {
        s.start(&item.into(), round, 0).unwrap();
        s.complete(&beacon(round)).unwrap();
    }

    #[test]
    fn honest_log_verifies() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        s.resolve(&"item-0".into(), Resolution::NotHere).unwrap();
        s.start(&"item-0".into(), 2, 0).unwrap();
        s.stall("drand unavailable").unwrap();
        s.abandon("abandoned by the organizer").unwrap();
        s.start(&"item-0".into(), 3, 0).unwrap();
        s.stall("timeout").unwrap();
        s.complete(&beacon(3)).unwrap();
        s.resolve(&"item-0".into(), Resolution::Claimed).unwrap();
        s.void(&"item-1".into()).unwrap();
        draw(&mut s, "item-2", 4);
        s.finish().unwrap();

        let report = check(&m, s.events());
        assert!(report.ok(), "{report:#?}");
        assert_eq!(report.draws.len(), 3);
        let abandoned: Vec<_> = report.abandoned.iter().map(|r| r.round).collect();
        assert_eq!(abandoned, [2]);
        assert_eq!(report.abandoned[0].pool.len(), 9);
    }

    #[test]
    fn a_test_chain_replays_but_is_not_ok() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        let report = verify_public(&m.canonical_json(), &m.hash().to_string(), s.events());
        assert!(!report.beacons_verified);
        assert!(!report.ok());
        assert!(report.errors.is_empty(), "{report:#?}");
        assert!(report.draws.iter().all(|d| d.ok));
    }

    #[test]
    fn forged_beacon_is_detected() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        let mut events = s.events().to_vec();
        if let Some(Event::Drawn {
            signature,
            randomness,
            ..
        }) = events.last_mut()
        {
            // Self-consistent but not signed by the chain: sha256 checks pass, BLS fails.
            let mut sig = hex::decode(&*signature).unwrap();
            sig[10] ^= 1;
            *randomness = hex::encode(hash::randomness(&sig));
            *signature = hex::encode(sig);
        }
        let report = check(&m, &events);
        assert!(!report.ok());
        assert!(!report.errors.is_empty());
    }

    #[test]
    fn randomness_that_is_not_the_signatures_is_detected() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        let mut events = s.events().to_vec();
        if let Some(Event::Drawn { randomness, .. }) = events.last_mut() {
            *randomness = hex::encode([1; 32]);
        }
        assert!(!check(&m, &events).ok());
    }

    #[test]
    fn silent_round_switch_is_detected() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        // A forged log drawing from round 2 although round 1 was announced:
        // a genuine beacon, but a redraw on a new round.
        let mut forged = Drawing::open(&m).unwrap();
        draw(&mut forged, "item-0", 2);
        let mut events = s.events()[..1].to_vec();
        events.push(forged.events()[1].clone());
        assert!(!check(&m, &events).ok());
        // So is a draw without an announcement, or a second announcement.
        assert!(!check(&m, &s.events()[1..]).ok());
        let mut twice = s.events().to_vec();
        twice.insert(1, twice[0].clone());
        assert!(!check(&m, &twice).ok());
    }

    #[test]
    fn tampered_winner_is_detected() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        let mut events = s.events().to_vec();
        if let Some(Event::Drawn { entrant, .. }) = events.last_mut() {
            *entrant = "f".repeat(32).into();
        }
        assert!(!check(&m, &events).ok());
    }

    #[test]
    fn tampered_index_is_detected() {
        let (m, mut s) = setup();
        draw(&mut s, "item-0", 1);
        let mut events = s.events().to_vec();
        if let Some(Event::Drawn { index, .. }) = events.last_mut() {
            *index += 1;
        }
        assert!(!check(&m, &events).ok());
    }

    #[test]
    fn an_invalid_manifest_is_detected() {
        // Canonical and hashing to the published hash, but not one an engine
        // would open: another algorithm.
        let (mut m, _) = setup();
        m.algo = "rafi-draw-v2".into();
        let report = check(&m, &[]);
        assert!(report.manifest_hash_ok);
        assert!(!report.ok(), "{report:#?}");
    }

    #[test]
    fn tampered_list_is_detected() {
        let (m, s) = setup();
        let mut other = m.clone();
        other.entrants.pop();
        let report = verify(
            &other.canonical_json(),
            &m.hash().to_string(),
            s.events(),
            resolve,
        );
        assert!(!report.manifest_hash_ok);
    }

    #[test]
    fn pool_change_while_pending_fails_verification() {
        let m = manifest(2, 3);
        let mut s = Drawing::open(&m).unwrap();
        draw(&mut s, "item-0", 1);
        draw(&mut s, "item-1", 2);
        s.start(&"item-2".into(), 3, 0).unwrap();
        // A forged log: "not here" while the pass-2 draw is pending.
        let mut events = s.events().to_vec();
        events.push(Event::Resolved {
            k: 1,
            resolution: Resolution::NotHere,
        });
        assert!(!check(&m, &events).ok());
    }
}
