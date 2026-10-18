-- Rafi schema. Times are unix seconds unless noted.

CREATE TABLE raffles (
    id TEXT PRIMARY KEY NOT NULL,
    owner_id TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    join_code TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    lang TEXT NOT NULL DEFAULT 'en',
    organizer_name TEXT NOT NULL DEFAULT '',
    contact_email TEXT NOT NULL DEFAULT '',
    rules TEXT NOT NULL DEFAULT '',
    event_date TEXT NOT NULL DEFAULT '',
    min_age INTEGER NOT NULL DEFAULT 16,
    state TEXT NOT NULL DEFAULT 'draft',
    test_mode INTEGER NOT NULL DEFAULT 0,
    -- Which draw engine owns this raffle's draw state ('restate' or 'local').
    engine TEXT NOT NULL DEFAULT 'local',
    -- Frozen entrant list (canonical JSON, exactly as hashed) and its hash.
    commit_doc TEXT,
    list_hash TEXT,
    frozen_at INTEGER,
    -- Projection of the draw engine's state (rafi_core::state::DrawState JSON).
    draw_state TEXT,
    draw_version INTEGER NOT NULL DEFAULT 0,
    finished_at INTEGER,
    purge_nonwinners_at INTEGER,
    purge_winners_at INTEGER,
    purged_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX raffles_owner ON raffles (owner_id);

CREATE TABLE items (
    id TEXT PRIMARY KEY NOT NULL,
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sponsor TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    qty INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX items_raffle ON items (raffle_id, position);

CREATE TABLE entrants (
    -- Public, random 128-bit hex ID (appears in the published entrant list).
    id TEXT PRIMARY KEY NOT NULL,
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    email_raw TEXT,
    email_norm TEXT,
    source TEXT NOT NULL DEFAULT 'self',
    hidden INTEGER NOT NULL DEFAULT 0,
    -- Private code shown only to the entrant and the organizer.
    claim_code TEXT NOT NULL,
    -- SHA-256 of the secret entrant token stored in the attendee's cookie.
    token_hash TEXT,
    consent_version TEXT,
    consent_rules_at INTEGER,
    consent_marketing INTEGER NOT NULL DEFAULT 0,
    delivered_at INTEGER,
    purged INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE (raffle_id, email_norm)
);

CREATE INDEX entrants_raffle ON entrants (raffle_id, created_at);
CREATE INDEX entrants_token ON entrants (token_hash);

CREATE TABLE audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raffle_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);

CREATE INDEX audit_raffle ON audit (raffle_id, id);
