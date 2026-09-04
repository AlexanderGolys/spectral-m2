# Changelog

All notable changes to Spectral M2 are documented here.

## [0.4.0] - 2026-09-04

### Added

- Added a normalized action and key-event layer covering every shell command and alternate-pane activation with `Alt+V`.
- Added retained feed rendering, render statistics, and live highlight-debug probes.
- Added editable multiline input with selection-aware history, cursor scrolloff, configurable height, and `Shift+Space` insertion.

### Changed

- Split workspace state into independent sessions, editors, feeds, preview caches, and viewports.
- Reworked output layout around aligned screen cells, consistent wrapping, shared consecutive-value boxes, and separate syntax-tree and wrapping components.
- Made appearance hierarchical: schemes own their complete native palettes and semantic UI groups.
- Limited redraws and retained-feed invalidation to state changes.

### Fixed

- Made runtime picker types available in normal builds.
- Kept overflowing output type annotations visible on a fallback row.
- Avoided redundant appearance persistence and feed invalidation during workspace synchronization.

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

[0.4.0]: https://github.com/AlexanderGolys/spectral-m2/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/AlexanderGolys/spectral-m2/releases/tag/v0.3.0
