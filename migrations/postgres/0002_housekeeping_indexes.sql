-- Housekeeping's hourly sweeps start from what is left to do, not from every
-- raffle the installation ever had (the EXPLAIN QUERY PLAN gate,
-- crates/store/src/query_plans.rs). The same in migrations/sqlite.

-- The entrants whose personal data isn't erased yet: those of raffles before
-- their erasure date (and winners held past it), whatever the installation's
-- age. The erasures (retention::erase_due, erase_objected) start here.
CREATE INDEX entrants_unerased ON entrants (raffle_id) WHERE erased = 0;

-- The raffles whose live updates haven't ended, by when they finished:
-- ending the channels (housekeeping::live_channels_due) starts here.
CREATE INDEX raffles_live ON raffles (finished_at) WHERE live_ended_at IS NULL;
