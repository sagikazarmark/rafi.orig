//! Reading what phones see from the database.
//!
//! Whatever pushes live updates (a live channel, the polling endpoint, the
//! participant page) reads the raffle through [`snapshot`], so every one of them
//! shows the same thing. The [`Snapshot`] and its rules are `rafi-lifecycle`'s;
//! the servers that show phones a raffle build the participant view from it with
//! `rafi_live_server::raffle::view`. Everything comes from one
//! [`Db::batch_rows`], so the version and the content are one consistent
//! snapshot: a write can't land between them and leave a view labelled newer
//! than what it holds.

use crate::{Db, Result, raffles};
use rafi_lifecycle::RaffleState;
use rafi_lifecycle::live::Snapshot;
use serde::Deserialize;

#[derive(Deserialize)]
struct RaffleRow {
    state: RaffleState,
    version: i64,
    live_ended_at: Option<i64>,
    manifest: Option<String>,
    opened_at: Option<i64>,
}

#[derive(Deserialize)]
struct NameRow {
    id: String,
    display_name: Option<String>,
}

/// Reads the raffle, or `None` if it doesn't exist.
pub async fn snapshot(db: &Db, raffle_id: &str) -> Result<Option<Snapshot>> {
    let mut sets = db
        .batch_rows(vec![
            db.query(
                "SELECT state, version, live_ended_at, manifest, opened_at FROM raffles WHERE id = ?1",
            )
            .bind(raffle_id),
            // Only the entrants that were drawn, and only the names shown: an
            // erased entrant has none (phones show the start of their ID).
            db.query(
                "SELECT e.id, CASE WHEN e.hidden = 1 THEN NULL ELSE e.display_name END AS display_name
                 FROM entrants e
                 WHERE e.raffle_id = ?1 AND e.erased = 0
                   AND e.id IN (SELECT entrant_id FROM draws WHERE raffle_id = ?1)",
            )
            .bind(raffle_id),
            db.query("SELECT COALESCE(SUM(qty), 0) FROM prizes WHERE raffle_id = ?1")
                .bind(raffle_id),
            raffles::events_query(db, raffle_id),
        ])
        .await?;
    let event_rows = sets.pop().unwrap_or_default();
    let qty_rows = sets.pop().unwrap_or_default();
    let name_rows = sets.pop().unwrap_or_default();
    let Some(raffle) = sets.pop().and_then(|rows| rows.into_iter().next()) else {
        return Ok(None);
    };
    let raffle: RaffleRow = raffle.decode()?;
    let names = name_rows
        .into_iter()
        .map(|r| r.decode::<NameRow>().map(|n| (n.id, n.display_name)))
        .collect::<Result<_>>()?;
    let prize_qty = qty_rows
        .into_iter()
        .next()
        .map(|r| r.decode::<(i64,)>())
        .transpose()?
        .map_or(0, |(q,)| q.max(0) as u32);
    Ok(Some(Snapshot {
        state: raffle.state,
        version: raffle.version.max(0) as u64,
        live_ended_at: raffle.live_ended_at,
        drawn: raffles::rebuild(raffle.opened_at, raffle.manifest.as_deref(), event_rows)?,
        names,
        prize_qty,
    }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::HashMap;

    use rafi_raffle::{BeaconSource, Event, Manifest, ManifestItem, Raffle, Resolution};

    use super::*;

    /// A raffle as its engine would have logged it.
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
                .filter(|e| matches!(e, Event::Drawn { .. }))
                .count() as u64
                + 1;
            self.events.push(Event::Drawn {
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

        fn resolve(&mut self, k: u64, resolution: Resolution) {
            self.events.push(Event::Resolved { k, resolution });
        }

        fn raffle(&self) -> Raffle {
            Raffle::restore(&self.manifest, &self.events).unwrap()
        }
    }

    async fn raffle(db: &Db) -> Log {
        let manifest = Manifest::new(
            "r1",
            BeaconSource::QUICKNET,
            vec!["a".repeat(32).into(), "b".repeat(32).into()],
            vec![
                ManifestItem {
                    id: "u1".into(),
                    name: "Book".into(),
                },
                ManifestItem {
                    id: "u2".into(),
                    name: "Shirt".into(),
                },
            ],
        );
        db.query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, manifest, manifest_hash, event_date,
                                  event_ends_at, retention_days, created_at, updated_at)
             VALUES ('r1', 'o', 's', 'j', 't', 'open', ?, ?, '2100-01-01', 4102444800, 7, 0, 0)",
        )
        .bind(manifest.canonical_json())
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

    /// Stores the raffle as its engine would, in `lifecycle`.
    async fn store(db: &Db, log: &Log, lifecycle: &str) {
        db.query("UPDATE raffles SET state = ? WHERE id = 'r1'")
            .bind(lifecycle)
            .run()
            .await
            .unwrap();
        assert!(crate::save_raffle(db, &log.raffle(), 1).await.unwrap());
    }

    #[tokio::test]
    async fn before_the_freeze_counts_prizes() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        raffle(&db).await;
        let snap = snapshot(&db, "r1").await.unwrap().unwrap();
        assert_eq!(snap.state, RaffleState::Open);
        assert_eq!(snap.prize_qty, 2);
        assert!(snap.drawn.is_none() && snap.names.is_empty());
        assert!(snap.accepts_sockets());
        assert!(snapshot(&db, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reads_only_drawn_names_and_not_hidden_ones() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        let mut log = raffle(&db).await;
        log.draw(&"b".repeat(32), "u1");
        store(&db, &log, "drawing").await;
        let snap = snapshot(&db, "r1").await.unwrap().unwrap();
        assert_eq!(
            snap.names,
            HashMap::from([("b".repeat(32), None)]),
            "Anna was not drawn, Bence is hidden: neither name is read"
        );
        let k = log.draw(&"a".repeat(32), "u2");
        log.resolve(k, Resolution::Claimed);
        store(&db, &log, "drawing").await;
        let snap = snapshot(&db, "r1").await.unwrap().unwrap();
        assert_eq!(snap.names[&"a".repeat(32)].as_deref(), Some("Anna"));
        assert_eq!(snap.drawn.unwrap().draws().len(), 2);
    }

    #[tokio::test]
    async fn a_log_that_does_not_load_is_an_error() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        let log = raffle(&db).await;
        store(&db, &log, "drawing").await;
        assert!(snapshot(&db, "r1").await.unwrap().unwrap().drawn.is_some());
        db.query(
            r#"INSERT INTO raffle_events (raffle_id, seq, type, body, recorded_at)
               VALUES ('r1', 1, 'drawn', '{"items": []}', 0)"#,
        )
        .run()
        .await
        .unwrap();
        assert!(snapshot(&db, "r1").await.is_err());
    }

    #[tokio::test]
    async fn an_unknown_state_is_an_error() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        raffle(&db).await;
        db.query("UPDATE raffles SET state = 'paused' WHERE id = 'r1'")
            .run()
            .await
            .unwrap();
        assert!(snapshot(&db, "r1").await.is_err(), "never guessed");
        assert!(crate::raffles::get(&db, "r1").await.is_err());
    }

    #[tokio::test]
    async fn ended_channels_refuse_sockets() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        raffle(&db).await;
        db.query("UPDATE raffles SET state = 'finished', live_ended_at = 5 WHERE id = 'r1'")
            .run()
            .await
            .unwrap();
        let snap = snapshot(&db, "r1").await.unwrap().unwrap();
        assert!(!snap.channel_open());
        assert!(!snap.accepts_sockets());
    }
}
