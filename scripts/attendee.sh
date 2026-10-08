#!/usr/bin/env bash
# Builds the attendee page's WebAssembly (crates/attendee) into a directory
# the server serves at /attendee/: rafi_attendee.js and rafi_attendee_bg.wasm.
#
#   ./scripts/attendee.sh [out-dir]   # default: crates/attendee/dist
#
# Needs wasm-bindgen-cli at the version in Cargo.lock, and wasm-opt (binaryen).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/crates/attendee/dist}"

want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$ROOT/Cargo.lock")
have=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen ${have:-missing}, Cargo.lock wants $want:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

cargo build -p rafi-attendee --target wasm32-unknown-unknown --release --locked
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  "$ROOT/target/wasm32-unknown-unknown/release/rafi_attendee.wasm"

# Only the features rustc emits by default, so no newer ones appear (older
# Safari can't compile them).
wasm-opt -Oz --strip-debug --strip-producers \
  --enable-bulk-memory --enable-mutable-globals --enable-nontrapping-float-to-int \
  --enable-sign-ext --enable-reference-types --enable-multivalue \
  "$OUT/rafi_attendee_bg.wasm" -o "$OUT/rafi_attendee_bg.wasm"

wasm=$(gzip -9c "$OUT/rafi_attendee_bg.wasm" | wc -c | tr -d ' ')
js=$(gzip -9c "$OUT/rafi_attendee.js" | wc -c | tr -d ' ')
total=$((wasm + js))
echo "rafi-attendee: wasm ${wasm} B + js ${js} B = ${total} B gzip -9"
# The budget (docs/workers-plan.md §2.5): 280 KB compressed.
if [ "$total" -gt 286720 ]; then
  echo "over the 280 KB budget" >&2
  exit 1
fi
