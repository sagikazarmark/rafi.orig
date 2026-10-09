//! The drawing: its state machine.
//!
//! [`Drawing`] is one drawing's state. Its owner, an engine, stores it and
//! changes it only through its protocol: [`Drawing::open_or_check`] to open
//! it, [`Drawing::announce`] and [`Drawing::settle`] for a draw, and
//! [`Drawing::decide`] for every decision. Every change is recorded as an
//! [`Event`], and the event log is what the public verifier replays.
//!
//! The pool rules are part of the algorithm ([`algorithm`](crate::algorithm)).
//!
//! Fairness rules for a pending draw (its round may already be public):
//! - A beacon that cannot be fetched or verified never clears the pending draw
//!   (it stalls); the next attempt fetches the *same* round.
//! - Nothing may change the draw's inputs (`k`, pass, pool, beacon source)
//!   while it is pending: other draws, voiding it, finishing, and forfeits
//!   ("not here"/"declined") are refused.
//! - The only exit without its beacon is an explicit, logged
//!   [`Decision::Abandon`]; the public verifier shows what the round would
//!   have picked.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::{
    algorithm,
    beacon::BeaconSource,
    command::{BeaconFetch, Decision, DrawRequest, Resolution},
    drand::VerifiedBeacon,
    draw::{CompletedDraw, Outcome, PendingDraw},
    error::Error,
    hash::{self, Hash, ManifestHash},
    id::{DrawingId, EntrantId, ItemId},
    manifest::{Manifest, ManifestItem},
};

/// Where an item stands, without its draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Available,
    Drawn,
    Claimed,
    Voided,
}

/// Where an item stands, with the draw that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    /// Can be drawn.
    Available,
    /// Draw `k` picked `winner`; the organizer has not resolved it yet.
    Drawn { k: u64, winner: EntrantId },
    /// Draw `k`'s winner took it.
    Claimed { k: u64, winner: EntrantId },
    /// Withdrawn: it will not be given away.
    Voided,
}

/// One item of a drawing, as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    pub state: ItemState,
}

impl Item {
    fn available(item: &ManifestItem) -> Self {
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            state: ItemState::Available,
        }
    }

    pub fn status(&self) -> ItemStatus {
        match self.state {
            ItemState::Available => ItemStatus::Available,
            ItemState::Drawn { .. } => ItemStatus::Drawn,
            ItemState::Claimed { .. } => ItemStatus::Claimed,
            ItemState::Voided => ItemStatus::Voided,
        }
    }

    /// The draw that holds this item (drawn or claimed).
    pub fn k(&self) -> Option<u64> {
        match self.state {
            ItemState::Drawn { k, .. } | ItemState::Claimed { k, .. } => Some(k),
            ItemState::Available | ItemState::Voided => None,
        }
    }

    /// The winner holding this item (drawn or claimed).
    pub fn winner(&self) -> Option<&EntrantId> {
        match &self.state {
            ItemState::Drawn { winner, .. } | ItemState::Claimed { winner, .. } => Some(winner),
            ItemState::Available | ItemState::Voided => None,
        }
    }
}

/// An entry in the drawing's event log, which is published and which the
/// verifier replays. Events are what happened, in the past tense; the
/// commands that cause them are in [`command`](crate::command).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A draw was announced: it committed to a future drand round.
    Announced {
        k: u64,
        item: ItemId,
        round: u64,
        /// Unix time at which the round is published.
        round_time: u64,
    },
    /// The announced draw's round arrived, and its randomness picked
    /// `entrant` from the pool.
    Drawn {
        k: u64,
        item: ItemId,
        round: u64,
        signature: String,
        randomness: String,
        pass: u8,
        pool_size: usize,
        index: usize,
        entrant: EntrantId,
    },
    /// The announced draw's beacon could not be had (yet). The draw stays
    /// pending on the same round.
    Stalled {
        k: u64,
        item: ItemId,
        round: u64,
        reason: String,
    },
    /// The organizer abandoned the pending draw; its item can be drawn again.
    /// The verifier shows whom the round would have picked.
    Abandoned {
        k: u64,
        item: ItemId,
        round: u64,
        reason: String,
    },
    /// The organizer resolved draw `k`'s winner.
    Resolved { k: u64, outcome: Resolution },
    /// The organizer withdrew an item.
    Voided { item: ItemId },
    /// The drawing ended.
    Finished,
}

/// One drawing: opened from its manifest, changed only by recording events.
///
/// Stored as its manifest and its event log (`{"manifest": …, "events": […]}`),
/// both published formats; everything else is rebuilt from them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "StoredDrawing", try_from = "StoredDrawing")]
pub struct Drawing {
    manifest: Manifest,
    manifest_hash: ManifestHash,
    items: Vec<Item>,
    events: Vec<Event>,
    next_k: u64,
    pending: Option<PendingDraw>,
    finished: bool,
}

/// A drawing as stored.
#[derive(Serialize, Deserialize)]
struct StoredDrawing {
    manifest: Manifest,
    events: Vec<Event>,
}

impl From<Drawing> for StoredDrawing {
    fn from(drawing: Drawing) -> Self {
        Self {
            manifest: drawing.manifest,
            events: drawing.events,
        }
    }
}

impl TryFrom<StoredDrawing> for Drawing {
    type Error = Error;

    fn try_from(stored: StoredDrawing) -> Result<Self, Error> {
        Self::restore(&stored.manifest, &stored.events)
    }
}

/// Refuses to draw from another beacon source than the drawing's.
fn check_beacon(drawing: &BeaconSource, engine: &BeaconSource) -> Result<(), Error> {
    if drawing == engine {
        Ok(())
    } else {
        Err(Error::BeaconSourceMismatch {
            drawing: *drawing,
            engine: *engine,
        })
    }
}

/// Reading a drawing.
impl Drawing {
    pub fn id(&self) -> &DrawingId {
        &self.manifest.drawing
    }

    /// The manifest the drawing was opened from.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn manifest_hash(&self) -> &ManifestHash {
        &self.manifest_hash
    }

    /// Where the draws' randomness comes from (from the manifest).
    pub fn beacon(&self) -> &BeaconSource {
        &self.manifest.beacon
    }

    /// The entrants, sorted.
    pub fn entrants(&self) -> &[EntrantId] {
        &self.manifest.entrants
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn item(&self, id: &ItemId) -> Option<&Item> {
        self.items.iter().find(|item| item.id == *id)
    }

    /// The event log, oldest first.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The draw announced and waiting for its beacon, if any.
    pub fn pending(&self) -> Option<&PendingDraw> {
        self.pending.as_ref()
    }

    /// Whether the drawing is finished: the engine may forget it.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Monotonic, one more on every change: of two copies of a drawing, the
    /// one with the higher version is newer.
    pub fn version(&self) -> u64 {
        1 + self.events.len() as u64
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
                Event::Stalled { k, .. } if *k == pending.k => Some(true),
                Event::Announced { k, .. } if *k == pending.k => Some(false),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Every completed draw so far, in order, with where its winner stands.
    pub fn draws(&self) -> Vec<CompletedDraw> {
        let mut outcomes: HashMap<u64, Outcome> = HashMap::new();
        for e in &self.events {
            if let Event::Resolved { k, outcome } = e {
                outcomes.insert(*k, (*outcome).into());
            }
        }
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Drawn {
                    k,
                    item,
                    round,
                    pass,
                    pool_size,
                    index,
                    entrant,
                    ..
                } => Some(CompletedDraw {
                    k: *k,
                    item: item.clone(),
                    round: *round,
                    pass: *pass,
                    pool_size: *pool_size,
                    index: *index,
                    entrant: entrant.clone(),
                    outcome: outcomes.get(k).copied().unwrap_or(Outcome::Drawn),
                }),
                _ => None,
            })
            .collect()
    }

    /// The pool for the next draw: `(pass, sorted entrant IDs)`.
    pub fn pool(&self) -> (u8, Vec<EntrantId>) {
        algorithm::pool(&self.manifest.entrants, &self.draws())
    }

    /// Number of items that can still be given away.
    pub fn items_left(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item.state, ItemState::Available | ItemState::Drawn { .. }))
            .count()
    }

    /// Where every drawn entrant stands, with the item of that draw: a claim
    /// over anything else, otherwise their latest draw.
    pub fn entrant_outcomes(&self) -> BTreeMap<EntrantId, (Outcome, ItemId)> {
        let mut out = BTreeMap::new();
        for draw in self.draws() {
            match out.get(&draw.entrant) {
                // Keep a claim over a later pass-2 forfeit.
                Some((Outcome::Claimed, _)) if draw.outcome != Outcome::Claimed => {}
                _ => {
                    out.insert(draw.entrant, (draw.outcome, draw.item));
                }
            }
        }
        out
    }

    /// The item draw `k` currently holds (drawn or claimed).
    fn item_held_by(&self, k: u64) -> Option<&Item> {
        self.items.iter().find(|item| item.k() == Some(k))
    }
}

/// The engine protocol.
impl Drawing {
    /// Opens a drawing from its manifest, which must be valid.
    pub fn open(manifest: &Manifest) -> Result<Self, Error> {
        manifest.validate()?;
        Ok(Self::open_unchecked(manifest))
    }

    /// Rebuilds a drawing from its manifest and its own event log.
    ///
    /// The log is trusted: only its consistency is checked (that it names
    /// items and draws that exist), not the rules or the draws' randomness.
    /// [`verify`](crate::verify) is what checks a published log.
    pub fn restore(manifest: &Manifest, events: &[Event]) -> Result<Self, Error> {
        let mut drawing = Self::open(manifest)?;
        for event in events {
            drawing.apply(event)?;
            drawing.events.push(event.clone());
        }
        Ok(drawing)
    }

    /// Opens a drawing from a manifest that may be invalid (the verifier
    /// replays it anyway, and reports it).
    pub(crate) fn open_unchecked(manifest: &Manifest) -> Self {
        Self {
            manifest: manifest.clone(),
            manifest_hash: manifest.hash(),
            items: manifest.items.iter().map(Item::available).collect(),
            events: Vec::new(),
            next_k: 1,
            pending: None,
            finished: false,
        }
    }

    /// Opening, as every engine applies it: `existing` is the engine's
    /// drawing for this ID, if any.
    ///
    /// Validates the manifest and refuses another beacon source than the
    /// engine's. Opening with the same manifest again is a no-op (`None`);
    /// another manifest is refused. Otherwise returns the fresh drawing to
    /// store.
    pub fn open_or_check(
        existing: Option<&Drawing>,
        manifest: &Manifest,
        engine_beacon: &BeaconSource,
    ) -> Result<Option<Drawing>, Error> {
        let fresh = Self::open(manifest)?;
        check_beacon(fresh.beacon(), engine_beacon)?;
        match existing {
            Some(existing) if existing.manifest_hash == fresh.manifest_hash => Ok(None),
            Some(_) => Err(Error::AlreadyOpen),
            None => Ok(Some(fresh)),
        }
    }

    /// Announces a draw: commits it to drand `round`, published at
    /// `round_time`. Refused with an engine on another beacon source than
    /// the drawing's.
    ///
    /// Announcing the item whose draw is pending again re-enters that draw on
    /// its own round (a retry after a stall), whatever `round` is now.
    pub fn announce(
        &mut self,
        request: &DrawRequest,
        engine_beacon: &BeaconSource,
        round: u64,
        round_time: u64,
    ) -> Result<PendingDraw, Error> {
        check_beacon(self.beacon(), engine_beacon)?;
        match &self.pending {
            Some(p) if p.item == request.item && !self.finished => Ok(p.clone()),
            _ => self.start(&request.item, round, round_time),
        }
    }

    /// Settles the pending draw with how its beacon fetch turned out.
    ///
    /// A verified beacon completes it (`Some`). A beacon that doesn't fit the
    /// draw (wrong round or source), an unavailable one and an interrupted
    /// attempt all stall it (`None`): the draw stays pending on its round,
    /// never abandoned, so a retry fetches the same round.
    pub fn settle(&mut self, fetch: BeaconFetch) -> Result<Option<CompletedDraw>, Error> {
        match fetch {
            BeaconFetch::Verified(beacon) => match self.complete(&beacon) {
                Ok(drawn) => Ok(Some(drawn)),
                Err(e) => self.stall(&format!("invalid beacon: {e}")).map(|()| None),
            },
            BeaconFetch::Unavailable(reason) => self
                .stall(&format!("drand unavailable: {reason}"))
                .map(|()| None),
            BeaconFetch::Interrupted(reason) => self.stall(&reason).map(|()| None),
        }
    }

    /// Applies a decision. After [`Decision::Finish`] the drawing is finished
    /// and the engine may forget it.
    pub fn decide(&mut self, decision: &Decision) -> Result<(), Error> {
        match decision {
            Decision::Resolve { item, outcome } => self.resolve(item, *outcome),
            Decision::Void { item } => self.void(item),
            Decision::Abandon { item } => match &self.pending {
                Some(p) if p.item == *item => self.abandon("abandoned by the organizer"),
                _ => Err(Error::NoDrawInProgress),
            },
            Decision::Finish => self.finish(),
        }
    }
}

/// The transitions: each checks the rules, then records its event. The
/// verifier replays a log through these.
impl Drawing {
    /// Starts a draw for `item`, committing to drand `round`.
    pub(crate) fn start(
        &mut self,
        item: &ItemId,
        round: u64,
        round_time: u64,
    ) -> Result<PendingDraw, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if self.pending.is_some() {
            return Err(Error::DrawInProgress);
        }
        let item = self
            .item(item)
            .ok_or_else(|| Error::UnknownItem(item.clone()))?;
        if item.state != ItemState::Available {
            return Err(Error::ItemNotAvailable(item.id.clone()));
        }
        let pending = PendingDraw {
            k: self.next_k,
            item: item.id.clone(),
            round,
            round_time,
        };
        if self.pool().1.is_empty() {
            return Err(Error::NoEligibleEntrants);
        }
        self.record(Event::Announced {
            k: pending.k,
            item: pending.item.clone(),
            round,
            round_time,
        });
        Ok(pending)
    }

    /// Completes the pending draw with the verified beacon for its committed
    /// round, from the drawing's beacon source.
    pub(crate) fn complete(&mut self, beacon: &VerifiedBeacon) -> Result<CompletedDraw, Error> {
        if BeaconSource::Drand(beacon.chain_hash()) != *self.beacon() {
            return Err(Error::WrongBeaconSource);
        }
        self.select(
            beacon.round(),
            *beacon.randomness(),
            &beacon.beacon().signature,
        )
    }

    /// Completes the pending draw with a beacon that could not be verified
    /// (the public verifier replaying a test chain's log). Never used to draw.
    pub(crate) fn complete_unverified(
        &mut self,
        round: u64,
        signature: &[u8],
    ) -> Result<CompletedDraw, Error> {
        self.select(round, hash::randomness(signature), signature)
    }

    /// Selects the pending draw's winner from `round`'s randomness.
    fn select(
        &mut self,
        round: u64,
        randomness: Hash,
        signature: &[u8],
    ) -> Result<CompletedDraw, Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        if round != pending.round {
            return Err(Error::WrongRound {
                expected: pending.round,
                got: round,
            });
        }
        let (pass, pool) = self.pool();
        if pool.is_empty() {
            // Unreachable while nothing may change a pending draw's pool; the
            // draw stays pending (only the organizer abandons it).
            return Err(Error::NoEligibleEntrants);
        }
        let index = algorithm::select(
            &randomness,
            self.manifest_hash.as_bytes(),
            pending.k,
            pool.len(),
        );
        let drawn = CompletedDraw {
            k: pending.k,
            item: pending.item,
            round,
            pass,
            pool_size: pool.len(),
            index,
            entrant: pool[index].clone(),
            outcome: Outcome::Drawn,
        };
        self.record(Event::Drawn {
            k: drawn.k,
            item: drawn.item.clone(),
            round,
            signature: hex::encode(signature),
            randomness: hex::encode(randomness),
            pass,
            pool_size: drawn.pool_size,
            index,
            entrant: drawn.entrant.clone(),
        });
        Ok(drawn)
    }

    /// Records that the pending draw's beacon could not be fetched or verified.
    /// The draw stays pending on the same round.
    pub(crate) fn stall(&mut self, reason: &str) -> Result<(), Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        self.record(Event::Stalled {
            k: pending.k,
            item: pending.item,
            round: pending.round,
            reason: reason.to_owned(),
        });
        Ok(())
    }

    /// Abandons the pending draw; its item can be drawn again. Logged
    /// publicly: the verifier shows what the committed round would have
    /// picked.
    pub(crate) fn abandon(&mut self, reason: &str) -> Result<(), Error> {
        let pending = self.pending.clone().ok_or(Error::NoDrawInProgress)?;
        self.record(Event::Abandoned {
            k: pending.k,
            item: pending.item,
            round: pending.round,
            reason: reason.to_owned(),
        });
        Ok(())
    }

    /// Resolves a drawn item's winner. A claim keeps the item with the
    /// winner; a forfeit makes it available again, and is refused while a
    /// draw is pending, because it can change that draw's pool.
    pub(crate) fn resolve(&mut self, id: &ItemId, outcome: Resolution) -> Result<(), Error> {
        if outcome.is_forfeit() && self.pending.is_some() {
            return Err(Error::DrawInProgress);
        }
        let item = self
            .item(id)
            .ok_or_else(|| Error::UnknownItem(id.clone()))?;
        let ItemState::Drawn { k, .. } = item.state else {
            return Err(Error::ItemNotDrawn(id.clone()));
        };
        self.record(Event::Resolved { k, outcome });
        Ok(())
    }

    /// Resolves the item draw `k` holds, as a published log names it.
    pub(crate) fn resolve_draw(
        &mut self,
        k: u64,
        outcome: Resolution,
    ) -> Option<Result<(), Error>> {
        let id = self.item_held_by(k)?.id.clone();
        Some(self.resolve(&id, outcome))
    }

    /// Withdraws an item that will not be given away.
    pub(crate) fn void(&mut self, id: &ItemId) -> Result<(), Error> {
        if self.pending.as_ref().is_some_and(|p| p.item == *id) {
            return Err(Error::DrawInProgress);
        }
        let item = self
            .item(id)
            .ok_or_else(|| Error::UnknownItem(id.clone()))?;
        if item.state != ItemState::Available {
            return Err(Error::ItemNotAvailable(id.clone()));
        }
        self.record(Event::Voided { item: id.clone() });
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<(), Error> {
        if self.finished {
            return Ok(());
        }
        if self.pending.is_some() {
            return Err(Error::DrawInProgress);
        }
        self.record(Event::Finished);
        Ok(())
    }

    /// Records an event the rules allowed: applies it and logs it.
    fn record(&mut self, event: Event) {
        self.apply(&event)
            .expect("a transition records an event that applies");
        self.events.push(event);
    }

    /// Applies an event to the state. Only the log's own consistency is
    /// checked (that it names items and draws that exist); the rules were
    /// checked when the event was recorded.
    fn apply(&mut self, event: &Event) -> Result<(), Error> {
        match event {
            Event::Announced {
                k,
                item,
                round,
                round_time,
            } => {
                self.pending = Some(PendingDraw {
                    k: *k,
                    item: item.clone(),
                    round: *round,
                    round_time: *round_time,
                });
            }
            Event::Drawn {
                k, item, entrant, ..
            } => {
                self.item_mut(item)?.state = ItemState::Drawn {
                    k: *k,
                    winner: entrant.clone(),
                };
                self.pending = None;
                self.next_k = k + 1;
            }
            Event::Stalled { .. } => {}
            Event::Abandoned { .. } => self.pending = None,
            Event::Resolved { k, outcome } => {
                let item = self
                    .items
                    .iter_mut()
                    .find(
                        |item| matches!(item.state, ItemState::Drawn { k: held, .. } if held == *k),
                    )
                    .ok_or(Error::UnknownDraw(*k))?;
                item.state = match (&item.state, outcome) {
                    (ItemState::Drawn { k, winner }, Resolution::Claimed) => ItemState::Claimed {
                        k: *k,
                        winner: winner.clone(),
                    },
                    _ => ItemState::Available,
                };
            }
            Event::Voided { item } => self.item_mut(item)?.state = ItemState::Voided,
            Event::Finished => self.finished = true,
        }
        Ok(())
    }

    fn item_mut(&mut self, id: &ItemId) -> Result<&mut Item, Error> {
        self.items
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or_else(|| Error::UnknownItem(id.clone()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{collections::HashSet, num::NonZeroU32};

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
            "rafi-drawing-test",
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

    pub(crate) fn source() -> BeaconSource {
        BeaconSource::Drand(test_chain().chain().hash())
    }

    /// `entrants` entrants and `items` items, `item-0`, `item-1`, ….
    pub(crate) fn manifest(entrants: usize, items: usize) -> Manifest {
        Manifest::new(
            "drawing",
            source(),
            (0..entrants).map(|i| format!("{i:032x}").into()).collect(),
            (0..items)
                .map(|i| ManifestItem {
                    id: format!("item-{i}").into(),
                    name: format!("Item {i}"),
                })
                .collect(),
        )
    }

    pub(crate) fn opened(entrants: usize, items: usize) -> Drawing {
        Drawing::open(&manifest(entrants, items)).unwrap()
    }

    fn draw(s: &mut Drawing, item: &str, round: u64) -> CompletedDraw {
        s.start(&item.into(), round, 0).unwrap();
        s.complete(&beacon(round)).unwrap()
    }

    fn status(s: &Drawing, item: &str) -> ItemStatus {
        s.item(&item.into()).unwrap().status()
    }

    fn request(item: &str) -> DrawRequest {
        DrawRequest { item: item.into() }
    }

    #[test]
    fn one_win_per_entrant() {
        let mut s = opened(3, 3);
        let mut winners = HashSet::new();
        for (i, item) in ["item-0", "item-1", "item-2"].into_iter().enumerate() {
            let d = draw(&mut s, item, 100 + i as u64);
            assert_eq!(d.pass, 1);
            assert!(winners.insert(d.entrant));
            s.resolve(&item.into(), Resolution::Claimed).unwrap();
        }
        assert_eq!(s.items_left(), 0);
    }

    #[test]
    fn not_here_is_out_and_the_item_is_drawn_again() {
        let mut s = opened(2, 1);
        let first = draw(&mut s, "item-0", 1);
        s.resolve(&"item-0".into(), Resolution::NotHere).unwrap();
        assert_eq!(status(&s, "item-0"), ItemStatus::Available);
        let second = draw(&mut s, "item-0", 2);
        assert_ne!(first.entrant, second.entrant);
        s.resolve(&"item-0".into(), Resolution::NotHere).unwrap();
        // Both entrants were drawn and forfeited: nobody is left.
        assert_eq!(
            s.start(&"item-0".into(), 3, 0),
            Err(Error::NoEligibleEntrants)
        );
    }

    #[test]
    fn pass_two_only_includes_winners() {
        let mut s = opened(2, 3);
        let a = draw(&mut s, "item-0", 1);
        s.resolve(&"item-0".into(), Resolution::Claimed).unwrap();
        draw(&mut s, "item-1", 2);
        s.resolve(&"item-1".into(), Resolution::Declined).unwrap();
        // Pass 1 exhausted; only the claimed winner is in pass 2.
        let (pass, pool) = s.pool();
        assert_eq!(pass, 2);
        assert_eq!(pool, vec![a.entrant.clone()]);
        let c = draw(&mut s, "item-1", 3);
        assert_eq!(c.entrant, a.entrant);
        assert_eq!(c.pass, 2);
        s.resolve(&"item-1".into(), Resolution::Claimed).unwrap();
        assert_eq!(
            s.start(&"item-2".into(), 4, 0),
            Err(Error::NoEligibleEntrants)
        );
    }

    #[test]
    fn announcing_again_re_enters_the_pending_draw() {
        let mut s = opened(5, 2);
        let p1 = s.announce(&request("item-0"), &source(), 10, 0).unwrap();
        let p2 = s.announce(&request("item-0"), &source(), 11, 0).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(s.events().len(), 1);
        assert_eq!(
            s.announce(&request("item-1"), &source(), 10, 0),
            Err(Error::DrawInProgress)
        );
        assert_eq!(s.void(&"item-0".into()), Err(Error::DrawInProgress));
        // The strict transition (what a replayed log goes through) refuses it.
        assert_eq!(s.start(&"item-0".into(), 10, 0), Err(Error::DrawInProgress));
    }

    #[test]
    fn announcing_refuses_another_beacon_source() {
        let mut s = opened(5, 1);
        let refused = s
            .announce(&request("item-0"), &BeaconSource::QUICKNET, 10, 0)
            .unwrap_err();
        assert!(matches!(refused, Error::BeaconSourceMismatch { .. }));
        assert_eq!(refused.kind(), ErrorKind::Conflict);
    }

    #[test]
    fn a_beacon_for_another_round_or_source_is_rejected() {
        let mut s = opened(5, 1);
        s.start(&"item-0".into(), 10, 0).unwrap();
        assert_eq!(
            s.complete(&beacon(11)),
            Err(Error::WrongRound {
                expected: 10,
                got: 11
            })
        );
        s.manifest.beacon = BeaconSource::QUICKNET;
        assert_eq!(s.complete(&beacon(10)), Err(Error::WrongBeaconSource));
        assert!(s.pending().is_some());
    }

    #[test]
    fn a_stall_keeps_the_draw_on_its_round() {
        let mut s = opened(5, 2);
        let p = s.announce(&request("item-0"), &source(), 10, 0).unwrap();
        s.stall("drand unavailable").unwrap();
        assert_eq!(s.pending(), Some(&p));
        // Retrying returns the same pending draw, even with a new round asked for.
        assert_eq!(s.announce(&request("item-0"), &source(), 99, 0).unwrap(), p);
        assert_eq!(
            s.announce(&request("item-1"), &source(), 99, 0),
            Err(Error::DrawInProgress)
        );
        let d = s.complete(&beacon(10)).unwrap();
        assert_eq!(d.k, 1);
    }

    #[test]
    fn an_abandon_keeps_k_and_makes_the_item_available() {
        let mut s = opened(5, 1);
        s.start(&"item-0".into(), 10, 0).unwrap();
        s.abandon("organizer abandoned the draw").unwrap();
        assert_eq!(s.next_k, 1);
        assert_eq!(status(&s, "item-0"), ItemStatus::Available);
        let d = draw(&mut s, "item-0", 12);
        assert_eq!(d.k, 1);
    }

    #[test]
    fn forfeits_wait_for_the_pending_draw() {
        let mut s = opened(2, 3);
        draw(&mut s, "item-0", 1);
        draw(&mut s, "item-1", 2);
        // Pass 2 is next; its pool depends on the pass-1 outcomes.
        s.start(&"item-2".into(), 3, 0).unwrap();
        let before = s.pool();
        assert_eq!(
            s.resolve(&"item-0".into(), Resolution::NotHere),
            Err(Error::DrawInProgress)
        );
        assert_eq!(
            s.resolve(&"item-0".into(), Resolution::Declined),
            Err(Error::DrawInProgress)
        );
        s.resolve(&"item-0".into(), Resolution::Claimed).unwrap();
        assert_eq!(s.pool(), before);
        assert_eq!(s.finish(), Err(Error::DrawInProgress));
    }

    #[test]
    fn void_and_finish() {
        let mut s = opened(5, 2);
        s.void(&"item-1".into()).unwrap();
        assert_eq!(s.items_left(), 1);
        assert_eq!(
            s.void(&"item-1".into()),
            Err(Error::ItemNotAvailable("item-1".into()))
        );
        s.finish().unwrap();
        assert!(s.is_finished());
        assert_eq!(s.start(&"item-0".into(), 1, 0), Err(Error::Finished));
        assert_eq!(
            s.announce(&request("item-0"), &source(), 1, 0),
            Err(Error::Finished)
        );
    }

    #[test]
    fn the_version_counts_the_events() {
        let mut s = opened(5, 1);
        assert_eq!(s.version(), 1);
        s.start(&"item-0".into(), 1, 0).unwrap();
        s.stall("x").unwrap();
        s.complete(&beacon(1)).unwrap();
        assert_eq!(s.version(), 4);
        assert_eq!(s.version(), 1 + s.events().len() as u64);
    }

    #[test]
    fn opening_validates_the_manifest() {
        assert!(matches!(
            Drawing::open(&manifest(0, 1)),
            Err(Error::InvalidManifest(_))
        ));
    }

    #[test]
    fn open_or_check_is_idempotent_per_manifest() {
        let m = manifest(3, 1);
        let fresh = Drawing::open_or_check(None, &m, &source())
            .unwrap()
            .unwrap();
        assert_eq!(fresh, Drawing::open(&m).unwrap());
        assert_eq!(
            Drawing::open_or_check(Some(&fresh), &m, &source()),
            Ok(None)
        );
        assert_eq!(
            Drawing::open_or_check(Some(&fresh), &manifest(4, 1), &source()),
            Err(Error::AlreadyOpen)
        );
    }

    #[test]
    fn open_or_check_refuses_another_beacon_source_and_invalid_manifests() {
        assert!(matches!(
            Drawing::open_or_check(None, &manifest(3, 1), &BeaconSource::QUICKNET),
            Err(Error::BeaconSourceMismatch { .. })
        ));
        assert!(matches!(
            Drawing::open_or_check(None, &manifest(0, 1), &source()),
            Err(Error::InvalidManifest(_))
        ));
    }

    #[test]
    fn settling_completes_or_stalls_on_the_same_round() {
        let mut s = opened(5, 2);
        let p = s.announce(&request("item-0"), &source(), 10, 0).unwrap();

        assert_eq!(s.settle(BeaconFetch::Verified(beacon(11))), Ok(None));
        assert!(s.stalled());
        assert_eq!(s.pending(), Some(&p));
        assert_eq!(
            s.settle(BeaconFetch::Unavailable("relays down".into())),
            Ok(None)
        );
        assert_eq!(
            s.settle(BeaconFetch::Interrupted("draw cancelled".into())),
            Ok(None)
        );
        assert!(matches!(
            s.events().last(),
            Some(Event::Stalled { reason, round: 10, .. }) if reason == "draw cancelled"
        ));

        let drawn = s
            .settle(BeaconFetch::Verified(beacon(10)))
            .unwrap()
            .unwrap();
        assert_eq!(drawn.k, 1);
        assert!(!s.stalled());

        assert_eq!(
            s.settle(BeaconFetch::Unavailable("x".into())),
            Err(Error::NoDrawInProgress)
        );
    }

    #[test]
    fn a_new_announcement_after_an_abandon_is_not_stalled() {
        let mut s = opened(5, 1);
        s.start(&"item-0".into(), 10, 0).unwrap();
        s.stall("x").unwrap();
        s.abandon("organizer").unwrap();
        assert!(!s.stalled());
        // The same k, announced again.
        s.start(&"item-0".into(), 12, 0).unwrap();
        assert!(!s.stalled());
    }

    #[test]
    fn decide_runs_every_decision() {
        let mut s = opened(5, 2);
        draw(&mut s, "item-0", 1);
        let resolve = Decision::Resolve {
            item: "item-0".into(),
            outcome: Resolution::Claimed,
        };
        s.decide(&resolve).unwrap();
        s.decide(&Decision::Void {
            item: "item-1".into(),
        })
        .unwrap();
        assert_eq!(
            s.decide(&Decision::Abandon {
                item: "item-1".into()
            }),
            Err(Error::NoDrawInProgress)
        );
        s.decide(&Decision::Finish).unwrap();
        assert!(s.is_finished());
    }

    #[test]
    fn draws_and_entrant_outcomes() {
        let mut s = opened(2, 2);
        let a = draw(&mut s, "item-0", 1);
        s.resolve(&"item-0".into(), Resolution::Claimed).unwrap();
        let b = draw(&mut s, "item-1", 2);
        let draws = s.draws();
        assert_eq!(draws.len(), 2);
        assert_eq!(draws[0].outcome, Outcome::Claimed);
        assert_eq!(draws[1].outcome, Outcome::Drawn);
        let outcomes = s.entrant_outcomes();
        assert_eq!(outcomes[&a.entrant], (Outcome::Claimed, "item-0".into()));
        assert_eq!(outcomes[&b.entrant], (Outcome::Drawn, "item-1".into()));
    }

    #[test]
    fn a_drawing_restores_from_its_log() {
        let m = manifest(3, 2);
        let mut s = Drawing::open(&m).unwrap();
        draw(&mut s, "item-0", 1);
        s.resolve(&"item-0".into(), Resolution::Claimed).unwrap();
        s.start(&"item-1".into(), 2, 0).unwrap();
        s.stall("x").unwrap();
        assert_eq!(Drawing::restore(&m, s.events()), Ok(s.clone()));

        let unknown = Event::Resolved {
            k: 9,
            outcome: Resolution::Claimed,
        };
        assert_eq!(Drawing::restore(&m, &[unknown]), Err(Error::UnknownDraw(9)));
    }

    #[test]
    fn a_drawing_is_stored_as_its_manifest_and_log() {
        let m = manifest(2, 1);
        let mut s = Drawing::open(&m).unwrap();
        draw(&mut s, "item-0", 1);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "manifest": serde_json::to_value(&m).unwrap(),
                "events": serde_json::to_value(s.events()).unwrap(),
            })
        );
        assert_eq!(serde_json::from_value::<Drawing>(json).unwrap(), s);

        // A log that isn't the drawing's own does not load.
        let bad = serde_json::json!({
            "manifest": serde_json::to_value(&m).unwrap(),
            "events": [{"type": "resolve", "k": 7, "outcome": "claimed"}],
        });
        assert!(serde_json::from_value::<Drawing>(bad).is_err());
    }

    #[test]
    fn error_kinds() {
        assert_eq!(
            Error::UnknownItem("item".into()).kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            Error::InvalidManifest("x".into()).kind(),
            ErrorKind::BadRequest
        );
        assert_eq!(Error::AlreadyOpen.kind(), ErrorKind::Conflict);
        assert_eq!(Error::DrawInProgress.kind(), ErrorKind::Conflict);
        assert_eq!(Error::WrongBeaconSource.kind(), ErrorKind::Invariant);
    }

    #[test]
    fn events_are_published_in_the_past_tense() {
        let mut s = opened(2, 2);
        s.start(&"item-0".into(), 1, 0).unwrap();
        s.stall("x").unwrap();
        s.abandon("y").unwrap();
        draw(&mut s, "item-0", 2);
        s.resolve(&"item-0".into(), Resolution::Claimed).unwrap();
        s.void(&"item-1".into()).unwrap();
        s.finish().unwrap();
        let types: Vec<_> = serde_json::to_value(s.events())
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["type"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            types,
            [
                "announced",
                "stalled",
                "abandoned",
                "announced",
                "drawn",
                "resolved",
                "voided",
                "finished"
            ]
        );
    }
}
