// A draw end to end on Workers: Restate (a local restate-server) calls this
// Worker in request-response mode; the Worker writes D1 and nudges the
// raffle's RaffleChannel (hosted by crates/cloudflare-check), which pushes to a
// phone connected over partyline.
//
// Setup (shared local state for both Workers):
//   wrangler d1 migrations apply rafi-cloudflare-check --local --persist-to $ST   (in crates/cloudflare-check)
//   restate-server &
//   (cd crates/cloudflare-check && wrangler dev --port 8790 --persist-to $ST) &
//   (cd crates/restate-worker && wrangler dev --port 8791 --persist-to $ST) &
//   curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
//        -d '{"uri":"http://127.0.0.1:8791","use_http_11":true}'
//   node e2e.mjs

const app = process.env.APP ?? "http://127.0.0.1:8790";
const ingress = process.env.INGRESS ?? "http://127.0.0.1:8080";
const raffle = `r${Date.now()}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;

function check(ok, what) {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
}

async function call(url, body) {
  const res = await fetch(url, {
    method: "POST",
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${url}: ${res.status} ${text}`);
  return text;
}

const views = [];
let ws;
function connect() {
  ws = new WebSocket(`${app.replace(/^http/, "ws")}/partyline/raffle/${raffle}?v=1`);
  ws.onmessage = (m) => {
    const f = JSON.parse(m.data);
    if (f.t === "event") views.push(f.event);
  };
  ws.onclose = (e) => console.log(`     (socket closed: ${e.code})`);
}
async function until(pred, ms) {
  for (let t = 0; t < ms; t += 100) {
    if (pred()) return true;
    await sleep(100);
  }
  return false;
}
const last = () => views[views.length - 1];

await call(`${app}/reset?r=${raffle}&nudge=0`);
connect();
check(await until(() => last()?.state === "open", 5000), "the phone starts from the open raffle");

// The app's half of the freeze (the manifest), then the raffle opened on it.
const doc = await call(`${app}/prepare?r=${raffle}`);
await call(`${ingress}/Raffle/${raffle}/open`, doc);
check(await until(() => last()?.state === "frozen", 5000), "after Restate opens the raffle the phone sees the raffle frozen");

// A draw: announced on a future drand round, then completed after a durable sleep.
await call(`${ingress}/Raffle/${raffle}/draw/send`, { item: "book-1" });
check(await until(() => last()?.current?.prize_name === "Book", 10000), "the phone sees the countdown for the Book");
const round = last()?.current?.round;
check(Number.isInteger(round) && round > 0, `the draw commits to a drand round (${round})`);
check(
  await until(() => last()?.winners?.length === 1 && last()?.current === null, 30000),
  "after the round, the phone sees the winner",
);
const winner = last()?.winners?.[0]?.entrant_id;
check([`${raffle}-anna`, `${raffle}-bence`].includes(winner), `the winner is an entrant (${winner})`);

const state = await (await fetch(`${ingress}/Raffle/${raffle}/get`, { method: "POST" })).text();
check(JSON.parse(state)?.events?.some((e) => e.type === "drawn"), "Restate's own state has the draw");

ws.close();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
