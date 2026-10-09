//! Reading what phones see from the database, and building the participant view.
//!
//! Whatever pushes live updates (a live channel, the polling endpoint, the
//! attendee page) reads the raffle through [`Snapshot`], so every one of them
//! shows the same thing. Everything comes from one [`Db::batch_rows`], so the version and the
//! content are one consistent snapshot: a write can't land between them and
//! leave a view labelled newer than what it holds.

use std::collections::HashMap;

use crate::{Db, Result, dialect};
use rafi_drawing::drawing::{DrawState, Event, Outcome, UnitStatus};
use rafi_live::{Current, DrawInput, DrawOutcome, ParticipantView, Phase};
use serde::Deserialize;

/// Display name used when the organizer hid an entrant's name.
pub const HIDDEN_NAME: &str = "★ ★ ★";

/// The raffle as phones see it, read in one snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub state: Phase,
    pub version: u64,
    pub live_ended_at: Option<i64>,
    drawing: Option<DrawState>,
    /// Names of the drawn entrants only, already masked when hidden.
    names: HashMap<String, String>,
    /// Prizes before the freeze, from the raffle's prizes.
    prize_qty: u32,
}

#[derive(Deserialize)]
struct RaffleRow {
    state: String,
    version: i64,
    live_ended_at: Option<i64>,
    drawing: Option<String>,
}

#[derive(Deserialize)]
struct NameRow {
    id: String,
    display_name: String,
    hidden: bool,
}

fn phase(state: &str) -> Phase {
    serde_json::from_value(serde_json::Value::String(state.to_owned())).unwrap_or(Phase::Draft)
}

impl Snapshot {
    /// Reads the raffle, or `None` if it doesn't exist.
    pub async fn read(db: &Db, raffle_id: &str) -> Result<Option<Self>> {
        let mut sets = db
            .batch_rows(vec![
                db.query(
                    "SELECT state, version, live_ended_at, drawing FROM raffles WHERE id = ?1",
                )
                .bind(raffle_id),
                // Only the entrants that were drawn: no other names leave the database.
                db.query(format!(
                    "SELECT e.id, e.display_name, e.hidden FROM entrants e
                     WHERE e.raffle_id = ?1 AND e.id IN ({})",
                    dialect::drawing_entrants(
                        db.dialect(),
                        "(SELECT drawing FROM raffles WHERE id = ?1)",
                        "events",
                        "type",
                        "draw",
                    )
                ))
                .bind(raffle_id),
                db.query("SELECT COALESCE(SUM(qty), 0) FROM prizes WHERE raffle_id = ?1")
                    .bind(raffle_id),
            ])
            .await?;
        let qty_rows = sets.pop().unwrap_or_default();
        let name_rows = sets.pop().unwrap_or_default();
        let Some(raffle) = sets.pop().and_then(|rows| rows.into_iter().next()) else {
            return Ok(None);
        };
        let raffle: RaffleRow = raffle.decode()?;
        let names = name_rows
            .into_iter()
            .map(|r| r.decode::<NameRow>())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|n| {
                let name = if n.hidden {
                    HIDDEN_NAME.to_owned()
                } else {
                    n.display_name
                };
                (n.id, name)
            })
            .collect();
        let prize_qty = qty_rows
            .into_iter()
            .next()
            .map(|r| r.decode::<(i64,)>())
            .transpose()?
            .map_or(0, |(q,)| q.max(0) as u32);
        Ok(Some(Self {
            state: phase(&raffle.state),
            version: raffle.version.max(0) as u64,
            live_ended_at: raffle.live_ended_at,
            drawing: raffle
                .drawing
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            names,
            prize_qty,
        }))
    }

    /// Whether the live channel still runs: live updates haven't been ended.
    pub fn channel_open(&self) -> bool {
        self.live_ended_at.is_none()
    }

    /// Whether phones should hold a socket: the channel is open and the raffle
    /// is public and not over.
    pub fn accepts_sockets(&self) -> bool {
        self.channel_open() && self.state.is_live()
    }

    /// The participant view.
    pub fn view(&self) -> ParticipantView {
        let Some(s) = &self.drawing else {
            return ParticipantView::new(self.version, self.state, None, [], self.prize_qty);
        };
        let unit_name = |unit_id: &str| {
            s.units
                .iter()
                .find(|u| u.unit_id == unit_id)
                .map(|u| u.item_name.clone())
                .unwrap_or_default()
        };
        let mut outcomes: HashMap<u64, Outcome> = HashMap::new();
        for e in &s.events {
            if let Event::Resolve { k, outcome } = e {
                outcomes.insert(*k, *outcome);
            }
        }
        let draws = s.events.iter().filter_map(|e| match e {
            Event::Draw {
                k,
                unit_id,
                entrant_id,
                ..
            } => Some(DrawInput {
                entrant_id: entrant_id.clone(),
                display_name: self
                    .names
                    .get(entrant_id)
                    .cloned()
                    .unwrap_or_else(|| format!("#{}", &entrant_id[..entrant_id.len().min(6)])),
                prize_name: unit_name(unit_id),
                outcome: match outcomes.get(k).copied().unwrap_or(Outcome::Drawn) {
                    Outcome::Drawn => DrawOutcome::Drawn,
                    Outcome::Claimed => DrawOutcome::Claimed,
                    Outcome::NotHere => DrawOutcome::NotHere,
                    Outcome::Declined => DrawOutcome::Declined,
                },
            }),
            _ => None,
        });
        let current = s.pending.as_ref().map(|p| Current {
            prize_name: unit_name(&p.unit_id),
            round: p.round,
            round_time: p.round_time,
        });
        let prizes_left = s
            .units
            .iter()
            .filter(|u| matches!(u.status, UnitStatus::Pending | UnitStatus::Drawn))
            .count() as u32;
        ParticipantView::new(
            self.version,
            self.state,
            current,
            draws.collect::<Vec<_>>(),
            prizes_left,
        )
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use rafi_drawing::manifest::{CommitDoc, UnitRef};

    use super::*;

    async fn raffle(db: &Db) -> DrawState {
        let doc = CommitDoc::new(
            "r1",
            &rafi_drawing::drand::chains::QUICKNET_HASH,
            vec!["a".repeat(32), "b".repeat(32)],
            vec![
                UnitRef {
                    unit_id: "u1".into(),
                    item_id: "i1".into(),
                    item_name: "Book".into(),
                },
                UnitRef {
                    unit_id: "u2".into(),
                    item_id: "i2".into(),
                    item_name: "Shirt".into(),
                },
            ],
        );
        let state = DrawState::freeze(&doc);
        db.query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, engine, manifest_hash, created_at, updated_at)
             VALUES ('r1', 'o', 's', 'j', 't', 'open', 'restate', ?, 0, 0)",
        )
        .bind(&state.list_hash)
        .run()
        .await
        .unwrap();
        for (id, name, hidden) in [
            ("a".repeat(32), "Anna", false),
            ("b".repeat(32), "Bence", true),
        ] {
            db.query(
                "INSERT INTO entrants (id, raffle_id, display_name, claim_code, hidden, created_at)
                 VALUES (?, 'r1', ?, 'X', ?, 0)",
            )
            .bind(id)
            .bind(name)
            .bind(hidden)
            .run()
            .await
            .unwrap();
        }
        db.query("INSERT INTO prizes (id, raffle_id, name, qty) VALUES ('i1', 'r1', 'Book', 1), ('i2', 'r1', 'Shirt', 1)")
            .run()
            .await
            .unwrap();
        state
    }

    fn draw(state: &mut DrawState, entrant: &str, unit: &str) -> u64 {
        let k = state.next_k;
        state.events.push(Event::Draw {
            k,
            unit_id: unit.into(),
            round: 1,
            signature: String::new(),
            randomness: String::new(),
            mode: rafi_drawing::drawing::DrawMode::Beacon,
            pass: 1,
            pool_size: 2,
            index: 0,
            entrant_id: entrant.into(),
        });
        state.next_k += 1;
        let u = state.units.iter_mut().find(|u| u.unit_id == unit).unwrap();
        u.status = UnitStatus::Drawn;
        u.entrant_id = Some(entrant.into());
        u.k = Some(k);
        k
    }

    async fn store(db: &Db, state: &DrawState, lifecycle: &str) {
        db.query("UPDATE raffles SET state = ?, drawing = ? WHERE id = 'r1'")
            .bind(lifecycle)
            .bind(serde_json::to_string(state).unwrap())
            .run()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn before_the_freeze_counts_prizes() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        raffle(&db).await;
        let snap = Snapshot::read(&db, "r1").await.unwrap().unwrap();
        let view = snap.view();
        assert_eq!(view.state, Phase::Open);
        assert_eq!(view.prizes_left, 2);
        assert!(view.winners.is_empty());
        assert_eq!(view.v, snap.version);
        assert!(snap.accepts_sockets());
        assert!(Snapshot::read(&db, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn draws_carry_only_drawn_names_masked_when_hidden() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        let mut state = raffle(&db).await;
        let _ = draw(&mut state, &"b".repeat(32), "u1");
        store(&db, &state, "drawing").await;
        let snap = Snapshot::read(&db, "r1").await.unwrap().unwrap();
        assert_eq!(
            snap.names.len(),
            1,
            "Anna was not drawn, so her name isn't read"
        );
        let view = snap.view();
        assert_eq!(view.last.as_ref().unwrap().display_name, HIDDEN_NAME);
        assert_eq!(view.winners[0].prize_name, "Book");
        assert_eq!(
            view.prizes_left, 2,
            "a drawn unit is still open until claimed"
        );
    }

    #[tokio::test]
    async fn no_shows_drop_out_and_claims_show() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        let mut state = raffle(&db).await;
        let k = draw(&mut state, &"a".repeat(32), "u1");
        state.events.push(Event::Resolve {
            k,
            outcome: Outcome::Claimed,
        });
        state.units[0].status = UnitStatus::Claimed;
        let k = draw(&mut state, &"b".repeat(32), "u2");
        state.events.push(Event::Resolve {
            k,
            outcome: Outcome::NotHere,
        });
        store(&db, &state, "drawing").await;
        let view = Snapshot::read(&db, "r1").await.unwrap().unwrap().view();
        assert_eq!(view.winners.len(), 1);
        assert!(view.winners[0].claimed);
        assert_eq!(
            view.last.unwrap().display_name,
            "Anna",
            "the no-show is skipped"
        );
    }

    #[tokio::test]
    async fn ended_channels_refuse_sockets() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        raffle(&db).await;
        db.query("UPDATE raffles SET state = 'finished', live_ended_at = 5 WHERE id = 'r1'")
            .run()
            .await
            .unwrap();
        let snap = Snapshot::read(&db, "r1").await.unwrap().unwrap();
        assert!(!snap.channel_open());
        assert!(!snap.accepts_sockets());
    }
}
