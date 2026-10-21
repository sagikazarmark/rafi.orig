-- Rafi schema for SQLite and D1. Times are unix seconds unless noted.

CREATE TABLE raffles (
    id TEXT PRIMARY KEY NOT NULL,
    owner_id TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    join_code TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    organizer_name TEXT NOT NULL DEFAULT '',
    contact_email TEXT NOT NULL DEFAULT '',
    -- The organizer's privacy contact (falls back to contact_email), and their
    -- own privacy text or notice, shown after the generated notice.
    privacy_email TEXT NOT NULL DEFAULT '',
    privacy_text TEXT NOT NULL DEFAULT '',
    privacy_url TEXT NOT NULL DEFAULT '',
    rules TEXT NOT NULL DEFAULT '',
    -- The event date as a calendar day (YYYY-MM-DD, for display), the instant
    -- that day ends, and the retention period: the erasure date follows,
    -- event_ends_at + retention_days days. All three are written by the
    -- settings write only, and lock with the rules once a real entrant has
    -- joined, so the date shown to entrants never moves afterwards.
    event_date TEXT NOT NULL,
    event_ends_at INTEGER NOT NULL,
    retention_days INTEGER NOT NULL,
    erase_at INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'draft',
    test_mode INTEGER NOT NULL DEFAULT 0,
    -- The raffle's manifest: the frozen entrant list (canonical JSON, exactly
    -- as hashed), and its hash.
    manifest TEXT,
    manifest_hash TEXT,
    frozen_at INTEGER,
    -- Set when the engine first wrote the raffle: opening records no event, so
    -- an opened raffle may have none.
    opened_at INTEGER,
    finished_at INTEGER,
    -- Set when the organizer ends live updates; the live channel is gone after it.
    live_ended_at INTEGER,
    purged_at INTEGER,
    -- Set when the owner removed their data: the organizer's name and
    -- contacts are gone for good, and the notice says so.
    organizer_left_at INTEGER,
    -- The raffle's version: bumped by every write that can change what screens
    -- or phones show (the triggers below). Live views carry it, so clients and
    -- the live channel can order them without a process-wide counter.
    version INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX raffles_owner ON raffles (owner_id);

-- Housekeeping finds what is due to be erased by this.
CREATE INDEX raffles_erase_at ON raffles (erase_at) WHERE purged_at IS NULL;

CREATE TABLE prizes (
    id TEXT PRIMARY KEY NOT NULL,
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sponsor TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    qty INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX prizes_raffle ON prizes (raffle_id, position);

CREATE TABLE entrants (
    -- Public, random 128-bit hex ID (appears in the published entrant list).
    id TEXT PRIMARY KEY NOT NULL,
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    email_raw TEXT,
    email_norm TEXT,
    source TEXT NOT NULL DEFAULT 'self',
    hidden INTEGER NOT NULL DEFAULT 0,
    -- Private code shown only to the entrant and the organizer; NULL once
    -- their personal data is erased.
    claim_code TEXT,
    -- SHA-256 of the secret entrant token stored in the attendee's cookie.
    token_hash TEXT,
    consent_version TEXT,
    consent_rules_at INTEGER,
    delivered_at INTEGER,
    erased INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE (raffle_id, email_norm)
);

CREATE INDEX entrants_raffle ON entrants (raffle_id, created_at);
CREATE INDEX entrants_token ON entrants (token_hash);

-- A write that already moves the version (`SET version = ...`) is left alone,
-- which also stops the bump from triggering itself.
CREATE TRIGGER raffles_version AFTER UPDATE ON raffles
WHEN NEW.version = OLD.version
BEGIN
    UPDATE raffles SET version = OLD.version + 1 WHERE id = NEW.id;
END;

-- Inserting, updating or deleting a prize or an entrant bumps its raffle.
CREATE TRIGGER prizes_version_insert AFTER INSERT ON prizes
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
END;

CREATE TRIGGER prizes_version_update AFTER UPDATE ON prizes
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
END;

CREATE TRIGGER prizes_version_delete AFTER DELETE ON prizes
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = OLD.raffle_id;
END;

CREATE TRIGGER entrants_version_insert AFTER INSERT ON entrants
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
END;

CREATE TRIGGER entrants_version_update AFTER UPDATE ON entrants
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
END;

CREATE TRIGGER entrants_version_delete AFTER DELETE ON entrants
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = OLD.raffle_id;
END;

-- The raffle's event log: append-only, one row per event, in the order the
-- raffle recorded them.
CREATE TABLE raffle_events (
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    -- The event's position in the log, from 1: the domain's, never the database's.
    seq INTEGER NOT NULL CHECK (seq >= 1),
    -- The event's type, as in its body.
    type TEXT NOT NULL,
    -- The event as published (serde JSON of rafi_raffle::Event).
    body TEXT NOT NULL,
    -- When it was stored (unix seconds); not part of the published event.
    recorded_at INTEGER NOT NULL,
    PRIMARY KEY (raffle_id, seq)
);

-- Events are never updated. A migration that renames events (rewriting `type`
-- and `body`) must drop and recreate this trigger around the rewrite.
CREATE TRIGGER raffle_events_no_update BEFORE UPDATE ON raffle_events
BEGIN
    SELECT RAISE(ABORT, 'raffle events are append-only');
END;

-- Deleting the raffle takes its events with it.
CREATE TRIGGER raffle_events_no_delete BEFORE DELETE ON raffle_events
WHEN EXISTS (SELECT 1 FROM raffles WHERE id = OLD.raffle_id)
BEGIN
    SELECT RAISE(ABORT, 'raffle events are append-only');
END;

-- A retried or stale event (seq already stored) passes, and the primary key
-- ignores it; only a gap is refused.
CREATE TRIGGER raffle_events_contiguous BEFORE INSERT ON raffle_events
WHEN NEW.seq > (SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = NEW.raffle_id) + 1
BEGIN
    SELECT RAISE(ABORT, 'raffle events must be contiguous');
END;

-- Every completed draw and where its winner stands, as Raffle::draws() says.
CREATE TABLE draws (
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    k INTEGER NOT NULL,
    item_id TEXT NOT NULL,
    entrant_id TEXT NOT NULL,
    -- rafi_raffle::Outcome::as_str(): the code writes only those, so the
    -- schema doesn't list them.
    outcome TEXT NOT NULL,
    PRIMARY KEY (raffle_id, k)
);

CREATE TABLE audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raffle_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);

CREATE INDEX audit_raffle ON audit (raffle_id, id);

-- Every erasure of an entrant's personal data, without any personal data, so
-- a database restore (D1 Time Travel) can re-apply them.
CREATE TABLE erasures (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raffle_id TEXT NOT NULL,
    entrant_id TEXT NOT NULL,
    at INTEGER NOT NULL
);

-- Proof-of-work challenges already used to join. A replayed solution fails the
-- primary key. Expired rows are deleted by the scheduled job.
CREATE TABLE altcha_used (
    challenge TEXT PRIMARY KEY NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX altcha_used_expiry ON altcha_used (expires_at);

-- What an owner's new raffles start with: who organizes them (the data
-- controller), their privacy contact and text, their additional rules and
-- their retention period (NULL: the installation's default). Copied into each
-- raffle as it is created, so a later change leaves existing raffles as they
-- are.
CREATE TABLE owner_defaults (
    owner_id TEXT PRIMARY KEY NOT NULL,
    organizer_name TEXT NOT NULL DEFAULT '',
    contact_email TEXT NOT NULL DEFAULT '',
    privacy_email TEXT NOT NULL DEFAULT '',
    privacy_text TEXT NOT NULL DEFAULT '',
    privacy_url TEXT NOT NULL DEFAULT '',
    rules TEXT NOT NULL DEFAULT '',
    retention_days INTEGER,
    updated_at INTEGER NOT NULL
);

-- Which version of which legal document (the operator's terms, with the
-- processing agreement) an owner accepted, and which of its users did. Kept
-- when the owner's data is removed: it holds opaque IDs only, and is the
-- operator's evidence.
CREATE TABLE legal_acceptances (
    owner_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    document TEXT NOT NULL,
    version TEXT NOT NULL,
    accepted_at INTEGER NOT NULL,
    PRIMARY KEY (owner_id, document, version)
);

-- Small operational facts: when housekeeping last succeeded
-- (`last_retention_ok_at`, unix seconds as text).
CREATE TABLE meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
