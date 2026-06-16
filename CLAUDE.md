# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`spectral-m2` is a terminal REPL for [Macaulay2](https://macaulay2.com) that adds tree-sitter syntax highlighting to a full-screen TUI. It does not reimplement M2 — it drives a real `M2` process and renders its output. `install.sh` also symlinks a short alias `sp2`.


## Commands

```bash
cargo run                    # debug build + launch the REPL (needs M2 on PATH)
cargo test                   # unit tests (parser logic in src/repl.rs)
cargo clippy --all-targets 
cargo fmt --all
./install.sh                 # full pipeline: fmt-check → clippy → test → release build → install to ~/.local/bin/spectral-m2
```

`cargo run` uses the **debug** build straight from `target/`; the installed `spectral-m2` command is the **release** copy in `~/.local/bin`. They are separate — after editing, `cargo run` to test in place, or `./install.sh` to refresh the `spectral-m2` command.

`install.sh` is the gated pipeline: `cargo fmt --all --check`, `cargo clippy -- -D warnings`, and `cargo test` all run as hard gates before the release build and install — any failure aborts. Running the binary requires the `M2` executable on `PATH`; it spawns a real Macaulay2 process per session.

Tests live in the `#[cfg(test)] mod tests` block at the bottom of `src/repl.rs` and cover `clean_m2_output` (stripping `capture`'s session scaffolding). Run a single one with `cargo test <name>`, e.g. `cargo test preserves_2d_net_alignment`.

## Code style

- `cargo fmt --all` and `cargo clippy -- -D warnings` are hard gates in `install.sh`; keep both green.

## Architecture

Two cooperating processes connected over a loopback TCP socket:

- **Rust TUI** (`src/main.rs`, `src/repl.rs`) — ratatui/crossterm full-screen app in the alternate screen. Owns input editing, the scrollback transcript, and all coloring.
- **Macaulay2 backend** (`src/m2_backend.rs` + `src/m2_startup.m2`) — `M2Backend::spawn()` launches `M2 --script <tmp>/spectral-m2_startup.m2 <port>`, passing the port as the script's sole argument (`scriptCommandLine#1`). The startup script opens a listener on that port and runs an eval loop calling `capture(code, UserMode => true)`.

### The wire protocol (must stay in sync across the Rust/M2 boundary)

Payloads are **JSON**, carried in **length-prefixed** frames: a marker line (`<<<M2_MSG>>>`), then a decimal byte count line, then exactly that many opaque payload bytes (so the JSON may itself contain the marker string or newlines). The marker string, the request `method` names, and the reply field names are defined on **both** sides — `m2_backend.rs` (the `MSG_MARKER` const + the `Request`/`*Reply` serde types) and `m2_startup.m2` (the `M2MSG` literal + `handleMessage`). Changing one side without the other breaks startup silently. Flow:

1. Rust binds an ephemeral port, drops the listener, then spawns M2 with the port as a script argument.
2. M2 opens the listener on that port and sends `<<<M2_READY>>>`.
3. Per request: Rust sends `<<<M2_MSG>>>\n<byte-count>\n{"method":"evaluate"|"check","code":…}`; M2 replies `<<<M2_MSG>>>\n<byte-count>\n<JSON result>`.
   - `evaluate` → `{"output": <string>, "error": <bool>}`
   - `check` (non-executing) → `{"verdict": "COMPLETE"|"INCOMPLETE"|"ERROR", "message": <string>}`

`evaluate()` and `check()` are synchronous and blocking (30s read timeout). The JSON is escaped/parsed by serde (Rust) and M2's `JSON` package (`toJSON`/`fromJSON`). Method dispatch is **manual** in `m2_startup.m2` — not M2's `JSONRPC` package, whose `handleRequest` error-trapping breaks `parseCheck`'s `try parse` completeness detection. Adding a query (e.g. an ambient `loadedPackages` status) is one new `method` branch in `handleMessage` plus a serde type — the multiplexing payoff of the JSON channel.

### Highlighting pipeline (`src/repl.rs`)

- Grammar comes from the `tree-sitter-macaulay2` crate; highlight queries live in `queries/highlights.scm` (compiled in via `include_str!`).
- `HIGHLIGHT_NAMES` (the capture names the highlighter is configured with) and `highlight_style()` (name → ratatui `Style`) are a matched pair. Adding a capture to the query means adding it to **both** the `HIGHLIGHT_NAMES` array and the `highlight_style()` match, or it renders unstyled.
- `clean_m2_output()` strips `capture`'s session scaffolding (echoed input, trailing prompt, `oN =`/`oN :` labels) and splits the value body from the trailing `oN : Type`. Covered by the `#[cfg(test)]` tests.
- Results render in a bordered box (`push_output_box`, `draw_transcript`); the value body is still plain `Color::White` inside the box, while input is fully tree-sitter highlighted. Re-highlighting output bodies is the known area of active work.

### Input vs. output rendering

`TranscriptEntry` distinguishes `Input` (highlighted via tree-sitter), `Output` (value body + optional type line, error-aware background), and `System` messages. The live input area (`draw_input_area`) re-highlights on every keystroke and manages multi-line cursor math manually.

## Gotchas

- Terminal state (raw mode, alternate screen, bracketed paste, keyboard enhancement flags) is set up in `ReplTerminal::enter` and torn down in both `leave()` and `Drop` — keep cleanup idempotent so a panic doesn't leave the terminal wedged.
- `<tmp>/spectral-m2_startup.m2` is written at startup (the script M2 runs); the listen port is passed as a script argument, not via a rendezvous file.
- An optional user init file (`$XDG_CONFIG_HOME/spectral-m2/init.m2`, else `~/.config/spectral-m2/init.m2`) is evaluated once at spawn, before the first prompt; a missing file is a silent no-op.
- The M2 child is killed on `M2Backend::drop`.
