//! Independent verification of a published draw log.
//!
//! Replays the events against the manifest and checks that every draw
//! picked exactly the entrant the algorithm dictates, from a drand beacon that
//! verifies against the committed chain. This is what the public verification
//! page runs in the browser (compiled to WASM) and what `tools/verify.py`
//! reimplements (without the BLS check).

use serde::{Deserialize, Serialize};

use crate::{
    drand::{Beacon, ChainHash, ChainInfo, Verifier, chains},
    drawing::{DrawMode, Drawing, Event},
    manifest::{self, Manifest},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawCheck {
    pub k: u64,
    pub item_id: String,
    pub round: u64,
    pub mode: DrawMode,
    pub pool_size: usize,
    pub expected_entrant: String,
    pub published_entrant: String,
    pub ok: bool,
}

/// How a committed round ended without its beacon deciding the draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Replaced {
    /// The organizer abandoned the draw.
    Abandoned,
    /// The draw was completed offline (server randomness) instead.
    Offline,
}

/// A round a draw committed to that did not decide it. Anyone can fetch the
/// round and run the algorithm on `pool` to see whom it would have picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedRound {
    pub k: u64,
    pub item_id: String,
    pub round: u64,
    pub pass: u8,
    /// The pool at the time, sorted (as the algorithm indexes it).
    pub pool: Vec<String>,
    pub replaced: Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub manifest_hash: String,
    pub manifest_hash_ok: bool,
    /// The drand chain hash from the manifest.
    pub chain: String,
    /// Whether the chain is known, so every beacon could be BLS-verified.
    /// False for test chains: such a log is never reported as verified.
    pub chain_known: bool,
    pub draws: Vec<DrawCheck>,
    /// Committed rounds that were abandoned or replaced by an offline draw.
    pub replaced: Vec<CommittedRound>,
    /// Problems found while replaying, if any.
    pub errors: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.manifest_hash_ok
            && self.chain_known
            && self.errors.is_empty()
            && self.draws.iter().all(|d| d.ok)
    }

    /// The chain is not a public drand chain (e.g. a test chain): the draws
    /// replay, but their randomness is not publicly verifiable.
    pub fn chain_unknown(&self) -> bool {
        !self.chain_known
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
pub fn verify(
    manifest_json: &str,
    expected_manifest_hash: &str,
    events: &[Event],
    resolve: impl Fn(&ChainHash) -> Option<ChainInfo>,
) -> Report {
    let computed = hex::encode(manifest::hash(manifest_json));
    let mut report = Report {
        manifest_hash_ok: computed.eq_ignore_ascii_case(expected_manifest_hash),
        manifest_hash: computed,
        chain: String::new(),
        chain_known: false,
        draws: Vec::new(),
        replaced: Vec::new(),
        errors: Vec::new(),
    };
    let doc: Manifest = match serde_json::from_str(manifest_json) {
        Ok(doc) => doc,
        Err(e) => {
            report.errors.push(format!("manifest is not valid: {e}"));
            return report;
        }
    };
    if doc.canonical_json() != manifest_json {
        report
            .errors
            .push("manifest is not in canonical form".into());
    }
    report.chain = doc.drand_chain.clone();
    let verifier = match doc.drand_chain.parse::<ChainHash>() {
        Ok(hash) => match resolve(&hash) {
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
        Err(_) => {
            report.errors.push("invalid drand chain hash".into());
            None
        }
    };
    report.chain_known = verifier.is_some();

    // Replay into a fresh state, re-deriving each draw from its beacon.
    let mut state = Drawing::open(&doc);
    for event in events {
        let result = match event {
            Event::Announce {
                k,
                item_id,
                round,
                round_time,
            } => {
                if let Some(p) = &state.pending {
                    report
                        .errors
                        .push(format!("draw {k}: announced while draw {} is pending", p.k));
                    continue;
                }
                match state.begin(item_id, *round, *round_time) {
                    Ok(p) if p.k != *k => {
                        report
                            .errors
                            .push(format!("draw {k}: announced as draw {}", p.k));
                        continue;
                    }
                    other => other.map(|_| ()),
                }
            }
            Event::Draw {
                k,
                item_id,
                round,
                signature,
                randomness,
                mode,
                pool_size,
                entrant_id,
                pass,
                ..
            } => {
                // A draw must settle the announced draw, on its committed round.
                match &state.pending {
                    Some(p) if p.k == *k && p.item_id == *item_id && p.round == *round => {}
                    _ => {
                        report.errors.push(format!(
                            "draw {k}: does not match the announced draw (item or round changed)"
                        ));
                        continue;
                    }
                }
                let drawn = match mode {
                    DrawMode::Beacon => {
                        let beacon = match Beacon::from_hex(*round, signature, None) {
                            Ok(b)
                                if hex::encode(crate::sha256(&[&b.signature]))
                                    .eq_ignore_ascii_case(randomness) =>
                            {
                                b
                            }
                            _ => {
                                report.errors.push(format!(
                                    "draw {k}: published randomness does not match the signature"
                                ));
                                continue;
                            }
                        };
                        match &verifier {
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
                        }
                    }
                    DrawMode::Offline => match crate::hash_from_hex(randomness) {
                        Some(r) => {
                            record_replaced(&mut report, &state, Replaced::Offline);
                            state.complete_offline(r)
                        }
                        None => {
                            report.errors.push(format!("draw {k}: invalid randomness"));
                            continue;
                        }
                    },
                };
                match drawn {
                    Ok(d) => {
                        let pool = match state.events.last() {
                            Some(Event::Draw { pool_size, .. }) => *pool_size,
                            _ => 0,
                        };
                        report.draws.push(DrawCheck {
                            k: *k,
                            item_id: item_id.clone(),
                            round: *round,
                            mode: *mode,
                            pool_size: pool,
                            ok: d.entrant_id == *entrant_id
                                && d.k == *k
                                && d.pass == *pass
                                && pool == *pool_size,
                            expected_entrant: d.entrant_id,
                            published_entrant: entrant_id.clone(),
                        });
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            Event::Stall {
                k, round, reason, ..
            } => match &state.pending {
                Some(p) if p.k == *k && p.round == *round => state.stall(reason),
                _ => {
                    report
                        .errors
                        .push(format!("draw {k}: stall does not match the pending draw"));
                    continue;
                }
            },
            Event::Abandon {
                k,
                item_id,
                round,
                reason,
            } => {
                match &state.pending {
                    Some(p) if p.k == *k && p.item_id == *item_id && p.round == *round => {}
                    _ => {
                        report
                            .errors
                            .push(format!("draw {k}: abandon does not match the pending draw"));
                        continue;
                    }
                }
                record_replaced(&mut report, &state, Replaced::Abandoned);
                state.abandon(reason)
            }
            Event::Resolve { k, outcome } => {
                let item = state
                    .items
                    .iter()
                    .find(|u| u.k == Some(*k))
                    .map(|u| u.id.clone());
                match item {
                    Some(item) => state.resolve(&item, *outcome),
                    None => {
                        report.errors.push(format!("resolve for unknown draw {k}"));
                        continue;
                    }
                }
            }
            Event::Void { item_id } => state.void(item_id),
            Event::Finish => state.finish(),
        };
        if let Err(e) = result {
            report.errors.push(format!("{event:?}: {e}"));
        }
    }
    report
}

/// Records the pending draw's committed round before it is replaced.
fn record_replaced(report: &mut Report, state: &Drawing, replaced: Replaced) {
    if let Some(p) = &state.pending {
        let (pass, pool) = state.pool();
        report.replaced.push(CommittedRound {
            k: p.k,
            item_id: p.item_id.clone(),
            round: p.round,
            pass,
            pool,
            replaced,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        drawing::{
            Outcome,
            tests::{beacon, test_chain},
        },
        manifest::ItemRef,
    };

    fn resolve(h: &ChainHash) -> Option<ChainInfo> {
        let info = test_chain().chain().clone();
        (info.hash() == *h).then_some(info)
    }

    fn setup() -> (Manifest, Drawing) {
        let doc = Manifest::new(
            "r",
            &test_chain().chain().hash(),
            (0..10).map(|i| format!("{i:032x}")).collect(),
            (0..3)
                .map(|i| ItemRef {
                    id: format!("u{i}"),
                    name: "Prize".into(),
                })
                .collect(),
        );
        let state = Drawing::open(&doc);
        (doc, state)
    }

    #[test]
    fn honest_log_verifies() {
        let (doc, mut s) = setup();
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        s.resolve("u0", Outcome::NotHere).unwrap();
        s.begin("u0", 2, 0).unwrap();
        s.stall("drand unavailable").unwrap();
        s.abandon("abandoned by the organizer").unwrap();
        s.begin("u0", 3, 0).unwrap();
        s.stall("timeout").unwrap();
        s.complete(&beacon(3)).unwrap();
        s.resolve("u0", Outcome::Claimed).unwrap();
        s.void("u1").unwrap();
        s.begin("u2", 4, 0).unwrap();
        s.complete_offline([5; 32]).unwrap();
        s.finish().unwrap();

        let report = verify(&doc.canonical_json(), &s.manifest_hash, &s.events, resolve);
        assert!(report.ok(), "{report:#?}");
        assert_eq!(report.draws.len(), 3);
        let replaced: Vec<_> = report
            .replaced
            .iter()
            .map(|r| (r.round, r.replaced))
            .collect();
        assert_eq!(replaced, [(2, Replaced::Abandoned), (4, Replaced::Offline)]);
        assert_eq!(report.replaced[0].pool.len(), 9);
    }

    #[test]
    fn unknown_chain_replays_but_is_not_ok() {
        let (doc, mut s) = setup();
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        let report = verify_public(&doc.canonical_json(), &s.manifest_hash, &s.events);
        assert!(report.chain_unknown());
        assert!(!report.ok());
        assert!(report.errors.is_empty(), "{report:#?}");
        assert!(report.draws.iter().all(|d| d.ok));
    }

    #[test]
    fn forged_beacon_is_detected() {
        let (doc, mut s) = setup();
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        let mut events = s.events.clone();
        if let Some(Event::Draw {
            signature,
            randomness,
            ..
        }) = events.last_mut()
        {
            // Self-consistent but not signed by the chain: sha256 checks pass, BLS fails.
            let mut sig = hex::decode(&*signature).unwrap();
            sig[10] ^= 1;
            *randomness = hex::encode(crate::sha256(&[&sig]));
            *signature = hex::encode(sig);
        }
        let report = verify(&doc.canonical_json(), &s.manifest_hash, &events, resolve);
        assert!(!report.ok());
        assert!(!report.errors.is_empty());
    }

    #[test]
    fn silent_round_switch_is_detected() {
        let (doc, mut s) = setup();
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        // A forged log drawing from round 2 although round 1 was announced:
        // a genuine beacon, but a redraw on a new round.
        let mut forged = Drawing::open(&doc);
        forged.begin("u0", 2, 0).unwrap();
        forged.complete(&beacon(2)).unwrap();
        let mut events = s.events[..1].to_vec();
        events.push(forged.events[1].clone());
        let report = verify(&doc.canonical_json(), &s.manifest_hash, &events, resolve);
        assert!(!report.ok());
        // So is a draw without an announcement, or a second announcement.
        let report = verify(
            &doc.canonical_json(),
            &s.manifest_hash,
            &s.events[1..],
            resolve,
        );
        assert!(!report.ok());
        let mut twice = s.events.clone();
        twice.insert(1, twice[0].clone());
        assert!(!verify(&doc.canonical_json(), &s.manifest_hash, &twice, resolve).ok());
    }

    #[test]
    fn tampered_winner_is_detected() {
        let (doc, mut s) = setup();
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        let mut events = s.events.clone();
        if let Some(Event::Draw { entrant_id, .. }) = events.last_mut() {
            *entrant_id = "f".repeat(32);
        }
        let report = verify(&doc.canonical_json(), &s.manifest_hash, &events, resolve);
        assert!(!report.ok());
    }

    #[test]
    fn tampered_list_is_detected() {
        let (doc, s) = setup();
        let mut other = doc.clone();
        other.entrant_ids.pop();
        let report = verify(
            &other.canonical_json(),
            &s.manifest_hash,
            &s.events,
            resolve,
        );
        assert!(!report.manifest_hash_ok);
    }

    #[test]
    fn pool_change_while_pending_fails_verification() {
        let doc = Manifest::new(
            "r",
            &test_chain().chain().hash(),
            (0..2).map(|i| format!("{i:032x}")).collect(),
            (0..3)
                .map(|i| ItemRef {
                    id: format!("u{i}"),
                    name: "Prize".into(),
                })
                .collect(),
        );
        let mut s = Drawing::open(&doc);
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        s.begin("u1", 2, 0).unwrap();
        s.complete(&beacon(2)).unwrap();
        s.begin("u2", 3, 0).unwrap();
        // A forged log: "not here" while the pass-2 draw is pending.
        let mut events = s.events.clone();
        events.push(Event::Resolve {
            k: 1,
            outcome: Outcome::NotHere,
        });
        let report = verify(&doc.canonical_json(), &s.manifest_hash, &events, resolve);
        assert!(!report.ok());
    }
}
