# How sp2 talks to Macaulay2

Spectral is a front end to one real, long-running Macaulay2 process. Knowing a little about the connection helps you predict what the shell will and will not do.

## One session, one reader

When `sp2` starts, it launches `M2` with your startup options, loads its backend, and lets M2 enter its normal top level, reading the whole session from standard input, exactly as an interactive M2 does. So `x := 1` in one cell is visible in the next, `exit` closes the shell, and your packages and `init.m2` behave as usual.

What the backend changes is only how results come back: instead of printing `oN =` and `oN :` lines, M2 hands each value over as an expression tree, preserving the class of every component. Separate `specNet` and `specType` methods describe the value and its type. The shell lays out that structure and applies the theme to its classes. Printed text and errors arrive as M2 produced them, in order.

You can read all of it: the backend is written out as `backend.m2` in the [configuration directory](../reference/files.md), so `code` shows Spectral's hooks like any other M2 code.

## Your code runs once

Each cell is sent to Macaulay2 exactly once and is never replayed behind your back. Deciding whether input is complete, splitting it into cells, highlighting and diagnostics are all done locally, by parsing the text, before anything is sent. An hour-long computation is never started twice.

Drawing a value calls `specNet` when M2 prints it. By default, it traverses the class's `expression` representation; a value held without further structure keeps its class and textual spelling. Switching renderers uses the data already received. A trailing `;` skips value formatting while `specType` still supplies its type annotation.

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

The first component is a string and the second a symbol. Their colours follow those classes. No shell extension is needed. An unfamiliar expression constructor remains inspectable with `speek`; outputs whose structure has no installed layout use M2's `net` formatter instead.

`MapExpression{target, source, label}` places a label above a leftward arrow. Nest maps in the target position to compose a chain without repeating its modules. `UnderExpression{base, label}` centers a label beneath its base while keeping the base on the surrounding baseline; complexes use it for homological degrees.

Use `speek` to inspect a value's formatting structure. `specNet` and `specType` are separate methods, both receiving the original value. You can change a type annotation independently:

```m2
specType' (ZZ, Pair) := (depth, p) -> specNet symbol Pair;
```

Type descriptions are a bounded fold of the expression over simple types. The default depth of one shows only the outer type, such as `Array` or `List`. `specType'(2, [1, 1.])` expands one layer to `Array[ZZ, RR]`; `specType'(3, [[1, 1.], ["a"]])` gives `Array[Array[ZZ, RR], Array[String]]`. Repeated components remain present at explicitly requested depths. Install `specType'` for `(ZZ, YourType)` to customize bounded type descriptions.
