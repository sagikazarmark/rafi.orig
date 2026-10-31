// A whole evening against Rafi on Workers, locally: the admin API (dev auth),
// a phone in headless Chromium joining through the real form, draws through
// Restate (the engine in the same Worker, under /restate), the live channel,
// and the end. Step 4 of scripts/workers-check.sh.
//
//   node apps/worker/evening.mjs "$(mktemp -d)"
//
// INGRESS (Restate's ingress, default http://127.0.0.1:18080) lets it read
// the engine's own state back after the draw.
//
// Every tab reports what the pages' Content-Security-Policy refused (or, under
// RAFI_CSP=report-only, would have): one is a failure, since a refusal means
// a broken page and a report a policy about to break one.

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
// What the pages' Content-Security-Policy refused, or under report-only would
// have, per tab: from a listener every document gets before its own scripts
// run (its `securitypolicyviolation` events, as console calls tagged
// VIOLATION), and from DevTools' log (Log.enable: its "security" entries),
// which also sees what a listener can't, a worker's. A violation shows in
// both, so each is a separate list and the log's is reported only when the
// listener saw nothing.
const violations = [];
const logged = [];
const VIOLATION = "rafi-csp-violation";
const listenForViolations = `document.addEventListener("securitypolicyviolation", (e) =>
  console.error("${VIOLATION}", e.disposition, e.effectiveDirective, e.blockedURI, (e.sample || "").slice(0, 80), location.pathname));`;
// Records `msg` if it is a violation of the page in tab `name`; whether it was.
function violation(name, msg) {
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.args[0]?.value === VIOLATION) {
    violations.push(`${name}: ${msg.params.args.slice(1).map((a) => a.value).join(" ")}`);
    return true;
  }
  if (msg.method === "Log.entryAdded" && msg.params.entry.source === "security" && /Content Security Policy/i.test(msg.params.entry.text)) {
    logged.push(`${name}: ${msg.params.entry.text}`);
    return true;
  }
  return false;
}
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  const csp = violation("phone", msg);
  if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
  if (!csp && msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
  if (msg.method === "Page.backForwardCacheNotUsed") bfcacheNotUsed = msg.params.notRestoredExplanations.map((e) => e.reason);
};
let bfcacheNotUsed = null;
const send = (method, params = {}) => new Promise((r) => { const i = ++seq; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const until = async (pred, ms = 10000) => { for (let t = 0; t < ms; t += 100) { if (await pred()) return true; await sleep(100); } return false; };
const card = () => js(`(document.getElementById("live")?.innerText ?? "").replace(/\\s+/g, " ").trim()`);

// Another tab (the projector, the admin, …) with its own DevTools connection,
// listening for violations (unless `watch` is false) before `before` and then
// loading `url`. `documents` has the status of each document it loads.
async function tab(name, url, { before = async () => {}, watch = true } = {}) {
  const t = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  const sock = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise((r) => (sock.onopen = r));
  let n = 0;
  const waiting = new Map();
  const documents = [];
  const documentRequests = new Set();
  sock.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); }
    if (watch) violation(name, msg);
    // The status off the wire: a revalidated document is a 304 there, and a
    // 200 (the cached one) to the page.
    if (msg.method === "Network.requestWillBeSent" && msg.params.type === "Document") documentRequests.add(msg.params.requestId);
    if (msg.method === "Network.responseReceivedExtraInfo" && documentRequests.has(msg.params.requestId)) documents.push(msg.params.statusCode);
  };
  const call = (method, params = {}) => new Promise((r) => { const i = ++n; waiting.set(i, r); sock.send(JSON.stringify({ id: i, method, params })); });
  for (const domain of ["Runtime", "Page", "Log", "Network"]) await call(`${domain}.enable`);
  await call("Page.addScriptToEvaluateOnNewDocument", { source: listenForViolations });
  await before(call);
  await call("Page.navigate", { url });
  const js = async (expr) => (await call("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
  return {
    call,
    js,
    text: async () => (await js(`(document.body?.innerText ?? "").replace(/\\s+/g, " ")`)) ?? "",
    documents,
    close: () => sock.close(),
  };
}
await send("Runtime.enable");
await send("Page.enable");
await send("Log.enable");
await send("Page.addScriptToEvaluateOnNewDocument", { source: listenForViolations });

// The policies, as the Worker sends them: the pages' by hash, the documents'
// with images from elsewhere, the web app's with eval (Dioxus) and no Clerk
// (dev auth), the projector and the verification page framable; the static
// files' from _headers; and no single-page fallback among those files.
const headersOf = async (path) => {
  const res = await fetch(`${base}${path}`);
  return { status: res.status, csp: res.headers.get("content-security-policy") ?? "", xfo: res.headers.get("x-frame-options"), nosniff: res.headers.get("x-content-type-options") === "nosniff" };
};
const byHash = (csp) => /script-src [^;]*'sha256-/.test(csp) && !csp.includes("'unsafe-eval'") && csp.includes("frame-ancestors 'none'");
for (const path of [`/j/${code}`, `/rules/${slug}`, `/privacy/${slug}`, "/j/NOSUCHCODE"]) {
  const h = await headersOf(path);
  check(byHash(h.csp) && h.xfo === "DENY", `${path}: the server page's policy, its inline scripts by hash`);
}
for (const path of ["/privacy", "/terms"]) {
  const h = await headersOf(path);
  check(byHash(h.csp) && /img-src [^;]*https:/.test(h.csp), `${path}: and images from elsewhere`);
}
for (const path of ["/", "/admin", `/admin/r/${id}`]) {
  const h = await headersOf(path);
  check(h.status === 200 && h.csp.includes("'unsafe-eval'") && !/clerk/.test(h.csp) && h.csp.includes("frame-ancestors 'none'") && h.xfo === "DENY",
    `${path}: the Worker serves the web app's shell with its policy (${h.status})`);
}
for (const path of [`/s/${slug}`, `/v/${slug}`]) {
  const h = await headersOf(path);
  check(h.status === 200 && h.csp.includes("'unsafe-eval'") && !h.csp.includes("frame-ancestors") && h.xfo === null, `${path}: may be framed`);
}
let h = await headersOf("/assets/tailwind.css");
check(h.status === 200 && h.nosniff && h.csp === "frame-ancestors 'none'", "a static file has _headers' headers");
h = await headersOf("/assets/rafi-web-dxh-missing.js");
check(h.status === 404, `a missing static file is a 404, not the shell (${h.status})`);

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
const screen = await tab("projector", `${base}/s/${slug}`);
// Read defensively: the projector loads the web app first (a large bundle).
const screenText = screen.text;
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

// The organizer's admin and the verification page, under the web app's
// policy.
const adminTab = await tab("admin", `${base}/admin`);
check(await until(async () => (await adminTab.text()).includes("Your raffles"), 45000), "the admin lists the raffles");
await adminTab.call("Page.navigate", { url: `${base}/admin/r/${id}` });
check(await until(async () => (await adminTab.text()).includes("Workers evening"), 45000), "the raffle's admin page shows it");
adminTab.close();
const verification = await tab("verification page", `${base}/v/${slug}`);
check(await until(async () => (await verification.text()).includes("Rust in Action"), 45000), "the verification page shows the draw");
verification.close();
// Loaded again, the admin is revalidated (a 304: the admin tab above has it
// cached) and keeps its policy, which a browser takes from the 304: an inline
// script is still refused, at each load. Its own tab, unwatched: the refusal
// is the point.
const inlineRuns = (t) => t.js(`new Promise((r) => {
  const s = document.createElement("script");
  s.textContent = "window.__inlineRan = true";
  document.head.appendChild(s);
  setTimeout(() => r(window.__inlineRan === true), 200);
})`);
const probe = await tab("probe", `${base}/admin`, { watch: false });
await until(async () => (await probe.text()).includes("Your raffles"), 45000);
const ranFirst = await inlineRuns(probe);
await probe.call("Page.reload");
await until(async () => probe.documents.length >= 2 && (await probe.text()).includes("Your raffles"), 45000);
const ranReloaded = await inlineRuns(probe);
check(probe.documents.includes(304), `the admin loaded again is revalidated (${probe.documents.join(", ")})`);
check(ranFirst === false && ranReloaded === false, "and still refuses an inline script");
probe.close();

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
const nowasm = await tab("no-wasm phone", `${base}/j/${otherCode}`,
  { before: (call) => call("Page.addScriptToEvaluateOnNewDocument", { source: "delete window.WebAssembly;" }) });
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
const seen = violations.length ? violations : logged;
check(seen.length === 0, `no Content-Security-Policy violations${seen.length ? ": " + seen.join(" | ") : ""}`);
ws.close();
chrome.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
