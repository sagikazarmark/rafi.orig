// Rafi participant page, JavaScript fallback: runs when the WebAssembly can't
// (no WebAssembly, iOS Lockdown Mode; not ready in time; a panic). It does the
// same job without the live channel: polls the participant view, renders the
// card the way rafi-live-client's Card does, solves the proof-of-work, and keeps the
// theme toggle and the wake/sound button working. The page's config is the
// JSON in #rafi-config (see crates/participant/src/config.rs).
// Class names here are scanned by Tailwind (see tailwind.css).
(function () {
  "use strict";
  if (window.__rafiFallbackRunning) return;
  window.__rafiFallbackRunning = true;

  var cfg = {};
  try { cfg = JSON.parse(document.getElementById("rafi-config").textContent); } catch (e) {}
  var S = cfg.strings || {};
  var me = cfg.entrant_id || null;
  var view = cfg.view || null;
  var live = document.getElementById("live");
  var t0 = window.__rafiT0 || Date.now();
  var clockOffset = (cfg.server_now_ms || t0) - t0;
  var wonAnnounced = false;
  var audioCtx = null;
  var countdownTimer = null;

  var CARD = "card bg-base-100 shadow-sm ring-1 ring-base-300 min-h-28 items-center justify-center gap-1 p-6 text-center";
  var WON = "card bg-success text-success-content shadow-xl items-center gap-2 p-7 text-center";

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function now() { return Date.now() + clockOffset; }
  function isOver(s) { return s === "finished" || s === "purged"; }
  function isLive(s) { return s === "ready" || s === "open" || s === "closed" || s === "frozen" || s === "drawing"; }

  // ---- The card (mirrors rafi_live_client::screen and Card) ----
  function render(v, announce) {
    if (!live || !v) return;
    if (!me && v.state === "open" && cfg.page_state !== "open") { location.reload(); return; }
    if (countdownTimer) { clearInterval(countdownTimer); countdownTimer = null; }
    live.textContent = "";
    var win = null;
    (v.winners || []).forEach(function (w) { if (w.entrant_id === me) win = w; });

    if (win) {
      var won = el("div", WON + (announce ? " animate-pop" : ""));
      if (announce) won.setAttribute("role", "alert");
      won.appendChild(el("p", "text-5xl", "🏆"));
      won.appendChild(el("p", "font-display text-5xl font-black tracking-tight", S.you_won));
      won.appendChild(el("p", "text-lg font-semibold", win.prize_name));
      if (cfg.claim_code) won.appendChild(el("span", "claim-code mt-1 rounded-2xl bg-base-100 px-5 py-2 text-4xl text-success shadow-sm", cfg.claim_code));
      won.appendChild(el("p", "text-sm opacity-90", win.claimed || isOver(v.state) ? S.thanks_won : S.go_to_organizer));
      live.appendChild(won);
      return;
    }
    var card = el("div", CARD);
    if (isOver(v.state)) {
      card.appendChild(el("p", "text-4xl", "🎉"));
      card.appendChild(el("p", "font-display text-2xl font-extrabold", S.thanks));
    } else if (v.current) {
      card.appendChild(el("span", "badge badge-primary badge-soft font-semibold", "🎲 " + S.drawing));
      card.appendChild(el("p", "font-display text-2xl font-extrabold", v.current.prize_name));
      var cd = el("div", "font-display text-6xl font-black text-primary", "");
      card.appendChild(cd);
      card.appendChild(el("p", "text-xs text-base-content/60", S.locking + " · " + S.drand_round + " " + v.current.round));
      var tick = function () {
        var s = Math.max(0, Math.ceil((v.current.round_time * 1000 - now()) / 1000));
        cd.textContent = s > 0 ? String(s) : "…";
      };
      tick();
      countdownTimer = setInterval(tick, 250);
    } else if (v.state === "frozen" || v.state === "drawing") {
      if (v.last) {
        card.appendChild(el("p", "text-sm text-base-content/60", "🎉 " + v.last.display_name + " → " + v.last.prize_name));
        card.appendChild(el("p", "text-xl font-bold", me ? S.not_this_time + " 🤞" : S.drawing));
      } else {
        card.appendChild(el("p", "text-3xl", "🥁"));
        card.appendChild(el("p", "text-xl font-bold", S.draw_starting));
      }
      card.appendChild(el("span", "badge badge-secondary badge-soft mt-1", v.prizes_left + " " + S.prizes_left));
    } else if (me) {
      card.appendChild(el("p", "text-3xl animate-float", "⏳"));
      card.appendChild(el("p", "text-lg font-semibold", S.waiting));
    } else {
      return;
    }
    live.appendChild(card);
  }

  function apply(v) {
    if (!v || (view && v.v <= view.v)) return;
    var wasWin = view && (view.winners || []).some(function (w) { return w.entrant_id === me; });
    var isWin = me && (v.winners || []).some(function (w) { return w.entrant_id === me; });
    view = v;
    var announce = !wasWin && isWin && !wonAnnounced;
    if (announce) { wonAnnounced = true; chime(); if (navigator.vibrate) navigator.vibrate([200, 100, 200, 100, 400]); }
    render(v, announce);
  }

  // ---- Polling (no live channel here) ----
  function poll() {
    if (!cfg.participant_url) return;
    var sent = Date.now();
    fetch(cfg.participant_url, { cache: "no-store" })
      .then(function (r) {
        var date = r.headers.get("date");
        if (date) {
          var server = Date.parse(date);
          if (isFinite(server)) {
            var offset = server + 500 - (sent + Date.now()) / 2;
            if (Math.abs(offset - clockOffset) > 1500) clockOffset = offset;
          }
        }
        return r.ok ? r.json() : null;
      })
      .then(apply)
      .catch(function () {});
  }
  var pollTimer = null;
  function schedule() {
    if (pollTimer) clearInterval(pollTimer);
    // Faster while a draw is pending, so the reveal isn't late.
    pollTimer = setInterval(function () {
      if (view && !isLive(view.state)) return;
      poll();
    }, view && view.current ? 1500 : 3000);
  }

  // ---- Theme: system -> light -> dark ----
  var themeBtn = document.getElementById("theme");
  function currentTheme() {
    try { return localStorage.getItem("rafi-theme") || "system"; } catch (e) { return "system"; }
  }
  function showTheme(t) {
    if (!themeBtn) return;
    var map = { system: "theme-icon-system", rafi: "theme-icon-light", "rafi-dark": "theme-icon-dark" };
    Object.keys(map).forEach(function (k) {
      var icon = themeBtn.querySelector("." + map[k]);
      if (icon) icon.classList.toggle("hidden", k !== t);
    });
  }
  if (themeBtn) {
    showTheme(currentTheme());
    themeBtn.addEventListener("click", function () {
      var next = { system: "rafi", rafi: "rafi-dark", "rafi-dark": "system" }[currentTheme()] || "system";
      try {
        if (next === "system") { localStorage.removeItem("rafi-theme"); document.documentElement.removeAttribute("data-theme"); }
        else { localStorage.setItem("rafi-theme", next); document.documentElement.setAttribute("data-theme", next); }
      } catch (e) {}
      showTheme(next);
    });
  }

  // ---- Proof of work (join form) ----
  // The WebAssembly's solver, dioxus-altcha's worker.js, served next to this
  // file: workers splitting the counters, or this page if they can't start.
  var SOLVER = "/participant/altcha-worker.js";
  var powDeadline = null, powRun = 0;
  function loadSolver() {
    return new Promise(function (resolve, reject) {
      if (typeof window.altchaSolve === "function") return resolve(window.altchaSolve);
      var s = document.createElement("script");
      s.src = SOLVER;
      s.onload = function () {
        if (typeof window.altchaSolve === "function") resolve(window.altchaSolve);
        else reject(new Error("no solver"));
      };
      s.onerror = function () { reject(new Error("no solver")); };
      document.head.appendChild(s);
    });
  }
  // Rejects with null when no worker could run: then the page solves.
  function solveInWorkers(raw) {
    return new Promise(function (resolve, reject) {
      var workers = [], errors = 0, done = false;
      var n = Math.min(Math.max(navigator.hardwareConcurrency || 2, 1), 4);
      function finish(settle, value) {
        if (done) return;
        done = true;
        workers.forEach(function (w) { w.onmessage = null; w.onerror = null; w.terminate(); });
        settle(value);
      }
      for (var i = 0; i < n; i++) {
        var w;
        try { w = new Worker(SOLVER); } catch (e) { break; }
        w.onmessage = function (e) {
          var r = JSON.parse(e.data);
          if (r.error) finish(reject, new Error(r.error)); else finish(resolve, r);
        };
        // A worker that can't load (a 404 served as the app's index.html
        // fails to parse) reports `error` before any message.
        w.onerror = function (e) {
          e.preventDefault();
          if (++errors >= workers.length) finish(reject, null);
        };
        w.postMessage('{"challenge":' + raw + ',"start":' + i + ',"step":' + n + "}");
        workers.push(w);
      }
      if (!workers.length) finish(reject, null);
    });
  }
  // The time left to post a solution, as dioxus-altcha's usable_for: until
  // expiry less a margin, or null if this phone's clock can't tell.
  function usableFor(challenge) {
    var at = challenge.parameters && challenge.parameters.expiresAt;
    if (!at) return null;
    var left = at * 1000 - Date.now();
    if (!(left > 0 && left <= 864e5)) return null;
    return left - Math.min(30000, left / 2);
  }
  async function solveAltcha(then) {
    var input = document.getElementById("altcha");
    var submit = document.getElementById("submit");
    if (!input || !submit) return;
    var run = ++powRun;
    input.value = "";
    powDeadline = null;
    submit.disabled = true;
    var value = "", deadline = null;
    try {
      if (window.isSecureContext && window.crypto && crypto.subtle) {
        var raw = await (await fetch(cfg.altcha_url || "/api/altcha", { cache: "no-store" })).text();
        var challenge = JSON.parse(raw);
        var solution = await solveInWorkers(raw).catch(function (e) {
          if (e) throw e;
          return loadSolver().then(function (solve) {
            return solve(challenge, 0, 1, { yieldMs: 150 });
          });
        });
        // The challenge as received: re-serializing it could change what was signed.
        value = btoa('{"challenge":' + raw + ',"solution":' + JSON.stringify(solution) + "}");
        var left = usableFor(challenge);
        if (left != null) deadline = performance.now() + left;
      }
    } catch (e) { /* the server will ask to retry */ }
    if (run !== powRun) return;
    input.value = value;
    powDeadline = deadline;
    var label = submit.getAttribute("data-label");
    if (label) submit.innerHTML = label;
    submit.disabled = false;
    if (then && value) {
      if (then.requestSubmit) then.requestSubmit(); else then.submit();
    }
  }

  // ---- Wake lock and sound (need a user gesture) ----
  function unlockAudio() {
    try {
      if (!audioCtx) audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      if (audioCtx.state === "suspended") audioCtx.resume();
      var b = audioCtx.createBuffer(1, 1, 22050), s = audioCtx.createBufferSource();
      s.buffer = b; s.connect(audioCtx.destination); s.start(0);
      if (navigator.audioSession) navigator.audioSession.type = "playback";
    } catch (e) { /* best effort */ }
  }
  function chime() {
    if (!audioCtx) return;
    try {
      var t = audioCtx.currentTime;
      [523.25, 659.25, 783.99, 1046.5].forEach(function (f, i) {
        var o = audioCtx.createOscillator(), g = audioCtx.createGain();
        o.frequency.value = f; o.type = "triangle";
        g.gain.setValueAtTime(0.0001, t + i * 0.12);
        g.gain.exponentialRampToValueAtTime(0.3, t + i * 0.12 + 0.02);
        g.gain.exponentialRampToValueAtTime(0.0001, t + i * 0.12 + 0.4);
        o.connect(g); g.connect(audioCtx.destination);
        o.start(t + i * 0.12); o.stop(t + i * 0.12 + 0.45);
      });
    } catch (e) { /* best effort */ }
  }
  var wakeBtn = document.getElementById("wake");
  var wakeLock = null;
  async function keepAwake() {
    unlockAudio();
    try {
      if ("wakeLock" in navigator) {
        wakeLock = await navigator.wakeLock.request("screen");
        if (wakeBtn) { wakeBtn.textContent = "✓ " + (cfg.screen_on || ""); wakeBtn.disabled = true; }
        wakeLock.addEventListener("release", function () {
          if (wakeBtn) { wakeBtn.textContent = cfg.keep_screen_on || ""; wakeBtn.disabled = false; }
        });
      }
    } catch (e) { /* iOS may refuse after backgrounding; the button stays */ }
  }
  // The WebAssembly may have wired these already before it stopped; a second
  // listener only repeats harmless work.
  if (wakeBtn) wakeBtn.addEventListener("click", keepAwake);
  var form = document.getElementById("join");
  if (form) form.addEventListener("submit", unlockAudio);
  // A solution past its deadline (a phone left on the form) is solved again
  // before the form posts.
  if (form && cfg.needs_altcha) form.addEventListener("submit", function (e) {
    var input = document.getElementById("altcha");
    if (powDeadline != null && performance.now() >= powDeadline && input && input.value) {
      e.preventDefault();
      solveAltcha(form);
    }
  });
  // Back/forward cache: the restored solution is spent.
  window.addEventListener("pageshow", function (e) {
    if (e.persisted && cfg.needs_altcha) solveAltcha(null);
  });

  function resume() { if (document.visibilityState === "visible") poll(); }
  document.addEventListener("visibilitychange", resume);
  window.addEventListener("pageshow", resume);
  window.addEventListener("online", resume);
  window.addEventListener("focus", resume);

  if (cfg.needs_altcha) solveAltcha(null);
  if (view) render(view, false);
  if (view && isLive(view.state)) { poll(); schedule(); setInterval(schedule, 5000); }
})();
