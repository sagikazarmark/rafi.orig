-- Running on Cloudflare Workers (docs/workers-plan.md).

-- The raffle's version: bumped by every write that can change what screens or
-- phones show. Live views carry it, so clients and the live channel can order
-- them without a process-wide counter.
ALTER TABLE raffles ADD COLUMN version INTEGER NOT NULL DEFAULT 0;

-- Set when the organizer ends live updates; the live channel is gone after it.
ALTER TABLE raffles ADD COLUMN live_ended_at INTEGER;

-- A write that already moves the version (`SET version = ...`) is left alone,
-- which also stops the bump from triggering itself.
CREATE TRIGGER raffles_version AFTER UPDATE ON raffles
WHEN NEW.version = OLD.version
BEGIN
    UPDATE raffles SET version = OLD.version + 1 WHERE id = NEW.id;
END;

CREATE TRIGGER items_version_insert AFTER INSERT ON items
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
END;

CREATE TRIGGER items_version_update AFTER UPDATE ON items
BEGIN
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
END;

CREATE TRIGGER items_version_delete AFTER DELETE ON items
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

-- Proof-of-work challenges already used to join. A replayed solution fails the
-- primary key. Expired rows are deleted by the scheduled job.
CREATE TABLE altcha_used (
    challenge TEXT PRIMARY KEY NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX altcha_used_expiry ON altcha_used (expires_at);

-- Every erasure of an entrant's personal data, without any personal data, so
-- a database restore (D1 Time Travel) can re-apply them.
CREATE TABLE erasures (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raffle_id TEXT NOT NULL,
    entrant_id TEXT NOT NULL,
    at INTEGER NOT NULL
);

-- Small operational facts, such as when retention last ran.
CREATE TABLE meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
