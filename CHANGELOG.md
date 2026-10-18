# Changelog

All notable changes to Spectral M2 are documented here.

## [Unreleased]

### Added

- Added a *math fonts* setting, off by default: rings are drawn by the name they have in code, `ZZ`, and on as blackboard glyphs, `ℤ`; `\mathcal` and `\mathfrak` capitals are drawn as script and fraktur glyphs, and `\mathbb` covers every capital.
- The latest three outputs are labelled `oo`, `ooo` and `oooo`, as Macaulay2 recalls them, and cell numbers are laid out for three digits so a number growing a digit moves nothing.
- Added `/values` (`Alt+V`) and `/stdout` (`Alt+P`) views beside `/code`, `/output`, and `/ast`.
- Added completion shown after the cursor and taken with `Tab`, which cycles through the candidates when pressed again (`Shift+Tab` backwards), while `Alt+Tab` inserts a tab: names Macaulay2 knows, reported by Macaulay2 as it starts and after each cell; file names inside strings; command names; and the file `/save` and `/load` take.
- Added undo (`Ctrl+Z`) and redo (`Ctrl+Shift+Z`) for the input, and `Ctrl+Backspace` and `Ctrl+Delete` to delete what `Ctrl+Left` and `Ctrl+Right` move over.
- History is shared by every running session: input submitted in one can be recalled in the others without restarting them.
- Added a *complete after* setting for how much of a name is typed before it completes.
- Added `/save [file]` and `/load <file>` to write the session's code to an `.m2` file and run one in a fresh session, and `/resume` to run the code of the previous session, which is kept on every exit and restart and by a bare `/save`.

### Fixed

- Text-package markup retains heading levels, emphasis, links, subscripts, nested lists and table layout. Documentation examples use the session's input, printed-text and output blocks, including multiline values and type annotations.
- Preserved normal Macaulay2 initialization and user package paths, so packages installed in the home directory can be found as in a regular M2 shell.
- The syntax tree view keeps one column clear on its left, sets its source between two rules with a margin inside each, and lists an opening bracket before what it encloses and the closing one after, in the order of the source. Field names and code fragments are told apart in Everforest.
- Printed text is drawn in a lighter, less saturated shade of the string colour, in every theme.
- `oN` output labels are no longer bold; `oo`, `ooo` and `oooo` stay bold, and the numbers of the cells behind the latest three outputs are drawn in the colour of their labels.
- Modules and sheaves whose matrices M2 pads with `\vphantom` are drawn from their structure instead of as raw TeX; names in typewriter face are coloured as code names them.
- Documentation markup such as `DIV` is drawn with titled headings of three depths, underlined links, muted `--` descriptions, bullet markers and code coloured as typed, instead of one flat colour.
- A cell still evaluating shows a turning spinner and keeps the bottom margin it has once its outputs arrive, so the block no longer changes height on completion.
- Operators, keywords, commands and scripted functors are coloured as what they are when returned: `symbol ^` as an operator, `symbol if` as a keyword, `Ext` as a function. Values built with `^^`, `**`, `@@`, `|`, `=>` and `==>` keep their operator colour instead of falling back to plain text.
- The whole screen keeps one column clear on its left, and the hint of an empty input starts at the cursor rather than a cell after it.
- `oo`, `ooo` and `oooo` name the latest lines that evaluated to something, muted ones included and null ones not, so a muted line no longer leaves an older output wearing the label and a null line takes no place in the rotation.
- A resumed or loaded session runs the cells that were entered together as one submission again, not all at once and not cell by cell.
- `Ctrl+Left` and `Ctrl+Right` move by a word (with `Shift`, select by one), `Ctrl+Home` and `Ctrl+End` jump to the ends of the input, and `Ctrl+J`, the newline character some terminals send for `Shift+Enter`, inserts a newline. Pane focus and resizing moved to `Alt+Left`, `Alt+Right` and with `Shift`.
- Cell numbers, output labels, borders and separators sit one cell further left, a label's last character always on its border.
- A command being typed is drawn in a new *Command* colour and its completion as a muted hint; the picker draws the part typed of each name in bold. `Tab` completes the command and closes the picker unless a file follows, `Left`, `Right`, `Home` and `End` move the cursor as in any input, and the completion goes while the cursor is inside the name and comes back at its end.
- Rings such as `QQ[x]/ideal(x^3-1)` and names such as `x_{1,1}` are drawn and coloured from their structure instead of falling back to plain text; a subscript with no lowered form stays on its row as `x_{1, 1}`.
- Types are no longer italic in any theme; packages are drawn as types in italics.
- Printed text is numbered only in the views beside the shell, not in the shell itself.
- A returned type of any kind, such as `Type` or `WrapperType`, inside a container too, is coloured as a type, and a package as a package.
- The code view adds no blank lines the code does not have, and the command list is only as tall as its matches.
- `Esc` while typing a command puts the list away and keeps what was typed; a command's completion is drawn as a muted hint.
- The status line no longer says "Ready".
- A curly brace of even height splits its cusp over the two middle rows, as at height two, instead of putting it off-centre.
- A mouse selection stays in the pane it starts in, and the first click after switching panes selects.
- Copying a selection keeps the blank lines inside it, such as those printed text contains.
- A command picked after typing part of its name is recorded in history whole.
- `/resume` and `/load` run their code cell by cell, so the feed reads as it did when the code was typed instead of as one block.
- Tests no longer write to the input history.
- The `stdout` view shows only printed text, without warnings, errors, or other blocks, each numbered by its cell as in the session.

### Changed

- `Left` and `Right` step through all five views, not only the output projections; `/output` now shows printed text and values together.
- Printed text carries the number of the cell that printed it, in the gutter where that cell's code carries it.
- Every view beside the session sits on the code background, printed output included.
- A focused view lists all five views in the middle of the status line, its own highlighted.

### Removed

- Removed the `/new`, `/quit`, and `/debug` commands; restarting, quitting, and the highlight inspector stay on their keys (`Alt+R`, `Ctrl+C`, `Ctrl+D`).
- Removed the `Alt+V` shortcut that opened a view in place of the focused pane; views always open beside the session.

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
