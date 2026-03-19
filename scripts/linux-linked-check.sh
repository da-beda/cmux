#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/linux-docker.sh" \
  'cd linux && cargo check --tests --features cmux/link-ghostty && cargo test --workspace --no-run --features cmux/link-ghostty && cd .. && ./scripts/linux-ci-smoke.sh && ./scripts/linux-session-restore-smoke.sh'
