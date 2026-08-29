# Changelog

All notable changes to Spectral M2 are documented here.

## [0.3.0] - 2026-08-29

First crates.io release.

### Added

- Added two independent Macaulay2 workspaces with wide split rendering, draggable and keyboard resizing, and narrow-layout pane switching.
- Added code, structured-output, and `m2-syn` syntax-tree previews with linked committed state, independent viewports, selection, and split opening through `Ctrl+Enter`.
- Added command, settings, style, keymap, and highlight-debug pickers, including persistent slash-command history.
- Added structured pending submissions with spinner and queue states, numbered cells, values, types, printed output, warnings, errors, and source-ranged syntax diagnostics.
- Added keyboard and mouse selection across input, feed, code, and syntax-tree content with OSC 52 copying.
- Added seven colorscheme families with ordered native variants and complete UI and Macaulay2 highlight-group mappings.
- Added an mdBook covering shell workflows and keys, with Cargo-built `mdbook-tsitter` highlighting for Macaulay2 examples.

### Changed

- Replaced Tree-sitter-based interactive parsing and highlighting with `m2-syn`'s native typed parser and visitor API.
- Reorganized the shell around workspace sessions, typed feed blocks, responsive panes, and independently rendered previews.
- Made installation Cargo-managed and removed manual binary copying and local-bin symlinks.

[0.3.0]: https://github.com/AlexanderGolys/spectral-m2/releases/tag/v0.3.0
