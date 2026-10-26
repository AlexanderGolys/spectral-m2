# Spectral M2

[![crates.io](https://img.shields.io/crates/v/spectral-m2.svg)](https://crates.io/crates/spectral-m2)
[![license](https://img.shields.io/crates/l/spectral-m2.svg)](#license)

Spectral M2 is a syntax-aware terminal shell for [Macaulay2](https://macaulay2.com/). It combines a real M2 session with local `m2-syn` parsing, structured output, multiline editing, selectable transcripts, preview modes, and two independent workspaces.

```m2
R = QQ[x, y]
I = ideal(x^2, x*y, y^2)
betti res I
```

## Features

- immediate syntax highlighting and local source-ranged diagnostics;
- optional semantic colors from `m2-ls` across live input, session code, and previews;
- immediate pending cells with spinner and queued states during evaluation;
- structured values, types, printed text, warnings, and errors;
- persistent multiline history and keyboard or mouse selection;
- code, output, and syntax-tree previews with independent navigation;
- views of the session opened beside it, including one-at-a-time narrow layouts;
- command, settings, keymap, and highlight-debug pickers;
- ordered native colorscheme variants with opaque or transparent canvas modes.

## Requirements

- Macaulay2 available as `M2` on `PATH`;
- a terminal with mouse events and OSC 52 clipboard support for the complete interface.

Optional semantic highlighting uses `m2-ls` 2.0 or later on `PATH`, installed with `cargo install m2-ls`. The shell also works without it.

## Install

Install the released package through Cargo:

```sh
cargo install spectral-m2 --locked
sp2
```

The package is named `spectral-m2`; its executable is `sp2`. Give it a file to run as the session starts, or pick up where the last session ended:

```sh
sp2 ring.m2      # run ring.m2 cell by cell, then keep going
sp2 --resume     # run the previous session's code
sp2 --help
```

M2 startup options can be combined with a file or `--resume`, for example `sp2 --no-debug --print-width 100 -e 'R = QQ[x]' ring.m2`. Use `-E` for expressions before initialization and `-e` for expressions after it; both can be repeated. The same configuration is applied when the session restarts. `sp2 --help` lists all supported options.

Run the current checkout without installing it:

```sh
cargo run --release
```

Press `Ctrl+C` during a computation to cancel it and discard queued work while keeping the Macaulay2 session. When idle, `Ctrl+C` quits. The *cancel execution* binding can be changed in `/keymaps`; its hint appears in the status line after three seconds. Macaulay2 commands that exit the process also close the shell.

Type `/` for the filtered command picker, `Ctrl+/` for all commands, or `Ctrl+O` for settings. Views open beside the session.

## Configuration

Spectral preserves normal Macaulay2 initialization, including `~/.Macaulay2/init.m2` and packages installed in your home directory. It then reads its own `init.m2` and materializes its generated `backend.m2` in `$XDG_CONFIG_HOME/spectral-m2`, falling back to `~/.config/spectral-m2`. The persistent backend source lets Macaulay2's `code` inspect Spectral's hooks; it is replaced whenever the backend embedded in the binary changes.

## Documentation

The [mdBook](https://github.com/AlexanderGolys/spectral-m2/tree/master/docs) describes editing, commands, workspaces, previews, styles, and every default key. Its Macaulay2 examples are highlighted by `mdbook-tsitter` using a Cargo-built parser. From a source checkout:

```sh
cargo install mdbook --version 0.5.3 --locked
cargo install mdbook-tsitter
cargo build --release --locked \
    --manifest-path docs/parser/Cargo.toml \
    --target-dir target/docs-parser
mdbook serve docs --hostname 127.0.0.1 --port 3000
```

## License

Spectral M2 is licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
