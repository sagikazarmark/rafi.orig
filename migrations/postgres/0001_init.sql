-- Rafi schema for PostgreSQL (self-hosting). Equivalent to
-- ../sqlite/0001_init.sql, which SQLite and D1 use: integers are BIGINT, flags
-- stay 0/1 integers, so the store's SQL is the same on both. Times are unix
-- seconds unless noted.

CREATE TABLE raffles (
    id TEXT PRIMARY KEY NOT NULL,
    owner_id TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    join_code TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    organizer_name TEXT NOT NULL DEFAULT '',
    contact_email TEXT NOT NULL DEFAULT '',
    rules TEXT NOT NULL DEFAULT '',
    event_date TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'draft',
    test_mode BIGINT NOT NULL DEFAULT 0,
    -- The raffle's manifest: the frozen entrant list (canonical JSON, exactly
    -- as hashed), and its hash.
    manifest TEXT,
    manifest_hash TEXT,
    frozen_at BIGINT,
    -- Set when the engine first wrote the raffle: opening records no event, so
    -- an opened raffle may have none.
    opened_at BIGINT,
    finished_at BIGINT,
    -- Set when the organizer ends live updates; the live channel is gone after it.
    live_ended_at BIGINT,
    purge_nonwinners_at BIGINT,
    purge_winners_at BIGINT,
    purged_at BIGINT,
    -- The raffle's version: bumped by every write that can change what screens
    -- or phones show (the triggers below). Live views carry it, so clients and
    -- the live channel can order them without a process-wide counter.
    version BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX raffles_owner ON raffles (owner_id);

CREATE TABLE prizes (
    id TEXT PRIMARY KEY NOT NULL,
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sponsor TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    qty BIGINT NOT NULL DEFAULT 1,
    position BIGINT NOT NULL DEFAULT 0
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
    hidden BIGINT NOT NULL DEFAULT 0,
    -- Private code shown only to the entrant and the organizer.
    claim_code TEXT NOT NULL,
    -- SHA-256 of the secret entrant token stored in the attendee's cookie.
    token_hash TEXT,
    consent_version TEXT,
    consent_rules_at BIGINT,
    consent_marketing BIGINT NOT NULL DEFAULT 0,
    delivered_at BIGINT,
    purged BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    UNIQUE (raffle_id, email_norm)
);

CREATE INDEX entrants_raffle ON entrants (raffle_id, created_at);
CREATE INDEX entrants_token ON entrants (token_hash);

-- A write that already moves the version (`SET version = ...`) is left alone;
-- any other update of a raffle bumps it once. SQLite does this with an AFTER
-- UPDATE trigger that updates the row again; here the row is changed before
-- it is written, with the same result.
CREATE FUNCTION raffles_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.version = OLD.version THEN
        NEW.version := OLD.version + 1;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER raffles_version BEFORE UPDATE ON raffles
FOR EACH ROW EXECUTE FUNCTION raffles_version();

-- Inserting, updating or deleting a prize or an entrant bumps its raffle.
CREATE FUNCTION raffles_version_bump() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        UPDATE raffles SET version = version + 1 WHERE id = OLD.raffle_id;
        RETURN OLD;
    END IF;
    UPDATE raffles SET version = version + 1 WHERE id = NEW.raffle_id;
    RETURN NEW;
END;
$$;

CREATE TRIGGER prizes_version AFTER INSERT OR UPDATE OR DELETE ON prizes
FOR EACH ROW EXECUTE FUNCTION raffles_version_bump();

CREATE TRIGGER entrants_version AFTER INSERT OR UPDATE OR DELETE ON entrants
FOR EACH ROW EXECUTE FUNCTION raffles_version_bump();

-- The raffle's event log (docs/events-plan.md): append-only, one row per
-- event, in the order the raffle recorded them.
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
-- is gone). A migration that renames events (rewriting `type` and `body`)
-- must disable this trigger around the rewrite.
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

CREATE TABLE audit (
    id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    raffle_id TEXT NOT NULL,
    at BIGINT NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);

CREATE INDEX audit_raffle ON audit (raffle_id, id);

-- Every erasure of an entrant's personal data, without any personal data, so
-- a database restore can re-apply them.
CREATE TABLE erasures (
    id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    raffle_id TEXT NOT NULL,
    entrant_id TEXT NOT NULL,
    at BIGINT NOT NULL
);

-- Proof-of-work challenges already used to join. A replayed solution fails the
-- primary key. Expired rows are deleted by the scheduled job.
CREATE TABLE altcha_used (
    challenge TEXT PRIMARY KEY NOT NULL,
    expires_at BIGINT NOT NULL
);

CREATE INDEX altcha_used_expiry ON altcha_used (expires_at);

-- Small operational facts, such as when retention last ran.
CREATE TABLE meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
