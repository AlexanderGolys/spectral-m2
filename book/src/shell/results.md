# Read results

Spectral draws a value from its structure, not from the text M2 would print: a fraction stacks, a matrix gets tall brackets, a subscript drops below its symbol, and each part takes the colour code gives it.

Hash tables retain their structured entries even when a nested value has only a text representation. Vertical lists align entries on the left; `NumberedVerticalList` adds zero-based indices in a separate column, with every entry starting at the same position after its index and arrow.

Coherent sheaves, sheaf expressions, structure sheaves (`SheafOfRings`), sheaf maps, and affine and projective varieties use M2's expression structure. Structure sheaves retain their variety subscripts and twists; sheaf maps show their source, target, and matrix. Named varieties keep their names. Scripted functors retain M2's mathematical notation in typeset output, including the calligraphic O for `OO`.

Complexes show their modules joined by leftward arrows, with each differential matrix above its arrow and the homological degrees beneath the modules. The structured preview retains the component colours in both Unicode and typeset output:

```m2
S = QQ[a_1..a_4];
C = res coker vars S
```

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
: Plain-text fallback without mathematical layout. The selected M2 formatter stays the same; choose `net` in the output border for M2's own terminal layout.

## Preview a representation

Each output block has one formatter selector on its outer border. It applies to every value in the block, without separators between values, and offers the representations available throughout the block:

`specNet`
: The structured expression produced by Spectral's formatter, retaining the expression tree and resolved component classes down to its leaves. These classes supply the highlighting.

`net`
: Macaulay2's two-dimensional terminal spelling.

`string`
: The result of M2's `toString` method.

`tex`
: The result of M2's `tex` method, displayed using the maths setting. It uses a single foreground colour because TeX alone does not carry M2's component classes.

`markup`
: A documentation page with its semantic layout, links and examples.

`external`
: The result of M2's `toExternalString`: source that reads the value back in, such as `new HashTable from {1 => 2}`, coloured as code.

The active icon is highlighted and its mode name appears beside the controls. Click another icon to select that preview, or click the active icon or mode name to cycle. This changes every value inside that border. The original input is evaluated once; switching previews uses its retained formatter results. A formatter that fails is omitted without hiding the other previews.

The *maths* setting controls how mathematical content is displayed within a preview. Changing between typeset, Unicode and ASCII leaves the selected preview and its icons unchanged, including when pictures are unavailable. Supported structured results start in `specNet`; documentation pages start in `markup`. A result without a supported structured layout starts in `net`, with no `specNet` option. Archived examples use `net` until a computed structured result is available. Choose `net` to compare a preview with M2's own terminal spelling.

Place the controls with *output mode position* in the rendering settings: upper or lower, aligned left, center or right; `off` hides them. Icon positions stay fixed when the mode label changes. The same controls work inside documentation examples.

## Typeset maths { #typeset }

With *maths* set to `typeset`, mathematical values become pictures in your theme's colours, with matrices and fractions at their full height. Symbols use the same mathematical face whether alone or beside numbers or strings. Strings use typewriter face. The `net` and `string` formatters retain their terminal text.

Set *maths size* to choose the preferred formula size independently of the terminal font. Matrix degrees sit beside their rows, and tuple subscripts follow M2's TeX spelling without extra parentheses.

Pictures scroll, fold and split with the panes, and a theme change repaints them. A value too wide for the pane wraps onto more rows after separators and suitable operators; a part that cannot break shrinks while it stays readable, and past that it is drawn in Unicode instead. Typewriter words, such as the `ideal` in `ideal(x, y)`, use the font your kitty or Ghostty configuration names, so they match the code around them.

You need a terminal speaking the kitty graphics protocol with Unicode placeholders: kitty and Ghostty do, inside tmux too, with [two lines of tmux setup](../troubleshooting.md#typeset-tmux). Elsewhere `typeset` is still offered, marked unavailable in that terminal, and values are drawn in Unicode meanwhile.

## Types, printed text and errors

A value's type description stands at the right edge of its box. Each type's `typeDepth` parameter controls its detail: arrays and lists show only their outer type by default, while modules show their base ring as `Module[R]`. Set parameters such as `SheafOfRings.typeDepth = 2` in the shell or your Spectral `init.m2` to show the variety as `SheafOfRings[X]`. Request component types explicitly with `specType'(2, x)`; see [Formatting your own types](../deep/how-it-works.md#formatting-your-own-types). A long value wraps short of it, and a type wider than a third of the box moves below the value. Type annotations use one uniform type colour. Brackets follow the scheme’s `Punctuation` group, matching brackets in code. Commas use `Delimiter`. The `=>` arrows in options and hash tables use `Operator`, and a key that is a plain name or string, like `Strategy` in `Strategy => 2`, uses `Option`, the group option names take in code. The arrow in `MapExpression` uses `MapArrow`, which inherits the normal `Text` colour for visual clarity. Typeset annotations copy as text when selected.

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
- *output mode position*: where the mode controls sit in each output border.
