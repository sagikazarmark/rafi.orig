#!/usr/bin/env bash
# Compiles crates/web/tailwind.css -> crates/web/assets/tailwind.css with the same
# standalone Tailwind version dx uses. `dx serve`/`dx bundle` do this
# automatically; run it before plain `cargo build`/`cargo test` on a fresh checkout.
set -euo pipefail
VERSION="${TAILWIND_VERSION:-v4.1.5}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ASSET=tailwindcss-macos-arm64 ;;
  Darwin-x86_64) ASSET=tailwindcss-macos-x64 ;;
  Linux-aarch64) ASSET=tailwindcss-linux-arm64 ;;
  Linux-x86_64) ASSET=tailwindcss-linux-x64 ;;
  *) echo "unsupported platform" >&2; exit 1 ;;
esac
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ROOT}/target/tools/tailwindcss-${VERSION}"
if [ ! -x "$BIN" ]; then
  mkdir -p "$(dirname "$BIN")"
  curl -sSfL -o "$BIN" "https://github.com/tailwindlabs/tailwindcss/releases/download/${VERSION}/${ASSET}"
  chmod +x "$BIN"
fi
cd "${ROOT}/crates/web"
"$BIN" -i tailwind.css -o assets/tailwind.css --minify "$@"
