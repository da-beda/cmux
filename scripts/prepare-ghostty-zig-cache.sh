#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
GHOSTTY_DIR="$ROOT_DIR/ghostty"
URL_FILE="$GHOSTTY_DIR/build.zig.zon.txt"

if ! command -v zig >/dev/null 2>&1; then
  echo "zig is required" >&2
  exit 1
fi

if [[ ! -f "$URL_FILE" ]]; then
  echo "missing $URL_FILE" >&2
  exit 1
fi

CACHE_DIR="${1:-${CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR:-$ROOT_DIR/.cache/ghostty-zig}}"
RETRIES="${CMUX_GHOSTTY_ZIG_FETCH_RETRIES:-3}"

mkdir -p "$CACHE_DIR"

fetch_url() {
  local url="$1"
  local attempt=1
  while true; do
    if ZIG_GLOBAL_CACHE_DIR="$CACHE_DIR" zig fetch "$url" >/dev/null 2>&1; then
      return 0
    fi
    if [[ "$attempt" -ge "$RETRIES" ]]; then
      echo "failed after $attempt attempts: $url" >&2
      return 1
    fi
    sleep "$attempt"
    attempt=$((attempt + 1))
  done
}

while IFS= read -r url; do
  [[ -n "$url" ]] || continue
  echo "fetching $url"
  fetch_url "$url"
done < "$URL_FILE"

cat <<EOF
Ghostty Zig cache is ready.

Use it for linked builds with:
  export CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR="$CACHE_DIR"
  cargo check --features cmux/link-ghostty

Or pass the exact offline package dir:
  export CMUX_GHOSTTY_ZIG_SYSTEM_DIR="$CACHE_DIR/p"
EOF
