# Spectral M2

Spectral M2 is a terminal shell for Macaulay2. It keeps the familiar numbered input and output flow while adding syntax-aware editing, structured output, local diagnostics, selectable transcripts, previews, and two independent workspaces.

```m2
R = QQ[x, y]
I = ideal(x^2, x*y, y^2)
betti res I
```

The input is parsed and highlighted locally with `m2-syn`. Complete cells are sent to a real Macaulay2 process, so package loading, state, values, printed text, warnings, and errors retain their ordinary M2 semantics.

The shell is organized around three parts:

- the feed contains submitted code and its results;
- the input area is a multiline, syntax-aware editor;
- the status line shows context-sensitive shortcuts and preview state.

The rest of this book describes the currently implemented interface.
