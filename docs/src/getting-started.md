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
