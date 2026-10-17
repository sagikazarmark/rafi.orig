-- Equivalent to ../0010_raffle_events.sql (SQLite and D1).

-- The raffle's event log: append-only, one row per event, in the order the
-- raffle recorded them.
CREATE TABLE raffle_events (
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    -- The event's position in the log, from 1: the domain's, never the database's.
    seq BIGINT NOT NULL CHECK (seq >= 1),
    -- The event's type, as in its body.
    type TEXT NOT NULL,
    -- The event as published (serde JSON of rafi_raffle::Event).
    body TEXT NOT NULL,
    -- When it was stored (unix seconds); not part of the published event.
    recorded_at BIGINT NOT NULL,
    PRIMARY KEY (raffle_id, seq)
);

-- Events are never updated, nor deleted while their raffle exists: deleting
-- the raffle takes its events with it (the cascade runs once the raffle row
-- is gone).
CREATE FUNCTION raffle_events_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND NOT EXISTS (SELECT 1 FROM raffles WHERE id = OLD.raffle_id) THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'raffle events are append-only';
END;
$$;

CREATE TRIGGER raffle_events_append_only BEFORE UPDATE OR DELETE ON raffle_events
FOR EACH ROW EXECUTE FUNCTION raffle_events_append_only();

-- A retried or stale event (seq already stored) passes, and the primary key
-- ignores it; only a gap is refused.
CREATE FUNCTION raffle_events_contiguous() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.seq > (SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = NEW.raffle_id) + 1 THEN
        RAISE EXCEPTION 'raffle events must be contiguous';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER raffle_events_contiguous BEFORE INSERT ON raffle_events
FOR EACH ROW EXECUTE FUNCTION raffle_events_contiguous();

-- Every completed draw and where its winner stands, as Raffle::draws() says.
CREATE TABLE draws (
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    k BIGINT NOT NULL,
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
ALTER TABLE raffles ADD COLUMN opened_at BIGINT;
ALTER TABLE raffles DROP COLUMN log;
ALTER TABLE raffles DROP COLUMN log_version;
ALTER TABLE raffles DROP COLUMN drawn_entrants;
ALTER TABLE raffles DROP COLUMN winners;
