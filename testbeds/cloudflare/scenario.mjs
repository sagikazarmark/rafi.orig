// Drives rafi-cloudflare's RaffleRoom end to end against `wrangler dev` (see src/lib.rs):
// phones connect over partyline, the raffle changes the way the app and the
// raffle engine change it, and every step checks what the phones received.
//
//   node scenario.mjs http://127.0.0.1:8790

const base = process.argv[2] ?? "http://127.0.0.1:8790";
const wsBase = base.replace(/^http/, "ws");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Raffle IDs are 32 hex digits (the gate checks the shape before anything
// else); a fresh one per run keeps runs independent.
const raffle = crypto.randomUUID().replaceAll("-", "");
const entrant = (name) => `${raffle}-${name}`;
let failures = 0;

function check(ok, what) {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
}

async function post(path) {
  const sep = path.includes("?") ? "&" : "?";
  const res = await fetch(`${base}${path}${sep}r=${raffle}`, { method: "POST" });
  const text = await res.text();
  if (!res.ok) throw new Error(`${path}: ${res.status} ${text}`);
  return text;
}

class Phone {
  constructor(name) {
    this.name = name;
    this.views = [];
    this.closed = null;
    this.ws = new WebSocket(`${wsBase}/partyline/raffle/${raffle}?v=1`);
    this.ws.onmessage = (m) => {
      if (m.data === "pong") return;
      const frame = JSON.parse(m.data);
      if (frame.t === "event") this.views.push(frame.event);
    };
    this.ws.onclose = (e) => (this.closed = e.code);
  }
  get last() {
    return this.views[this.views.length - 1];
  }
  async until(pred, ms = 3000) {
    for (let t = 0; t < ms; t += 50) {
      if (pred(this)) return true;
      await sleep(50);
    }
    return false;
  }
}

await post("/reset");

// Connecting brings the stored view up to date and sends it after Hello.
const anna = new Phone("anna");
check(await anna.until((p) => p.views.length === 1), "a new socket gets the current view");
check(anna.last?.state === "open" && anna.last?.prizes_left === 2, "the view is the open raffle with two prizes");

// A join changes nothing phones see.
const before = anna.views.length;
check((await post("/join")) === "Unchanged", "a join is Unchanged for phones");
await sleep(300);
check(anna.views.length === before, "no message after a join");

// Lifecycle changes are pushed.
check((await post("/close")) === "Publish", "closing registration publishes");
check(await anna.until((p) => p.last?.state === "closed"), "phones see registration closed");
await post("/freeze");
check(await anna.until((p) => p.last?.state === "frozen"), "phones see the freeze");

// A second phone joins mid-evening and gets the latest view at once.
const bence = new Phone("bence");
check(await bence.until((p) => p.last?.state === "frozen"), "a late socket starts from the latest view");

// A room reconnecting at once (an eviction, a deploy: partyline retries from
// half a second) reads the raffle once, not once per phone: the gate reads
// two columns of its row, and the object lets a read a moment old stand in
// for the next. Every socket still gets the view.
const stats = async () => (await fetch(`${base}/stats?r=${raffle}`)).json();
await sleep(1100);
const readsBefore = (await stats()).reads;
const crowd = Array.from({ length: 50 }, (_, i) => new Phone(`crowd-${i}`));
check(
  (await Promise.all(crowd.map((p) => p.until((q) => q.last?.state === "frozen", 10000)))).every(Boolean),
  "fifty sockets connecting at once each get the current view",
);
const readsDuring = (await stats()).reads - readsBefore;
check(readsDuring <= 2, `the burst cost the object at most two reads of the raffle (${readsDuring})`);
for (const p of crowd) p.ws.close();

// Draws, as Restate's projector writes them.
await post("/draw/anna");
check(
  await anna.until((p) => p.last?.winners?.some((w) => w.entrant_id === entrant("anna"))),
  "phones see Anna win",
);
check(bence.last?.last?.display_name === "Anna", "Bence's phone sees the last draw");

// A nudge with nothing new is Stale (same version), not a duplicate push.
const n = anna.views.length;
check((await post("/nudge")).startsWith("Stale"), "a repeated nudge is Stale");
await sleep(300);
check(anna.views.length === n, "no duplicate message");

// A lost nudge is repaired by the periodic re-read (every 15 s while sockets are open).
await post("/draw/bence?nudge=0");
check(
  await anna.until((p) => p.last?.winners?.length === 2, 20000),
  "the re-read alarm delivers an update whose nudge was lost",
);

// The end: the final view, then 4403, and the channel refuses to come back.
await post("/finish");
check(await anna.until((p) => p.last?.state === "finished"), "phones see the raffle finish");
check((await post("/end")) === "destroyed", "ending live updates destroys the channel");
const endedAt = Date.now();
// Under `wrangler dev` the close reaches Node clients only after ~10 s (the
// local proxy's close timeout); phones close on the final view anyway.
check(await anna.until((p) => p.closed !== null, 15000), "sockets are closed");
console.log(`     (closed ${Date.now() - endedAt} ms after the end)`);
check(anna.closed === 4403 && bence.closed === 4403, `closed with 4403 (got ${anna.closed}, ${bence.closed})`);

const late = new Phone("late");
check(await late.until((p) => p.closed !== null), "a socket after the end is refused");
check(late.closed === 4403, `refused with 4403 (got ${late.closed})`);
check(late.views.length === 0, "and gets no view");
check((await post("/nudge")) === "Ignore", "a nudge after the end is ignored");

const view = await (await fetch(`${base}/participant?r=${raffle}`)).json();
check(view?.state === "finished" && view?.winners?.length === 2, "the polling endpoint still serves the final view");

console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
