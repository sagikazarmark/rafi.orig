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
        SELECT json_group_array(entrant) FROM (
            SELECT DISTINCT json_extract(e.value, '$.entrant') AS entrant
            FROM json_each(raffles.drawing, '$.events') e
            WHERE json_extract(e.value, '$.type') = 'draw'
            ORDER BY entrant
        )
    ),
    winners = (
        SELECT json_group_array(entrant) FROM (
            SELECT DISTINCT json_extract(i.value, '$.entrant') AS entrant
            FROM json_each(raffles.drawing, '$.items') i
            WHERE json_extract(i.value, '$.status') = 'claimed'
            ORDER BY entrant
        )
    )
WHERE drawing IS NOT NULL;

-- ... then the new shape: the manifest and the events.
UPDATE raffles SET
    drawing = json_object(
        'manifest', json(manifest),
        'events', json(json_extract(drawing, '$.events'))
    )
WHERE drawing IS NOT NULL AND manifest IS NOT NULL;
