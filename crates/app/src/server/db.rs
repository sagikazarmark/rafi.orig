//! Database access, through rafi-store.

use rafi_core::raffle::RaffleState;
pub use rafi_store::Db;
use serde::Deserialize;

#[cfg(not(target_arch = "wasm32"))]
pub async fn connect(url: &str) -> anyhow::Result<Db> {
    Ok(Db::connect(url).await?)
}

#[allow(dead_code)] // mirrors the table; not every column is read
#[derive(Debug, Clone, Deserialize)]
pub struct RaffleRow {
    pub id: String,
    pub owner_id: String,
    pub slug: String,
    pub join_code: String,
    pub title: String,
    pub description: String,
    pub lang: String,
    pub organizer_name: String,
    pub contact_email: String,
    pub rules: String,
    pub event_date: String,
    pub min_age: i64,
    pub state: String,
    pub test_mode: bool,
    pub engine: String,
    pub commit_doc: Option<String>,
    pub list_hash: Option<String>,
    pub frozen_at: Option<i64>,
    pub draw_state: Option<String>,
    pub draw_version: i64,
    pub finished_at: Option<i64>,
    pub purge_nonwinners_at: Option<i64>,
    pub purge_winners_at: Option<i64>,
    pub purged_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Bumped by every write that can change what screens or phones show
    /// (triggers, migration 0002). Live views carry it as `v`.
    pub version: i64,
    /// When live updates ended: the raffle's live channel is gone.
    pub live_ended_at: Option<i64>,
}

impl RaffleRow {
    pub fn state(&self) -> RaffleState {
        RaffleState::parse(&self.state).unwrap_or(RaffleState::Draft)
    }

    pub fn draw_state(&self) -> Option<rafi_core::state::DrawState> {
        self.draw_state
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
    }
}

#[allow(dead_code)] // mirrors the table; not every column is read
#[derive(Debug, Clone, Deserialize)]
pub struct ItemRow {
    pub id: String,
    pub raffle_id: String,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: i64,
    pub position: i64,
}

#[allow(dead_code)] // mirrors the table; not every column is read
#[derive(Debug, Clone, Deserialize)]
pub struct EntrantRow {
    pub id: String,
    pub raffle_id: String,
    pub display_name: String,
    pub email_raw: Option<String>,
    pub email_norm: Option<String>,
    pub source: String,
    pub hidden: bool,
    pub claim_code: String,
    pub token_hash: Option<String>,
    pub consent_version: Option<String>,
    pub consent_rules_at: Option<i64>,
    pub consent_marketing: bool,
    pub delivered_at: Option<i64>,
    pub purged: bool,
    pub created_at: i64,
}

pub async fn raffle(db: &Db, id: &str) -> rafi_store::Result<Option<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE id = ?")
        .bind(id)
        .optional()
        .await
}

pub async fn raffle_by_slug(db: &Db, slug: &str) -> rafi_store::Result<Option<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE slug = ?")
        .bind(slug)
        .optional()
        .await
}

pub async fn raffle_by_join_code(db: &Db, code: &str) -> rafi_store::Result<Option<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE join_code = ?")
        .bind(code)
        .optional()
        .await
}

pub async fn items(db: &Db, raffle_id: &str) -> rafi_store::Result<Vec<ItemRow>> {
    db.query("SELECT * FROM items WHERE raffle_id = ? ORDER BY position, rowid")
        .bind(raffle_id)
        .all()
        .await
}

pub async fn entrants(db: &Db, raffle_id: &str) -> rafi_store::Result<Vec<EntrantRow>> {
    db.query("SELECT * FROM entrants WHERE raffle_id = ? ORDER BY created_at, rowid")
        .bind(raffle_id)
        .all()
        .await
}

pub async fn entrant(db: &Db, raffle_id: &str, id: &str) -> rafi_store::Result<Option<EntrantRow>> {
    db.query("SELECT * FROM entrants WHERE raffle_id = ? AND id = ?")
        .bind(raffle_id)
        .bind(id)
        .optional()
        .await
}

pub async fn entrant_by_token(
    db: &Db,
    raffle_id: &str,
    token_hash: &str,
) -> rafi_store::Result<Option<EntrantRow>> {
    db.query("SELECT * FROM entrants WHERE raffle_id = ? AND token_hash = ?")
        .bind(raffle_id)
        .bind(token_hash)
        .optional()
        .await
}

pub async fn audit(
    db: &Db,
    raffle_id: &str,
    actor: &str,
    action: &str,
    detail: &str,
) -> rafi_store::Result<()> {
    db.query("INSERT INTO audit (raffle_id, at, actor, action, detail) VALUES (?, ?, ?, ?, ?)")
        .bind(raffle_id)
        .bind(super::ids::now())
        .bind(actor)
        .bind(action)
        .bind(detail)
        .run()
        .await
        .map(|_| ())
}
