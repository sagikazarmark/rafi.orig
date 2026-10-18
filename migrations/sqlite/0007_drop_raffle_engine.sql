-- Raffles no longer record a draw engine: the local fallback is gone
-- (docs/PLAN.md, revision 5), so every raffle said 'restate', and the app
-- can't tell one engine from another (docs/engine-plan.md, revision 7).
ALTER TABLE raffles DROP COLUMN engine;
