# Settings

Open the settings with `Ctrl+O` or `/settings`. They are kept between sessions: the appearance in `appearance.json`, everything else, keys included, in `settings.json`, both in the [configuration directory](files.md).

The picker has six tabs along its top; `Tab` and `Shift+Tab` switch them, or move onto the row of tabs and use `Left` and `Right`. `Up` and `Down` pick a row, `Left` and `Right` change it, and the selected row describes itself under the list. *Restore defaults*, beneath the rows, resets everything: settings, keys and appearance.

## Appearance

theme
: The colour-scheme family. Default: Everforest. See [Colour schemes](../custom/colours.md).

contrast
: The family's variant for the current mode. Default: medium.

mode
: Dark or light. Default: dark.

background
: An opaque canvas, or a transparent one that lets the terminal show through.

## Layout

max input height
: How much of the pane a growing multiline input may take, in 5% steps. Default: 75%.

max output height
: Rows a value shows before its middle folds away. Default: 40 rows, at least 5.

scrolloff
: Rows kept visible above and below the input cursor. Default: 2.

complete after
: Characters of a name, or of a file name in a string, typed before it completes. Default: 1.

hint below
: Show the inline completion hint only when there are fewer candidates than this; 0 turns hints off. Default: 10. Saved as `hintCompletionBelow`, from 0 to 65535.

## Statusline

position
: Below the input, above it, or off. Default: below input.

One row per component
: Its segment: `off`, `L`, `LC`, `C`, `RC` or `R`. `Left` and `Right` move it; `0` hides it and `1` to `5` choose a segment. See [Status line](../custom/statusline.md) for every component and the defaults.

## Rendering { #rendering }

language server
: Run an installed `m2-ls` alongside the session. Default: on.

semantic highlighting
: Colour code by what the language server knows. Default: on; greyed out while the server is off.

maths
: `unicode`, `ascii` or `typeset`. Default: `unicode`. See [Read results](../shell/results.md#renderers).

original output text
: Show every value as the text Macaulay2 printed, unrendered; the rows that shape rendered maths are locked meanwhile. Default: off.

maths colours
: `syntax` colours maths by role; `monochrome` draws it in the value colour, with types in the type colour. Default: `syntax`.

index colours, exponent colours
: `original`, the indexed symbol's colour (`symbol`), or the `operator` colour, for subscripts and exponents. Delimiters and punctuation keep their own. Default: `original`.

italics
: Draw emphasised and commented text in italics. Typeset pictures keep TeX's math italic either way. Default: on.

fractions
: `stacked`, `underlined` or `inline`. Default: `stacked`. Greyed out while maths is typeset.

math fonts
: Draw rings as blackboard glyphs, `ℤ` instead of `ZZ`, and `\mathcal` and `\mathfrak` capitals as script and fraktur. Default: off. Greyed out while maths is typeset. How the glyphs look depends on your terminal's fonts; see [Troubleshooting](../troubleshooting.md#blackboard).

matrix row spacing
: Distance between the rows of a typeset matrix, relative to TeX's own, from 100% to 300%. Default: 125%. Greyed out unless maths is typeset.

compute examples
: How long each documentation example may run when it is computed again, so its value is drawn from its structure. Default: off, which keeps the output the documentation recorded. See [Documentation](../shell/documentation.md#examples-computed-for-real).

swap reference backgrounds
: Use the feed background for code in the reference pane. Default: off.

## Selection { #selection }

copy on select
: Copy a mouse selection when it finishes. Off leaves it marked for `Ctrl+Y`. Default: on.

copy cells as
: `raw text` copies a whole cell or result as typed and printed; `session` adds Macaulay2's prompts, `i1 : code`, `o1 = value`, `o1 : Type`. Default: `raw text`.

raw selection
: Select every character as drawn: cell numbers, frames, types beside values, and each wrapped row as its own line. Typeset pictures still copy as code. Default: off.

## Keymaps

Every rebindable key, by context. See [Keymaps](../custom/keymaps.md).
