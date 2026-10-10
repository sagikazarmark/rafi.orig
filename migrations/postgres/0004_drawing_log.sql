-- A drawing is stored as its manifest and its event log, both published
-- formats (docs/drawing-review.md); nothing in SQL reads inside it any more.
-- What the database itself needs from a drawing are columns written with it:
-- the entrants drawn (only their names may leave the database) and the
-- winners, who claimed an item (their data is kept longer). JSON arrays of
-- entrant IDs.
ALTER TABLE raffles ADD COLUMN drawn_entrants TEXT;
ALTER TABLE raffles ADD COLUMN winners TEXT;

-- Drawings stored before: their entrants, from the old shape (the draw events
-- and the claimed items) ...
UPDATE raffles SET
    drawn_entrants = (
        SELECT COALESCE(json_agg(DISTINCT e ->> 'entrant' ORDER BY e ->> 'entrant'), '[]'::json)::text
        FROM jsonb_array_elements(drawing::jsonb -> 'events') AS e
        WHERE e ->> 'type' = 'draw'
    ),
    winners = (
        SELECT COALESCE(json_agg(DISTINCT i ->> 'entrant' ORDER BY i ->> 'entrant'), '[]'::json)::text
        FROM jsonb_array_elements(drawing::jsonb -> 'items') AS i
        WHERE i ->> 'status' = 'claimed'
    )
WHERE drawing IS NOT NULL;

-- ... then the new shape: the manifest and the events.
UPDATE raffles SET
    drawing = json_build_object('manifest', manifest::json, 'events', drawing::json -> 'events')::text
WHERE drawing IS NOT NULL AND manifest IS NOT NULL;
