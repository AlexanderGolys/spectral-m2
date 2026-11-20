# How sp2 talks to Macaulay2

Spectral is a front end to one real, long-running Macaulay2 process. Knowing a little about the connection helps you predict what the shell will and will not do.

## One session, one reader

When `sp2` starts, it launches `M2` with your startup options, loads its backend, and lets M2 enter its normal top level, reading the whole session from standard input, exactly as an interactive M2 does. So `x := 1` in one cell is visible in the next, `exit` closes the shell, and your packages and `init.m2` behave as usual.

What the backend changes is only how results come back: instead of printing `oN =` and `oN :` lines, M2 hands each value over already laid out, as a tree of rows, scripts, fractions, fences, grids and diagrams, each piece of text tagged with its class. Separate `rod` and `specType` methods lay out the value and its type; they come from the [Spectral](https://github.com/AlexanderGolys/Spectral) package, which works in a plain M2 too. The shell fits that layout to the pane, as text or as a typeset picture, and colours each piece by its class. Printed text and errors arrive as M2 produced them, in order.

You can read all of it: the backend is written out as `backend.m2` in the [configuration directory](../reference/files.md), so `code` shows Spectral's hooks like any other M2 code.

## Drawing your own types

How a result is drawn is decided by the [Spectral](https://github.com/AlexanderGolys/Spectral) package alone. The backend only calls it; it doesn't hand you its output hooks. A value is drawn by the `rod` installed for its class, which returns a drawing, a `Rod`; a class with none is spelled as M2 spells it. Install `rod` for a type in a cell, or in Spectral's own [`init.m2`](../reference/files.md) to have it in every session:

```m2
Point = new Type of List
rod Point := p -> rod' FunctionApplication {symbol point, toSequence p}
new Point from {1, 1/2}     -- drawn as point(1, ½)
```

`rod'` draws a part of a drawing, here the expression the point is spelled by, laid out as any of M2's expressions is: a function applied to its arguments.

A drawing is a table of how something is drawn, `Rod{shape, Highlight => …}`, read as `x.Shape`, `x.Class`, `x.Highlight` and so on. Its `Shape` is an expression whose parts are drawings in turn, of one of the classes the shell draws. They are `Holder{text}` for an atom, `RowExpression`, `Parenthesize`, `Divide`, `Subscript`, `Superscript`, the lists, `MatrixExpression`, `Table`, and Spectral's own `AlignedExpression`, `UnderExpression`, `OverExpression`, `DiagramExpression` and `TreeExpression`. An atom takes its colour from its `Group`, one of the shell's colours, an `HLG` such as `HLG "Keyword"`. A `Highlight` colours a part and everything in it in place of their own colours, reaching `Reach` levels below it, or all of them when unset. A `Font` sets the text of a part in a font by its name: `"MathNormal"` for TeX's own maths, a maths alphabet such as `"MathBB"`, where `Q` is ℚ, or a text face such as `"TextTT"`, the typewriter face; the atoms in it take it unless they name their own. A field given as `null` is not set:

```m2
Gene = new Type of HashTable
rod Gene := g -> Rod {Holder {g.name}, Group => HLG "Keyword"}
new Gene from {symbol name => "BRCA1"}
```

A drawing that throws falls back to the value's plain text, so a broken `rod` can't break the session.

## Your code runs once

Each cell is sent to Macaulay2 exactly once and is never replayed behind your back. Deciding whether input is complete, splitting it into cells, highlighting and diagnostics are all done locally, by parsing the text, before anything is sent. An hour-long computation is never started twice.

Each result arrives once, complete: drawn by `rod` with its type from `specType`, its drawing shown by Spectral's `speek`, and also spelled by M2's own `net`, `toString` and `toExternalString`, every one of them cheap. Each piece of text in the drawing comes with the colour and the font it is drawn in, so the shell only lays it out. Switching renderers uses the data already received. A result that is `null`, or silenced by a trailing `;`, is not formatted at all; a silenced one is still bound to its `oN`.

## Other processes

Documentation examples, when you [compute them](../shell/documentation.md#examples-computed-for-real), run in a separate Macaulay2 per page, never in your session. The optional language server `m2-ls` reads your session's source to colour it and never evaluates anything.

## Pictures

Typeset maths is set inside `sp2` by the built-in RaTeX engine, encoded off the main thread, and sent to the terminal through the kitty graphics protocol. The terminal keeps only the pictures near what is on screen: those no longer shown are deleted, and past a memory budget the furthest go first. Inside tmux, pictures tmux dropped while the pane was hidden are sent again when it regains focus.

## Formatting your own types

The shapes compose as M2's expressions do. Draw a type by the expression it reads as, and each of its parts keeps its own class and colour, even when their text is the same:

```m2
Pair = new Type of HashTable;
rod Pair := p -> rod' RowExpression {p#0, p#1};
new Pair from {0 => "a", 1 => symbol a}
```

The first part is a string and the second a symbol, coloured as those are.

`AlignedExpression` sets rows of cells in aligned columns, with each row's last cell running on. When that cell is too long for the pane it wraps beneath where it began, and when the columns before it leave it too little room it moves beneath them, indented. Hash tables draw their pairs this way, so every `=>` stands in one column. Use it for any type that reads as labelled lines:

```m2
Report = new Type of HashTable;
rod Report := r -> rod' AlignedExpression apply(sort keys r, k -> {k, symbol :, r#k});
new Report from {"status" => "converged", "steps" => toList(1..40)}
```

`MapExpression{target, source, label}` places a label above a leftward arrow. Nest maps in the target position to compose a chain without repeating its modules. `UnderExpression{base, label}` centers a label beneath its base while keeping the base on the surrounding baseline; a complex sets each module over its degree this way.

Plain M2 prints a `Rod` as the text it lays out. `speek` shows the drawing itself, the table of fields it is: `speek(1/2)` is `Rod` over `Class => Divide`, `Shape => Divide{1, 2}` and `Typeset => true`. The parts are drawings too, nested as deep as the value is, so `speek` shows them as their text, and `speek'_2 x` opens one more level of them.

### Names

Inside a drawing, a ring, an ideal, a module, a matrix, a map of rings, a variety, a coherent sheaf, a map of sheaves, a complex or a map of complexes that a global name holds is drawn by that name, as M2 spells a named ring. After `I = ideal(x^3, y^3)`, the quotient `S/I` is drawn as S/I, while `J := ideal(x^3)` has no global name and leaves `S/J` drawn in full. The result itself is always drawn by its value: `I` shows its generators, and `R = QQ[x]` shows QQ[x]. `rod'_n x` resolves the names inside `x` to their values `n` times over:

```m2
R = QQ[x, y]; I = ideal(x^3, y^3);
rod'_1(R/I)     -- QQ[x .. y] over ideal(x³, y³)
```

Only drawings change: the `net` and `string` previews keep M2's own spelling. A complex is drawn as its modules, each over its degree, and the differentials between them; a map of complexes as its two complexes, one above the other, with its components between them.

### Types

`specType x` is the class of `x`, over the values its type is over, by their names: `R^2` is `Module[R]`, an ideal is `Ideal[R]`, a matrix `Matrix[R ⟵ R²]`, a sheaf of rings `SheafOfRings[X]`, a complex `Complex[R]`. `specType'_n x` resolves those names `n` times, so `specType'_1(R^2)` is `Module[QQ[x .. y]]`. What a type is over is what `typeParameters` of its class gives, none for most types; install it for a type of your own, or `specType` to draw its annotation some other way:

```m2
typeParameters Pair := p -> {class p#0, class p#1};   -- Pair[String, Symbol]
```
