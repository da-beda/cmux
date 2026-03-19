# cmux-linux

Rust + GTK4/libadwaita port of cmux (terminal multiplexer for AI coding agents).

## Build

```bash
cargo check          # Type check
cargo test           # Run tests
cargo build          # Debug build
cargo build --release # Release build
```

Recommended Linux Docker path:

```bash
./scripts/linux-linked-check.sh
./scripts/linux-docker.sh 'cd linux && cargo check --tests'
./scripts/linux-docker.sh 'cd linux && cargo check --tests --features cmux/link-ghostty'
./scripts/linux-docker.sh './scripts/linux-ci-smoke.sh'
./scripts/linux-docker.sh './scripts/linux-session-restore-smoke.sh'
```

`./scripts/linux-linked-check.sh` is the shortest end-to-end linked Ghostty
verification path.

`./scripts/linux-docker.sh` is the general-purpose helper. It mounts persistent
Cargo and Ghostty Zig caches under `.cache/`, installs the GTK/linker dependencies,
and seeds the Ghostty Zig cache before running your command. It is the recommended
way to avoid repeated live fetches from `deps.files.ghostty.org` during linked
Ghostty builds.

## Architecture

- `ghostty-sys/` — Raw FFI bindings to libghostty C API (`ghostty.h`)
- `ghostty-gtk/` — Safe Rust wrapper: GhosttyApp, GhosttyGlSurface, key mapping
- `cmux/` — Main application (GTK4/libadwaita)
  - `model/` — TabManager, Workspace, Panel, LayoutNode
  - `ui/` — Window, Sidebar, SplitView, TerminalPanel
  - `socket/` — Unix socket server, v2 JSON protocol, auth
  - `notifications.rs` — Notification store + desktop notifications
- `cmux-cli/` — CLI client (`cmux workspace list`, `cmux surface send-text`, etc.)

The Linux MVP is terminal-only. Browser-panel placeholders are intentionally absent.
The Linux app and CLI are local-only: they do not include remote telemetry, analytics,
or crash-reporting SDKs. The `report_*` and `clear_*` socket commands update local workspace
metadata only.

## Architecture Review

**Read `docs/architecture-review.md` and `docs/ubuntu-mvp-spec.md` before making structural changes.**
They document the current Ubuntu MVP tradeoffs, Ghostty integration constraints, and review scope.

## Ghostty Integration

The `link-ghostty` feature enables actual FFI linking to libghostty.
Without it (default), the crates compile in stub mode for development.

To build with ghostty:
1. Initialize the ghostty submodule
2. Build with `cargo build --features cmux/link-ghostty`

If linked Ghostty builds are flaky because Zig keeps fetching from
`deps.files.ghostty.org`, use Ghostty's upstream offline-cache flow once and
then point `ghostty-sys` at that cache:

```bash
./scripts/prepare-ghostty-zig-cache.sh
export CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR="$PWD/.cache/ghostty-zig"
cargo check --features cmux/link-ghostty
```

`linux/ghostty-sys/build.rs` now respects:
- `CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR`
- `CMUX_GHOSTTY_ZIG_SYSTEM_DIR`

`CMUX_GHOSTTY_ZIG_GLOBAL_CACHE_DIR` is the normal cmux setting: it reuses a
prefetched Zig cache without switching Ghostty into packager-style system
library linking.

`CMUX_GHOSTTY_ZIG_SYSTEM_DIR` is an explicit advanced override for Ghostty's
`zig build --system ...` mode. That is useful for distro-style packaging, but it
can change dependency linking behavior and is not the recommended default for
cmux's embedded `libghostty` build.

## Socket Protocol

Unix socket at `$XDG_RUNTIME_DIR/cmux.sock` (falls back to `/tmp/cmux-$UID.sock`).
Line-delimited JSON v2 protocol. Compatible with macOS cmux socket API.

## Local Verification

To verify the Linux port stays free of remote telemetry behavior, run:

```bash
../scripts/linux-no-remote-telemetry.sh
```

That script checks both the Linux dependency graph and a syscall-traced local app session.

## Reference

- macOS cmux source: root of this repository (Swift/AppKit)
- ghostty C API: `ghostty.h` in the repo root
- Ghostty GTK runtime: `ghostty/src/apprt/gtk/` (reference for GL/input integration)
