# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`spectral-m2` is a terminal REPL for [Macaulay2](https://macaulay2.com) that adds tree-sitter syntax highlighting to a full-screen TUI. It does not reimplement M2 — it drives a real `M2` process and renders its output. `install.sh` also symlinks a short alias `sp2`.

The name is a triple play: the **spectrum** of light (the colors), the **spectrum of a ring** (`Spec`, M2's algebraic-geometry home turf), and **spectral flux** (the author's handle).

## Commands

```bash
cargo run                    # debug build + launch the REPL (needs M2 on PATH)
cargo test                   # unit tests (parser logic in src/repl.rs)
cargo clippy --all-targets -- -D warnings
cargo fmt --all
./install.sh                 # full pipeline: fmt-check → clippy → test → release build → install to ~/.local/bin/spectral-m2
```

`cargo run` uses the **debug** build straight from `target/`; the installed `spectral-m2` command is the **release** copy in `~/.local/bin`. They are separate — after editing, `cargo run` to test in place, or `./install.sh` to refresh the `spectral-m2` command.

`install.sh` is the gated pipeline: `cargo fmt --all --check`, `cargo clippy -- -D warnings`, and `cargo test` all run as hard gates before the release build and install — any failure aborts. Running the binary requires the `M2` executable on `PATH`; it spawns a real Macaulay2 process per session.

Tests live in the `#[cfg(test)] mod tests` block at the bottom of `src/repl.rs` and cover `clean_m2_output` (stripping `capture`'s session scaffolding). Run a single one with `cargo test <name>`, e.g. `cargo test preserves_2d_net_alignment`.

## Code style

- **No long inline path chaining.** Bring names into scope with `use` at the top of the file and refer to them unqualified. Write `Duration::from_secs(30)` (with `use std::time::Duration`), not `std::time::Duration::from_secs(30)`; `fs::write(..)` (with `use std::fs`), not `std::fs::write(..)`. `use` modules for free functions (`use std::fs; fs::write`) and types directly (`use std::time::Duration; Duration::…`). This applies to `std`, `crate::`, and external crates alike.
- `cargo fmt --all` and `cargo clippy -- -D warnings` are hard gates in `install.sh`; keep both green.

## Architecture

Two cooperating processes connected over a loopback TCP socket:

- **Rust TUI** (`src/main.rs`, `src/repl.rs`) — ratatui/crossterm full-screen app in the alternate screen. Owns input editing, the scrollback transcript, and all coloring.
- **Macaulay2 backend** (`src/m2_backend.rs` + `src/m2_startup.m2`) — `M2Backend::spawn()` launches `M2 --script src/m2_startup.m2`. The startup script reads a port from `/tmp/spectral-m2_port`, opens a listener, and runs an eval loop calling `capture(code, UserMode => true)`.

### The wire protocol (must stay in sync across the Rust/M2 boundary)

The framing constants are defined **twice** — in `m2_backend.rs` (top) and `m2_startup.m2` (header comment + literals). Changing one without the other breaks startup silently. Flow:

1. Rust binds an ephemeral port, writes it to `/tmp/spectral-m2_port`, then spawns M2.
2. M2 reads the port, opens the listener, sends `<<<M2_READY>>>`.
3. Per evaluation: Rust sends `<<<M2_CODE>>>\n<code>\n<<<M2_END>>>\n`.
4. M2 replies `<<<M2_RESULT>>>\n<ok|error>\n<output>\n<<<M2_END>>>\n`.

`evaluate()` is synchronous and blocking (30s read timeout). Multi-line input is sent as-is; the M2 loop reassembles lines until `<<<M2_END>>>`.

### Highlighting pipeline (`src/repl.rs`)

- Grammar comes from the `tree-sitter-macaulay2` crate; highlight queries live in `queries/highlights.scm` (compiled in via `include_str!`).
- `HIGHLIGHT_NAMES` (the capture names the highlighter is configured with) and `highlight_style()` (name → ratatui `Style`) are a matched pair. Adding a capture to the query means adding it to **both** the `HIGHLIGHT_NAMES` array and the `highlight_style()` match, or it renders unstyled.
- `clean_m2_output()` strips `capture`'s session scaffolding (echoed input, trailing prompt, `oN =`/`oN :` labels) and splits the value body from the trailing `oN : Type`. Covered by the `#[cfg(test)]` tests.
- Results render in a bordered box (`push_output_box`, `draw_transcript`); the value body is still plain `Color::White` inside the box, while input is fully tree-sitter highlighted. Re-highlighting output bodies is the known area of active work.

### Input vs. output rendering

`TranscriptEntry` distinguishes `Input` (highlighted via tree-sitter), `Output` (value body + optional type line, error-aware background), and `System` messages. The live input area (`draw_input_area`) re-highlights on every keystroke and manages multi-line cursor math manually.

## Gotchas

- Terminal state (raw mode, alternate screen, bracketed paste, keyboard enhancement flags) is set up in `ReplTerminal::enter` and torn down in both `leave()` and `Drop` — keep cleanup idempotent so a panic doesn't leave the terminal wedged.
- `/tmp/spectral-m2_port` and `/tmp/spectral-m2_startup.m2` are written at startup; the port file is removed after handshake.
- The M2 child is killed on `M2Backend::drop`.
