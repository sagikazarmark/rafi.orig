//! Independent verification of a published draw log.
//!
//! Replays the events against the commit document and checks that every draw
//! picked exactly the entrant the algorithm dictates. This is what the public
//! verification page runs in the browser (compiled to WASM) and what
//! `tools/verify.py` reimplements.

use serde::{Deserialize, Serialize};

use crate::{
    commit::{CommitDoc, list_hash},
    drand::Beacon,
    state::{DrawMode, DrawState, Event},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawCheck {
    pub k: u64,
    pub unit_id: String,
    pub round: u64,
    pub mode: DrawMode,
    pub pool_size: usize,
    pub expected_entrant: String,
    pub published_entrant: String,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub list_hash: String,
    pub list_hash_ok: bool,
    pub draws: Vec<DrawCheck>,
    /// Problems found while replaying, if any.
    pub errors: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.list_hash_ok && self.errors.is_empty() && self.draws.iter().all(|d| d.ok)
    }
}

/// Verifies `events` against the published canonical commit document.
///
/// `expected_list_hash` is the hash that was announced at freeze time.
pub fn verify(commit_json: &str, expected_list_hash: &str, events: &[Event]) -> Report {
    let computed = hex::encode(list_hash(commit_json));
    let mut report = Report {
        list_hash_ok: computed.eq_ignore_ascii_case(expected_list_hash),
        list_hash: computed,
        draws: Vec::new(),
        errors: Vec::new(),
    };
    let doc: CommitDoc = match serde_json::from_str(commit_json) {
        Ok(doc) => doc,
        Err(e) => {
            report
                .errors
                .push(format!("commit document is not valid: {e}"));
            return report;
        }
    };
    if doc.canonical_json() != commit_json {
        report
            .errors
            .push("commit document is not in canonical form".into());
    }

    // Replay into a fresh state, re-deriving each draw from its beacon.
    let mut state = DrawState::freeze(&doc);
    for event in events {
        let result = match event {
            Event::Announce {
                unit_id,
                round,
                round_time,
                ..
            } => state.begin(unit_id, *round, *round_time).map(|_| ()),
            Event::Draw {
                k,
                unit_id,
                round,
                signature,
                randomness,
                mode,
                pool_size,
                entrant_id,
                ..
            } => {
                let drawn = match mode {
                    DrawMode::Beacon => state.complete(&Beacon {
                        round: *round,
                        signature: signature.clone(),
                        randomness: randomness.clone(),
                    }),
                    DrawMode::Offline => match crate::hash_from_hex(randomness) {
                        Some(r) => state.complete_offline(r),
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
                            unit_id: unit_id.clone(),
                            round: *round,
                            mode: *mode,
                            pool_size: pool,
                            ok: d.entrant_id == *entrant_id && d.k == *k && pool == *pool_size,
                            expected_entrant: d.entrant_id,
                            published_entrant: entrant_id.clone(),
                        });
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            Event::Fail { reason, .. } => state.fail(reason),
            Event::Resolve { k, outcome } => {
                let unit = state
                    .units
                    .iter()
                    .find(|u| u.k == Some(*k))
                    .map(|u| u.unit_id.clone());
                match unit {
                    Some(unit) => state.resolve(&unit, *outcome),
                    None => {
                        report.errors.push(format!("resolve for unknown draw {k}"));
                        continue;
                    }
                }
            }
            Event::Void { unit_id } => state.void(unit_id),
            Event::Finish => state.finish(),
        };
        if let Err(e) = result {
            report.errors.push(format!("{event:?}: {e}"));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commit::UnitRef, drand::randomness_from_signature, state::Outcome};

    fn beacon(round: u64) -> Beacon {
        let sig = crate::sha256(&[b"sig", &round.to_be_bytes()]).to_vec();
        Beacon {
            round,
            randomness: hex::encode(randomness_from_signature(&sig)),
            signature: hex::encode(sig),
        }
    }

    fn setup() -> (CommitDoc, DrawState) {
        let doc = CommitDoc::new(
            "r",
            (0..10).map(|i| format!("{i:032x}")).collect(),
            (0..3)
                .map(|i| UnitRef {
                    unit_id: format!("u{i}"),
                    item_id: "i".into(),
                    item_name: "Prize".into(),
                })
                .collect(),
        );
        let state = DrawState::freeze(&doc);
        (doc, state)
    }

    #[test]
    fn honest_log_verifies() {
        let (doc, mut s) = setup();
        s.begin("u0", 1, 0).unwrap();
        s.complete(&beacon(1)).unwrap();
        s.resolve("u0", Outcome::NotHere).unwrap();
        s.begin("u0", 2, 0).unwrap();
        s.fail("timeout").unwrap();
        s.begin("u0", 3, 0).unwrap();
        s.complete(&beacon(3)).unwrap();
        s.resolve("u0", Outcome::Claimed).unwrap();
        s.void("u1").unwrap();
        s.begin("u2", 4, 0).unwrap();
        s.complete_offline([5; 32]).unwrap();
        s.finish().unwrap();

        let report = verify(&doc.canonical_json(), &s.list_hash, &s.events);
        assert!(report.ok(), "{report:#?}");
        assert_eq!(report.draws.len(), 3);
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
        let report = verify(&doc.canonical_json(), &s.list_hash, &events);
        assert!(!report.ok());
    }

    #[test]
    fn tampered_list_is_detected() {
        let (doc, s) = setup();
        let mut other = doc.clone();
        other.entrant_ids.pop();
        let report = verify(&other.canonical_json(), &s.list_hash, &s.events);
        assert!(!report.list_hash_ok);
    }
}
