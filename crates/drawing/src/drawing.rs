//! The drawing: its state machine.
//!
//! [`Drawing`] is owned by the draw engine (the Restate virtual object; the
//! local runner in tests) and projected into the app database after every step.
//! All changes are recorded as [`Event`]s, which is also what the public
//! verifier replays.
//!
//! The engine protocol is here too, so engines only do I/O:
//! [`Drawing::open_or_check`] to open it, [`Drawing::check_chain`],
//! [`Drawing::begin`] and [`Drawing::conclude`] for a draw, and
//! [`Drawing::apply`] for every other command.
//!
//! Pool rules:
//! - Pass 1: everyone who has not been drawn yet. Once drawn, you are out,
//!   whatever the outcome (claimed, not here, declined): one win per person.
//! - Pass 2 (only when pass 1 is exhausted and items remain): pass-1 winners
//!   whose draw was not forfeited or declined and who were not yet drawn in
//!   pass 2.
//!
//! Fairness rules for a pending draw (its round may already be public):
//! - A beacon that cannot be fetched or verified never clears the pending draw
//!   ([`Drawing::stall`]); the next attempt fetches the *same* round.
//! - Nothing may change the draw's inputs (`k`, pass, pool, chain) while it is
//!   pending: other draws, voiding it, finishing, and "not here"/"declined"
//!   resolutions are refused.
//! - The only exits without its beacon are an offline draw and an explicit,
//!   logged [`Drawing::abandon`]; the public verifier flags both.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    Hash,
    drand::{ChainHash, QUICKNET_CHAIN, VerifiedBeacon},
    draw,
    engine::{BeaconOutcome, Command},
    error::Error,
    hash_from_hex,
    manifest::{self, ItemRef, Manifest},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    Drawn,
    Claimed,
    Voided,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub status: ItemStatus,
    /// The draw that currently holds this item (when drawn or claimed).
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
        item_id: String,
        round: u64,
        /// Unix time at which the round is published.
        round_time: u64,
    },
    Draw {
        k: u64,
        item_id: String,
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
        item_id: String,
        round: u64,
        reason: String,
    },
    /// The organizer abandoned the pending draw (its item goes back to pending).
    /// Published logs (and `tools/verify.py`) call it `fail`.
    #[serde(rename = "fail")]
    Abandon {
        k: u64,
        item_id: String,
        round: u64,
        reason: String,
    },
    Resolve {
        k: u64,
        outcome: Outcome,
    },
    Void {
        item_id: String,
    },
    Finish,
}

/// A draw that has been announced but not completed yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub k: u64,
    pub item_id: String,
    pub round: u64,
    pub round_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: String,
    /// Hex `SHA256` of the canonical manifest.
    pub manifest_hash: String,
    /// Hex hash of the drand chain the draws use (from the manifest).
    #[serde(default = "quicknet_chain")]
    pub drand_chain: String,
    /// Sorted entrant IDs.
    pub entrants: Vec<String>,
    pub items: Vec<Item>,
    pub events: Vec<Event>,
    pub next_k: u64,
    pub pending: Option<Pending>,
    pub finished: bool,
    /// Monotonic version, bumped on every change. Projections keep the highest.
    pub version: u64,
}

/// Refuses a drawing committed to another drand chain than the engine's.
fn check_chain(committed: &str, engine: &ChainHash) -> Result<(), Error> {
    let engine = engine.to_string();
    if committed.eq_ignore_ascii_case(&engine) {
        Ok(())
    } else {
        Err(Error::ChainMismatch {
            committed: committed.to_owned(),
            engine,
        })
    }
}

/// Result of a completed draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawn {
    pub k: u64,
    pub item_id: String,
    pub entrant_id: String,
    pub pass: u8,
}

fn quicknet_chain() -> String {
    QUICKNET_CHAIN.to_owned()
}

impl Drawing {
    /// Opens a drawing from its manifest.
    pub fn open(manifest: &Manifest) -> Self {
        let json = manifest.canonical_json();
        Self {
            id: manifest.raffle_id.clone(),
            manifest_hash: hex::encode(manifest::hash(&json)),
            drand_chain: manifest.drand_chain.clone(),
            entrants: manifest.entrant_ids.clone(),
            items: manifest.items.iter().map(Item::pending).collect(),
            events: Vec::new(),
            next_k: 1,
            pending: None,
            finished: false,
            version: 1,
        }
    }

    /// Opening, as every engine applies it: `existing` is the engine's
    /// drawing for this ID, if any.
    ///
    /// Validates the manifest and refuses another chain than the engine's.
    /// Opening with the same manifest again is a no-op (`None`); another
    /// manifest is refused. Otherwise returns the fresh drawing to store.
    pub fn open_or_check(
        existing: Option<&Drawing>,
        manifest: &Manifest,
        engine_chain: &ChainHash,
    ) -> Result<Option<Drawing>, Error> {
        manifest.validate()?;
        check_chain(&manifest.drand_chain, engine_chain)?;
        let fresh = Self::open(manifest);
        match existing {
            Some(existing) if existing.manifest_hash == fresh.manifest_hash => Ok(None),
            Some(_) => Err(Error::AlreadyOpen),
            None => Ok(Some(fresh)),
        }
    }

    /// Refuses to draw with an engine on another drand chain than the one the
    /// drawing committed to.
    pub fn check_chain(&self, engine_chain: &ChainHash) -> Result<(), Error> {
        check_chain(&self.drand_chain, engine_chain)
    }

    /// Whether the pending draw has stalled: its latest attempt couldn't get
    /// the committed round's beacon. (A retry in progress re-enters the same
    /// pending draw without an event, so it still reads as stalled.)
    pub fn stalled(&self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        // `k` is reused after an abandon: only the latest announcement counts.
        self.events
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::Stall { k, .. } if *k == pending.k => Some(true),
                Event::Announce { k, .. } if *k == pending.k => Some(false),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Settles the pending draw with its beacon's outcome.
    ///
    /// A verified beacon completes it (`Some`); offline randomness completes it
    /// as an offline draw. A beacon that doesn't fit the draw (wrong round or
    /// chain), an unavailable one and an interrupted attempt all stall it
    /// (`None`): the draw stays pending on its round, never abandoned, so a
    /// retry fetches the same round.
    pub fn conclude(&mut self, outcome: BeaconOutcome) -> Result<Option<Drawn>, Error> {
        match outcome {
            BeaconOutcome::Verified(beacon) => match self.complete(&beacon) {
                Ok(drawn) => Ok(Some(drawn)),
                Err(e) => self.stall(&format!("invalid beacon: {e}")).map(|()| None),
            },
            BeaconOutcome::Offline(randomness) => self.complete_offline(randomness).map(Some),
            BeaconOutcome::Unavailable(reason) => self
                .stall(&format!("drand unavailable: {reason}"))
                .map(|()| None),
            BeaconOutcome::Interrupted(reason) => self.stall(&reason).map(|()| None),
        }
    }

    /// Applies a command other than a draw (draws go through [`Self::begin`]
    /// and [`Self::conclude`]). After [`Command::Finish`] the state is
    /// `finished` and the engine may forget it.
    pub fn apply(&mut self, cmd: &Command) -> Result<(), Error> {
        match cmd {
            Command::Resolve { item_id, outcome } => self.resolve(item_id, *outcome),
            Command::Void { item_id } => self.void(item_id),
            Command::Abandon { item_id } => self.abandon_item(item_id),
            Command::Finish => self.finish(),
            Command::Draw { .. } => Err(Error::NotACommand),
        }
    }

    fn item_mut(&mut self, item_id: &str) -> Result<&mut Item, Error> {
        self.items
            .iter_mut()
            .find(|u| u.id == item_id)
            .ok_or_else(|| Error::UnknownItem(item_id.to_owned()))
    }

    pub fn item(&self, item_id: &str) -> Option<&Item> {
        self.items.iter().find(|u| u.id == item_id)
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

    /// Number of items that can still be given away.
    pub fn items_left(&self) -> usize {
        self.items
            .iter()
            .filter(|u| matches!(u.status, ItemStatus::Pending | ItemStatus::Drawn))
            .count()
    }

    /// Starts a draw for `item_id`, committing to drand `round`.
    ///
    /// Calling it again for the same item while that draw is pending returns
    /// the existing pending draw (idempotent).
    pub fn begin(&mut self, item_id: &str, round: u64, round_time: u64) -> Result<Pending, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if let Some(p) = &self.pending {
            if p.item_id == item_id {
                return Ok(p.clone());
            }
            return Err(Error::DrawInProgress);
        }
        let item = self
            .item(item_id)
            .ok_or_else(|| Error::UnknownItem(item_id.to_owned()))?;
        if item.status != ItemStatus::Pending {
            return Err(Error::ItemNotPending(item_id.to_owned()));
        }
        if self.pool().1.is_empty() {
            return Err(Error::NoEligibleEntrants);
        }
        let pending = Pending {
            k: self.next_k,
            item_id: item_id.to_owned(),
            round,
            round_time,
        };
        self.events.push(Event::Announce {
            k: pending.k,
            item_id: pending.item_id.clone(),
            round,
            round_time,
        });
        self.pending = Some(pending.clone());
        self.version += 1;
        Ok(pending)
    }

    /// Completes the pending draw with the verified beacon for its committed
    /// round, from the drawing's drand chain.
    pub fn complete(&mut self, beacon: &VerifiedBeacon) -> Result<Drawn, Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        if beacon.round() != pending.round {
            return Err(Error::WrongRound {
                expected: pending.round,
                got: beacon.round(),
            });
        }
        if !beacon
            .chain_hash()
            .to_string()
            .eq_ignore_ascii_case(&self.drand_chain)
        {
            return Err(Error::WrongChain);
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
    ) -> Result<Drawn, Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        if round != pending.round {
            return Err(Error::WrongRound {
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
    pub fn complete_offline(&mut self, randomness: Hash) -> Result<Drawn, Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        self.settle(pending, randomness, String::new(), DrawMode::Offline)
    }

    fn settle(
        &mut self,
        pending: Pending,
        randomness: Hash,
        signature: String,
        mode: DrawMode,
    ) -> Result<Drawn, Error> {
        let manifest_hash = hash_from_hex(&self.manifest_hash).ok_or(Error::BadManifestHash)?;
        let (pass, pool) = self.pool();
        if pool.is_empty() {
            // Unreachable while nothing may change a pending draw's pool; the
            // draw stays pending (only the organizer abandons it).
            return Err(Error::NoEligibleEntrants);
        }
        let index = draw::select(&randomness, &manifest_hash, pending.k, pool.len());
        let entrant_id = pool[index].clone();

        let item = self.item_mut(&pending.item_id)?;
        item.status = ItemStatus::Drawn;
        item.k = Some(pending.k);
        item.entrant_id = Some(entrant_id.clone());

        self.events.push(Event::Draw {
            k: pending.k,
            item_id: pending.item_id.clone(),
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
            item_id: pending.item_id,
            entrant_id,
            pass,
        })
    }

    /// Records that the pending draw's beacon could not be fetched or verified.
    ///
    /// The draw stays pending on the same round; drawing that item again
    /// fetches the same round.
    pub fn stall(&mut self, reason: &str) -> Result<(), Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        self.events.push(Event::Stall {
            k: pending.k,
            item_id: pending.item_id,
            round: pending.round,
            reason: reason.to_owned(),
        });
        self.version += 1;
        Ok(())
    }

    /// Abandons the pending draw on the organizer's explicit request (the item
    /// goes back to pending). Logged publicly: the verifier shows what the
    /// committed round would have picked.
    pub fn abandon(&mut self, reason: &str) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::NoDrawInProgress)?;
        self.events.push(Event::Abandon {
            k: pending.k,
            item_id: pending.item_id,
            round: pending.round,
            reason: reason.to_owned(),
        });
        self.version += 1;
        Ok(())
    }

    /// [`Drawing::abandon`] for the organizer's command, which names the item.
    pub fn abandon_item(&mut self, item_id: &str) -> Result<(), Error> {
        match &self.pending {
            Some(p) if p.item_id == item_id => self.abandon("abandoned by the organizer"),
            _ => Err(Error::NoDrawInProgress),
        }
    }

    /// Records the organizer's decision about a drawn item.
    ///
    /// `Claimed` keeps the item with the winner. `NotHere` and `Declined` put
    /// the item back to pending so it can be drawn again; they are refused
    /// while a draw is pending, because they can change its pool.
    pub fn resolve(&mut self, item_id: &str, outcome: Outcome) -> Result<(), Error> {
        if outcome == Outcome::Drawn {
            return Err(Error::BadOutcome);
        }
        if outcome != Outcome::Claimed && self.pending.is_some() {
            return Err(Error::DrawInProgress);
        }
        let item = self.item_mut(item_id)?;
        if item.status != ItemStatus::Drawn {
            return Err(Error::ItemNotDrawn(item_id.to_owned()));
        }
        let k = item.k.expect("drawn item has k");
        match outcome {
            Outcome::Claimed => item.status = ItemStatus::Claimed,
            _ => {
                item.status = ItemStatus::Pending;
                item.k = None;
                item.entrant_id = None;
            }
        }
        self.events.push(Event::Resolve { k, outcome });
        self.version += 1;
        Ok(())
    }

    /// Withdraws a item that will not be given away (e.g. the sponsor did not bring it).
    pub fn void(&mut self, item_id: &str) -> Result<(), Error> {
        if self.pending.as_ref().is_some_and(|p| p.item_id == item_id) {
            return Err(Error::DrawInProgress);
        }
        let item = self.item_mut(item_id)?;
        if item.status != ItemStatus::Pending {
            return Err(Error::ItemNotPending(item_id.to_owned()));
        }
        item.status = ItemStatus::Voided;
        self.events.push(Event::Void {
            item_id: item_id.to_owned(),
        });
        self.version += 1;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        if self.finished {
            return Ok(());
        }
        if self.pending.is_some() {
            return Err(Error::DrawInProgress);
        }
        self.finished = true;
        self.events.push(Event::Finish);
        self.version += 1;
        Ok(())
    }

    /// The final outcome for every entrant who has been drawn (latest draw wins).
    pub fn entrant_outcomes(&self) -> HashMap<String, (Outcome, String)> {
        let mut item_by_k: HashMap<u64, &str> = HashMap::new();
        for e in &self.events {
            if let Event::Draw { k, item_id, .. } = e {
                item_by_k.insert(*k, item_id);
            }
        }
        let mut out = HashMap::new();
        for (k, _, entrant, outcome) in self.draws() {
            let item = item_by_k.get(&k).copied().unwrap_or_default().to_owned();
            // Keep a claimed or undecided outcome over a later pass-2 forfeit.
            match out.get(entrant) {
                Some((Outcome::Claimed, _)) if outcome != Outcome::Claimed => {}
                _ => {
                    out.insert(entrant.to_owned(), (outcome, item));
                }
            }
        }
        out
    }
}

impl Item {
    fn pending(u: &ItemRef) -> Self {
        Self {
            id: u.id.clone(),
            name: u.name.clone(),
            status: ItemStatus::Pending,
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
    use crate::ErrorKind;

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

    fn doc(n: usize, items: usize) -> Manifest {
        Manifest::new(
            "raffle",
            &test_chain().chain().hash(),
            (0..n).map(|i| format!("{i:032x}")).collect(),
            (0..items)
                .map(|i| ItemRef {
                    id: format!("u{i}"),
                    name: "Prize".into(),
                })
                .collect(),
        )
    }

    fn draw(s: &mut Drawing, item: &str, round: u64) -> Drawn {
        s.begin(item, round, 0).unwrap();
        s.complete(&beacon(round)).unwrap()
    }

    #[test]
    fn one_win_per_person() {
        let mut s = Drawing::open(&doc(3, 3));
        let mut winners = HashSet::new();
        for (i, u) in ["u0", "u1", "u2"].iter().enumerate() {
            let d = draw(&mut s, u, 100 + i as u64);
            assert_eq!(d.pass, 1);
            assert!(winners.insert(d.entrant_id));
            s.resolve(u, Outcome::Claimed).unwrap();
        }
        assert_eq!(s.items_left(), 0);
    }

    #[test]
    fn not_here_is_out_and_unit_is_redrawn() {
        let mut s = Drawing::open(&doc(2, 1));
        let first = draw(&mut s, "u0", 1);
        s.resolve("u0", Outcome::NotHere).unwrap();
        assert_eq!(s.item("u0").unwrap().status, ItemStatus::Pending);
        let second = draw(&mut s, "u0", 2);
        assert_ne!(first.entrant_id, second.entrant_id);
        s.resolve("u0", Outcome::NotHere).unwrap();
        // Both entrants were drawn and forfeited: nobody is left.
        assert_eq!(s.begin("u0", 3, 0), Err(Error::NoEligibleEntrants));
    }

    #[test]
    fn pass_two_only_includes_winners() {
        let mut s = Drawing::open(&doc(2, 3));
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
        assert_eq!(s.begin("u2", 4, 0), Err(Error::NoEligibleEntrants));
    }

    #[test]
    fn begin_is_idempotent_and_exclusive() {
        let mut s = Drawing::open(&doc(5, 2));
        let p1 = s.begin("u0", 10, 0).unwrap();
        let p2 = s.begin("u0", 11, 0).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(s.begin("u1", 10, 0), Err(Error::DrawInProgress));
        assert_eq!(s.void("u0"), Err(Error::DrawInProgress));
    }

    #[test]
    fn wrong_round_or_chain_beacon_is_rejected() {
        let mut s = Drawing::open(&doc(5, 1));
        s.begin("u0", 10, 0).unwrap();
        assert_eq!(
            s.complete(&beacon(11)),
            Err(Error::WrongRound {
                expected: 10,
                got: 11
            })
        );
        s.drand_chain = QUICKNET_CHAIN.into();
        assert_eq!(s.complete(&beacon(10)), Err(Error::WrongChain));
        assert!(s.pending.is_some());
    }

    #[test]
    fn a_stall_keeps_the_draw_on_its_round() {
        let mut s = Drawing::open(&doc(5, 2));
        let p = s.begin("u0", 10, 0).unwrap();
        s.stall("drand unavailable").unwrap();
        assert_eq!(s.pending.as_ref(), Some(&p));
        // Retrying returns the same pending draw, even with a new round asked for.
        assert_eq!(s.begin("u0", 99, 0).unwrap(), p);
        assert_eq!(s.begin("u1", 99, 0), Err(Error::DrawInProgress));
        let d = s.complete(&beacon(10)).unwrap();
        assert_eq!(d.k, 1);
    }

    #[test]
    fn abandon_keeps_k_and_returns_unit() {
        let mut s = Drawing::open(&doc(5, 1));
        s.begin("u0", 10, 0).unwrap();
        s.abandon("organizer abandoned the draw").unwrap();
        assert_eq!(s.next_k, 1);
        assert_eq!(s.item("u0").unwrap().status, ItemStatus::Pending);
        let d = draw(&mut s, "u0", 12);
        assert_eq!(d.k, 1);
    }

    #[test]
    fn pool_changing_resolutions_wait_for_the_pending_draw() {
        let mut s = Drawing::open(&doc(2, 3));
        draw(&mut s, "u0", 1);
        draw(&mut s, "u1", 2);
        // Pass 2 is next; its pool depends on the pass-1 outcomes.
        s.begin("u2", 3, 0).unwrap();
        let before = s.pool();
        assert_eq!(
            s.resolve("u0", Outcome::NotHere),
            Err(Error::DrawInProgress)
        );
        assert_eq!(
            s.resolve("u0", Outcome::Declined),
            Err(Error::DrawInProgress)
        );
        s.resolve("u0", Outcome::Claimed).unwrap();
        assert_eq!(s.pool(), before);
        assert_eq!(s.finish(), Err(Error::DrawInProgress));
    }

    #[test]
    fn void_and_finish() {
        let mut s = Drawing::open(&doc(5, 2));
        s.void("u1").unwrap();
        assert_eq!(s.items_left(), 1);
        assert_eq!(s.void("u1"), Err(Error::ItemNotPending("u1".into())));
        s.finish().unwrap();
        assert_eq!(s.begin("u0", 1, 0), Err(Error::Finished));
    }

    #[test]
    fn version_increases_on_every_change() {
        let mut s = Drawing::open(&doc(5, 1));
        let v0 = s.version;
        s.begin("u0", 1, 0).unwrap();
        let v1 = s.version;
        s.stall("x").unwrap();
        let v2 = s.version;
        s.complete(&beacon(1)).unwrap();
        let v3 = s.version;
        assert!(v0 < v1 && v1 < v2 && v2 < v3);
    }

    fn chain() -> ChainHash {
        test_chain().chain().hash()
    }

    #[test]
    fn open_or_check_is_idempotent_per_manifest() {
        let d = doc(3, 1);
        let fresh = Drawing::open_or_check(None, &d, &chain()).unwrap().unwrap();
        assert_eq!(fresh, Drawing::open(&d));
        assert_eq!(Drawing::open_or_check(Some(&fresh), &d, &chain()), Ok(None));
        assert_eq!(
            Drawing::open_or_check(Some(&fresh), &doc(4, 1), &chain()),
            Err(Error::AlreadyOpen)
        );
    }

    #[test]
    fn open_or_check_refuses_another_chain_and_invalid_manifests() {
        let quicknet = crate::drand::chains::QUICKNET_HASH;
        assert!(matches!(
            Drawing::open_or_check(None, &doc(3, 1), &quicknet),
            Err(Error::ChainMismatch { .. })
        ));
        assert!(matches!(
            Drawing::open_or_check(None, &doc(0, 1), &chain()),
            Err(Error::InvalidManifest(_))
        ));
        let s = Drawing::open(&doc(3, 1));
        assert_eq!(s.check_chain(&chain()), Ok(()));
        assert_eq!(
            s.check_chain(&quicknet).unwrap_err().kind(),
            ErrorKind::Conflict
        );
    }

    #[test]
    fn conclude_completes_or_stalls_on_the_same_round() {
        let mut s = Drawing::open(&doc(5, 2));
        let p = s.begin("u0", 10, 0).unwrap();

        assert_eq!(s.conclude(BeaconOutcome::Verified(beacon(11))), Ok(None));
        assert!(s.stalled());
        assert_eq!(s.pending.as_ref(), Some(&p));
        assert_eq!(
            s.conclude(BeaconOutcome::Unavailable("relays down".into())),
            Ok(None)
        );
        assert_eq!(
            s.conclude(BeaconOutcome::Interrupted("draw cancelled".into())),
            Ok(None)
        );
        assert!(matches!(
            s.events.last(),
            Some(Event::Stall { reason, round: 10, .. }) if reason == "draw cancelled"
        ));

        let drawn = s
            .conclude(BeaconOutcome::Verified(beacon(10)))
            .unwrap()
            .unwrap();
        assert_eq!(drawn.k, 1);
        assert!(!s.stalled());

        s.begin("u1", 20, 0).unwrap();
        assert!(!s.stalled());
        let offline = s
            .conclude(BeaconOutcome::Offline([7; 32]))
            .unwrap()
            .unwrap();
        assert_eq!(offline.k, 2);
        assert!(matches!(
            s.events.last(),
            Some(Event::Draw {
                mode: DrawMode::Offline,
                ..
            })
        ));
        assert_eq!(
            s.conclude(BeaconOutcome::Unavailable("x".into())),
            Err(Error::NoDrawInProgress)
        );
    }

    #[test]
    fn a_new_announcement_after_an_abandon_is_not_stalled() {
        let mut s = Drawing::open(&doc(5, 1));
        s.begin("u0", 10, 0).unwrap();
        s.stall("x").unwrap();
        s.abandon("organizer").unwrap();
        assert!(!s.stalled());
        // The same k, announced again.
        s.begin("u0", 12, 0).unwrap();
        assert!(!s.stalled());
    }

    #[test]
    fn apply_runs_every_command_but_draws() {
        let mut s = Drawing::open(&doc(5, 2));
        draw(&mut s, "u0", 1);
        let resolve = Command::Resolve {
            item_id: "u0".into(),
            outcome: Outcome::Claimed,
        };
        s.apply(&resolve).unwrap();
        s.apply(&Command::Void {
            item_id: "u1".into(),
        })
        .unwrap();
        let draw_cmd = Command::Draw {
            item_id: "u1".into(),
            offline: false,
        };
        assert_eq!(s.apply(&draw_cmd), Err(Error::NotACommand));
        assert_eq!(
            s.apply(&Command::Abandon {
                item_id: "u1".into()
            }),
            Err(Error::NoDrawInProgress)
        );
        s.apply(&Command::Finish).unwrap();
        assert!(s.finished);
    }

    #[test]
    fn error_kinds() {
        assert_eq!(Error::NotOpen.kind(), ErrorKind::NotFound);
        assert_eq!(Error::UnknownItem("u".into()).kind(), ErrorKind::NotFound);
        assert_eq!(Error::BadOutcome.kind(), ErrorKind::BadRequest);
        assert_eq!(Error::AlreadyOpen.kind(), ErrorKind::Conflict);
        assert_eq!(Error::DrawInProgress.kind(), ErrorKind::Conflict);
        assert_eq!(Error::WrongChain.kind(), ErrorKind::Invariant);
        assert_eq!(Error::BadManifestHash.kind(), ErrorKind::Invariant);
    }

    #[test]
    fn an_abandon_is_published_as_fail() {
        let e = Event::Abandon {
            k: 1,
            item_id: "u0".into(),
            round: 10,
            reason: "x".into(),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["type"], "fail");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), e);
    }

    #[test]
    fn states_without_a_chain_default_to_quicknet() {
        let mut v = serde_json::to_value(Drawing::open(&doc(1, 1))).unwrap();
        v.as_object_mut().unwrap().remove("drand_chain");
        let s: Drawing = serde_json::from_value(v).unwrap();
        assert_eq!(s.drand_chain, QUICKNET_CHAIN);
    }
}
