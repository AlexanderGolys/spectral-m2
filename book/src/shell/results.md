# Read results

Spectral draws a value from its structure, not from the text M2 would print: a fraction stacks, a matrix gets tall brackets, a subscript drops below its symbol, and each part takes the colour code gives it.

<!-- capture: the same matrix value three times, under the unicode, ascii and typeset maths settings, side by side. -->

```m2
R = QQ[x_1..x_3]
M = matrix {{x_1, 1/2}, {x_2^2, x_3}}
```

## Three ways to draw maths { #renderers }

The *maths* setting picks how values are drawn:

`unicode` (default)
: Laid out in characters, in two dimensions: stacked fractions, tall brackets, lowered subscripts. Works in any terminal.

`typeset`
: Real TeX pictures, set by the built-in RaTeX engine with Computer Modern fonts, so matrices and fractions appear as in a paper. Needs a terminal that shows pictures; see [below](#typeset).

`ascii`
: Macaulay2's own plain-text spelling, exactly as M2 lays it out.

Each numbered output carries a small icon before its `oN` label naming what actually drew it: documentation markup, a typeset picture, Unicode maths, ASCII or plain text. If the chosen way could not draw a value, the icon says which way did. Click the icon to redraw just that value the next available way; the setting stays as it was. On a documentation page, the plain-text way shows the page exactly as Macaulay2 printed it, which tells a rendering fault from a mistake in the page itself.

## Typeset maths { #typeset }

With *maths* set to `typeset`, mathematical values become pictures in your theme's colours, with matrices and fractions at their full height. Strings, `null`, names and nets stay terminal text, and a list mixing words with maths is still set as one formula, the words in typewriter face.

Pictures scroll, fold and split with the panes, and a theme change repaints them. A value too wide for the pane wraps onto more rows after the commas between its entries; a part that cannot break shrinks while it stays readable, and past that it is drawn in Unicode instead. Typewriter words, such as the `ideal` in `ideal(x, y)`, use the font your kitty or Ghostty configuration names, so they match the code around them.

You need a terminal speaking the kitty graphics protocol with Unicode placeholders: kitty and Ghostty do, inside tmux too, with [two lines of tmux setup](../troubleshooting.md#typeset-tmux). Elsewhere `typeset` is still offered, marked unavailable in that terminal, and values are drawn in Unicode meanwhile.

## Types, printed text and errors

A value's type stands at the right edge of its box, `o1 : Matrix R^2 <-- R^2`. A long value wraps short of it, and a type wider than a third of the box moves below the value. Types are drawn in the type colour.

Text a cell prints with `print` or `<<` comes before its value, in a lighter shade of the string colour. Warnings and errors get boxes of their own: an error names itself in the box title and shows its message in red, and the failed cell sits on a red background. See [When things go wrong](errors.md).

## Long values fold

A value taller than *max output height*, 40 rows by default, folds: its first and last rows stay, with a separator saying how much is hidden in between. Double-click the value to show all of it; double-click its border to fold it again.

Very large plain-text values, such as a long `net`, are previewed rather than drawn whole: up to 256 KiB of text and 262,144 cells per value, keeping the beginning and the end with a notice between. The whole value is still there in M2.

## Fine-tuning

The *rendering* settings shape the details; each is in the [settings reference](../reference/settings.md#rendering):

- *fractions*: stacked, underlined, or inline;
- *math fonts*: `ZZ` as written, or as the blackboard `ℤ`;
- *maths colours*: by role, or monochrome;
- *index colours* and *exponent colours*: their own colour, their symbol's, or the operator colour;
- *matrix row spacing* for typeset matrices;
- *original output text*: every value exactly as M2 printed it, nothing rendered.
