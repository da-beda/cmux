#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LINUX_DIR="$REPO_ROOT/linux"
TARGET_DIR="$LINUX_DIR/target/debug"
RUNTIME_DIR="${CMUX_NO_TELEMETRY_RUNTIME_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/cmux-linux-no-telemetry.XXXXXX")}"
SOCKET_PATH="$RUNTIME_DIR/cmux.sock"
TRACE_LOG="${CMUX_NO_TELEMETRY_TRACE_LOG:-$RUNTIME_DIR/network.trace}"
APP_LOG="${CMUX_NO_TELEMETRY_APP_LOG:-$RUNTIME_DIR/cmux-app.log}"
DISPLAY_ID="${CMUX_NO_TELEMETRY_DISPLAY:-:99}"
APP_PID=""
XVFB_PID=""

stop_app() {
  if [ -z "$APP_PID" ]; then
    return 0
  fi

  if kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
    for _ in $(seq 1 20); do
      if ! kill -0 "$APP_PID" 2>/dev/null; then
        break
      fi
      sleep 0.1
    done
    if kill -0 "$APP_PID" 2>/dev/null; then
      kill -9 "$APP_PID" 2>/dev/null || true
    fi
  fi

  wait "$APP_PID" 2>/dev/null || true
  APP_PID=""
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

wait_for_socket() {
  for _ in $(seq 1 80); do
    if [ -S "$SOCKET_PATH" ]; then
      return 0
    fi
    sleep 0.25
  done

  echo "Timed out waiting for cmux socket at $SOCKET_PATH" >&2
  exit 1
}

cleanup() {
  stop_app

  if [ -n "$XVFB_PID" ] && kill -0 "$XVFB_PID" 2>/dev/null; then
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
  fi

  if [ "${CMUX_NO_TELEMETRY_KEEP_RUNTIME:-0}" != "1" ]; then
    rm -rf "$RUNTIME_DIR"
  fi
}

start_display_if_needed() {
  if [ -n "${DISPLAY:-}" ]; then
    return 0
  fi

  if command -v Xvfb >/dev/null 2>&1; then
    Xvfb "$DISPLAY_ID" -screen 0 1280x800x24 >/dev/null 2>&1 &
    XVFB_PID=$!
    export DISPLAY="$DISPLAY_ID"
    sleep 1
    return 0
  fi

  echo "No DISPLAY set and Xvfb is unavailable; cannot run traced GTK smoke test locally." >&2
  exit 1
}

check_dependency_graph() {
  local tree_output
  tree_output="$(
    cd "$LINUX_DIR"
    cargo tree --workspace --all-features --format '{p}'
  )"

  local denied
  denied="$(printf '%s\n' "$tree_output" | grep -Ei '^(sentry(|-.*)|posthog(|-.*)|opentelemetry(|-.*)|segment(|-.*)|mixpanel(|-.*)|amplitude(|-.*)|rollbar(|-.*)|bugsnag(|-.*)) v' || true)"
  if [ -n "$denied" ]; then
    echo "Detected forbidden telemetry-related crates in Linux dependency graph:" >&2
    printf '%s\n' "$denied" >&2
    exit 1
  fi
}

check_artifacts_exist() {
  if [ ! -x "$TARGET_DIR/cmux-app" ] || [ ! -x "$TARGET_DIR/cmux" ]; then
    echo "Expected built Linux binaries at $TARGET_DIR/cmux-app and $TARGET_DIR/cmux" >&2
    echo "Build them first, for example:" >&2
    echo "  cd linux && cargo build --features cmux/link-ghostty" >&2
    exit 1
  fi
}

run_traced_smoke() {
  mkdir -p "$RUNTIME_DIR"
  chmod 700 "$RUNTIME_DIR"

  export XDG_RUNTIME_DIR="$RUNTIME_DIR"
  export GDK_BACKEND="${GDK_BACKEND:-x11}"
  export NO_AT_BRIDGE=1
  export LD_LIBRARY_PATH="$TARGET_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  export CMUX_SOCKET_MODE="${CMUX_SOCKET_MODE:-localUser}"

  strace -f -qq -e trace=network -s 256 -o "$TRACE_LOG" \
    "$TARGET_DIR/cmux-app" >"$APP_LOG" 2>&1 &
  APP_PID=$!

  wait_for_socket

  "$TARGET_DIR/cmux" --socket "$SOCKET_PATH" ping >/dev/null

  local initial_json after_new_json after_split_json after_select_json
  initial_json="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
  printf '%s\n' "$initial_json" | jq -e '.result.workspaces | length >= 1' >/dev/null

  "$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace new --title "NoTelemetry" --directory "$REPO_ROOT" >/dev/null

  after_new_json="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
  printf '%s\n' "$after_new_json" | jq -e '
    .result.workspaces
    | map(select(.selected == true))
    | length == 1
  ' >/dev/null
  printf '%s\n' "$after_new_json" | jq -e '
    .result.workspaces
    | map(select(.selected == true))[0].title == "NoTelemetry"
  ' >/dev/null

  "$TARGET_DIR/cmux" --socket "$SOCKET_PATH" pane new --orientation vertical >/dev/null

  after_split_json="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
  printf '%s\n' "$after_split_json" | jq -e '
    .result.workspaces
    | map(select(.selected == true))[0].panel_count >= 2
  ' >/dev/null

  "$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace previous --wrap false >/dev/null

  after_select_json="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
  printf '%s\n' "$after_select_json" | jq -e '
    .result.workspaces
    | map(select(.selected == true))
    | length == 1
  ' >/dev/null

  sleep 0.5
  stop_app
}

assert_no_outbound_inet_connects() {
  local outbound
  outbound="$(grep -E 'connect\(.*sa_family=AF_INET6?' "$TRACE_LOG" || true)"
  if [ -n "$outbound" ]; then
    echo "Detected outbound AF_INET/AF_INET6 connect attempts during Linux smoke run:" >&2
    printf '%s\n' "$outbound" >&2
    echo "Trace log: $TRACE_LOG" >&2
    echo "App log: $APP_LOG" >&2
    exit 1
  fi
}

require_cmd cargo
require_cmd jq
require_cmd strace

check_dependency_graph
check_artifacts_exist
start_display_if_needed
run_traced_smoke
assert_no_outbound_inet_connects

echo "Linux no-remote-telemetry checks passed."
echo "Trace log: $TRACE_LOG"
echo "App log: $APP_LOG"
