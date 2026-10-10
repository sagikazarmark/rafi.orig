-- Names follow the domain (docs/drawing-plan.md): a raffle's prizes, and the
-- drawing that takes over once the entrant list is frozen.

ALTER TABLE items RENAME TO prizes;

DROP INDEX items_raffle;
CREATE INDEX prizes_raffle ON prizes (raffle_id, position);

DROP TRIGGER items_version_insert;
DROP TRIGGER items_version_update;
DROP TRIGGER items_version_delete;

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

-- The frozen entrant list is the drawing's manifest (canonical JSON, exactly
-- as hashed), and the projection of the draw engine's state is the drawing.
ALTER TABLE raffles RENAME COLUMN commit_doc TO manifest;
ALTER TABLE raffles RENAME COLUMN list_hash TO manifest_hash;
ALTER TABLE raffles RENAME COLUMN draw_state TO drawing;
ALTER TABLE raffles RENAME COLUMN draw_version TO drawing_version;
