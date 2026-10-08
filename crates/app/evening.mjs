// A whole evening against the app on Workers, locally: the admin API (dev
// auth), a phone in headless Chromium joining through the real form, draws
// through Restate, the live channel, and the end. See scripts/app-check.sh.
//
//   node crates/app/evening.mjs "$(mktemp -d)"

import { spawn } from "node:child_process";

const base = process.env.APP ?? "http://127.0.0.1:8787";
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

// The organizer sets up a raffle with one prize and opens registration.
const id = await api("create_raffle", { title: `Workers evening ${Date.now()}` });
await api("save_item", { id, item: { id: null, name: "Rust in Action", sponsor: "Sponsor Kft.", description: "", qty: 1, position: 0 } });
await api("lifecycle", { id, action: "publish" });
await api("lifecycle", { id, action: "open" });
let admin = await api("get_raffle", { id });
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
};
const send = (method, params = {}) => new Promise((r) => { const i = ++seq; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const until = async (pred, ms = 10000) => { for (let t = 0; t < ms; t += 100) { if (await pred()) return true; await sleep(100); } return false; };
const card = () => js(`(document.getElementById("live")?.innerText ?? "").replace(/\\s+/g, " ").trim()`);
await send("Runtime.enable");
await send("Page.enable");

await send("Page.navigate", { url: `${base}/j/${code}` });
check(await until(() => js(`!!document.getElementById("submit") && !document.getElementById("submit").disabled`), 20000),
  "the proof-of-work enables Join");
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
check(cfg.channel === true, "the page knows there's a live channel");

// The draw, through Restate; the phone follows over the live channel.
await api("lifecycle", { id, action: "close" });
await api("start_draw", { id });
check(await until(async () => /draw is starting|starting/i.test(await card()), 15000), "the freeze reaches the phone");
admin = await api("get_raffle", { id });
const unit = admin.view.units[0].unit_id;
await api("draw_unit", { id, unit_id: unit, offline: false, command_id: commandId() });
check(await until(async () => /Rust in Action/.test(await card()) && /\d|…/.test(await card()), 15000), "the phone shows the countdown");
check(await until(async () => (await card()).includes(claim), 30000), "the phone shows the win with its claim code");
check((await js(`!!document.querySelector("#live [role=alert]")`)) === true, "the win is announced");

// The full view for screens: version from the database, the draw in it.
const view = await (await fetch(`${base}/api/r/${slug}/view`)).json();
check(view.draws.length === 1 && view.channel === true, "the full view has the draw and the channel flag");
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

const real = errors.filter((e) => !/WebSocket|4403|ERR_/i.test(e));
check(real.length === 0, `no console errors${real.length ? ": " + real.join(" | ") : ""}`);
ws.close();
chrome.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
