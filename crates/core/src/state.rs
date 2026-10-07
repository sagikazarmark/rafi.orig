//! The draw state machine.
//!
//! [`DrawState`] is owned by the draw engine (the Restate virtual object, or the
//! local fallback engine) and projected into the app database after every step.
//! All changes are recorded as [`Event`]s, which is also what the public
//! verifier replays.
//!
//! Pool rules:
//! - Pass 1: everyone who has not been drawn yet. Once drawn, you are out,
//!   whatever the outcome (claimed, not here, declined): one win per person.
//! - Pass 2 (only when pass 1 is exhausted and prizes remain): pass-1 winners
//!   whose draw was not forfeited or declined and who were not yet drawn in
//!   pass 2.
//!
//! Fairness rules for a pending draw (its round may already be public):
//! - A beacon that cannot be fetched or verified never clears the pending draw
//!   ([`DrawState::stall`]); the next attempt fetches the *same* round.
//! - Nothing may change the draw's inputs (`k`, pass, pool, chain) while it is
//!   pending: other draws, voiding it, finishing, and "not here"/"declined"
//!   resolutions are refused.
//! - The only exits without its beacon are an offline draw and an explicit,
//!   logged [`DrawState::abandon`]; the public verifier flags both.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    Hash,
    commit::{CommitDoc, UnitRef, list_hash},
    drand::{QUICKNET_CHAIN, VerifiedBeacon},
    draw, hash_from_hex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitStatus {
    Pending,
    Drawn,
    Claimed,
    Voided,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    pub unit_id: String,
    pub item_id: String,
    pub item_name: String,
    pub status: UnitStatus,
    /// The draw that currently holds this unit (when drawn or claimed).
    pub k: Option<u64>,
    pub entrant_id: Option<String>,
}

/// How a drawn entrant's turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Drawn, not yet resolved by the organizer.
    Drawn,
    Claimed,
    NotHere,
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawMode {
    /// Randomness from a drand beacon round committed in advance.
    Beacon,
    /// Emergency mode: server randomness. Not publicly verifiable.
    Offline,
}

/// An entry in the public draw log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A draw was started and committed to a future drand round.
    Announce {
        k: u64,
        unit_id: String,
        round: u64,
        /// Unix time at which the round is published.
        round_time: u64,
    },
    Draw {
        k: u64,
        unit_id: String,
        round: u64,
        signature: String,
        randomness: String,
        mode: DrawMode,
        pass: u8,
        pool_size: usize,
        index: usize,
        entrant_id: String,
    },
    /// The pending draw's beacon could not be fetched (yet). The draw stays
    /// pending on the same round.
    Stall {
        k: u64,
        unit_id: String,
        round: u64,
        reason: String,
    },
    /// The organizer abandoned the pending draw (its unit goes back to pending).
    Fail {
        k: u64,
        unit_id: String,
        round: u64,
        reason: String,
    },
    Resolve {
        k: u64,
        outcome: Outcome,
    },
    Void {
        unit_id: String,
    },
    Finish,
}

/// A draw that has been announced but not completed yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub k: u64,
    pub unit_id: String,
    pub round: u64,
    pub round_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawState {
    pub raffle_id: String,
    /// Hex `SHA256` of the canonical commit document.
    pub list_hash: String,
    /// Hex hash of the drand chain the draws use (from the commit document).
    #[serde(default = "quicknet_chain")]
    pub drand_chain: String,
    /// Sorted entrant IDs.
    pub entrants: Vec<String>,
    pub units: Vec<Unit>,
    pub events: Vec<Event>,
    pub next_k: u64,
    pub pending: Option<Pending>,
    pub finished: bool,
    /// Monotonic version, bumped on every change. Projections keep the highest.
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DrawError {
    #[error("the raffle is finished")]
    Finished,
    #[error("unknown prize unit {0}")]
    UnknownUnit(String),
    #[error("prize unit {0} is not available for drawing")]
    UnitNotPending(String),
    #[error("prize unit {0} has no undecided winner")]
    UnitNotDrawn(String),
    #[error("another draw is in progress")]
    DrawInProgress,
    #[error("no draw is in progress")]
    NoDrawInProgress,
    #[error("there are no eligible entrants left")]
    NoEligibleEntrants,
    #[error("the beacon is for round {got}, but the draw committed to round {expected}")]
    WrongRound { expected: u64, got: u64 },
    #[error("the beacon is from another drand chain")]
    WrongChain,
    #[error("invalid list hash")]
    BadListHash,
    #[error("invalid resolution outcome")]
    BadOutcome,
}

/// Result of a completed draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawn {
    pub k: u64,
    pub unit_id: String,
    pub entrant_id: String,
    pub pass: u8,
}

fn quicknet_chain() -> String {
    QUICKNET_CHAIN.to_owned()
}

impl DrawState {
    /// Creates the state for a frozen raffle.
    pub fn freeze(doc: &CommitDoc) -> Self {
        let json = doc.canonical_json();
        Self {
            raffle_id: doc.raffle_id.clone(),
            list_hash: hex::encode(list_hash(&json)),
            drand_chain: doc.drand_chain.clone(),
            entrants: doc.entrant_ids.clone(),
            units: doc.units.iter().map(Unit::pending).collect(),
            events: Vec::new(),
            next_k: 1,
            pending: None,
            finished: false,
            version: 1,
        }
    }

    fn unit_mut(&mut self, unit_id: &str) -> Result<&mut Unit, DrawError> {
        self.units
            .iter_mut()
            .find(|u| u.unit_id == unit_id)
            .ok_or_else(|| DrawError::UnknownUnit(unit_id.to_owned()))
    }

    pub fn unit(&self, unit_id: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.unit_id == unit_id)
    }

    /// Outcome of every draw so far, keyed by `k`, with the pass and entrant.
    fn draws(&self) -> Vec<(u64, u8, &str, Outcome)> {
        let mut outcomes: HashMap<u64, Outcome> = HashMap::new();
        for e in &self.events {
            if let Event::Resolve { k, outcome } = e {
                outcomes.insert(*k, *outcome);
            }
        }
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Draw {
                    k,
                    pass,
                    entrant_id,
                    ..
                } => Some((
                    *k,
                    *pass,
                    entrant_id.as_str(),
                    outcomes.get(k).copied().unwrap_or(Outcome::Drawn),
                )),
                _ => None,
            })
            .collect()
    }

    /// The pool for the next draw: `(pass, sorted entrant IDs)`.
    pub fn pool(&self) -> (u8, Vec<String>) {
        let draws = self.draws();
        let drawn_pass1: HashSet<&str> = draws.iter().filter(|d| d.1 == 1).map(|d| d.2).collect();
        let pass1: Vec<String> = self
            .entrants
            .iter()
            .filter(|e| !drawn_pass1.contains(e.as_str()))
            .cloned()
            .collect();
        if !pass1.is_empty() {
            return (1, pass1);
        }

        let winners: HashSet<&str> = draws
            .iter()
            .filter(|d| d.1 == 1 && matches!(d.3, Outcome::Drawn | Outcome::Claimed))
            .map(|d| d.2)
            .collect();
        let drawn_pass2: HashSet<&str> = draws.iter().filter(|d| d.1 == 2).map(|d| d.2).collect();
        let pass2 = self
            .entrants
            .iter()
            .filter(|e| winners.contains(e.as_str()) && !drawn_pass2.contains(e.as_str()))
            .cloned()
            .collect();
        (2, pass2)
    }

    /// Number of units that can still be given away.
    pub fn units_left(&self) -> usize {
        self.units
            .iter()
            .filter(|u| matches!(u.status, UnitStatus::Pending | UnitStatus::Drawn))
            .count()
    }

    /// Starts a draw for `unit_id`, committing to drand `round`.
    ///
    /// Calling it again for the same unit while that draw is pending returns
    /// the existing pending draw (idempotent).
    pub fn begin(
        &mut self,
        unit_id: &str,
        round: u64,
        round_time: u64,
    ) -> Result<Pending, DrawError> {
        if self.finished {
            return Err(DrawError::Finished);
        }
        if let Some(p) = &self.pending {
            if p.unit_id == unit_id {
                return Ok(p.clone());
            }
            return Err(DrawError::DrawInProgress);
        }
        let unit = self
            .unit(unit_id)
            .ok_or_else(|| DrawError::UnknownUnit(unit_id.to_owned()))?;
        if unit.status != UnitStatus::Pending {
            return Err(DrawError::UnitNotPending(unit_id.to_owned()));
        }
        if self.pool().1.is_empty() {
            return Err(DrawError::NoEligibleEntrants);
        }
        let pending = Pending {
            k: self.next_k,
            unit_id: unit_id.to_owned(),
            round,
            round_time,
        };
        self.events.push(Event::Announce {
            k: pending.k,
            unit_id: pending.unit_id.clone(),
            round,
            round_time,
        });
        self.pending = Some(pending.clone());
        self.version += 1;
        Ok(pending)
    }

    /// Completes the pending draw with the verified beacon for its committed
    /// round, from the raffle's drand chain.
    pub fn complete(&mut self, beacon: &VerifiedBeacon) -> Result<Drawn, DrawError> {
        let pending = self.pending.clone().ok_or(DrawError::NoDrawInProgress)?;
        if beacon.round() != pending.round {
            return Err(DrawError::WrongRound {
                expected: pending.round,
                got: beacon.round(),
            });
        }
        if !beacon
            .chain_hash()
            .to_string()
            .eq_ignore_ascii_case(&self.drand_chain)
        {
            return Err(DrawError::WrongChain);
        }
        self.settle(
            pending,
            *beacon.randomness(),
            hex::encode(&beacon.beacon().signature),
            DrawMode::Beacon,
        )
    }

    /// Completes the pending draw with a beacon that could not be verified
    /// (the public verifier replaying a test chain's log). Never used to draw.
    pub(crate) fn complete_unverified(
        &mut self,
        round: u64,
        signature: &[u8],
    ) -> Result<Drawn, DrawError> {
        let pending = self.pending.clone().ok_or(DrawError::NoDrawInProgress)?;
        if round != pending.round {
            return Err(DrawError::WrongRound {
                expected: pending.round,
                got: round,
            });
        }
        let randomness = crate::sha256(&[signature]);
        self.settle(
            pending,
            randomness,
            hex::encode(signature),
            DrawMode::Beacon,
        )
    }

    /// Completes the pending draw with server randomness (emergency mode).
    pub fn complete_offline(&mut self, randomness: Hash) -> Result<Drawn, DrawError> {
        let pending = self.pending.clone().ok_or(DrawError::NoDrawInProgress)?;
        self.settle(pending, randomness, String::new(), DrawMode::Offline)
    }

    fn settle(
        &mut self,
        pending: Pending,
        randomness: Hash,
        signature: String,
        mode: DrawMode,
    ) -> Result<Drawn, DrawError> {
        let list_hash = hash_from_hex(&self.list_hash).ok_or(DrawError::BadListHash)?;
        let (pass, pool) = self.pool();
        if pool.is_empty() {
            // Unreachable while nothing may change a pending draw's pool; the
            // draw stays pending (only the organizer abandons it).
            return Err(DrawError::NoEligibleEntrants);
        }
        let index = draw::select(&randomness, &list_hash, pending.k, pool.len());
        let entrant_id = pool[index].clone();

        let unit = self.unit_mut(&pending.unit_id)?;
        unit.status = UnitStatus::Drawn;
        unit.k = Some(pending.k);
        unit.entrant_id = Some(entrant_id.clone());

        self.events.push(Event::Draw {
            k: pending.k,
            unit_id: pending.unit_id.clone(),
            round: pending.round,
            signature,
            randomness: hex::encode(randomness),
            mode,
            pass,
            pool_size: pool.len(),
            index,
            entrant_id: entrant_id.clone(),
        });
        self.pending = None;
        self.next_k += 1;
        self.version += 1;
        Ok(Drawn {
            k: pending.k,
            unit_id: pending.unit_id,
            entrant_id,
            pass,
        })
    }

    /// Records that the pending draw's beacon could not be fetched or verified.
    ///
    /// The draw stays pending on the same round; drawing that unit again
    /// fetches the same round.
    pub fn stall(&mut self, reason: &str) -> Result<(), DrawError> {
        let pending = self.pending.clone().ok_or(DrawError::NoDrawInProgress)?;
        self.events.push(Event::Stall {
            k: pending.k,
            unit_id: pending.unit_id,
            round: pending.round,
            reason: reason.to_owned(),
        });
        self.version += 1;
        Ok(())
    }

    /// Abandons the pending draw on the organizer's explicit request (the unit
    /// goes back to pending). Logged publicly: the verifier shows what the
    /// committed round would have picked.
    pub fn abandon(&mut self, reason: &str) -> Result<(), DrawError> {
        let pending = self.pending.take().ok_or(DrawError::NoDrawInProgress)?;
        self.events.push(Event::Fail {
            k: pending.k,
            unit_id: pending.unit_id,
            round: pending.round,
            reason: reason.to_owned(),
        });
        self.version += 1;
        Ok(())
    }

    /// [`DrawState::abandon`] for the organizer's command, which names the unit.
    pub fn abandon_unit(&mut self, unit_id: &str) -> Result<(), DrawError> {
        match &self.pending {
            Some(p) if p.unit_id == unit_id => self.abandon("abandoned by the organizer"),
            _ => Err(DrawError::NoDrawInProgress),
        }
    }

    /// Records the organizer's decision about a drawn unit.
    ///
    /// `Claimed` keeps the prize with the winner. `NotHere` and `Declined` put
    /// the unit back to pending so it can be drawn again; they are refused
    /// while a draw is pending, because they can change its pool.
    pub fn resolve(&mut self, unit_id: &str, outcome: Outcome) -> Result<(), DrawError> {
        if outcome == Outcome::Drawn {
            return Err(DrawError::BadOutcome);
        }
        if outcome != Outcome::Claimed && self.pending.is_some() {
            return Err(DrawError::DrawInProgress);
        }
        let unit = self.unit_mut(unit_id)?;
        if unit.status != UnitStatus::Drawn {
            return Err(DrawError::UnitNotDrawn(unit_id.to_owned()));
        }
        let k = unit.k.expect("drawn unit has k");
        match outcome {
            Outcome::Claimed => unit.status = UnitStatus::Claimed,
            _ => {
                unit.status = UnitStatus::Pending;
                unit.k = None;
                unit.entrant_id = None;
            }
        }
        self.events.push(Event::Resolve { k, outcome });
        self.version += 1;
        Ok(())
    }

    /// Withdraws a unit that will not be given away (e.g. the sponsor did not bring it).
    pub fn void(&mut self, unit_id: &str) -> Result<(), DrawError> {
        if self.pending.as_ref().is_some_and(|p| p.unit_id == unit_id) {
            return Err(DrawError::DrawInProgress);
        }
        let unit = self.unit_mut(unit_id)?;
        if unit.status != UnitStatus::Pending {
            return Err(DrawError::UnitNotPending(unit_id.to_owned()));
        }
        unit.status = UnitStatus::Voided;
        self.events.push(Event::Void {
            unit_id: unit_id.to_owned(),
        });
        self.version += 1;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), DrawError> {
        if self.finished {
            return Ok(());
        }
        if self.pending.is_some() {
            return Err(DrawError::DrawInProgress);
        }
        self.finished = true;
        self.events.push(Event::Finish);
        self.version += 1;
        Ok(())
    }

    /// The final outcome for every entrant who has been drawn (latest draw wins).
    pub fn entrant_outcomes(&self) -> HashMap<String, (Outcome, String)> {
        let mut unit_by_k: HashMap<u64, &str> = HashMap::new();
        for e in &self.events {
            if let Event::Draw { k, unit_id, .. } = e {
                unit_by_k.insert(*k, unit_id);
            }
        }
        let mut out = HashMap::new();
        for (k, _, entrant, outcome) in self.draws() {
            let unit = unit_by_k.get(&k).copied().unwrap_or_default().to_owned();
            // Keep a claimed or undecided outcome over a later pass-2 forfeit.
            match out.get(entrant) {
                Some((Outcome::Claimed, _)) if outcome != Outcome::Claimed => {}
                _ => {
                    out.insert(entrant.to_owned(), (outcome, unit));
                }
            }
        }
        out
    }
}

impl Unit {
    fn pending(u: &UnitRef) -> Self {
        Self {
            unit_id: u.unit_id.clone(),
            item_id: u.item_id.clone(),
            item_name: u.item_name.clone(),
            status: UnitStatus::Pending,
            k: None,
            entrant_id: None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::num::NonZeroU32;

    use drand::{Scheme, TestChain, Verifier};

    use super::*;

    /// A signing test chain: real BLS signatures, small round numbers.
    pub(crate) fn test_chain() -> TestChain {
        TestChain::new(
            [9; 32],
            Scheme::BlsUnchainedG1Rfc9380,
            NonZeroU32::new(3).unwrap(),
            0,
            "rafi-core-test",
        )
        .unwrap()
    }

    pub(crate) fn beacon(round: u64) -> VerifiedBeacon {
        let tc = test_chain();
        Verifier::new(tc.chain())
            .unwrap()
            .verify_round(round, &tc.sign(round))
            .unwrap()
    }

    fn doc(n: usize, units: usize) -> CommitDoc {
        CommitDoc::new(
            "raffle",
            &test_chain().chain().hash(),
            (0..n).map(|i| format!("{i:032x}")).collect(),
            (0..units)
                .map(|i| UnitRef {
                    unit_id: format!("u{i}"),
                    item_id: "item".into(),
                    item_name: "Prize".into(),
                })
                .collect(),
        )
    }

    fn draw(s: &mut DrawState, unit: &str, round: u64) -> Drawn {
        s.begin(unit, round, 0).unwrap();
        s.complete(&beacon(round)).unwrap()
    }

    #[test]
    fn one_win_per_person() {
        let mut s = DrawState::freeze(&doc(3, 3));
        let mut winners = HashSet::new();
        for (i, u) in ["u0", "u1", "u2"].iter().enumerate() {
            let d = draw(&mut s, u, 100 + i as u64);
            assert_eq!(d.pass, 1);
            assert!(winners.insert(d.entrant_id));
            s.resolve(u, Outcome::Claimed).unwrap();
        }
        assert_eq!(s.units_left(), 0);
    }

    #[test]
    fn not_here_is_out_and_unit_is_redrawn() {
        let mut s = DrawState::freeze(&doc(2, 1));
        let first = draw(&mut s, "u0", 1);
        s.resolve("u0", Outcome::NotHere).unwrap();
        assert_eq!(s.unit("u0").unwrap().status, UnitStatus::Pending);
        let second = draw(&mut s, "u0", 2);
        assert_ne!(first.entrant_id, second.entrant_id);
        s.resolve("u0", Outcome::NotHere).unwrap();
        // Both entrants were drawn and forfeited: nobody is left.
        assert_eq!(s.begin("u0", 3, 0), Err(DrawError::NoEligibleEntrants));
    }

    #[test]
    fn pass_two_only_includes_winners() {
        let mut s = DrawState::freeze(&doc(2, 3));
        let a = draw(&mut s, "u0", 1);
        s.resolve("u0", Outcome::Claimed).unwrap();
        draw(&mut s, "u1", 2);
        s.resolve("u1", Outcome::Declined).unwrap();
        // Pass 1 exhausted; only the claimed winner is in pass 2.
        let (pass, pool) = s.pool();
        assert_eq!(pass, 2);
        assert_eq!(pool, vec![a.entrant_id.clone()]);
        let c = draw(&mut s, "u1", 3);
        assert_eq!(c.entrant_id, a.entrant_id);
        assert_eq!(c.pass, 2);
        s.resolve("u1", Outcome::Claimed).unwrap();
        assert_eq!(s.begin("u2", 4, 0), Err(DrawError::NoEligibleEntrants));
    }

    #[test]
    fn begin_is_idempotent_and_exclusive() {
        let mut s = DrawState::freeze(&doc(5, 2));
        let p1 = s.begin("u0", 10, 0).unwrap();
        let p2 = s.begin("u0", 11, 0).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(s.begin("u1", 10, 0), Err(DrawError::DrawInProgress));
        assert_eq!(s.void("u0"), Err(DrawError::DrawInProgress));
    }

    #[test]
    fn wrong_round_or_chain_beacon_is_rejected() {
        let mut s = DrawState::freeze(&doc(5, 1));
        s.begin("u0", 10, 0).unwrap();
        assert_eq!(
            s.complete(&beacon(11)),
            Err(DrawError::WrongRound {
                expected: 10,
                got: 11
            })
        );
        s.drand_chain = QUICKNET_CHAIN.into();
        assert_eq!(s.complete(&beacon(10)), Err(DrawError::WrongChain));
        assert!(s.pending.is_some());
    }

    #[test]
    fn a_stall_keeps_the_draw_on_its_round() {
        let mut s = DrawState::freeze(&doc(5, 2));
        let p = s.begin("u0", 10, 0).unwrap();
        s.stall("drand unavailable").unwrap();
        assert_eq!(s.pending.as_ref(), Some(&p));
        // Retrying returns the same pending draw, even with a new round asked for.
        assert_eq!(s.begin("u0", 99, 0).unwrap(), p);
        assert_eq!(s.begin("u1", 99, 0), Err(DrawError::DrawInProgress));
        let d = s.complete(&beacon(10)).unwrap();
        assert_eq!(d.k, 1);
    }

    #[test]
    fn abandon_keeps_k_and_returns_unit() {
        let mut s = DrawState::freeze(&doc(5, 1));
        s.begin("u0", 10, 0).unwrap();
        s.abandon("organizer abandoned the draw").unwrap();
        assert_eq!(s.next_k, 1);
        assert_eq!(s.unit("u0").unwrap().status, UnitStatus::Pending);
        let d = draw(&mut s, "u0", 12);
        assert_eq!(d.k, 1);
    }

    #[test]
    fn pool_changing_resolutions_wait_for_the_pending_draw() {
        let mut s = DrawState::freeze(&doc(2, 3));
        draw(&mut s, "u0", 1);
        draw(&mut s, "u1", 2);
        // Pass 2 is next; its pool depends on the pass-1 outcomes.
        s.begin("u2", 3, 0).unwrap();
        let before = s.pool();
        assert_eq!(
            s.resolve("u0", Outcome::NotHere),
            Err(DrawError::DrawInProgress)
        );
        assert_eq!(
            s.resolve("u0", Outcome::Declined),
            Err(DrawError::DrawInProgress)
        );
        s.resolve("u0", Outcome::Claimed).unwrap();
        assert_eq!(s.pool(), before);
        assert_eq!(s.finish(), Err(DrawError::DrawInProgress));
    }

    #[test]
    fn void_and_finish() {
        let mut s = DrawState::freeze(&doc(5, 2));
        s.void("u1").unwrap();
        assert_eq!(s.units_left(), 1);
        assert_eq!(s.void("u1"), Err(DrawError::UnitNotPending("u1".into())));
        s.finish().unwrap();
        assert_eq!(s.begin("u0", 1, 0), Err(DrawError::Finished));
    }

    #[test]
    fn version_increases_on_every_change() {
        let mut s = DrawState::freeze(&doc(5, 1));
        let v0 = s.version;
        s.begin("u0", 1, 0).unwrap();
        let v1 = s.version;
        s.stall("x").unwrap();
        let v2 = s.version;
        s.complete(&beacon(1)).unwrap();
        let v3 = s.version;
        assert!(v0 < v1 && v1 < v2 && v2 < v3);
    }

    #[test]
    fn states_without_a_chain_default_to_quicknet() {
        let mut v = serde_json::to_value(DrawState::freeze(&doc(1, 1))).unwrap();
        v.as_object_mut().unwrap().remove("drand_chain");
        let s: DrawState = serde_json::from_value(v).unwrap();
        assert_eq!(s.drand_chain, QUICKNET_CHAIN);
    }
}
