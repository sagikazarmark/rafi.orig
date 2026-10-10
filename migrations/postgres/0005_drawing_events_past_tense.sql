-- A drawing's events are named in the past tense (docs/drawing-review.md):
-- announced, drawn, stalled, abandoned (was "fail"), resolved, voided,
-- finished. Stored drawings follow; their order is kept.
UPDATE raffles SET
    drawing = jsonb_set(drawing::jsonb, '{events}', COALESCE((
        SELECT jsonb_agg(jsonb_set(e, '{type}', to_jsonb(
            CASE e ->> 'type'
                WHEN 'announce' THEN 'announced'
                WHEN 'draw' THEN 'drawn'
                WHEN 'stall' THEN 'stalled'
                WHEN 'fail' THEN 'abandoned'
                WHEN 'resolve' THEN 'resolved'
                WHEN 'void' THEN 'voided'
                WHEN 'finish' THEN 'finished'
                ELSE e ->> 'type'
            END)) ORDER BY n)
        FROM jsonb_array_elements(drawing::jsonb -> 'events') WITH ORDINALITY AS x(e, n)
    ), '[]'::jsonb))::text
WHERE drawing IS NOT NULL;
