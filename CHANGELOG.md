# Changelog

All notable changes to Spectral M2 are documented here.

## [Unreleased]

## [1.1.0] - 2026-10-27

### Added

- M2 startup configuration through `-q`, `-E`, `-e`, `--no-preload`, `--prefix`, `--srcdir`, `--no-backtrace`, `--no-debug`, `--debug`, `--notify`, `--print-width`, `--no-randomize`, `--no-threads`, and `--no-time`, retained across session restarts.
- Initialization output is shown on first startup and after restarting M2.

- `Ctrl+C` interrupts a running computation and discards queued work while preserving the Macaulay2 session; when idle it still quits.

- `sp2 FILE` runs a file's code as the session starts, `sp2 --resume` the previous session's, and `sp2 --help` and `--version` describe the shell.
- Optional semantic highlighting through an installed `m2-ls`, covering live input, session code, and code previews, with configurable semantic theme groups and a persisted rendering setting.

- Added a *maths* setting: `unicode`, as before; `typeset`, which draws returned values as pictures typeset by the built-in RaTeX, through the kitty graphics protocol, on terminals that support it, tmux included; and `ascii`, Macaulay2's own plain-text spelling. A type holding maths, such as a matrix's rings, is typeset too, in the type colour. Where the terminal cannot show pictures, `typeset` is still offered, struck through and marked unavailable in that terminal.
- Selecting a typeset picture copies its Macaulay2 code, once however many rows it covers; repeated formulas each copy separately.
- Double-click selects a word, a bracketed block, or a whole formula in the input, session, and previews. `Ctrl+A` selects the whole input, and dragging against a pane edge scrolls the selection.

### Changed

- M2 enters its normal top level after backend initialization, so `quit`, `exit`, and `end` close the shell cleanly; writes racing with process exit are handled as termination.
- Spectral M2 is now licensed under MIT or Apache-2.0, at your option, instead of GPL-3.0-only.
- Built on `m2-syn` 0.7, now also MIT or Apache-2.0. The syntax tree view still nests what brackets hold under the opening bracket, and no longer lists empty groups.
- Settings are grouped into *appearance*, *layout* and *rendering* tabs, switched with `Tab`. Each row shows only its name and value, aligned in one column at any pane width: each shows its current value between arrows. The *restore defaults* button, no longer a row of its own, and the selected row's description sit beneath the rows, and the status line names the keys the open picker answers.
- The mouse wheel over the input moves through its lines instead of changing *input max height*, which it used to lower without showing it.
- Semantic colours from `m2-ls` 2.0 are layered over syntax colours and retained while typing until new tokens arrive.
- Typeset type annotations use the terminal's font for typewriter words, and wide formulas wrap across rows.

### Removed

- Removed the `Alt+arrow` pane shortcuts; panes remain accessible by mouse and the view commands.

### Fixed

- Pasting control characters or picture placeholders copied from a terminal no longer breaks the input: they are dropped and the text around them kept.
- Inside tmux the first key pressed after starting is no longer lost: pictures are detected from what tmux reports instead of a terminal query that tmux leaves unanswered.
- A typeset type sits beside its value with its last row on the value's last, rather than one row lower.
- A cell sharing a line with the one before it, as in `a = 1; b = 2`, is no longer drawn indented by the space between them.
- Input wrapping, cursor placement, and vertical movement account for the display width of wide characters.
- Dragging outside the input copies the selection on release, and rows joined by a tall picture copy as one line.

## [1.0.0] - 2026-10-19

### Added

- Added a *math fonts* setting, off by default: rings are drawn by the name they have in code, `ZZ`, and on as blackboard glyphs, `ℤ`; `\mathcal` and `\mathfrak` capitals are drawn as script and fraktur glyphs, and `\mathbb` covers every capital.
- Cell numbers are laid out for three digits so a number growing a digit moves nothing.
- Added `/values` (`Alt+V`) and `/stdout` (`Alt+P`) views beside `/code`, `/output`, and `/ast`.
- Added completion shown as a muted hint after the cursor and taken with `Tab`, which cycles through the candidates when pressed again (`Shift+Tab` backwards), while `Alt+Tab` inserts a tab: names Macaulay2 knows, reported by Macaulay2 as it starts and after each cell; file names inside strings; command names; and the file `/save` and `/load` take.
- Added undo (`Ctrl+Z`) and redo (`Ctrl+Shift+Z`) for the input, and `Ctrl+Backspace` and `Ctrl+Delete` to delete what `Ctrl+Left` and `Ctrl+Right` move over.
- Added a *compute examples* setting, off by default: a time limit for each documentation example, which is then run again in a Macaulay2 of its own so its value is drawn from its structure, as a live value is.
- Links in documentation are underlined under the pointer and run `help` for their page on Ctrl+click, without disturbing the input.
- History is shared by every running session: input submitted in one can be recalled in the others without restarting them.
- The prompt takes a typing form while you type and returns to rest shortly after the last keystroke that changed the input, without moving the text.
- Added a *complete after* setting for how much of a name is typed before it completes.
- Added `/save [file]` and `/load <file>` to write the session's code to an `.m2` file and run one in a fresh session, and `/resume` to run the code of the previous session, which is kept on every exit and restart and by a bare `/save`.

### Fixed

- After an evaluation error, the failed cell and unevaluated code have a red background, while unevaluated code is muted without syntax colours. Error boxes put the error prefix in the title and draw the message in light red; queued and rejected markers are configured with the other UI glyphs.
- Centralized interface glyphs and the pending spinner in UI configuration, with simple symbols for limited terminals.
- Documentation headings use the light blue, light yellow, and aqua highlight colours; example code blocks share the transcript width and wrap in narrow panes. Folded outputs show a three-row separator and label, recent cell numbers are darker, and the outer margin follows each adjacent row.
- A documentation page's title is ruled off beneath, with its name in bold and its brief in italics; its usage, inputs and outputs read as headers rather than bullets; and the location of its source reads as text. Inline code has no background and links no underline of their own. Gruvbox functions are no longer bold.
- Documentation examples are drawn without cell numbers, on the page's own surface, with values M2 wrapped joined back into one and a blank line kept after the last.
- Text-package markup retains heading levels, emphasis, links, subscripts, nested lists and table layout. Documentation examples use the session's input, printed-text and output blocks, including multiline values and type annotations.
- Preserved normal Macaulay2 initialization and user package paths, so packages installed in the home directory can be found as in a regular M2 shell.
- The syntax tree view keeps one column clear on its left, sets its source between two rules with a margin inside each, and lists an opening bracket before what it encloses and the closing one after, in the order of the source. Field names and code fragments are told apart in Everforest.
- Printed text is drawn in a lighter, less saturated shade of the string colour, in every theme.
- Output labels keep their `oN` numbers, with the latest three in bold, and the numbers of the cells behind the latest three outputs are drawn in the colour of their labels.
- Modules and sheaves whose matrices M2 pads with `\vphantom` are drawn from their structure instead of as raw TeX; names in typewriter face are coloured as code names them.
- Documentation markup such as `DIV` is drawn with titled headings of three depths, links underlined on hover, muted `--` descriptions, bullet markers and code coloured as typed, instead of one flat colour.
- A cell still evaluating shows a turning spinner and keeps the bottom margin it has once its outputs arrive, so the block no longer changes height on completion.
- Operators, keywords, commands and scripted functors are coloured as what they are when returned: `symbol ^` as an operator, `symbol if` as a keyword, `Ext` as a function. Values built with `^^`, `**`, `@@`, `|`, `=>` and `==>` keep their operator colour instead of falling back to plain text.
- The whole screen keeps one column clear on its left, and the hint of an empty input starts at the cursor rather than a cell after it.
- Recent-output highlighting follows the latest lines that evaluated to something, muted ones included and null ones not.
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

[Unreleased]: https://github.com/AlexanderGolys/spectral-m2/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/AlexanderGolys/spectral-m2/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/AlexanderGolys/spectral-m2/compare/v0.4.0...v1.0.0
[0.4.0]: https://github.com/AlexanderGolys/spectral-m2/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/AlexanderGolys/spectral-m2/releases/tag/v0.3.0
