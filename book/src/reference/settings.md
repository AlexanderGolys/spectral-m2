# Settings

Open the settings with `Ctrl+O` or `/settings`. They are kept between sessions: the scheme, contrast, mode and background in `appearance.json`, everything else, keys and the maths colours of the appearance tab included, in `settings.json`, both in the [configuration directory](files.md).

The picker has eight tabs along its top; `Tab` and `Shift+Tab` switch them, or move onto the row of tabs and use `Left` and `Right`. `Up` and `Down` pick a row, `Left` and `Right` change it (`Space` also steps right), and the selected row describes itself under the list. *Restore defaults*, beneath the rows, resets everything: settings, keys and appearance.

## Appearance

theme
: The colour-scheme family. Default: Everforest. See [Colour schemes](../custom/colours.md).

contrast
: A continuous slider from the current mode's lowest contrast variant at 0% to its highest at 100%, blending their colours linearly in between; contrast is that of plain text against the canvas. Each arrow changes one percentage point. Default: 50%. A mode with one variant keeps that palette throughout the range.

mode
: Dark or light. Default: dark.

background
: An opaque canvas, or a transparent one that lets the terminal show through.

maths colours
: `syntax` colours maths by role; `monochrome` draws it in the value colour. Default: `syntax`.

index colours, exponent colours
: `original`, the indexed symbol's colour (`symbol`), or the `operator` colour, for subscripts and exponents. Delimiters and punctuation keep their own. Default: `original`.

## Rendering { #rendering }

maths
: `unicode`, `ascii` or `typeset`. Default: `unicode`. See [Read results](../shell/results.md#renderers).

output mode position
: Place the output mode controls in the border: `upper left`, `upper center`, `upper right`, `lower left`, `lower center` or `lower right`; `off` hides them. Default: `upper right`. The icons select the output representation (`rod`, `net`, `string`, `tex`, `markup` or `external`). The maths setting controls display within that preview.

maths size
: Preferred typeset size relative to terminal text, from 50% to 200% in 10% steps. Default: 100%. Long formulas wrap at available break points before shrinking to fit the pane.

italics
: Draw emphasised and commented text in italics. Typeset pictures keep TeX's math italic either way. Default: on.

symbols
: `nerd font`, `unicode` or `ascii`: the symbols the prompt, dividers, separators, tree branches, icons and spinner are drawn with. `nerd font` draws Nerd Font icons, `unicode` only symbols most fonts have, such as `❯` and `│`, and `ascii` plain ones such as `>` and `|`. Colours stay as they are. When the screen is too garbled to reach the picker, set `"charset": "ascii"` under `rendering` in [`settings.json`](files.md) by hand; an older `"ascii": true` still reads as `ascii`. Default: `nerd font`.

fractions
: `stacked`, `underlined` or `inline`. Default: `stacked`. Greyed out while maths is typeset.

math fonts
: Draw rings as blackboard glyphs, `ℤ` instead of `ZZ`, and `\mathcal` and `\mathfrak` capitals as script and fraktur. Default: off. Greyed out while maths is typeset. How the glyphs look depends on your terminal's fonts; see [Troubleshooting](../troubleshooting.md#blackboard).

matrix row spacing
: Distance between the rows of a typeset matrix, relative to TeX's own, from 100% to 300%. Default: 125%. Greyed out unless maths is typeset.

matrix degrees
: Show the degrees of a matrix's rows beside them, greyed out in the `Degree` colour. Default: off.

matrix labels
: Label the maps of a complex or a map of complexes by their matrices. Off, each is labelled by its name: a differential by `dd_i`, a component of a map by the map's name, or `f` when no global name holds it, as `f_i`. Default: off.

## Side panel { #side-panel }

The [views beside the session](../shell/views.md): the code, output and syntax-tree views and the reference pane.

output view
: What the [output view](../shell/views.md) shows: `everything` in the feed except the code, only returned `values`, or only `printed` text. Default: everything.

code overflow
: What code wider than the panel does: `wrap` breaks it onto further rows as *code wrapping* in the editor settings chooses, `trim` cuts each line at the right edge. Default: `wrap`. Code in the shell always wraps.

compute examples
: How long each documentation example may run when it is computed again, so its value is drawn from its structure. Default: off, which keeps the output the documentation recorded. See [Documentation](../shell/documentation.md#examples-computed-for-real).

## LSP

language server
: Run an installed `m2-ls` alongside the session. Default: on.

semantic highlighting
: Colour code by what the language server knows. Default: on; greyed out while the server is off.

diagnostics
: Show automatic diagnostic messages and source underlines. Default: off. Saved as `layout.diagnostics`.

signature hints
: Show the overloads returned for the current call. Default: off. Saved as `rendering.signature_hints`.

## Editor

max input height
: How much of the pane a growing multiline input may take, in 5% steps. Default: 75%.

max output height
: Rows a value shows before its middle folds away. Default: 40 rows, at least 5.

scrolloff
: Rows kept visible above and below the input cursor, and of the previous page when `PageUp` or `PageDown` scrolls. Default: 2.

scroll speed
: Rows one notch of the mouse wheel scrolls the feed or a view. Default: 3, at least 1.

input wrapping, code wrapping
: `word` breaks live input at words and submitted or previewed code at token boundaries; `character` breaks at terminal columns. Default: `word` for both. A word or token wider than the pane still breaks to fit.

completion
: Offer name and path completions. Default: on. Commands remain available through the command picker when it is off.

completion sorting
: `usage` ranks symbols by recorded frequency, then length and name; `alphabetical` ignores frequency; `length` offers shortest names first. Default: `usage`. Paths keep directories first and then alphabetical order.

completion list
: Show candidates when completing with `Tab`. Off inserts and cycles candidates inline. Default: on.

completion rows
: Maximum candidates shown at once; the list scrolls to keep the current candidate visible. Default: 8, at least 1.

complete after
: Characters of a name, or of a file name in a string, typed before it completes. Default: 1.

hint below
: Show the inline completion hint only when there are fewer candidates than this; 0 turns hints off. Default: 10. Saved as `hintCompletionBelow`, from 0 to 65535.

### Selection { #selection }

copy on select
: Copy a mouse selection when it finishes. Off leaves it marked for `Ctrl+Y`. Default: on.

copy cells as
: `raw text` copies a whole cell or result as typed and printed; `session` adds Macaulay2's prompts, `i1 : code`, `o1 = value`, `o1 : Type`. Default: `raw text`.

raw selection
: Select every character as drawn: cell numbers, frames, types beside values, and each wrapped row as its own line. Typeset pictures still copy as code. Default: off.

## Statusline

position
: Below the input, above it, or off. Default: below input.

One row per component
: Its segment: `off`, `L`, `LC`, `C`, `RC` or `R`. `Left` and `Right` move it; `0` hides it and `1` to `5` choose a segment. See [Status line](../custom/statusline.md) for every component and the defaults.

## Session

local session files
: Keep session archives, the previous session, notes and input history in `.spectral-m2/` at the nearest ancestor containing `.git`, or the current directory outside a repository. Off uses the global state directory. Default: off. Switching takes effect immediately and keeps the two histories separate; existing files stay in their original location.

## Keymap

Every rebindable key, by context. See [Keymaps](../custom/keymaps.md).

Edit `settings.json` and `appearance.json` in your configuration directory to change values directly, then restart the shell. Both files are created on first launch. Settings changed in the picker are saved immediately; close the shell before editing the files so later picker changes do not overwrite your edits. Missing fields use their defaults.
