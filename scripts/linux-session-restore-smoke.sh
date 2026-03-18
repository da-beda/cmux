#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LINUX_DIR="$REPO_ROOT/linux"
TARGET_DIR="$LINUX_DIR/target/debug"
RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/cmux-linux-restore.XXXXXX")"
STATE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/cmux-linux-state.XXXXXX")"
SOCKET_PATH="$RUNTIME_DIR/cmux.sock"
APP_LOG="$RUNTIME_DIR/cmux-app.log"
DISPLAY_ID="${CMUX_SMOKE_DISPLAY:-:99}"
APP_PID=""
XVFB_PID=""
SESSION_PATH="$STATE_DIR/cmux/session-v1.json"

wait_for_socket() {
  for _ in $(seq 1 80); do
    if [ -S "$SOCKET_PATH" ]; then
      return 0
    fi
    sleep 0.25
  done

  echo "cmux restore smoke test timed out waiting for socket at $SOCKET_PATH" >&2
  exit 1
}

stop_app() {
  if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
  fi
  APP_PID=""
}

wait_for_session_snapshot() {
  for _ in $(seq 1 80); do
    if [ -s "$SESSION_PATH" ]; then
      return 0
    fi
    sleep 0.25
  done

  echo "cmux restore smoke test timed out waiting for session snapshot at $SESSION_PATH" >&2
  exit 1
}

cleanup() {
  stop_app
  if [ -n "$XVFB_PID" ] && kill -0 "$XVFB_PID" 2>/dev/null; then
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
  fi
  rm -rf "$RUNTIME_DIR" "$STATE_DIR"
}

trap cleanup EXIT

if [ -z "${DISPLAY:-}" ]; then
  if command -v Xvfb >/dev/null 2>&1; then
    Xvfb "$DISPLAY_ID" -screen 0 1280x800x24 >/dev/null 2>&1 &
    XVFB_PID=$!
    export DISPLAY="$DISPLAY_ID"
    sleep 1
  else
    echo "linux restore smoke test requires DISPLAY or Xvfb" >&2
    exit 1
  fi
fi

mkdir -p "$RUNTIME_DIR" "$STATE_DIR"
chmod 700 "$RUNTIME_DIR" "$STATE_DIR"
export XDG_RUNTIME_DIR="$RUNTIME_DIR"
export XDG_STATE_HOME="$STATE_DIR"
export GDK_BACKEND="${GDK_BACKEND:-x11}"
export LD_LIBRARY_PATH="$TARGET_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export CMUX_SOCKET_MODE="${CMUX_SOCKET_MODE:-localUser}"

start_app() {
  "$TARGET_DIR/cmux-app" >>"$APP_LOG" 2>&1 &
  APP_PID=$!
  wait_for_socket
}

start_app

PERSISTED_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace new --title "Persisted" --directory "$REPO_ROOT")"
PERSISTED_WORKSPACE_ID="$(echo "$PERSISTED_JSON" | jq -r '.result.workspace_id')"
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" pane new --orientation vertical >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace rename --title "Persisted Renamed" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace pin >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-git-branch --branch persisted-main >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-shell-state --state running --label "restore" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-pr --number 828 --url https://example.com/pr/828 --title "Persisted PR" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace report-meta --key task --label Task --value persisted >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace new --title "Other" >/dev/null
"$TARGET_DIR/cmux" --socket "$SOCKET_PATH" workspace select --workspace "$PERSISTED_WORKSPACE_ID" >/dev/null

PRE_RESTART_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$PRE_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].title == "Persisted Renamed"
' >/dev/null
echo "$PRE_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].pinned == true
' >/dev/null
echo "$PRE_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].panel_count >= 2
' >/dev/null
echo "$PRE_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].git_branch.branch == "persisted-main"
' >/dev/null
echo "$PRE_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].pr.number == 828
' >/dev/null

wait_for_session_snapshot
stop_app
rm -f "$SOCKET_PATH"

start_app

POST_RESTART_JSON="$("$TARGET_DIR/cmux" --socket "$SOCKET_PATH" --json workspace list)"
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces | length >= 2
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].title == "Persisted Renamed"
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].pinned == true
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].panel_count >= 2
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].git_branch.branch == "persisted-main"
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].shell_state.state == "running"
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].pr.number == 828
' >/dev/null
echo "$POST_RESTART_JSON" | jq -e '
  .result.workspaces
  | map(select(.selected == true))[0].meta_items[0].key == "task"
' >/dev/null
