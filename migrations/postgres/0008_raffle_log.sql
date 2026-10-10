-- The drawing is the raffle (docs/raffle-plan.md): the row stores it as its
-- log (the manifest and the event log), and the log's version.
ALTER TABLE raffles RENAME COLUMN drawing TO log;
ALTER TABLE raffles RENAME COLUMN drawing_version TO log_version;
