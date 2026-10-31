// A whole evening against Rafi on Workers, locally: the admin API (dev auth),
// a phone in headless Chromium joining through the real form, draws through
// Restate (the engine in the same Worker, under /restate), the live channel,
// and the end. Step 4 of scripts/workers-check.sh.
//
//   node apps/worker/evening.mjs "$(mktemp -d)"
//
// INGRESS (Restate's ingress, default http://127.0.0.1:18080) lets it read
// the engine's own state back after the draw.

import { spawn } from "node:child_process";

const base = process.env.APP ?? "http://127.0.0.1:8787";
const ingress = process.env.INGRESS ?? "http://127.0.0.1:18080";
const port = 9335;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
};
async function api(name, args = {}) {
  const res = await fetch(`${base}/api/admin/${name}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${name}: ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}
const commandId = () => `cmd-${Date.now()}-${Math.random().toString(16).slice(2)}`;

// The organizer sets up a raffle dated today with one prize, names
// themselves (registration won't open otherwise) and opens registration.
const me = await api("whoami");
check(me.owner_id === "dev", "dev auth signs the organizer in");
const today = new Date().toISOString().slice(0, 10);
async function newRaffle(title) {
  const id = await api("create_raffle", { title, event_date: today, event_ends_at: null });
  const { settings } = await api("get_raffle", { id });
  await api("save_settings", { id, settings: { ...settings, organizer_name: "Workers Meetup", contact_email: "hello@example.com" } });
  return id;
}
const id = await newRaffle(`Workers evening ${Date.now()}`);
await api("save_prize", { id, prize: { id: null, name: "Rust in Action", sponsor: "Sponsor Kft.", description: "", qty: 1, position: 0 } });
await api("set_lifecycle", { id, action: "publish" });
await api("set_lifecycle", { id, action: "open" });
let admin = await api("get_raffle", { id });
check(admin.retention.erase_at > Date.now() / 1000, "the erasure date is ahead");
const { slug, join_code: code } = admin.view;
check(admin.view.state === "open", "registration is open");

// A phone.
const chrome = spawn(process.env.CHROMIUM ?? "chromium", ["--headless=new", `--remote-debugging-port=${port}`,
  "--no-first-run", `--user-data-dir=${process.argv[2]}`, "about:blank"], { stdio: "ignore" });
let targets;
for (let i = 0; i < 150 && !targets; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch { await sleep(200); }
}
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let seq = 0;
const pending = new Map();
const errors = [];
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
  if (msg.method === "Page.backForwardCacheNotUsed") bfcacheNotUsed = msg.params.notRestoredExplanations.map((e) => e.reason);
};
let bfcacheNotUsed = null;
const send = (method, params = {}) => new Promise((r) => { const i = ++seq; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const until = async (pred, ms = 10000) => { for (let t = 0; t < ms; t += 100) { if (await pred()) return true; await sleep(100); } return false; };
const card = () => js(`(document.getElementById("live")?.innerText ?? "").replace(/\\s+/g, " ").trim()`);

// The projector: a second tab with its own DevTools connection.
async function tab(url) {
  const t = await (await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
  const sock = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise((r) => (sock.onopen = r));
  let n = 0;
  const waiting = new Map();
  sock.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); }
  };
  const call = (method, params = {}) => new Promise((r) => { const i = ++n; waiting.set(i, r); sock.send(JSON.stringify({ id: i, method, params })); });
  return {
    call,
    js: async (expr) => (await call("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value,
    close: () => sock.close(),
  };
}
await send("Runtime.enable");
await send("Page.enable");

await send("Page.navigate", { url: `${base}/j/${code}` });
check(await until(() => js(`!!document.getElementById("submit") && !document.getElementById("submit").disabled`), 20000),
  "the proof-of-work enables Join");

// Back/forward cache: leave the join page and come back. Restored from the
// cache, it holds a payload that may be spent, so it must solve again.
const proofValue = `document.getElementById("proof")?.value ?? ""`;
const before = await js(proofValue);
await js("window.__notReloaded = true");
await send("Page.navigate", { url: `${base}/healthz` });
await until(async () => (await js("location.pathname")) === "/healthz");
const history = (await send("Page.getNavigationHistory")).result;
await send("Page.navigateToHistoryEntry", { entryId: history.entries[history.currentIndex - 1].id });
await until(async () => (await js("location.pathname")) === `/j/${code}`);
const joinReady = `!!document.getElementById("submit") && !document.getElementById("submit").disabled && !!document.getElementById("proof").value`;
if (await js("window.__notReloaded === true")) {
  check(await until(async () => { const v = await js(proofValue); return !!v && v !== before; }, 20000),
    "a join page restored from the back/forward cache solves again");
} else {
  // Chrome doesn't cache the join page (it's no-store; the reasons are
  // logged), but other browsers may. Its handler, then, with the event a
  // restore fires.
  console.log(`     (not restored from the back/forward cache: ${bfcacheNotUsed?.join(", ") ?? "unknown"}; simulating)`);
  check(await until(() => js(joinReady), 20000), "Join is enabled again after coming back");
  const reloaded = await js(proofValue);
  await js(`window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }))`);
  check(await until(async () => { const v = await js(proofValue); return !!v && v !== reloaded; }, 20000),
    "a pageshow from the back/forward cache solves again");
}
check(await until(() => js(joinReady), 20000), "Join is enabled after it");
await js(`(() => {
  document.getElementById("name").value = "Anna";
  document.getElementById("email").value = "anna@example.com";
  document.querySelector("input[name=rules]").checked = true;
  document.getElementById("join").requestSubmit();
})()`);
check(await until(() => js(`!!document.querySelector(".claim-code")`), 15000), "Anna joined and has a claim code");
const claim = await js(`document.querySelector(".claim-code").innerText.trim()`);
check(await until(() => js("window.__rafiReady === true"), 15000), "the phone's WebAssembly is ready");
check((await card()).includes("Waiting"), "the card waits for the draw");
const cfg = await js(`JSON.parse(document.getElementById("rafi-config").textContent)`);
check(cfg.follow?.kind === "partyline", "the page subscribes over partyline");

// The draw, through Restate; the phone follows over the live channel.
await api("set_lifecycle", { id, action: "close" });
await api("start_draw", { id });
check(await until(async () => /draw is starting|starting/i.test(await card()), 15000), "the freeze reaches the phone");
admin = await api("get_raffle", { id });
const item = admin.view.items[0].item;
const screen = await tab(`${base}/s/${slug}`);
// Read defensively: the projector loads the web app first (a large bundle).
const screenText = async () => (await screen.js(`(document.body?.innerText ?? "").replace(/\\s+/g, " ")`)) ?? "";
check(await until(async () => (await screenText()).includes("Rust in Action"), 45000), "the projector shows the raffle");
await api("draw_item", { id, item, command_id: commandId() });
check(await until(async () => /Rust in Action/.test(await card()) && /\d|…/.test(await card()), 15000), "the phone shows the countdown");
check(await until(() => screen.js(`!!document.querySelector(".countdown")`), 15000), "the projector shows the countdown");
// The reveal: when does each screen show the winner?
let phoneAt = 0, screenAt = 0;
for (let t = 0; t < 30000 && !(phoneAt && screenAt); t += 50) {
  if (!phoneAt && (await card()).includes(claim)) phoneAt = Date.now();
  if (!screenAt && (await screenText()).includes("Anna")) screenAt = Date.now();
  await sleep(50);
}
check(phoneAt > 0, "the phone shows the win with its claim code");
check(screenAt > 0, "the projector reveals Anna");
check(phoneAt > 0 && screenAt > 0 && screenAt - phoneAt < 1500,
  `the projector reveals within 1.5 s of the phone (projector ${screenAt - phoneAt} ms after the phone)`);
screen.close();
check((await js(`!!document.querySelector("#live [role=alert]")`)) === true, "the win is announced");

// The full view for screens: version from the database, the draw in it.
const view = await (await fetch(`${base}/api/r/${slug}/view`)).json();
check(view.draws.length === 1 && view.follow?.kind === "partyline", "the full view has the draw and says to follow over partyline");
const round = view.draws[0]?.round;
check(Number.isInteger(round) && round > 0, `the draw committed to a drand round (${round})`);
// The engine's own record of the raffle, in Restate.
const state = await (await fetch(`${ingress}/Raffle/${id}/get`, { method: "POST" })).json();
check(state?.events?.some((e) => e.type === "drawn"), "Restate's own state has the draw");
const participant = await (await fetch(`${base}/api/r/${slug}/participant`)).json();
check(participant.winners.length === 1 && !JSON.stringify(participant).includes("anna@example.com"),
  "the participant view has the winner and no email");

// The end.
await api("finish_raffle", { id, command_id: commandId() });
check(await until(async () => /enjoy your prize/i.test(await card()), 15000), "the winner's phone sees the raffle finish");
await api("end_live_updates", { id });
admin = await api("get_raffle", { id });
check(admin.view.live_ended === true, "live updates are ended");
const late = new WebSocket(`${base.replace(/^http/, "ws")}/partyline/raffle/${id}?v=1`);
const closed = await new Promise((r) => { late.onclose = (e) => r(e.code); setTimeout(() => r(null), 5000); });
check(closed === 4403, `a socket after the end is refused (${closed})`);

// A phone without WebAssembly joins through fallback.js, which solves with
// the same worker script (/participant/pow-worker.js). Its own raffle, so
// the draw above keeps one entrant.
const other = await newRaffle(`Workers evening, no wasm ${Date.now()}`);
await api("save_prize", { id: other, prize: { id: null, name: "Sticker", sponsor: "", description: "", qty: 1, position: 0 } });
await api("set_lifecycle", { id: other, action: "publish" });
await api("set_lifecycle", { id: other, action: "open" });
const otherCode = (await api("get_raffle", { id: other })).view.join_code;
const nowasm = await tab("about:blank");
await nowasm.call("Page.enable");
await nowasm.call("Page.addScriptToEvaluateOnNewDocument", { source: "delete window.WebAssembly;" });
await nowasm.call("Page.navigate", { url: `${base}/j/${otherCode}` });
check(await until(() => nowasm.js(`!!window.__rafiFallbackRunning && !!document.getElementById("submit") && !document.getElementById("submit").disabled && document.getElementById("proof").value.length > 0`), 30000),
  "no WebAssembly: the fallback solves the proof-of-work");
const fallbackValue = await nowasm.js(`document.getElementById("proof").value`);
await nowasm.js(`window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }))`);
check(await until(async () => { const v = await nowasm.js(`document.getElementById("proof").value`); return !!v && v !== fallbackValue; }, 20000),
  "and solves again on a pageshow from the back/forward cache");
await nowasm.js(`(() => {
  document.getElementById("name").value = "Bence";
  document.getElementById("email").value = "bence@example.com";
  document.querySelector("input[name=rules]").checked = true;
  document.getElementById("join").requestSubmit();
})()`);
check(await until(() => nowasm.js(`!!document.querySelector(".claim-code")`), 15000), "and Bence joins with it");
nowasm.close();

const real = errors.filter((e) => !/WebSocket|4403|ERR_/i.test(e));
check(real.length === 0, `no console errors${real.length ? ": " + real.join(" | ") : ""}`);
ws.close();
chrome.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
