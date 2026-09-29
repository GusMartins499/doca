# PrimoDock Linux

A native dock for Linux with per-environment profiles and live widgets.
No Electron, no webview.

Port of [PrimoDock](https://dock.oprimo.dev) (macOS) to Linux.

## Status

**Phase 0 — spine.** A GTK3 bar anchored with `_NET_WM_STRUT_PARTIAL`, a Rust
daemon listing windows over EWMH, and the two talking over D-Bus. Nothing is
drawn in the bar yet on purpose: this phase exists to prove the hard part
holds before anything is built on top of it.

See [PLANO.md](PLANO.md) for the full technical plan.

## Target

| | |
|---|---|
| Session | X11 (Wayland requires a different shell layer — see plan §3) |
| Desktop | GNOME Shell 42+ |
| Toolkit | GTK3 (deliberately not GTK4 — see plan §5) |

## Layout

```
crates/primodock-ipc     shared D-Bus contract
crates/primodockd        the daemon: windows, workspaces, state
crates/primodock-shell   the bar: draws, anchors, never talks X11 policy
```

The D-Bus boundary between the two is load-bearing: when the X11 session goes
away, the shell is replaced and the daemon survives intact.

## Build

```bash
sudo apt-get install -y pkg-config libgtk-3-dev
cargo build --release
```

## Run

```bash
cargo run --bin primodockd      # terminal 1
cargo run --bin primodock-shell # terminal 2
```
