#!/usr/bin/env bash
# Builds the participant page's WebAssembly (apps/participant) into a directory
# the server serves at /participant/: rafi_participant.js, rafi_participant_bg.wasm
# and fallback.js. (/participant/altcha-worker.js is a rafi-server route.)
#
#   ./scripts/participant.sh [out-dir]   # default: apps/participant/dist
#
# Needs wasm-bindgen-cli at the version in Cargo.lock, and wasm-opt (binaryen).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/apps/participant/dist}"

want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$ROOT/Cargo.lock")
have=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen ${have:-missing}, Cargo.lock wants $want:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

cargo build -p rafi-participant --target wasm32-unknown-unknown --release --locked
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  "$ROOT/target/wasm32-unknown-unknown/release/rafi_participant.wasm"

# Only the features rustc emits by default, so no newer ones appear (older
# Safari can't compile them).
wasm-opt -Oz --strip-debug --strip-producers \
  --enable-bulk-memory --enable-mutable-globals --enable-nontrapping-float-to-int \
  --enable-sign-ext --enable-reference-types --enable-multivalue \
  "$OUT/rafi_participant_bg.wasm" -o "$OUT/rafi_participant_bg.wasm"

# The JavaScript fallback, loaded by the page when the WebAssembly can't run.
# The proof-of-work solver both start their workers from, altcha-worker.js, is
# served by rafi-server (dioxus-altcha's, as built in), not from here. Remove
# a copy left by an older build: Workers Assets would serve it first.
cp "$ROOT/apps/participant/fallback.js" "$OUT/fallback.js"
rm -f "$OUT/altcha-worker.js"

# Precompressed variants next to each file, for the native server (tower-http
# serves `x.br` or `x.gz` for `x` when the browser accepts it; Cloudflare
# compresses at the edge and ignores them). Brotli when the tool is there.
for f in "$OUT/rafi_participant.js" "$OUT/rafi_participant_bg.wasm" "$OUT/fallback.js"; do
  rm -f "$f.gz" "$f.br"
  gzip -9 -k -n "$f"
  if command -v brotli >/dev/null 2>&1; then brotli -9 -k -f "$f"; fi
done

wasm=$(gzip -9c "$OUT/rafi_participant_bg.wasm" | wc -c | tr -d ' ')
js=$(gzip -9c "$OUT/rafi_participant.js" | wc -c | tr -d ' ')
total=$((wasm + js))
echo "rafi-participant: wasm ${wasm} B + js ${js} B = ${total} B gzip -9"
# The budget: 280 KB compressed, so the page stays usable on slow venue networks.
if [ "$total" -gt 286720 ]; then
  echo "over the 280 KB budget" >&2
  exit 1
fi
