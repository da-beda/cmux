#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE="${CMUX_LINUX_DOCKER_IMAGE:-rust:trixie}"
ZIG_CACHE_DIR="${CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR:-$REPO_ROOT/.cache/ghostty-zig}"
CARGO_CACHE_DIR="${CMUX_LINUX_CARGO_CACHE_DIR:-$REPO_ROOT/.cache/cargo}"
APT_PACKAGES=(
  pkg-config
  libgtk-4-dev
  libadwaita-1-dev
  curl
  xz-utils
  ncurses-bin
  ca-certificates
  libonig-dev
  xvfb
  jq
  tmux
)

mkdir -p "$ZIG_CACHE_DIR" "$CARGO_CACHE_DIR"

ZIG_VERSION="$(
  sed -n -E 's/^\s*\.?minimum_zig_version\s*=\s*"([^"]+)".*/\1/p' \
    "$REPO_ROOT/ghostty/build.zig.zon" | head -n1
)"
if [[ -z "$ZIG_VERSION" ]]; then
  echo "failed to parse Ghostty minimum Zig version" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64) ZIG_ARCH="x86_64" ;;
  aarch64|arm64) ZIG_ARCH="aarch64" ;;
  *)
    echo "unsupported host architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

if [[ $# -eq 0 ]]; then
  USER_COMMAND="bash"
else
  USER_COMMAND="$*"
fi

TTY_FLAGS=()
if [[ -t 0 && -t 1 ]]; then
  TTY_FLAGS=(-it)
fi

docker run --rm "${TTY_FLAGS[@]}" \
  -v "$REPO_ROOT:/work" \
  -v "$ZIG_CACHE_DIR:/cache/zig" \
  -v "$CARGO_CACHE_DIR:/cargo" \
  -w /work \
  "$IMAGE" bash -lc "
set -euo pipefail
export CARGO_HOME=/cargo
export PATH=/usr/local/cargo/bin:\$PATH
apt-get update >/dev/null
DEBIAN_FRONTEND=noninteractive apt-get install -y ${APT_PACKAGES[*]} >/dev/null
cd /tmp
if [ ! -x /tmp/zig-${ZIG_ARCH}-linux-${ZIG_VERSION}/zig ]; then
  curl -L -o zig.tar.xz https://ziglang.org/download/${ZIG_VERSION}/zig-${ZIG_ARCH}-linux-${ZIG_VERSION}.tar.xz >/dev/null 2>&1
  tar -xf zig.tar.xz
fi
export PATH=/tmp/zig-${ZIG_ARCH}-linux-${ZIG_VERSION}:\$PATH
cd /work
./scripts/prepare-ghostty-zig-cache.sh /cache/zig >/dev/null
export CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR=/cache/zig
exec bash -c $(printf '%q' "$USER_COMMAND")
"
