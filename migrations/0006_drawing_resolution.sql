-- A resolved event names the organizer's resolution `resolution` (was
-- `outcome`), like the decision that records it (docs/drawing-review.md).
-- Stored drawings follow; their order is kept.
UPDATE raffles SET
    drawing = json_set(drawing, '$.events', (
        SELECT json_group_array(json(
            CASE WHEN json_extract(e.value, '$.type') = 'resolved'
                      AND json_type(e.value, '$.outcome') IS NOT NULL
                THEN json_remove(
                    json_set(e.value, '$.resolution', json_extract(e.value, '$.outcome')),
                    '$.outcome'
                )
                ELSE e.value
            END))
        FROM (SELECT value FROM json_each(raffles.drawing, '$.events') ORDER BY key) e
    ))
WHERE drawing IS NOT NULL;
