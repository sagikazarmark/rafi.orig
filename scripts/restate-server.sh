#!/usr/bin/env bash
# Downloads the restate-server binary the end-to-end tests spawn, and prints its path:
#
#   export RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)
#   cargo test --workspace --all-features -- --include-ignored
set -euo pipefail

VERSION="${RESTATE_VERSION:-1.7.13}"
DIR="${RESTATE_SERVER_DIR:-$(cd "$(dirname "$0")/.." && pwd)/target/restate-server-$VERSION}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) TARGET=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) TARGET=aarch64-unknown-linux-musl ;;
  Darwin-arm64) TARGET=aarch64-apple-darwin ;;
  Darwin-x86_64) TARGET=x86_64-apple-darwin ;;
  *) echo "unsupported platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

BIN="$DIR/restate-server-$TARGET/restate-server"
if [ ! -x "$BIN" ]; then
  mkdir -p "$DIR"
  ARCHIVE="restate-server-$TARGET.tar.xz"
  URL="https://github.com/restatedev/restate/releases/download/v$VERSION/$ARCHIVE"
  curl -fsSL -o "$DIR/$ARCHIVE" "$URL"
  curl -fsSL -o "$DIR/$ARCHIVE.sha256" "$URL.sha256"
  (cd "$DIR" && { sha256sum -c "$ARCHIVE.sha256" 2>/dev/null || shasum -a 256 -c "$ARCHIVE.sha256"; } >&2)
  tar -xJf "$DIR/$ARCHIVE" -C "$DIR"
  rm "$DIR/$ARCHIVE" "$DIR/$ARCHIVE.sha256"
fi
echo "$BIN"
