-- Names follow the domain (docs/drawing-plan.md): a raffle's prizes, and the
-- drawing that takes over once the entrant list is frozen.

ALTER TABLE items RENAME TO prizes;
ALTER INDEX items_raffle RENAME TO prizes_raffle;
ALTER TRIGGER items_version ON prizes RENAME TO prizes_version;

-- The frozen entrant list is the drawing's manifest (canonical JSON, exactly
-- as hashed), and the projection of the draw engine's state is the drawing.
ALTER TABLE raffles RENAME COLUMN commit_doc TO manifest;
ALTER TABLE raffles RENAME COLUMN list_hash TO manifest_hash;
ALTER TABLE raffles RENAME COLUMN draw_state TO drawing;
ALTER TABLE raffles RENAME COLUMN draw_version TO drawing_version;
