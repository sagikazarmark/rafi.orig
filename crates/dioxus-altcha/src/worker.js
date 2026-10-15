// The ALTCHA v2 solver: `altcha::solve_challenge` (counter mode Uint32) on
// WebCrypto. In a Web Worker it answers messages; on a page or under node it
// only defines `altchaSolve`.
//
// ES2017, a classic script: no modules, `?.`, `??`, BigInt or class fields.
//
// Worker protocol (part of dioxus-altcha's public contract), JSON strings
// both ways:
//   in:  {"challenge": <the challenge as sent>, "start": i, "step": n}
//   out: {"counter": c, "derivedKey": "<hex>", "time": ms} or {"error": "<message>"}
"use strict";

var altchaSolve = (function () {
  var HASHES = { "SHA-256": 1, "SHA-384": 1, "SHA-512": 1 };
  var MAX_COUNTER = 0xffffffff;

  function hexToBytes(hex, what) {
    if (typeof hex !== "string" || hex.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(hex)) {
      throw new Error("invalid " + what + ": not hex");
    }
    var bytes = new Uint8Array(hex.length / 2);
    for (var i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.substr(i * 2, 2), 16);
    return bytes;
  }

  function toHex(bytes) {
    var s = "";
    for (var i = 0; i < bytes.length; i++) s += (bytes[i] < 16 ? "0" : "") + bytes[i].toString(16);
    return s;
  }

  // "PBKDF2/SHA-256" -> { pbkdf2: true, hash: "SHA-256" }; "SHA-512" -> { pbkdf2: false, ... }.
  function algorithm(name) {
    var upper = String(name).toUpperCase();
    var pbkdf2 = upper.indexOf("PBKDF2/") === 0;
    var hash = pbkdf2 ? upper.slice(7) : upper;
    if (!HASHES[hash]) throw new Error("unsupported algorithm: " + name);
    return { pbkdf2: pbkdf2, hash: hash };
  }

  async function derive(alg, p, salt, password) {
    // `max(1, cost)`: pbkdf2 0.12 runs 0 rounds as 1, WebCrypto rejects 0.
    var rounds = Math.max(1, p.cost);
    if (alg.pbkdf2) {
      var key = await crypto.subtle.importKey("raw", password, "PBKDF2", false, ["deriveBits"]);
      var bits = await crypto.subtle.deriveBits(
        { name: "PBKDF2", hash: alg.hash, salt: salt, iterations: rounds },
        key,
        p.keyLength * 8
      );
      return new Uint8Array(bits);
    }
    // Iterated SHA: round 1 over salt || password, then over the full digest.
    var data = new Uint8Array(salt.length + password.length);
    data.set(salt);
    data.set(password, salt.length);
    for (var i = 0; i < rounds; i++) data = new Uint8Array(await crypto.subtle.digest(alg.hash, data));
    return data.slice(0, p.keyLength);
  }

  function pause() {
    return new Promise(function (resolve) { setTimeout(resolve, 0); });
  }

  // Tries counters start, start + step, ... until the derived key starts with
  // `keyPrefix`. `ctl` (main thread only): {yieldMs, cancelled}.
  return async function (challenge, start, step, ctl) {
    var p = challenge && challenge.parameters;
    if (!p) throw new Error("invalid challenge: no parameters");
    var alg = algorithm(p.algorithm);
    var nonce = hexToBytes(p.nonce, "nonce");
    var salt = hexToBytes(p.salt, "salt");
    // An even-length prefix compares as bytes, an odd one as a lowercase hex
    // string: both are a prefix of the lowercase hex key.
    var prefix = String(p.keyPrefix).toLowerCase();
    if (!/^[0-9a-f]*$/.test(prefix)) throw new Error("invalid keyPrefix: not hex");
    if (!(step >= 1)) throw new Error("invalid step");

    var password = new Uint8Array(nonce.length + 4);
    password.set(nonce);
    var view = new DataView(password.buffer);
    var t0 = performance.now();
    var yielded = t0;
    for (var counter = start; counter <= MAX_COUNTER; counter += step) {
      if (ctl) {
        if (ctl.cancelled) throw new Error("cancelled");
        if (ctl.yieldMs && performance.now() - yielded >= ctl.yieldMs) {
          await pause();
          yielded = performance.now();
        }
      }
      view.setUint32(nonce.length, counter); // big-endian
      var key = toHex(await derive(alg, p, salt, password));
      if (key.slice(0, prefix.length) === prefix) {
        return { counter: counter, derivedKey: key, time: performance.now() - t0 };
      }
    }
    throw new Error("no solution up to counter " + MAX_COUNTER);
  };
})();

if (typeof WorkerGlobalScope !== "undefined" && self instanceof WorkerGlobalScope) {
  self.onmessage = function (event) {
    var reply = function (value) { self.postMessage(JSON.stringify(value)); };
    var fail = function (err) { reply({ error: String((err && err.message) || err) }); };
    try {
      var m = JSON.parse(event.data);
      altchaSolve(m.challenge, m.start, m.step).then(reply, fail);
    } catch (err) {
      fail(err);
    }
  };
}
