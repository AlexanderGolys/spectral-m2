# Editing and evaluation

## Syntax-aware submission

Press `Enter` to submit. Spectral first asks `m2-syn` whether the buffer is complete:

- complete input is split into cells and queued for Macaulay2;
- input that ended while a construct was still open receives another line;
- invalid input stays in the editor with a local diagnostic and source range.

For example, pressing `Enter` after the first line keeps editing because the parenthesized expression is unfinished:

```m2
f = x -> (
    x^2 + 1)
```

Use `Shift+Space`, `Shift+Enter`, or `Alt+Enter` when you want a newline regardless of completeness.

## Pending and completed cells

Submitted code moves into the feed immediately. As Macaulay2 answers, Spectral appends printed text, values, types, warnings, or errors; consecutive values share one border. Cell numbers remain session metadata but are not displayed until cells become focusable.

Only one submission can evaluate in a workspace at a time. Trying to submit another while it is busy produces an input diagnostic and does not disturb the running evaluation.

Semicolons remain part of their cell, so M2 still controls whether a value is printed:

```m2
R = QQ[x, y];
I = ideal(x^2, x*y, y^2);
degree I
```

## History and navigation

`Up` and `Down` move inside multiline input. At the first or last line they recall older or newer submissions. Commands chosen after typing `/` are recorded in the same history; opening the full command picker with `Ctrl+/` does not add a history entry.

The usual editing keys are available for horizontal movement, line boundaries, deletion, and tabs. `Ctrl+L` clears the current input without clearing the feed.

## Selection and clipboard

Hold `Shift` with `Left`, `Right`, `Home`, or `End` to extend an input selection. `Ctrl+Y` copies it. Typing or deleting replaces the selected range.

You can also drag over input, feed, code previews, or syntax trees with the mouse. Releasing a real drag copies the selectable text through OSC 52. UI markers, borders, type annotations, and picker rows are excluded where appropriate so copied text stays useful.
