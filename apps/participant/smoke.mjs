// The participant WASM in a real browser: headless Chromium over the DevTools
// protocol (trusted clicks, console capture) against testbeds/cloudflare,
// which serves a stand-in participant page and the live channel.
//
//   ./scripts/participant.sh testbeds/cloudflare/site/participant
//   (cd testbeds/cloudflare && wrangler dev --port 8790) &
//   CHROMIUM=$(which chromium) node apps/participant/smoke.mjs "$(mktemp -d)"

import { spawn } from "node:child_process";

const base = process.env.BASE ?? "http://127.0.0.1:8790";
const port = 9334;
let raffle = `r${Date.now()}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
};
const post = async (path) => {
  const sep = path.includes("?") ? "&" : "?";
  const res = await fetch(`${base}${path}${sep}r=${raffle}`, { method: "POST" });
  if (!res.ok) throw new Error(`${path}: ${res.status} ${await res.text()}`);
};

const chrome = spawn(process.env.CHROMIUM ?? "chromium", ["--headless=new", `--remote-debugging-port=${port}`,
  "--no-first-run", `--user-data-dir=${process.argv[2]}`, "about:blank"], { stdio: "ignore" });
let targets;
for (let i = 0; i < 150 && !targets; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch { await sleep(200); }
}
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const pending = new Map();
const errors = [];
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
};
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const click = async (sel) => {
  const r = await js(`(() => { const b = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`);
  for (const type of ["mousePressed", "mouseReleased"]) await send("Input.dispatchMouseEvent", { type, x: r[0], y: r[1], button: "left", clickCount: 1 });
};
const card = () => js(`document.getElementById("live").innerText.replace(/\\s+/g, " ").trim()`);
const until = async (pred, ms = 8000) => { for (let t = 0; t < ms; t += 100) { if (await pred()) return true; await sleep(100); } return false; };
const open = async (path) => { await send("Page.navigate", { url: `${base}${path}` }); await sleep(300); };

await send("Runtime.enable");
await send("Page.enable");
await post("/reset");

// Anna's page: the server-rendered card, then the WASM takes over without duplicating it.
const html = await (await fetch(`${base}/page?r=${raffle}&me=anna`)).text();
check(html.includes("Waiting for the draw"), "the server renders the card");
await open(`/page?r=${raffle}&me=anna`);
check(await until(() => js("window.__rafiReady === true")), "the WASM reports ready");
check((await js(`document.querySelectorAll("#live > *").length`)) === 1, "one card in #live (no duplicate)");
check((await card()).includes("Waiting for the draw"), "the mounted card shows the same state");

// Live updates from the channel.
await post("/close");
await post("/freeze");
check(await until(async () => (await card()).includes("The draw is starting")), "the freeze is pushed to the card");
await click("#wake"); // unlocks audio (a user gesture), so the chime can play
await post("/draw/anna");
check(await until(async () => (await card()).includes("You won!")), "Anna's phone shows the win");
if (errors.length) console.log("     console errors so far:", errors.join(" | "));
check((await js(`!!document.querySelector("#live [role=alert].animate-pop")`)) === true, "a live win is announced (alert, pop)");
check((await card()).includes("K2P9"), "with the claim code");

// The theme toggle.
await click("#theme");
check((await js(`document.documentElement.getAttribute("data-theme")`)) === "rafi", "the theme toggle works");
await click("#theme");
await click("#theme");

// The end: the final view, the socket closes, and the endpoint keeps the card current.
await post("/finish");
check(await until(async () => (await card()).includes("Thanks, enjoy!")), "the finished raffle thanks the winner");
await post("/end");
await sleep(1000);

// Reloading after the end: the server renders the final card, and it stays.
await open(`/page?r=${raffle}&me=anna`);
check(await until(() => js("window.__rafiReady === true")), "the WASM starts on the final page");
check((await card()).includes("Thanks, enjoy!"), "the final page shows the win");
check((await js(`!!document.querySelector("#live [role=alert]")`)) === false, "an old win is not announced again");

// A visitor's page from before registration opened reloads when it opens.
raffle = `v${Date.now()}`;
const visitor = raffle;
await post("/reset?nudge=0");
await post("/ready");
await open(`/page?r=${visitor}`);
check(await until(() => js("window.__rafiReady === true")), "the visitor's page is ready");
await js(`window.__beforeReload = true`);
await post("/open");
check(await until(() => js("window.__beforeReload !== true"), 10000), "the visitor's page reloads when registration opens");

// Without WebAssembly, the loader hands over to the JavaScript fallback,
// which renders the same card and follows the raffle by polling.
raffle = `f${Date.now()}`;
await post("/reset?nudge=0");
await open(`/page?r=${raffle}&me=anna&nowasm=1`);
check(await until(() => js("window.__rafiFallbackRunning === true"), 3000), "no WebAssembly: the fallback starts");
check((await card()).includes("Waiting for the draw"), "the fallback shows the card");
await post("/close");
await post("/freeze");
check(await until(async () => (await card()).includes("The draw is starting"), 8000), "the fallback picks up the freeze by polling");
await post("/draw/anna");
check(await until(async () => (await card()).includes("You won!"), 8000), "the fallback shows Anna's win");
check((await js(`!!document.querySelector("#live [role=alert].animate-pop")`)) === true, "and announces it");

const real = errors.filter((e) => !/WebSocket|4403|ERR_|rafiFallback|fallback/i.test(e));
check(real.length === 0, `no console errors${real.length ? ": " + real.join(" | ") : ""}`);

ws.close();
chrome.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
