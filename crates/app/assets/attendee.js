// Rafi attendee page: live status over SSE, proof-of-work for joining,
// theme toggle, wake lock and a win sound. `window.RAFI` is injected by the
// server. Class names here are scanned by Tailwind (see tailwind.css).
(function () {
  "use strict";
  var cfg = window.RAFI || {};
  var S = cfg.strings || {};
  var live = document.getElementById("live");
  var lastV = 0;
  var lastMessageAt = Date.now();
  var clockOffset = 0;
  var wonAnnounced = false;
  var audioCtx = null;
  var es = null;
  var pollTimer = null;
  var countdownTimer = null;

  var CARD = "card bg-base-100 shadow-sm ring-1 ring-base-300 min-h-28 items-center justify-center gap-1 p-6 text-center";
  var WON = "card bg-success text-success-content shadow-xl items-center gap-2 p-7 text-center animate-pop";

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
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
    if (!input || !submit || !window.crypto || !crypto.subtle) return;
    var label = submit.innerHTML;
    submit.disabled = true;
    submit.innerHTML = '<span class="loading loading-dots loading-md"></span>';
    try {
      var c = await (await fetch("/api/altcha", { cache: "no-store" })).json();
      var enc = new TextEncoder();
      for (var n = 0; n <= c.maxnumber; n++) {
        if (hex(await crypto.subtle.digest("SHA-256", enc.encode(c.salt + n))) === c.challenge) {
          input.value = btoa(JSON.stringify({ algorithm: c.algorithm, challenge: c.challenge, number: n, salt: c.salt, signature: c.signature }));
          break;
        }
      }
    } catch (e) { /* the server will ask to retry */ }
    submit.disabled = false;
    submit.innerHTML = label;
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
        if (wakeBtn) { wakeBtn.textContent = "✓ " + S.screen_on; wakeBtn.disabled = true; }
        wakeLock.addEventListener("release", function () {
          if (wakeBtn) { wakeBtn.textContent = S.keep_screen_on; wakeBtn.disabled = false; }
        });
      }
    } catch (e) { /* iOS may refuse after backgrounding; the button stays */ }
  }
  if (wakeBtn) wakeBtn.addEventListener("click", keepAwake);

  // ---- Live view ----
  function now() { return Date.now() + clockOffset; }

  function render(v) {
    if (!live) return;
    if (!cfg.entrantId && v.state === "open" && cfg.state !== "open") { location.reload(); return; }
    cfg.state = v.state;
    if (countdownTimer) { clearInterval(countdownTimer); countdownTimer = null; }
    live.className = CARD;
    live.textContent = "";
    live.removeAttribute("role");
    var me = cfg.entrantId;
    var myWin = null;
    (v.draws || []).forEach(function (d) {
      if (d.entrant_id === me && (d.outcome === "drawn" || d.outcome === "claimed")) myWin = d;
    });
    var left = (v.prizes || []).reduce(function (n, p) { return n + p.left; }, 0);
    var last = (v.draws || [])[v.draws.length - 1];

    if (myWin) {
      live.className = WON;
      live.appendChild(el("p", "text-5xl", "🏆"));
      live.appendChild(el("p", "font-display text-5xl font-black tracking-tight", S.you_won));
      live.appendChild(el("p", "text-lg font-semibold", myWin.item_name));
      if (cfg.claimCode) live.appendChild(el("span", "claim-code mt-1 rounded-2xl bg-base-100 px-5 py-2 text-4xl text-success shadow-sm", cfg.claimCode));
      var done = v.state === "finished" || v.state === "purged";
      live.appendChild(el("p", "text-sm opacity-90", done ? S.thanks_won : S.go_to_organizer));
      if (!wonAnnounced) {
        wonAnnounced = true;
        chime();
        if (navigator.vibrate) navigator.vibrate([200, 100, 200, 100, 400]);
        live.setAttribute("role", "alert");
      }
      return;
    }
    if (v.state === "finished" || v.state === "purged") {
      live.appendChild(el("p", "text-4xl", "🎉"));
      live.appendChild(el("p", "font-display text-2xl font-extrabold", S.thanks));
      return;
    }
    if (v.current) {
      live.appendChild(el("span", "badge badge-primary badge-soft font-semibold", "🎲 " + S.drawing));
      live.appendChild(el("p", "font-display text-2xl font-extrabold", v.current.item_name));
      var cd = el("div", "font-display text-6xl font-black text-primary", "");
      live.appendChild(cd);
      live.appendChild(el("p", "text-xs text-base-content/60", S.locking + " · " + S.drand_round + " " + v.current.round));
      var tick = function () {
        var s = Math.max(0, Math.ceil((v.current.round_time * 1000 - now()) / 1000));
        cd.textContent = s > 0 ? String(s) : "…";
      };
      tick();
      countdownTimer = setInterval(tick, 250);
      return;
    }
    if (v.state === "frozen" || v.state === "drawing") {
      if (last) {
        live.appendChild(el("p", "text-sm text-base-content/60", "🎉 " + last.display_name + " → " + last.item_name));
        live.appendChild(el("p", "text-xl font-bold", me ? S.not_this_time + " 🤞" : S.drawing));
      } else {
        live.appendChild(el("p", "text-3xl", "🥁"));
        live.appendChild(el("p", "text-xl font-bold", S.draw_starting));
      }
      live.appendChild(el("span", "badge badge-secondary badge-soft mt-1", left + " " + S.prizes_left));
      return;
    }
    if (me) {
      live.appendChild(el("p", "text-3xl animate-float", "⏳"));
      live.appendChild(el("p", "text-lg font-semibold", S.waiting));
    }
  }

  function onView(v) {
    lastMessageAt = Date.now();
    if (!v || v.v < lastV) return;
    lastV = v.v;
    if (v.server_now_ms) clockOffset = v.server_now_ms - Date.now();
    render(v);
  }

  function poll() {
    fetch("/api/r/" + cfg.slug + "/view", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(onView)
      .catch(function () {});
  }

  function connect() {
    if (es) { try { es.close(); } catch (e) {} }
    if (!window.EventSource) { if (!pollTimer) pollTimer = setInterval(poll, 3000); poll(); return; }
    es = new EventSource("/api/r/" + cfg.slug + "/events");
    es.addEventListener("ping", function () { lastMessageAt = Date.now(); });
    es.addEventListener("view", function (e) {
      try { onView(JSON.parse(e.data)); } catch (err) {}
      if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
    });
    es.onerror = function () {
      if (es.readyState === 2 && !pollTimer) { pollTimer = setInterval(poll, 3000); setTimeout(connect, 5000); }
    };
  }

  // Phones suspend pages in the background; on return, reconnect if stale.
  function resume() {
    if (document.visibilityState !== "visible") return;
    if (Date.now() - lastMessageAt > 20000 || !es || es.readyState === 2) connect();
    if (wakeLock && wakeLock.released && wakeBtn) { wakeBtn.disabled = false; wakeBtn.textContent = S.keep_screen_on; }
  }
  document.addEventListener("visibilitychange", resume);
  window.addEventListener("pageshow", resume);
  window.addEventListener("online", resume);
  window.addEventListener("focus", resume);
  setInterval(function () { if (document.visibilityState === "visible" && Date.now() - lastMessageAt > 35000) connect(); }, 10000);

  if (cfg.needsAltcha) solveAltcha();
  var form = document.getElementById("join");
  if (form) form.addEventListener("submit", unlockAudio);
  if (cfg.slug) connect();
})();
