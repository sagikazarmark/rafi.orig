// Runs worker.js's altchaSolve under node for tests/worker.rs.
//
//   node solve.mjs <worker.js> < [{"challenge": .., "start": i, "step": n}, ...]
//   -> [{"counter": .., "derivedKey": .., "time": ..} | {"error": ".."}, ...]
//
// worker.js is a classic script, loaded in this realm (so its typed arrays
// are node's own) with `self` set as in a page.
import { readFileSync } from "node:fs";

globalThis.self = globalThis;
const source = readFileSync(process.argv[2], "utf8");
const altchaSolve = new Function(source + "\nreturn altchaSolve;")();

const input = JSON.parse(readFileSync(0, "utf8"));
const out = [];
for (const { challenge, start, step } of input) {
  try {
    out.push(await altchaSolve(challenge, start, step));
  } catch (err) {
    out.push({ error: String((err && err.message) || err) });
  }
}
process.stdout.write(JSON.stringify(out));
