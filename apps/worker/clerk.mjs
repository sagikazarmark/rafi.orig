// The admin behind Clerk, against the production Worker served locally with a
// Clerk development instance's keys: the one path nothing else runs, Clerk's
// layer on Workers fetching the instance's JWKS through the Worker's own HTTP
// (tower-worker-fetch), and the web app under the real Clerk.js, whose
// sources the page's Content-Security-Policy names from the publishable key.
// Part of scripts/workers-check.sh (step 5), when CLERK_TEST_SECRET_KEY and
// CLERK_TEST_PUBLISHABLE_KEY are set (CI's secrets; docs/development/testing.md
// says how to make the instance), skipped otherwise.
//
//   CLERK_TEST_SECRET_KEY=sk_test_… CLERK_TEST_PUBLISHABLE_KEY=pk_test_… \
//     APP=http://127.0.0.1:8795 node apps/worker/clerk.mjs "$(mktemp -d)"
//
// Over the API, with sessions Clerk's Backend API mints for a test user of the
// instance (CLERK_TEST_USER_ID, or the instance's rafi-workers-check user,
// made if missing): no credentials are 401; a session token as a bearer is
// 200 and the user; the same token with a byte of its signature changed is
// 401; a bearer from another site is 403. In headless Chromium, the admin
// loads Clerk.js (its script and frame sources allowed by the policy), shows
// "Sign in", opens Clerk's sign-in (Turnstile and all), signs in with a
// sign-in token the way Clerk documents it (the ticket strategy), renders the
// organizer's raffles with the organization switcher and the user button,
// calls the admin API with Clerk's cookie (the token it carries names this
// origin as its authorized party), signs out, and is 401 again; no violation,
// no error.
//
// What it can't do locally: make api.clerk.com unreachable, so the 503 for a
// JWKS that can't be had stays the native suite's (TestClerk).

import { spawn } from "node:child_process";

const base = process.env.APP ?? "http://127.0.0.1:8795";
const api = process.env.CLERK_API ?? "https://api.clerk.com/v1";
const secretKey = process.env.CLERK_TEST_SECRET_KEY;
const publishableKey = process.env.CLERK_TEST_PUBLISHABLE_KEY;
const port = 9336;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
};
if (!secretKey || !publishableKey) {
  console.error("CLERK_TEST_SECRET_KEY and CLERK_TEST_PUBLISHABLE_KEY are needed");
  process.exit(2);
}

// Clerk's Backend API, as the operator's own tools would call it.
async function clerk(method, path, body) {
  const res = await fetch(`${api}${path}`, {
    method,
    headers: { authorization: `Bearer ${secretKey}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`Clerk ${method} ${path}: ${res.status} ${text.slice(0, 300)}`);
  return text ? JSON.parse(text) : null;
}
// The admin API: the status and the body, with `headers` on top of JSON.
async function whoami(headers = {}) {
  const res = await fetch(`${base}/api/admin/whoami`, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: "{}",
  });
  const text = await res.text();
  let body = null;
  try { body = JSON.parse(text); } catch {}
  return { status: res.status, body };
}

// The Worker is the production shape with the instance's keys: /config.js
// hands the web app the publishable key, and the shell's policy names the
// instance's frontend API, decoded from it.
const frontendApi = Buffer.from(publishableKey.split("_")[2] ?? "", "base64").toString().replace(/\$$/, "");
check(/\.clerk\.accounts\.dev$|\.clerk\.[a-z.]+$/.test(frontendApi), `the publishable key names a frontend API (${frontendApi})`);
const configJs = await (await fetch(`${base}/config.js`)).text();
check(configJs.includes(publishableKey), "/config.js carries the publishable key");
const shell = await fetch(`${base}/admin`);
const csp = shell.headers.get("content-security-policy") ?? "";
check(shell.status === 200 && csp.includes(frontendApi), `the admin's policy names the instance's frontend API`);

const anonymous = await whoami();
check(anonymous.status === 401, `no credentials: 401 (${anonymous.status})`);

// The test user: named, or the instance's rafi-workers-check user, made once.
const CHECK_EMAIL = "rafi-workers-check@example.com";
let userId = process.env.CLERK_TEST_USER_ID;
if (!userId) {
  const found = await clerk("GET", `/users?email_address=${encodeURIComponent(CHECK_EMAIL)}`);
  userId = found[0]?.id;
}
if (!userId) {
  const made = await clerk("POST", "/users", { email_address: [CHECK_EMAIL], skip_password_requirement: true });
  userId = made.id;
  console.log(`     (made the test user ${CHECK_EMAIL}: ${userId})`);
}
check(/^user_/.test(userId), `the test user is ${userId}`);

// Over the API.
const session = await clerk("POST", "/sessions", { user_id: userId });
const { jwt } = await clerk("POST", `/sessions/${session.id}/tokens`, {});
check(typeof jwt === "string" && jwt.split(".").length === 3, "the Backend API minted a session token");
const bearer = await whoami({ authorization: `Bearer ${jwt}` });
check(bearer.status === 200, `a session token as a bearer: 200 (${bearer.status}: the Worker fetched the instance's JWKS)`);
check(bearer.body?.owner_id === userId, `and the organizer is the user (${bearer.body?.owner_id})`);
const [h, p, s] = jwt.split(".");
const flipped = s.slice(0, -2) + (s.at(-2) === "A" ? "B" : "A") + s.slice(-1);
const tampered = await whoami({ authorization: `Bearer ${h}.${p}.${flipped}` });
check(tampered.status === 401, `the token with a byte of its signature changed: 401 (${tampered.status})`);
const crossSite = await whoami({ authorization: `Bearer ${jwt}`, "sec-fetch-site": "cross-site" });
check(crossSite.status === 403, `a bearer from another site: 403 (${crossSite.status})`);
const again = await whoami({ authorization: `Bearer ${jwt}` });
check(again.status === 200, `a second call: 200 (${again.status}: the JWKS from the layer's cache)`);

// In Chromium: one tab on the admin, its violations and errors watched.
const chrome = spawn(process.env.CHROMIUM ?? "chromium", ["--headless=new", `--remote-debugging-port=${port}`,
  "--no-first-run", `--user-data-dir=${process.argv[2]}`, "about:blank"], { stdio: "ignore" });
let targets;
for (let i = 0; i < 150 && !targets; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch { await sleep(200); }
}
const sock = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => (sock.onopen = r));
let seq = 0;
const pending = new Map();
const violations = [];
const logged = [];
const errors = [];
const VIOLATION = "rafi-csp-violation";
const listenForViolations = `document.addEventListener("securitypolicyviolation", (e) =>
  console.error("${VIOLATION}", e.disposition, e.effectiveDirective, e.blockedURI, (e.sample || "").slice(0, 80), location.pathname));`;
sock.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.args[0]?.value === VIOLATION) {
    violations.push(msg.params.args.slice(1).map((a) => a.value).join(" "));
  } else if (msg.method === "Log.entryAdded" && msg.params.entry.source === "security" && /Content Security Policy/i.test(msg.params.entry.text)) {
    logged.push(msg.params.entry.text);
  } else if (msg.method === "Runtime.exceptionThrown") {
    errors.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
  } else if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") {
    errors.push(msg.params.args.map((a) => a.value ?? a.description).join(" "));
  }
};
const call = (method, params = {}) => new Promise((r) => { const i = ++seq; pending.set(i, r); sock.send(JSON.stringify({ id: i, method, params })); });
const js = async (expr) => {
  const res = await call("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
  if (res.result?.exceptionDetails) throw new Error(res.result.exceptionDetails.exception?.description ?? res.result.exceptionDetails.text);
  return res.result?.result?.value;
};
const text = async () => (await js(`(document.body?.innerText ?? "").replace(/\\s+/g, " ")`)) ?? "";
const until = async (pred, ms = 15000) => { for (let t = 0; t < ms; t += 100) { if (await pred()) return true; await sleep(100); } return false; };
for (const domain of ["Runtime", "Page", "Log", "Network"]) await call(`${domain}.enable`);
await call("Page.addScriptToEvaluateOnNewDocument", { source: listenForViolations });

// Clerk.js loads from the instance, and the admin asks to sign in.
await call("Page.navigate", { url: `${base}/admin` });
check(await until(() => js("!!(window.Clerk && window.Clerk.loaded)")), "Clerk.js loaded under the page's policy");
check(await until(async () => (await text()).includes("Organizer sign in")), "the admin asks the organizer to sign in");
// Clerk's own sign-in, as the button opens it: its frame, styles and bot
// check come from the sources the policy names.
await js("window.Clerk.openSignIn()");
check(await until(() => js(`!!document.querySelector(".cl-signIn-root, .cl-modal, .cl-rootBox, [class*='cl-signIn']")`)), "Clerk's sign-in opens");
await sleep(2000);
await js("window.Clerk.closeSignIn()");

// Signing in with a sign-in token (Clerk's ticket strategy): what an
// invitation link does, without a password or a mailbox.
const ticket = await clerk("POST", "/sign_in_tokens", { user_id: userId, expires_in_seconds: 600 });
const status = await js(`(async () => {
  const attempt = await window.Clerk.client.signIn.create({ strategy: "ticket", ticket: ${JSON.stringify(ticket.token)} });
  if (attempt.status === "complete") await window.Clerk.setActive({ session: attempt.createdSessionId });
  return attempt.status;
})()`);
check(status === "complete", `the sign-in token signs the user in (${status})`);
check(await until(async () => (await text()).includes("Your raffles")), "the admin shows the organizer's raffles");
check(await until(() => js(`!!document.querySelector(".cl-userButton-root, .cl-userButtonTrigger, [class*='cl-userButton']")`)), "the user button is mounted");
check(await until(() => js(`!!document.querySelector(".cl-organizationSwitcher-root, .cl-organizationSwitcherTrigger, [class*='cl-organizationSwitcher']")`)), "the organization switcher is mounted");
// The page's own call: Clerk's cookie, whose token names this origin as its
// authorized party, which the Worker requires of a token that has one.
const fromPage = await js(`fetch("/api/admin/whoami", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" })
  .then(async (r) => ({ status: r.status, body: await r.json().catch(() => null) }))`);
check(fromPage?.status === 200, `the page's call with Clerk's cookie: 200 (${fromPage?.status})`);
check(fromPage?.body?.owner_id === userId, `and the organizer is the user (${fromPage?.body?.owner_id})`);
const listed = await js(`fetch("/api/admin/list_raffles", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" }).then((r) => r.status)`);
check(listed === 200, `the organizer's raffles are listed for them (${listed})`);

// Signing out.
await js("window.Clerk.signOut()");
check(await until(async () => (await text()).includes("Organizer sign in")), "signing out asks to sign in again");
const signedOut = await js(`fetch("/api/admin/whoami", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" }).then((r) => r.status)`);
check(signedOut === 401, `the page's call signed out: 401 (${signedOut})`);

// Clerk's development keys warn in the console; that is not an error here.
const real = errors.filter((e) => !/development keys|development instance|clerk.*development/i.test(e));
check(real.length === 0, `no console errors${real.length ? ": " + real.join(" | ") : ""}`);
const seen = violations.length ? violations : logged;
check(seen.length === 0, `no Content-Security-Policy violations${seen.length ? ": " + seen.join(" | ") : ""}`);
sock.close();
chrome.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
