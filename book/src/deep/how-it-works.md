# How sp2 talks to Macaulay2

Spectral is a front end to one real, long-running Macaulay2 process. Knowing a little about the connection helps you predict what the shell will and will not do.

## One session, one reader

When `sp2` starts, it launches `M2` with your startup options, loads its backend, and lets M2 enter its normal top level, reading the whole session from standard input, exactly as an interactive M2 does. So `x := 1` in one cell is visible in the next, `exit` closes the shell, and your packages and `init.m2` behave as usual.

What the backend changes is only how results come back: instead of printing `oN =` and `oN :` lines, M2 hands each value over already laid out, as a tree of rows, scripts, fractions, fences, grids and diagrams, each piece of text tagged with its class. Separate `specNet` and `specType` methods lay out the value and its type; they come from the [Spectral](https://github.com/AlexanderGolys/Spectral) package, which works in a plain M2 too. The shell fits that layout to the pane, as text or as a typeset picture, and colours each piece by its class. Printed text and errors arrive as M2 produced them, in order.

You can read all of it: the backend is written out as `backend.m2` in the [configuration directory](../reference/files.md), so `code` shows Spectral's hooks like any other M2 code.

## Drawing your own types

How a result is drawn is decided by the [Spectral](https://github.com/AlexanderGolys/Spectral) package alone. The backend only calls it; it doesn't hand you its output hooks. A type that already has an `expression` is drawn from it, each class of expression laid out by Spectral's `formatted`: a sum between plus signs, a power as a script, a matrix as a grid. Install `specNet` (or `specType`, for the annotation) to draw a type some other way, or `formatted` to lay out an expression class differently. Do it in a cell, or in Spectral's own [`init.m2`](../reference/files.md) to have it in every session:

```m2
Point = new Type of List
expression Point := p -> new FunctionApplication from (symbol point, toSequence p)
speek new Point from {1, 1/2}   -- Row{Atom{"point", Symbol}, Fence{...}}
formatted Sum := s -> joined(apply(toList s, specNet), " ⊞ ", "Operator")
```

A drawing is built from Spectral's shape constructors: `atom(text, class)`, `row`, `joined`, `listed`, `script`, `fraction`, `fence`, `parenthesized`, `grid` and `labelled`. Any part may carry a `Highlight`, a class it is coloured as in place of its own, reaching `Reach` levels below it, or all of them when unset:

```m2
specNet Ideal := I -> (lookup(specNet, Thing)) I ++ {Highlight => Keyword, Reach => 1}
```

A drawing that throws falls back to the value's plain text, so a broken `specNet` can't break the session.

## Your code runs once

Each cell is sent to Macaulay2 exactly once and is never replayed behind your back. Deciding whether input is complete, splitting it into cells, highlighting and diagnostics are all done locally, by parsing the text, before anything is sent. An hour-long computation is never started twice.

Each result arrives once, complete: drawn by `specNet` with its type from `specType`, and also formatted by M2's own `net`, `toString`, `tex`, `toExternalString` and `peek`, and by Spectral's `speek`. By default `specNet` traverses the class's `expression` representation; a value held without further structure keeps its class and textual spelling. Switching renderers uses the data already received. A result that is `null`, or silenced by a trailing `;`, is not formatted at all; a silenced one is still bound to its `oN`.

## Other processes

Documentation examples, when you [compute them](../shell/documentation.md#examples-computed-for-real), run in a separate Macaulay2 per page, never in your session. The optional language server `m2-ls` reads your session's source to colour it and never evaluates anything.

## Pictures

Typeset maths is set inside `sp2` by the built-in RaTeX engine, encoded off the main thread, and sent to the terminal through the kitty graphics protocol. The terminal keeps only the pictures near what is on screen: those no longer shown are deleted, and past a memory budget the furthest go first. Inside tmux, pictures tmux dropped while the pane was hidden are sent again when it regains focus.

## Formatting your own types

An `expression` method lets a new type compose the same shapes as ordinary mathematics. Its components keep their own classes, even when their text is identical:

```m2
Pair = new Type of HashTable;
expression Pair := p -> RowExpression{p#0, p#1};
new Pair from {0 => "a", 1 => symbol a}
```

The first component is a string and the second a symbol. Their colours follow those classes. No shell extension is needed. An expression class with no layout of its own is spelled by its name over its parts, as `Equation{a, b}`.

`AlignedExpression` sets rows of cells in aligned columns, with each row's last cell running on. When that cell is too long for the pane it wraps beneath where it began, and when the columns before it leave it too little room it moves beneath them, indented. Hash tables draw their pairs this way, so every `=>` stands in one column. Use it for any type that reads as labelled lines:

```m2
Report = new Type of HashTable;
expression Report := r -> AlignedExpression apply(sort keys r, k -> {k, symbol :, r#k});
new Report from {"status" => "converged", "steps" => toList(1..40)}
```

`MapExpression{target, source, label}` places a label above a leftward arrow. Nest maps in the target position to compose a chain without repeating its modules. `UnderExpression{base, label}` centers a label beneath its base while keeping the base on the surrounding baseline; complexes use it for homological degrees.

Use `speek` to inspect how a value is laid out. Like `peek`, it spells one level of the drawing, each part by its shape and each piece of text by its class; `speek'(depth, value)` spells `depth` levels, and parts past them as `{...}`. `specNet` and `specType` are separate methods, both receiving the original value. You can change a type annotation independently:

```m2
specType' (ZZ, Pair) := (depth, p) -> specNet symbol Pair;
```

Type annotations take their depth from the nearest `typeDepth` parameter in the value's type ancestry. `Thing.typeDepth = 1` keeps ordinary values and containers compact; `Module.typeDepth = 2` shows the base ring as `Module[R]`. A subclass inherits its parent's setting unless it has its own.

Set these parameters in the shell, or keep them in your Spectral `init.m2`:

```m2
Array.typeDepth = 1;
List.typeDepth = 1;
Module.typeDepth = 2;
SheafOfRings.typeDepth = 2;
CoherentSheaf.typeDepth = 2;
```

Depth zero or one shows only the outer type. At depth two or greater, modules and matrices show their base ring, coherent sheaves and sheaf maps show the sheaf of rings they are modules over, and a sheaf of rings shows its variety. These are mathematical parameters: a module over `R` displays as `Module[R]`, a sheaf on `X` as `CoherentSheaf[OO_X]`, and `OO_X` as `SheafOfRings[X]`, independently of their presentation matrices or twists. Depth three looks into the variety under `OO_X` when it has no name. A named parameter keeps its name. An unnamed one shows a placeholder at depth two, and each further level spells it out one more step: `specType'(3, (ZZ/7[t])^2)` shows the ring as `ZZ/7[t]`.

Other types use a bounded fold of their expression components. Set `Array.typeDepth = 2` to display `[1, 1.]` as `Array[ZZ, RR]`. The depth must be a non-negative integer. A bad setting leaves the ordinary class annotation available, while a direct `specType` call reports the error.

An explicit `specType'(depth, value)` overrides the configured depth: `specType'(2, [1, 1.])` expands one layer to `Array[ZZ, RR]`; `specType'(3, [[1, 1.], ["a"]])` gives `Array[Array[ZZ, RR], Array[String]]`. Repeated components remain present at explicitly requested depths. Install `specType'` for `(ZZ, YourType)` to customize which parameters a type exposes.

A type's `typeset` setting chooses whether its values are set as typeset maths. The nearest setting in the type's ancestry applies, and everything inside a typeset part is typeset with it. `Thing.typeset` is `false`; `RingElement`, `Ring`, `Module`, `Matrix` and the expressions `Divide`, `Power`, `Subscript` and `MatrixExpression` are `true`. Turn it on for a type of your own, or off for one you prefer as text:

```m2
Pair.typeset = true;     -- draw pairs as typeset maths
Ring.typeset = false;    -- spell rings such as QQ as text
```

### How deep values unfold

A value is drawn from its `expression`, and unfolding stops at names: a ring, module or variety bound to a symbol is shown by that symbol. Unnamed structures inside a value, such as the quotient ring of a resolution computed from `coker vars (S/I)`, are unfolded only up to the nearest `valueDepth` setting in the value's type ancestry. Past that depth they keep a conventional placeholder: `R` for a ring, `I` for an ideal, `M` for a module, `X` for a variety, `F` for a sheaf and `C` for a complex. `Thing.valueDepth = 1` spells out the outer value and shows the structures it is built over by placeholder. Ring elements, numbers and the items of lists are always drawn in full.

A complex unfolds one name at a time. At depth one it is only its chain, `0 ⟵ C₀ ⟵ C₁ ⟵ ⋯ ⟵ C₄ ⟵ 0`, with each arrow labelled by its differential `dᵢ`. At depth two the names give way to the modules and the matrices of the differentials, with their rings as placeholders. Every further level substitutes one more name, the quotient ring first and then the ring it is a quotient of. `Complex.valueDepth = 2` is the default, so a resolution shows its modules and matrices.

```m2
Complex.valueDepth = 3;    -- also spell out the ring of every module
specNet'(3, OO_(Proj(QQ[a,b]))^2)
```

An explicit `specNet'(depth, value)` overrides the configured depth.
