//! Reading what phones see from the database, and building the participant view.
//!
//! Whatever pushes live updates (a live channel, the polling endpoint, the
//! participant page) reads the raffle through [`Snapshot`], so every one of them
//! shows the same thing. Everything comes from one [`Db::batch_rows`], so the version and the
//! content are one consistent snapshot: a write can't land between them and
//! leave a view labelled newer than what it holds.

use std::collections::HashMap;

use crate::{Db, Result, dialect};
use rafi_drawing::{Drawing, ItemId, Outcome};
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
    drawing: Option<Drawing>,
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
                    dialect::entrants_in(
                        db.dialect(),
                        "(SELECT drawn_entrants FROM raffles WHERE id = ?1)",
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
                .map(crate::parse_drawing)
                .transpose()?,
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
        let item_name = |id: &ItemId| s.item(id).map(|i| i.name.clone()).unwrap_or_default();
        let draws = s.draws().into_iter().map(|d| DrawInput {
            display_name: self
                .names
                .get(d.entrant.as_str())
                .cloned()
                .unwrap_or_else(|| {
                    let id = d.entrant.as_str();
                    format!("#{}", &id[..id.len().min(6)])
                }),
            entrant_id: d.entrant.into_string(),
            prize_name: item_name(&d.item),
            outcome: match d.outcome {
                Outcome::Drawn => DrawOutcome::Drawn,
                Outcome::Claimed => DrawOutcome::Claimed,
                Outcome::NotHere => DrawOutcome::NotHere,
                Outcome::Declined => DrawOutcome::Declined,
            },
        });
        let current = s.pending().map(|p| Current {
            prize_name: item_name(&p.item),
            round: p.round,
            round_time: p.round_time,
        });
        let prizes_left = s.items_left() as u32;
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
    use rafi_drawing::{BeaconSource, Event, ItemRef, Manifest, Resolution};

    use super::*;

    /// A drawing as its engine would have logged it.
    struct Log {
        manifest: Manifest,
        events: Vec<Event>,
    }

    impl Log {
        /// Logs a completed draw of `item` for `entrant`; returns its `k`.
        fn draw(&mut self, entrant: &str, item: &str) -> u64 {
            let k = self
                .events
                .iter()
                .filter(|e| matches!(e, Event::Draw { .. }))
                .count() as u64
                + 1;
            self.events.push(Event::Draw {
                k,
                item: item.into(),
                round: 1,
                signature: String::new(),
                randomness: String::new(),
                pass: 1,
                pool_size: 2,
                index: 0,
                entrant: entrant.into(),
            });
            k
        }

        fn resolve(&mut self, k: u64, outcome: Resolution) {
            self.events.push(Event::Resolve { k, outcome });
        }

        fn drawing(&self) -> Drawing {
            Drawing::restore(&self.manifest, &self.events).unwrap()
        }
    }

    async fn raffle(db: &Db) -> Log {
        let manifest = Manifest::new(
            "r1",
            BeaconSource::QUICKNET,
            vec!["a".repeat(32).into(), "b".repeat(32).into()],
            vec![
                ItemRef {
                    id: "u1".into(),
                    name: "Book".into(),
                },
                ItemRef {
                    id: "u2".into(),
                    name: "Shirt".into(),
                },
            ],
        );
        db.query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, engine, manifest_hash, created_at, updated_at)
             VALUES ('r1', 'o', 's', 'j', 't', 'open', 'restate', ?, 0, 0)",
        )
        .bind(manifest.hash().to_string())
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
        Log {
            manifest,
            events: Vec::new(),
        }
    }

    async fn store(db: &Db, log: &Log, lifecycle: &str) {
        let stored = crate::StoredDrawing::of(&log.drawing()).unwrap();
        db.query("UPDATE raffles SET state = ?, drawing = ?, drawn_entrants = ? WHERE id = 'r1'")
            .bind(lifecycle)
            .bind(stored.json)
            .bind(stored.drawn_entrants)
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
        let mut log = raffle(&db).await;
        log.draw(&"b".repeat(32), "u1");
        store(&db, &log, "drawing").await;
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
            "a drawn item is still open until claimed"
        );
    }

    #[tokio::test]
    async fn no_shows_drop_out_and_claims_show() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        let mut log = raffle(&db).await;
        let k = log.draw(&"a".repeat(32), "u1");
        log.resolve(k, Resolution::Claimed);
        let k = log.draw(&"b".repeat(32), "u2");
        log.resolve(k, Resolution::NotHere);
        store(&db, &log, "drawing").await;
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
    async fn a_drawing_that_does_not_load_is_an_error() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        raffle(&db).await;
        db.query(r#"UPDATE raffles SET drawing = '{"items": []}' WHERE id = 'r1'"#)
            .run()
            .await
            .unwrap();
        assert!(Snapshot::read(&db, "r1").await.is_err());
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
