#!/usr/bin/env bash
# Builds the participant page's WebAssembly (apps/participant) with dx, into
# apps/participant/dist/public, which the server serves at /participant/:
#
#   assets/rafi-participant-dxh<hash>.js       the glue, with the snippets bundled in
#   assets/rafi-participant_bg-dxh<hash>.wasm  the module, which the glue names
#   fallback.js                                the JavaScript fallback, as it is
#   index.html                                 the page's loader, with the glue's name in it
#
# The names carry a hash of the content, so phones keep the assets for good
# (Cache-Control: immutable), and rafi-server compiles index.html in, so a
# server is built against one participant build and links that one. The
# output has this one place: scripts/app-assets.sh and scripts/workers-check.sh
# copy from it. (/participant/altcha-worker.js is a rafi-server route.)
#
#   ./scripts/participant.sh
#
# Needs dx 0.7.10 with wasm-bindgen-cli at the version in Cargo.lock and
# wasm-opt (binaryen), as the devenv shell has them.
set -euo pipefail
shopt -s nullglob

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/participant/dist"
# dx writes `public/` under the directory it is given: the build is staged
# here, checked, and moved into place only when it differs from the one there,
# so a build of the same sources (the `just` recipes run this first) doesn't
# move index.html and make the server compile again for nothing. `devenv up`
# watches $OUT for a new index.html, which this never deletes.
STAGE="$OUT/.build"
PUBLIC="$STAGE/public"

want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$ROOT/Cargo.lock")
have=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen ${have:-missing}, Cargo.lock wants $want:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

# The profile (opt-level z) is the workspace's wasm-release; wasm-opt's flags
# are in apps/participant/Dioxus.toml. dx's release defaults would keep the
# DWARF sections in and append its own loader to index.html. dx leaves
# earlier bundles (other hashes) in its output: start empty.
rm -rf "${STAGE:?}" "${ROOT:?}/target/dx/rafi-participant/release/web/public"
(cd "$ROOT/apps/participant" && dx bundle --platform web --release --locked \
  --debug-symbols false --inject-loading-scripts false --out-dir "$STAGE")

js=("$PUBLIC"/assets/*.js)
wasm=("$PUBLIC"/assets/*.wasm)
if [ "${#js[@]}" -ne 1 ] || [ "${#wasm[@]}" -ne 1 ] || [ ! -f "$PUBLIC/index.html" ] || [ ! -f "$PUBLIC/fallback.js" ]; then
  echo "participant.sh: not the expected layout in $PUBLIC:" >&2
  find "$PUBLIC" >&2
  exit 1
fi
js_name="${js[0]##*/}"
# The loader links the glue dx just named; a placeholder left in it would be
# a page that loads nothing.
if ! grep -q "assets/$js_name" "$PUBLIC/index.html" || grep -Eq '\{[a-z_]+\}' "$PUBLIC/index.html"; then
  echo "participant.sh: index.html doesn't link $js_name:" >&2
  cat "$PUBLIC/index.html" >&2
  exit 1
fi

# gzip next to dx's brotli, for the native server (tower-http serves `x.br`
# or `x.gz` for `x` when the browser accepts it; Cloudflare compresses at the
# edge, and app-assets.sh drops these). The fallback, which dx copies as it
# is, gets brotli here too when the tool is there.
for f in "${js[0]}" "${wasm[0]}" "$PUBLIC/fallback.js"; do
  gzip -9 -k -n -f "$f"
  if [ ! -f "$f.br" ] && command -v brotli >/dev/null 2>&1; then brotli -9 -k -f "$f"; fi
done

wasm_gz=$(wc -c <"${wasm[0]}.gz" | tr -d ' ')
js_gz=$(wc -c <"${js[0]}.gz" | tr -d ' ')
total=$((wasm_gz + js_gz))
echo "rafi-participant: wasm ${wasm_gz} B + js ${js_gz} B = ${total} B gzip -9"
# The budget: 280 KB compressed, so the page stays usable on slow venue
# networks. dx ships the unoptimised module when wasm-opt fails; this catches it.
if [ "$total" -gt 286720 ]; then
  echo "over the 280 KB budget" >&2
  exit 1
fi

if [ -d "$OUT/public" ] && diff -rq "$PUBLIC" "$OUT/public" >/dev/null 2>&1; then
  echo "rafi-participant: unchanged ($js_name)"
else
  rm -rf "${OUT:?}/public"
  mv "$PUBLIC" "$OUT/public"
  echo "rafi-participant: $OUT/public ($js_name)"
fi
rm -rf "${STAGE:?}"
