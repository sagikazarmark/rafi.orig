// A whole evening against Rafi on Workers, locally: the admin API (dev auth),
// a phone in headless Chromium joining through the real form, draws through
// Restate (the engine in the same Worker, under /restate), the countdowns on
// the projector and the draw console, the live channel, and the end, its
// verify.json through tools/verify.py; then a
// projector cut off from the server, and one at an address with no raffle.
// Step 4 of scripts/workers-check.sh, which runs the Worker with a drand lead
// of 8 s, so the countdowns have seconds to watch.
//
//   node apps/worker/evening.mjs "$(mktemp -d)"
//
// INGRESS (Restate's ingress, default http://127.0.0.1:18080) lets it read
// the engine's own state back after the draw.
//
// Every tab reports what the pages' Content-Security-Policy refused (or, under
// RAFI_CSP=report-only, would have): one is a failure, since a refusal means
// a broken page and a report a policy about to break one.

import { spawn, spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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

// The organizer sets up a raffle dated today with two prizes, names
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
await api("save_prize", { id, prize: { id: null, name: "Ferris plush", sponsor: "", description: "", qty: 1, position: 1 } });
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
  // Every request's URL, in order: what a check can say happened in between.
  const requests = [];
  // What the page threw or logged as an error.
  const pageErrors = [];
  sock.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); }
    if (watch) violation(name, msg);
    // The status off the wire: a revalidated document is a 304 there, and a
    // 200 (the cached one) to the page.
    if (msg.method === "Network.requestWillBeSent") requests.push(msg.params.request.url);
    if (msg.method === "Runtime.exceptionThrown") pageErrors.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
    if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") pageErrors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
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
    requests,
    pageErrors,
    // Trusted clicks (DevTools' input, not element.click()) on the enabled
    // button (matching `selector`) whose text is `text`; `times` in quick
    // succession. Whether there was one.
    click: async (text, times = 1, selector = "button") => {
      const at = await js(`(() => {
        const b = [...document.querySelectorAll(${JSON.stringify(selector)})]
          .find((b) => !b.disabled && b.innerText.replace(/\\s+/g, " ").trim() === ${JSON.stringify(text)});
        if (!b) return null;
        b.scrollIntoView({ block: "center" });
        const r = b.getBoundingClientRect();
        return [r.x + r.width / 2, r.y + r.height / 2];
      })()`);
      if (!at) return false;
      for (let n = 1; n <= times; n++) {
        for (const type of ["mousePressed", "mouseReleased"]) {
          await call("Input.dispatchMouseEvent", { type, x: at[0], y: at[1], button: "left", clickCount: n });
        }
      }
      return true;
    },
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

// The organizer adds Bence, who has no phone, with the one-person form: a
// second entrant, so a "Not here" leaves someone to draw again.
const bence = await api("add_entrants", { id, add: { batch_id: `bence-${Date.now()}`, origin: "one", rows: [{ name: "Bence" }] } });
check(bence.added.length === 1, "the organizer adds Bence, who has no phone");

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
// The organizer's draw console, open through the draw.
const drawConsole = await tab("draw console", `${base}/admin/r/${id}`);
check(await until(async () => (await drawConsole.text()).includes("Prizes to draw"), 45000), "the draw console is up");
// The organizer draws from the console, double-clicking as people do: one
// draw, and the console says it's drawing.
check(await drawConsole.click("Draw", 2, "li button.btn-primary"), "the console offers Draw, and the organizer double-clicks it");
const clickedAt = Date.now();
// (innerText is as shown: the label is upper-cased by CSS.)
check(await until(async () => /drawing now/i.test(await drawConsole.text()), 5000),
  `the console says it's drawing (${Date.now() - clickedAt} ms after the click)`);
check(await until(async () => /Rust in Action/.test(await card()) && /\d|…/.test(await card()), 15000), "the phone shows the countdown");
check(await until(() => screen.js(`!!document.querySelector(".countdown")`), 15000), "the projector shows the countdown");
const consoleSecs = () => drawConsole.js(`Number(document.querySelector(".radial-progress")?.innerText.trim()) || 0`);
check(await until(async () => (await consoleSecs()) > 0, 15000), "the console shows the countdown");
// Both count down between fetches, not only when a fetch brings the server's
// time again: sampled for a few seconds, the seconds drop between two
// samples with no fetch of what the screen shows in between (the
// projector's view, the console's raffle).
const projectorSecs = () => screen.js(`Number(document.querySelector(".countdown span")?.getAttribute("aria-label")) || 0`);
const fetches = (t, pattern) => t.requests.filter((u) => pattern.test(u)).length;
const samples = [];
for (let t = 0; t < 3500; t += 250) {
  samples.push({
    projector: await projectorSecs(), projectorFetches: fetches(screen, /\/api\/r\/[^/]+\/view/),
    console: await consoleSecs(), consoleFetches: fetches(drawConsole, /\/api\/admin\/get_raffle/),
  });
  await sleep(250);
}
const ticks = (key, fetched) => samples.some((b, i) => i > 0 && b[key] > 0 && b[key] < samples[i - 1][key] && b[fetched] === samples[i - 1][fetched]);
const trace = (key) => samples.map((x) => x[key]).join(" ");
check(ticks("projector", "projectorFetches"), `the projector's countdown runs between fetches (${trace("projector")})`);
check(ticks("console", "consoleFetches"), `the console's countdown runs between fetches (${trace("console")})`);
const engineState = async () => (await fetch(`${ingress}/Raffle/${id}/get`, { method: "POST" })).json();
const announced = (await engineState()).events.filter((e) => e.type === "announced").length;
check(announced === 1, `a double click draws once (${announced} announced)`);
// The reveal: when does each screen show the first draw? Either may win it:
// Anna's phone shows the win with its claim code, or "Not this time"; the
// projector, the winner's name.
let phoneAt = 0, screenAt = 0, revealed = "";
for (let t = 0; t < 30000 && !(phoneAt && screenAt); t += 50) {
  if (!phoneAt) { const c = await card(); if (c.includes(claim) || /not this time/i.test(c)) phoneAt = Date.now(); }
  if (!screenAt) {
    revealed = (await screen.js(`document.querySelector("h2.animate-reveal bdi")?.textContent.trim() ?? ""`)) ?? "";
    if (revealed) screenAt = Date.now();
  }
  await sleep(50);
}
check(phoneAt > 0, "the phone shows the draw's outcome");
check(revealed === "Anna" || revealed === "Bence", `the projector reveals the winner (${revealed || "nobody"})`);
// Both get it pushed; the bound is loose for a loaded CI runner, the measure
// is in the line either way.
check(phoneAt > 0 && screenAt > 0 && screenAt - phoneAt < 2500,
  `the projector reveals within 2.5 s of the phone (projector ${screenAt - phoneAt} ms after the phone)`);
screen.close();

// Every draw, from the console: the winner shows at the top, waiting; Anna
// claims, and Bence isn't here, so the organizer presses Not here and the
// prize is back to be drawn, Draw enabled again, and drawn again. Bence is
// drawn once (pass 1 draws everyone once) and every draw after his Not here
// goes to Anna (pass 2: the pass-1 winners who didn't forfeit), so whoever
// the first draw picks, there are three draws and Anna claims both prizes.
const viewDraws = async () => (await (await fetch(`${base}/api/r/${slug}/view`)).json()).draws ?? [];
// The console's "Waiting for the winner" panel, as shown ("" without one).
const waitingPanel = async () => (await drawConsole.js(`[...document.querySelectorAll("section")]
  .find((s) => /waiting for the/i.test(s.querySelector("h2")?.innerText ?? ""))?.innerText.replace(/\\s+/g, " ") ?? ""`)) ?? "";
// Whether the queue's row for `prize` has its Draw button, enabled.
const offersDraw = (prize) => drawConsole.js(`[...document.querySelectorAll("li")].some((li) => li.innerText.includes(${JSON.stringify(prize)})
  && [...li.querySelectorAll("button.btn-primary")].some((b) => !b.disabled && b.innerText.trim() === "Draw"))`);
let notHere = null, annaWon = false;
for (let n = 1; n <= 3; n++) {
  // The first draw is the double click's; then the queue's first Draw,
  // the prize put back first.
  if (n > 1) check(await until(() => drawConsole.click("Draw", 1, "li button.btn-primary"), 10000), `draw ${n}: the organizer clicks Draw`);
  let d = null;
  await until(async () => { d = (await viewDraws())[n - 1] ?? null; return !!d; }, 30000);
  if (!d) { check(false, `draw ${n} is decided`); break; }
  const prize = d.prize_name, who = d.display_name;
  check(await until(async () => { const p = await waitingPanel(); return p.includes(prize) && p.includes(who); }, 15000),
    `draw ${n}: the console shows ${who} for the ${prize} at the top, waiting`);
  if (who === "Bence") {
    check(notHere === null, "Bence is drawn once");
    notHere = d;
    check(await until(() => drawConsole.click("Not here", 1, "li button"), 5000), "Bence isn't here: the organizer presses Not here");
    check(await until(() => drawConsole.click("Not here", 1, ".modal-open .modal-action button"), 5000), "and confirms it");
    check(await until(async () => (await waitingPanel()) === "" && (await offersDraw(prize)), 15000),
      `the ${prize} is back to be drawn, Draw enabled again`);
    const outcome = (await viewDraws()).find((x) => x.k === d.k)?.outcome;
    check(outcome === "not_here", `the draw is recorded as not here (${outcome})`);
    continue;
  }
  check(who === "Anna", `draw ${n} picks Anna or Bence (${who})`);
  if (notHere?.item === d.item) {
    const again = (await viewDraws()).filter((x) => x.item === d.item);
    check(again.length === 2 && again[0].outcome === "not_here" && again[1].k === d.k,
      `the ${prize} is drawn a second time, the first draw not here (${again.map((x) => x.outcome).join(", ")})`);
  }
  if (!annaWon) {
    annaWon = true;
    check(await until(async () => (await card()).includes(claim), 15000), "the phone shows the win with its claim code");
    check((await js(`!!document.querySelector("#live [role=alert]")`)) === true, "the win is announced");
  }
  check(await until(() => drawConsole.click("Claimed"), 5000), `the organizer presses Claimed for the ${prize}`);
  check(await until(async () => new RegExp(`${prize}\\s*claimed`).test(await drawConsole.text()), 15000), `the console shows the ${prize} claimed`);
}

// The full view for screens: version from the database, the draws in it.
const view = await (await fetch(`${base}/api/r/${slug}/view`)).json();
check(view.draws.length === 3 && view.follow?.kind === "partyline", `the full view has the three draws and says to follow over partyline (${view.draws.length})`);
check(view.draws.filter((x) => x.outcome === "not_here").map((x) => x.display_name).join() === "Bence"
  && view.draws.filter((x) => x.outcome === "claimed" && x.display_name === "Anna").length === 2,
  "Bence was not here, Anna claimed both prizes");
const round = view.draws[0]?.round;
check(Number.isInteger(round) && round > 0, `the draw committed to a drand round (${round})`);
// The engine's own record of the raffle, in Restate.
const state = await (await fetch(`${ingress}/Raffle/${id}/get`, { method: "POST" })).json();
const drawnEvents = state?.events?.filter((e) => e.type === "drawn").length;
check(drawnEvents === 3 && state.events.some((e) => e.type === "resolved" && e.resolution === "not_here"),
  `Restate's own state has the draws (${drawnEvents}) and the Not here`);
const participant = await (await fetch(`${base}/api/r/${slug}/participant`)).json();
const anna = view.draws.find((x) => x.display_name === "Anna")?.entrant;
check(participant.winners.length === 2 && participant.winners.every((w) => w.entrant_id === anna) && !JSON.stringify(participant).includes("anna@example.com"),
  "the participant view has the winner, twice, and no email");

// The organizer's admin and the verification page, under the web app's
// policy.
const adminTab = await tab("admin", `${base}/admin`);
check(await until(async () => (await adminTab.text()).includes("Your raffles"), 45000), "the admin lists the raffles");
await adminTab.call("Page.navigate", { url: `${base}/admin/r/${id}` });
check(await until(async () => (await adminTab.text()).includes("Workers evening"), 45000), "the raffle's admin page shows it");
adminTab.close();
const verification = await tab("verification page", `${base}/v/${slug}`);
check(await until(async () => (await verification.text()).includes("Rust in Action"), 45000), "the verification page shows the draw");
check((await verification.text()).includes("2 entrants: 1 joined on their phone, 1 added by the organizer"),
  "and counts the list by source");
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

// The end, from the console: Finish, confirmed.
check(await drawConsole.click("Finish the raffle"), "the console offers Finish");
check(await until(() => drawConsole.click("Finish"), 5000), "and asks first");
check(await until(async () => (await drawConsole.text()).includes("That's a wrap"), 15000), "the raffle is finished, and the console says so");
check(drawConsole.pageErrors.length === 0, `the draw console threw nothing${drawConsole.pageErrors.length ? ": " + drawConsole.pageErrors.join(" | ") : ""}`);
drawConsole.close();
// The evening is its own record: its verify.json, as anyone downloads it,
// passes tools/verify.py with --test-chain (the evening draws from the mock
// chain), and fails it without, so the option is what lets it pass.
const verifyFile = `${process.argv[2]}/verify.json`;
const verifyRes = await fetch(`${base}/api/r/${slug}/verify.json`);
writeFileSync(verifyFile, await verifyRes.text());
const verifier = fileURLToPath(new URL("../../tools/verify.py", import.meta.url));
const verifyPy = (...args) => {
  const r = spawnSync("python3", [verifier, ...args, verifyFile], { encoding: "utf8" });
  return { status: r.status, last: (r.stdout ?? "").trim().split("\n").pop() ?? "", out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};
const verified = verifyPy("--test-chain");
check(verifyRes.ok && verified.status === 0 && verified.last === "VERIFIED (test chain: not publicly verifiable)",
  `the evening's verify.json passes tools/verify.py --test-chain (${verified.last})${verified.status === 0 ? "" : "\n" + verified.out}`);
const strict = verifyPy();
check(strict.status === 1 && strict.last === "VERIFICATION FAILED",
  `and fails it without, the mock chain not being public (exit ${strict.status}: ${strict.last})`);
// Claimed, and the prize sent later (the default): the phone says so.
check(await until(async () => /send you your prize|enjoy your prize/i.test(await card()), 15000), "the winner's phone sees the raffle finish");
await api("end_live_updates", { id });
admin = await api("get_raffle", { id });
check(admin.view.live_ended === true, "live updates are ended");
const late = new WebSocket(`${base.replace(/^http/, "ws")}/partyline/raffle/${id}?v=1`);
const closed = await new Promise((r) => { late.onclose = (e) => r(e.code); setTimeout(() => r(null), 5000); });
check(closed === 4403, `a socket after the end is refused (${closed})`);

// A phone without WebAssembly joins through fallback.js, which solves with
// the same worker script (/participant/pow-worker.js). Its own raffle, so
// the draws above keep their two entrants.
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

// Long names: the organizer adds someone from a list with a full name of 64
// characters, the longest they may give, and a projector-sized screen fits it
// without scrolling sideways in the recent names, at the reveal and on the
// thanks screen. Its own raffle, where they are the only entrant, so the
// winner is known.
const longName = "Maria Fernanda de los Ángeles García-López y Hernández Rodríguez";
const long = await newRaffle(`Workers evening, long names ${Date.now()}`);
await api("save_prize", { id: long, prize: { id: null, name: "Mug", sponsor: "", description: "", qty: 1, position: 0 } });
await api("set_lifecycle", { id: long, action: "publish" });
await api("set_lifecycle", { id: long, action: "open" });
const added = await api("add_entrants", { id: long, add: { batch_id: `long-${Date.now()}`, origin: "list", rows: [{ name: longName }] } });
check(added.added.length === 1, `the organizer adds a ${[...longName].length}-character name`);
const longSlug = (await api("get_raffle", { id: long })).view.slug;
const projectorSize = (call, width, height) => call("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
const projector = await tab("projector, long names", `${base}/s/${longSlug}`, { before: (call) => projectorSize(call, 1280, 720) });
const fits = () => projector.js(`document.documentElement.scrollWidth <= document.documentElement.clientWidth`);
const shows = (selector) => projector.js(`[...document.querySelectorAll(${JSON.stringify(selector)})].some((e) => e.textContent.includes(${JSON.stringify(longName)}))`);
check(await until(() => shows("li bdi"), 45000), "the projector has it in the recent names");
check((await projector.text()).includes("Registered? You're already in."), "and tells the people on the list they're in");
check(await fits(), "in the recent names, it fits a 1280×720 projector");
await projectorSize(projector.call, 1920, 1080);
check(await until(fits, 2000), "and a 1920×1080 one");
await projectorSize(projector.call, 1280, 720);
// The projector can't reach the server for a while: it says so, keeps the last
// view, and stops saying so once it can again.
await projector.call("Network.setBlockedURLs", { urls: ["*/api/r/*/view"] });
// Polls back off while failing (2.5, 5, then 10 s apart), and the badge
// waits for 10 s of failures.
check(await until(async () => (await projector.text()).includes("Reconnecting…"), 30000),
  "the projector cut off from the server says it's reconnecting");
check((await projector.text()).includes("Registered? You're already in."), "and keeps the last view meanwhile");
await projector.call("Network.setBlockedURLs", { urls: [] });
check(await until(async () => !(await projector.text()).includes("Reconnecting…"), 15000),
  "and stops once it's back (polls back off to 10 s while failing)");
await api("set_lifecycle", { id: long, action: "close" });
await api("start_draw", { id: long });
let longItem = null;
await until(async () => { const v = (await api("get_raffle", { id: long })).view; longItem = v.manifest_hash ? v.items[0]?.item : null; return !!longItem; }, 15000);
await api("draw_item", { id: long, item: longItem, command_id: commandId() });
check(await until(() => shows("h2 bdi"), 30000), "the projector reveals the long name");
check(await until(async () => (await projector.text()).includes("Come to the front!"), 5000),
  "and calls them to the front: no phone tells them");
check(await fits(), "revealed, it fits the projector");
await api("resolve_item", { id: long, item: longItem, resolution: "claimed", command_id: commandId() });
await api("finish_raffle", { id: long, command_id: commandId() });
check(await until(async () => (await projector.text()).includes("Thanks for playing") && (await shows(".card bdi")), 15000),
  "the thanks screen lists the winner");
check(await fits(), "and fits the projector");
projector.close();
const longVerification = await tab("verification page, long names", `${base}/v/${longSlug}`);
check(await until(async () => (await longVerification.text()).includes("1 entrant: 0 joined on their phones, 1 added by the organizer"), 45000),
  "the verification page counts the organizer's list");
longVerification.close();

// A projector at an address with no raffle says so, rather than loading for
// ever.
const startedAt = Date.now();
const nowhere = await tab("projector, no raffle", `${base}/s/no-such-raffle-${Date.now()}`);
const said = await until(async () => (await nowhere.text()).includes("No raffle at this address, or it isn't published yet."), 15000);
check(said, `a projector at an unknown address says there's no raffle there (${Date.now() - startedAt} ms)`);
nowhere.close();

const real = errors.filter((e) => !/WebSocket|4403|ERR_/i.test(e));
check(real.length === 0, `no console errors${real.length ? ": " + real.join(" | ") : ""}`);
const seen = violations.length ? violations : logged;
check(seen.length === 0, `no Content-Security-Policy violations${seen.length ? ": " + seen.join(" | ") : ""}`);
ws.close();
chrome.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
