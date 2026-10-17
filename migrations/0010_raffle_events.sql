-- The raffle's log becomes a table (docs/events-plan.md): its events, append
-- only, and its completed draws, which SQL reads. The manifest is stored once,
-- in raffles.manifest.

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

-- Events are never updated. A future rename of events (as 0005 and 0006 did
-- for raffles.drawing) must drop and recreate this trigger around the rewrite.
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
    outcome TEXT NOT NULL CHECK (outcome IN ('drawn', 'claimed', 'not_here', 'declined')),
    PRIMARY KEY (raffle_id, k)
);

-- Raffles opened before this migration can't be carried over (their draws are
-- worked out in Rust, not SQL): they are deleted, with their prizes and
-- entrants. There are no real raffles yet.
DELETE FROM raffles WHERE log IS NOT NULL;

-- Set when the engine first wrote the raffle: opening records no event, so an
-- opened raffle may have none.
ALTER TABLE raffles ADD COLUMN opened_at INTEGER;
ALTER TABLE raffles DROP COLUMN log;
ALTER TABLE raffles DROP COLUMN log_version;
ALTER TABLE raffles DROP COLUMN drawn_entrants;
ALTER TABLE raffles DROP COLUMN winners;
