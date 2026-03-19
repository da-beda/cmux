#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LINUX_DIR="$REPO_ROOT/linux"
TARGET_DIR="$LINUX_DIR/target/debug"
RUNTIME_DIR="${CMUX_SMOKE_RUNTIME_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/cmux-linux-smoke.XXXXXX")}"
SOCKET_PATH="$RUNTIME_DIR/cmux.sock"
APP_LOG="${CMUX_SMOKE_LOG:-$RUNTIME_DIR/cmux-app.log}"
APP_PID=""
XVFB_PID=""
DISPLAY_ID="${CMUX_SMOKE_DISPLAY:-:99}"

wait_for_socket() {
  for _ in $(seq 1 80); do
    if [ -S "$SOCKET_PATH" ]; then
      return 0
    fi
    sleep 0.25
  done

  echo "cmux smoke test timed out waiting for socket at $SOCKET_PATH" >&2
  exit 1
}

cleanup() {
  if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
  fi

  if [ -n "$XVFB_PID" ] && kill -0 "$XVFB_PID" 2>/dev/null; then
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
  fi

  if [ "${CMUX_SMOKE_KEEP_RUNTIME:-0}" != "1" ]; then
    rm -rf "$RUNTIME_DIR"
  fi
}

trap cleanup EXIT

if [ -z "${DISPLAY:-}" ]; then
  if command -v Xvfb >/dev/null 2>&1; then
    Xvfb "$DISPLAY_ID" -screen 0 1280x800x24 >/dev/null 2>&1 &
    XVFB_PID=$!
    export DISPLAY="$DISPLAY_ID"
    sleep 1
  else
    echo "linux smoke test requires DISPLAY or Xvfb" >&2
    exit 1
  fi
fi

mkdir -p "$RUNTIME_DIR"
chmod 700 "$RUNTIME_DIR"
export XDG_RUNTIME_DIR="$RUNTIME_DIR"
export GDK_BACKEND="${GDK_BACKEND:-x11}"
export LD_LIBRARY_PATH="$TARGET_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# The smoke test drives the app from a sibling CLI process, so descendant-only
# CmuxOnly auth would reject the client even when the app is healthy.
export CMUX_SOCKET_MODE="${CMUX_SOCKET_MODE:-localUser}"

"$TARGET_DIR/cmux-app" >"$APP_LOG" 2>&1 &
APP_PID=$!

wait_for_socket

"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" ping >/dev/null

INITIAL_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$INITIAL_JSON" | jq -e '.result.workspaces | length >= 1' >/dev/null

"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace new --title "Smoke" --directory "$REPO_ROOT" >/dev/null

AFTER_NEW_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$AFTER_NEW_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))
  | length == 1
' >/dev/null
echo "$AFTER_NEW_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].title == "Smoke"
' >/dev/null

PANE_NEW_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json pane new --orientation vertical)"
SECOND_PANE_ID="$(echo "$PANE_NEW_JSON" | jq -r '.result.pane_id')"
SECOND_SURFACE_ID="$(echo "$PANE_NEW_JSON" | jq -r '.result.surface')"
echo "$PANE_NEW_JSON" | jq -e '.result.pane_id | type == "string"' >/dev/null
echo "$PANE_NEW_JSON" | jq -e '.result.surface | type == "string"' >/dev/null

PANE_FOCUS_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json pane focus --pane "$SECOND_PANE_ID")"
echo "$PANE_FOCUS_JSON" | jq -e '
  .result.focused == true and .result.pane_id == "'"$SECOND_PANE_ID"'"
' >/dev/null

PANE_RESIZE_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json pane resize --pane "$SECOND_PANE_ID" --direction down --step 0.1)"
echo "$PANE_RESIZE_JSON" | jq -e '
  .result.resized == true and .result.pane_id == "'"$SECOND_PANE_ID"'" and .result.direction == "down"
' >/dev/null

SURFACE_NEXT_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json surface next --pane "$SECOND_PANE_ID")"
echo "$SURFACE_NEXT_JSON" | jq -e '
  .result.focused == true and .result.pane_id == "'"$SECOND_PANE_ID"'" and .result.surface == "'"$SECOND_SURFACE_ID"'"
' >/dev/null

"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-git-branch --branch smoke-main --dirty >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-pwd --path "$REPO_ROOT/linux" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-shell-state --state running --label "cargo test" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-ports --port 3000 --port 8080 >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-tty --tty-name pts/42 >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-pr --number 828 --url https://example.com/pr/828 --title "Smoke PR" --branch linux-port --checks pending >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-meta --key task --label Task --value review >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-meta-block --key notes --title Notes --content "line one" >/dev/null

AFTER_SPLIT_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$AFTER_SPLIT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].panel_count >= 2
' >/dev/null
echo "$AFTER_SPLIT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].git_branch.branch == "smoke-main"
' >/dev/null
echo "$AFTER_SPLIT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].shell_state.state == "running"
' >/dev/null
echo "$AFTER_SPLIT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].listening_ports == [3000,8080]
' >/dev/null
echo "$AFTER_SPLIT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].pr.number == 828
' >/dev/null
echo "$AFTER_SPLIT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].meta_items[0].key == "task"
' >/dev/null

"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace clear-pwd >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace clear-shell-state >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace clear-tty >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace clear-meta --key task >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace clear-meta-block --key notes >/dev/null

AFTER_CLEAR_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$AFTER_CLEAR_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].directory == "'"$REPO_ROOT"'"
' >/dev/null
echo "$AFTER_CLEAR_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].shell_state == null
' >/dev/null
echo "$AFTER_CLEAR_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].tty_name == null
' >/dev/null
echo "$AFTER_CLEAR_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].meta_items == []
' >/dev/null
echo "$AFTER_CLEAR_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].meta_blocks == []
' >/dev/null

PANE_CLOSE_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json pane close --pane "$SECOND_PANE_ID")"
echo "$PANE_CLOSE_JSON" | jq -e '
  .result.closed == true and .result.pane_id == "'"$SECOND_PANE_ID"'" and (.result.removed_surfaces | length) >= 1
' >/dev/null

AFTER_CLOSE_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$AFTER_CLOSE_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].panel_count == 1
' >/dev/null

"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace rename --title "Smoke Renamed" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace pin >/dev/null

AFTER_PIN_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$AFTER_PIN_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].title == "Smoke Renamed"
' >/dev/null
echo "$AFTER_PIN_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].pinned == true
' >/dev/null

"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace previous --wrap false >/dev/null

AFTER_SELECT_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$AFTER_SELECT_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))
  | length == 1
' >/dev/null
