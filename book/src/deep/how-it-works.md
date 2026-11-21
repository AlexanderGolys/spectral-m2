# How sp2 talks to Macaulay2

Spectral is a front end to one real, long-running Macaulay2 process. Knowing a little about the connection helps you predict what the shell will and will not do.

## One session, one reader

When `sp2` starts, it launches `M2` with your startup options, loads its backend, and lets M2 enter its normal top level, reading the whole session from standard input, exactly as an interactive M2 does. So `x := 1` in one cell is visible in the next, `exit` closes the shell, and your packages and `init.m2` behave as usual.

What the backend changes is only how results come back: instead of printing `oN =` and `oN :` lines, M2 hands each value over already laid out, as a tree of rows, scripts, fractions, fences, grids and diagrams, each piece of text tagged with its class. Separate `shape` and `typeShape` methods lay out the value and its type; they come from the [Spectral](https://github.com/AlexanderGolys/Spectral) package, which works in a plain M2 too. The shell fits that layout to the pane, as text or as a typeset picture, and colours each piece by its class. Printed text and errors arrive as M2 produced them, in order.

You can read all of it: the backend is written out as `backend.m2` in the [configuration directory](../reference/files.md), so `code` shows Spectral's hooks like any other M2 code.

## Drawing your own types

How a result is drawn is decided by the [Spectral](https://github.com/AlexanderGolys/Spectral) package alone. The backend only calls it; it doesn't hand you its output hooks. A value is drawn by `shape`, which draws most values through their M2 `expression`, so a class whose expression reads well needs nothing. Install `shape` for a type in a cell, or in Spectral's own [`init.m2`](../reference/files.md) to have it in every session:

```m2
Point = new Type of List
shape Point := p -> shape FunctionApplication {symbol point, toSequence p}
new Point from {1, 1/2}     -- drawn as point(1, ½)
```

`shape` of an expression draws it as any of M2's expressions is laid out, here a function applied to its arguments, each part drawn by `shape` in turn.

A drawing is an expression of one of the classes the shell draws, its parts drawings in turn, down to atoms of text: `RowExpression`, `Parenthesize`, `Divide`, `Subscript`, `Superscript`, the lists, `MatrixExpression`, `Table`, and Spectral's own `AlignedExpression`, `UnderExpression`, `OverExpression`, `DiagramExpression` and `TreeExpression`. An atom, `atom(text, group, font)`, is text in one of the shell's highlight groups, an `HLG` such as `HLG "Keyword"`, set in a font by its name: `"MathNormal"` for TeX's own maths, a maths alphabet such as `"MathBB"`, where `Q` is ℚ, or a text face such as `"TextTT"`, the typewriter face:

```m2
Gene = new Type of HashTable
shape Gene := g -> atom(g.name, HLG "Keyword", "TextTT")
new Gene from {symbol name => "BRCA1"}
```

The shell tells Spectral its highlight groups as it starts, so `HLG "Keywrd"` is an error where it is written. A value M2 spells by itself, and a name standing for one, is an atom coloured as `highlightGroup` of its class says and set in the class's `font`; install both for a class of your own, as `highlightGroup Point := T -> HLG "Operator"`.

A drawing that throws falls back to the value's plain text, so a broken `shape` can't break the session.

## Your code runs once

Each cell is sent to Macaulay2 exactly once and is never replayed behind your back. Deciding whether input is complete, splitting it into cells, highlighting and diagnostics are all done locally, by parsing the text, before anything is sent. An hour-long computation is never started twice.

Each result arrives once, complete: drawn by `shape` with its type from `typeShape`, the tree of its expression's classes beside it, and also spelled by M2's own `net`, `toString` and `toExternalString`, every one of them cheap. Each piece of text in the drawing comes with the colour and the font it is drawn in, so the shell only lays it out. Switching renderers uses the data already received. A result that is `null`, or silenced by a trailing `;`, is not formatted at all; a silenced one is still bound to its `oN`.

## Other processes

Documentation examples, when you [compute them](../shell/documentation.md#examples-computed-for-real), run in a separate Macaulay2 per page, never in your session. The optional language server `m2-ls` reads your session's source to colour it and never evaluates anything.

## Pictures

Typeset maths is set inside `sp2` by the built-in RaTeX engine, encoded off the main thread, and sent to the terminal through the kitty graphics protocol. The terminal keeps only the pictures near what is on screen: those no longer shown are deleted, and past a memory budget the furthest go first. Inside tmux, pictures tmux dropped while the pane was hidden are sent again when it regains focus.

## Formatting your own types

The shapes compose as M2's expressions do. Draw a type by the expression it reads as, and each of its parts keeps its own class and colour, even when their text is the same:

```m2
Pair = new Type of HashTable;
shape Pair := p -> shape RowExpression {p#0, p#1};
new Pair from {0 => "a", 1 => symbol a}
```

The first part is a string and the second a symbol, coloured as those are.

`AlignedExpression` sets rows of cells in aligned columns, with each row's last cell running on. When that cell is too long for the pane it wraps beneath where it began, and when the columns before it leave it too little room it moves beneath them, indented. Hash tables draw their pairs this way, so every `=>` stands in one column. Use it for any type that reads as labelled lines:

```m2
Report = new Type of HashTable;
shape Report := r -> shape AlignedExpression ((k -> {k, symbol :, r#k}) \ sort keys r);
new Report from {"status" => "converged", "steps" => toList(1..40)}
```

`MapExpression{target, source, label}` places a label above a leftward arrow. Nest maps in the target position to compose a chain without repeating its modules. `UnderExpression{base, label}` centers a label beneath its base while keeping the base on the surrounding baseline; a complex sets each module over its degree this way.

Plain M2 shows a drawing as the table it is, `Shape` naming its class and `Parts` holding its parts: `shape(1/2)` is a `Divide` of two atoms.

### Names

A name M2's own expressions keep is kept in a drawing: after `S = QQ[x, y]`, the quotient `S/ideal(x^3)` is drawn as S over its ideal, and the ideal by its generators. The result itself is always drawn by its value, so `R = QQ[x]` shows QQ[x]. The doubled-letter rings are set in the alphabet maths writes them in: `QQ`, `ZZ`, `RR` and `CC` as ℚ, ℤ, ℝ and ℂ, `OO` as 𝒪.

A complex is drawn as its modules, each over its degree, and the differentials between them; a map of complexes as its two complexes, one above the other, with its components between them.

### Types

`typeShape x` is the class of `x`, over the values its type is built over: `R^2` is `Module[R]`, an ideal is `Ideal[R]`, a matrix `Matrix[R ⟵ R²]`, a sheaf of rings `SheafOfRings[X]`, a complex `Complex[R]`; any other value is its class alone. Install it for a type of your own:

```m2
typeShape Pair := p -> shape FunctionApplication {class p, new Array from {class p#0, class p#1}};   -- Pair[String, Symbol]
```
