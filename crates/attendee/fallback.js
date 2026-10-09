// Rafi attendee page, JavaScript fallback: runs when the WebAssembly can't
// (no WebAssembly, iOS Lockdown Mode; not ready in time; a panic). It does the
// same job without the live channel: polls the participant view, renders the
// card the way rafi-live's Card does, solves the proof-of-work, and keeps the
// theme toggle and the wake/sound button working. The page's config is the
// JSON in #rafi-config (see crates/attendee/src/config.rs).
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

  // ---- The card (mirrors rafi_live::screen and Card) ----
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
  function hex(buf) {
    return Array.prototype.map.call(new Uint8Array(buf), function (b) { return ("0" + b.toString(16)).slice(-2); }).join("");
  }
  async function solveAltcha() {
    var input = document.getElementById("altcha");
    var submit = document.getElementById("submit");
    if (!input || !submit) return;
    try {
      if (window.crypto && crypto.subtle) {
        var c = await (await fetch(cfg.altcha_url || "/api/altcha", { cache: "no-store" })).json();
        var enc = new TextEncoder();
        for (var n = 0; n <= c.maxnumber; n++) {
          if (hex(await crypto.subtle.digest("SHA-256", enc.encode(c.salt + n))) === c.challenge) {
            input.value = btoa(JSON.stringify({ algorithm: c.algorithm, challenge: c.challenge, number: n, salt: c.salt, signature: c.signature }));
            break;
          }
        }
      }
    } catch (e) { /* the server will ask to retry */ }
    var label = submit.getAttribute("data-label");
    if (label) submit.innerHTML = label;
    submit.disabled = false;
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

  function resume() { if (document.visibilityState === "visible") poll(); }
  document.addEventListener("visibilitychange", resume);
  window.addEventListener("pageshow", resume);
  window.addEventListener("online", resume);
  window.addEventListener("focus", resume);

  if (cfg.needs_altcha) solveAltcha();
  if (view) render(view, false);
  if (view && isLive(view.state)) { poll(); schedule(); setInterval(schedule, 5000); }
})();
