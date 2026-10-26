# Getting started

## Requirements

Spectral M2 needs:

- Macaulay2 available as `M2` on `PATH`;
- a Rust toolchain with Cargo for building from this repository;
- a terminal with mouse events and OSC 52 clipboard support for the full interface.

## Run the shell

Build and run directly through Cargo:

```sh
cargo run --release
```

Spectral starts a Macaulay2 process immediately. If `M2` cannot be found or started, it exits with an explanation instead of opening an unusable interface.

Start from code you already have: a file runs cell by cell before you type anything, as `/load` would run it, and `--resume` brings back the code of the session you last closed.

File loads and resumed code omit comments from the replayed source. Comment-only files leave the session empty. Comment markers inside string literals remain unchanged; comments entered interactively stay visible as pending input.

```sh
sp2 ring.m2
sp2 --resume
```

A name without an extension gets `.m2`. Themes, keys, and display settings are set inside the shell: `Ctrl+O` opens the settings and `Ctrl+/` lists every command.

## M2 startup options

Macaulay2 loads its normal user `init.m2` before the shell opens. Spectral then loads its own optional `init.m2` from `$XDG_CONFIG_HOME/spectral-m2`, falling back to `~/.config/spectral-m2`. Initialization output appears in the transcript. `-q` skips both user initialization files and disables M2's home-directory package paths.

These options are passed to M2 and kept when the session restarts, including when `/load` starts a fresh session:

| Option | Effect |
| --- | --- |
| `-q` | Skip user initialization and home-directory packages. |
| `-E EXPR` | Evaluate before M2 initialization; repeatable. |
| `-e EXPR` | Evaluate after M2 initialization, before Spectral's init file and backend hooks; repeatable. |
| `--no-preload` | Skip preloaded packages. |
| `--prefix DIR` | Set M2's prefix directory. |
| `--srcdir DIR` | Add an M2 source or build tree to the initial path. |
| `--no-backtrace` | Suppress error backtraces. |
| `--no-debug` | Do not enter the debugger on error. |
| `--debug N` | Turn warning N into an error. |
| `--notify` | Report files loaded and command-line expressions evaluated. |
| `--print-width N` | Set M2's output width. |
| `--no-randomize` | Keep the fixed initial random seed. |
| `--no-threads` | Disable additional M2 threads and tasks. |
| `--no-time` | Make M2's current time always zero. |

For example:

```sh
sp2 --no-debug --print-width 100 -e 'R = QQ[x]' ring.m2
sp2 -q --no-randomize --resume
```

Expressions run inside M2 without creating transcript input cells. Files and resumed code run afterward, cell by cell. Documentation examples continue to use their separate default M2 configuration.

## Serve this book locally

Install the documentation tools through Cargo once:

```sh
cargo install mdbook --version 0.5.3 --locked
cargo install mdbook-tsitter
```

Build the Macaulay2 parser from its Cargo crate, then serve the book:

```sh
cargo build --release \
    --manifest-path docs/parser/Cargo.toml \
    --target-dir target/docs-parser
mdbook serve docs --hostname 127.0.0.1 --port 3000
```

Open [http://127.0.0.1:3000](http://127.0.0.1:3000). The server rebuilds the book when its Markdown changes. A one-off build is written to `target/mdbook`:

```sh
mdbook build docs
```

`mdbook-tsitter` highlights every fenced `m2` or `macaulay2` block at build time. The parser is built from the published `tree-sitter-macaulay2` crate; no compiled parser is stored in the repository.
