-- A resolved event names the organizer's resolution `resolution` (was
-- `outcome`), like the decision that records it (docs/drawing-review.md).
-- Stored drawings follow; their order is kept.
UPDATE raffles SET
    drawing = jsonb_set(drawing::jsonb, '{events}', COALESCE((
        SELECT jsonb_agg(
            CASE WHEN e ->> 'type' = 'resolved' AND e ? 'outcome'
                THEN (e - 'outcome') || jsonb_build_object('resolution', e -> 'outcome')
                ELSE e
            END ORDER BY n)
        FROM jsonb_array_elements(drawing::jsonb -> 'events') WITH ORDINALITY AS x(e, n)
    ), '[]'::jsonb))::text
WHERE drawing IS NOT NULL;
