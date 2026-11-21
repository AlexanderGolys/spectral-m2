# Changelog

All notable changes to Spectral M2 are documented here.

## [Unreleased]

### Changed
- Matrix degrees are hidden unless the new *matrix degrees* rendering setting is on, and are greyed out when shown; a complex numbers its degrees in the same grey, a new `Degree` highlight group linked to `Muted`. The zero modules a complex begins and ends with are white, not number-coloured.
- A map of complexes draws every component between modules both complexes are concentrated in, a zero one too, so a zero map of degree 1 still shows how it shifts degrees; none goes to or from the bounding zeros.
- A typeset value too wide for the pane always stays a picture, shrinking as far as it takes, instead of falling back to Unicode text past 60%.
- `subquotient` modules, and `HH` of a map of complexes, are drawn instead of falling back to M2's text.

- The *ascii* switch in the rendering settings is now *symbols*, with three choices: `nerd font`, `unicode` (no Nerd Font icons) and `ascii`. A saved `ascii` choice carries over. The choice now also reaches the spinner, tree branches, key hints, ellipses and documentation rules, and changing it redraws results at once.
- Picker key hints spell arrow keys as arrows (`↑↓ select`, `←→ change`) instead of `updown`.
- The `speek` preview shows the drawing of the value, the table of fields it is, its parts one level deep; `speek'_n x` opens `n` levels.
- Spectral 0.6.0: a drawing is a table of how it is drawn, `Rod{shape, Highlight => …}`, its `Shape` one of the expression classes the shell draws, every field read as `x.Shape`, `x.Class`, `x.Highlight`. Its colours are `HLG` groups such as `HLG "Operator"`, a name outside the shell's groups being an error. A `rod` method builds the expression it draws as, `Rod {RowExpression {…}}`, in place of the removed constructors `atom`, `row`, `joined`, `listed`, `script`, `fence`, `parenthesized`, `grid` and `labelled`.
- A ring, ideal, module, matrix, ring map, variety, sheaf, complex or map of complexes that a global name holds is drawn by that name inside other values: after `I = ideal(x^3, y^3)`, `S/I` reads S/I, while one assigned with `:=` is drawn whole. A result itself is always spelled out, a named ring `R = QQ[x]` as QQ[x]. Structures without a name are drawn in full rather than under a made-up `R`, `I` or `M`, a complex as its modules and differentials rather than `Cᵢ` and `dᵢ`.
- Spectral 0.7.0: what a class says about how its values are drawn is in the drawing its `rod` returns, an atom's `Group`, `Typeset`, a `Highlight`, in place of the setting methods `highlightGroup`, `typeset`, `typesetOperands`, `typewriter`, `typeDepth`, `valueDepth` and `keepNames` and the `formatted` hook. A class without a `rod` is spelled as M2 spells it. `rod'_n x` draws `x` with its names resolved `n` times; `specType x` is its class over its `typeParameters` by name, `Module[R]`, and `specType'_n x` resolves them. A `Rod` prints in plain M2 as the text it lays out.
- Spectral 0.8.0: a drawing names the `Font` its text is set in, `"MathNormal"`, a maths alphabet such as `"MathBB"` or a text face such as `"TextTT"`, in place of `Typeset` and an atom's `Tex`; `QQ` is the letter `Q` in the blackboard alphabet. A result arrives drawn with every piece of text in its colour and font, the drawing alone deciding how it looks.
- Spectral 1.0.0: a value is drawn by `shape`, the shape of its M2 expression down to atoms, `atom(text, HLG "Keyword", "TextTT")`, and typed by `typeShape`, in place of `rod`, `rod'`, `Rod`, `specType`, `typeParameters` and `speek'`; `highlightGroup` and `font` of a class colour and set its values. sp2 tells Spectral its highlight groups at startup, so a misspelt `HLG "Keywrd"` is an error where it is written. Strings are coloured as strings again. Inside a drawing only the names M2's own expressions keep are kept, a ring's; an ideal or a module is drawn by its value. The `speek` preview shows the tree of a value's expression classes.
