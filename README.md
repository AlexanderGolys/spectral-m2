# Spectral M2

[![crates.io](https://img.shields.io/crates/v/spectral-m2.svg)](https://crates.io/crates/spectral-m2)
[![license](https://img.shields.io/crates/l/spectral-m2.svg)](LICENSE)

Spectral M2 is a syntax-aware terminal shell for [Macaulay2](https://macaulay2.com/). It combines a real M2 session with local `m2-syn` parsing, structured output, multiline editing, selectable transcripts, preview modes, and two independent workspaces.

```m2
R = QQ[x, y]
I = ideal(x^2, x*y, y^2)
betti res I
```

## Features

- syntax highlighting and source-ranged diagnostics driven by `m2-syn`;
- immediate pending cells with spinner and queued states during evaluation;
- structured values, types, printed text, warnings, and errors;
- persistent multiline history and keyboard or mouse selection;
- code, output, and syntax-tree previews with independent navigation;
- responsive two-workspace splits, including one-at-a-time narrow layouts;
- command, settings, style, keymap, and highlight-debug pickers;
- ordered native colorscheme variants with opaque or transparent canvas modes.

## Requirements

- Macaulay2 available as `M2` on `PATH`;
- a terminal with mouse events and OSC 52 clipboard support for the complete interface.

## Install

Install the released package through Cargo:

```sh
cargo install spectral-m2 --locked
spectral-m2
```

Run the current checkout without installing it:

```sh
cargo run --release
```

Type `/` for the filtered command picker, `Ctrl+/` for all commands, or `Ctrl+O` for settings. Splittable commands accept `Ctrl+Enter` to open in a second pane.

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

Spectral M2 is licensed under [GPL-3.0-only](LICENSE).
