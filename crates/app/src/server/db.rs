//! SQLite access.

use rafi_core::raffle::RaffleState;
use sqlx::{FromRow, SqlitePool, sqlite::SqlitePoolOptions};

pub type Db = SqlitePool;

pub async fn connect(url: &str) -> anyhow::Result<Db> {
    let in_memory = url.contains(":memory:");
    let opts = rafi_store::options(url)?.create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        // An in-memory database exists per connection; keep exactly one.
        .max_connections(if in_memory { 1 } else { 8 })
        .min_connections(if in_memory { 1 } else { 0 })
        .idle_timeout(if in_memory {
            None
        } else {
            Some(std::time::Duration::from_secs(600))
        })
        .max_lifetime(if in_memory {
            None
        } else {
            Some(std::time::Duration::from_secs(3600))
        })
        .connect_with(opts)
        .await?;
    rafi_store::migrate(&pool).await?;
    Ok(pool)
}

#[allow(dead_code)] // mirrors the table; not every column is read
#[derive(Debug, Clone, FromRow)]
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
#[derive(Debug, Clone, FromRow)]
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
#[derive(Debug, Clone, FromRow)]
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

pub async fn raffle(db: &Db, id: &str) -> sqlx::Result<Option<RaffleRow>> {
    sqlx::query_as("SELECT * FROM raffles WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn raffle_by_slug(db: &Db, slug: &str) -> sqlx::Result<Option<RaffleRow>> {
    sqlx::query_as("SELECT * FROM raffles WHERE slug = ?")
        .bind(slug)
        .fetch_optional(db)
        .await
}

pub async fn raffle_by_join_code(db: &Db, code: &str) -> sqlx::Result<Option<RaffleRow>> {
    sqlx::query_as("SELECT * FROM raffles WHERE join_code = ?")
        .bind(code)
        .fetch_optional(db)
        .await
}

pub async fn items(db: &Db, raffle_id: &str) -> sqlx::Result<Vec<ItemRow>> {
    sqlx::query_as("SELECT * FROM items WHERE raffle_id = ? ORDER BY position, rowid")
        .bind(raffle_id)
        .fetch_all(db)
        .await
}

pub async fn entrants(db: &Db, raffle_id: &str) -> sqlx::Result<Vec<EntrantRow>> {
    sqlx::query_as("SELECT * FROM entrants WHERE raffle_id = ? ORDER BY created_at, rowid")
        .bind(raffle_id)
        .fetch_all(db)
        .await
}

pub async fn entrant(db: &Db, raffle_id: &str, id: &str) -> sqlx::Result<Option<EntrantRow>> {
    sqlx::query_as("SELECT * FROM entrants WHERE raffle_id = ? AND id = ?")
        .bind(raffle_id)
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn entrant_by_token(
    db: &Db,
    raffle_id: &str,
    token_hash: &str,
) -> sqlx::Result<Option<EntrantRow>> {
    sqlx::query_as("SELECT * FROM entrants WHERE raffle_id = ? AND token_hash = ?")
        .bind(raffle_id)
        .bind(token_hash)
        .fetch_optional(db)
        .await
}

pub async fn audit(
    db: &Db,
    raffle_id: &str,
    actor: &str,
    action: &str,
    detail: &str,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO audit (raffle_id, at, actor, action, detail) VALUES (?, ?, ?, ?, ?)")
        .bind(raffle_id)
        .bind(super::ids::now())
        .bind(actor)
        .bind(action)
        .bind(detail)
        .execute(db)
        .await
        .map(|_| ())
}
