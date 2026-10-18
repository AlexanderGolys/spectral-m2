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

Submitted code moves into the feed immediately. As Macaulay2 answers, Spectral appends printed text, values, types, warnings, or errors; consecutive values share one border. If a cell fails, its code and the unevaluated cells below it sit on a red background; the unevaluated code is muted without syntax highlighting. The error box puts “Error” in its title and shows the message in light red. Cell numbers remain session metadata but are not displayed until cells become focusable.

Only one submission can evaluate in a workspace at a time. Trying to submit another while it is busy produces an input diagnostic and does not disturb the running evaluation.

Semicolons remain part of their cell, so M2 still controls whether a value is printed:

```m2
R = QQ[x, y];
I = ideal(x^2, x*y, y^2);
degree I
```

## Documentation

Plain-text results such as `net help "arithmetic with complex maps"` use a bounded preview: at most 256 KiB of source text and 262,144 display cells per value. Larger previews retain their beginning and end with a truncation notice. Expanding a result keeps these limits; the full value remains available in Macaulay2.

`help` pages are drawn in the feed: the title over a rule, its name apart from its brief; the usage, inputs and outputs as headers over their entries; and each example as the session draws code and values, without its numbers. A link to another page is underlined while the pointer is over it, and Ctrl+click runs `help` for that page beside what you are typing.

## History and navigation

`Up` and `Down` move inside multiline input. At the first or last line they recall older or newer submissions. As in a shell's history search, the text before the cursor when you start browsing is a prefix: typing `R =` and pressing `Up` steps only through submissions beginning with `R =`, each distinct one once. Editing a recalled entry starts a new search from what the input then begins with. Stepping past the newest match restores what you had typed. History is shared by every running session: what one submits, the others recall the next time they start browsing. Commands chosen after typing `/` are recorded in the same history, under their whole name however much of it was typed; opening the full command picker with `Ctrl+/` does not add a history entry.

The usual editing keys are available for horizontal movement, `Ctrl` with the arrows for words, `Home` and `End` for line boundaries (`Ctrl` for the whole input), and deletion. `Ctrl+Backspace` and `Ctrl+Delete` delete exactly what `Ctrl+Left` and `Ctrl+Right` move over. `Ctrl+L` clears the current input without clearing the feed.

`Ctrl+Z` undoes the last edit to the input and `Ctrl+Shift+Z` redoes it. Typing undoes a word at a time and a run of deletions all at once; clearing, pasting, recalling history and taking a completion each undo as one step. Submitting starts a fresh input with nothing to undo.

## Completion

As you type, the rest of a completion is drawn after the cursor as a muted hint. `Tab` takes it, and pressing `Tab` again cycles through the other completions rather than completing further; `Shift+Tab` cycles backwards. A name Macaulay2 already knows is not completed. `Tab` only completes; `Alt+Tab` inserts a tab. What completes depends on where the cursor is, as Macaulay2 reads the code before it:

- a name: every symbol Macaulay2 knows, the names its own terminal completes. Names the session bound come first, then shorter names before longer ones. Completion starts once the name is as long as the *complete after* setting (one character by default).
- a string literal: a file or directory, so `load "ri` completes to `load "ring.m2`.
- a comment: nothing.

Macaulay2 reports its names as it starts and after every cell, without evaluating anything extra, so a name bound with `=` or a top-level `:=`, or exported by a package just loaded, completes from the next keystroke on.

Commands and their files complete too; see [Commands and pickers](commands.md).

## Selection and clipboard

Hold `Shift` with `Left`, `Right`, `Home`, or `End` to extend an input selection. `Ctrl+Y` copies it. Typing or deleting replaces the selected range.

You can also drag over input, feed, code previews, or syntax trees with the mouse. Releasing a real drag copies the selectable text through OSC 52. UI markers, borders, type annotations, and picker rows are excluded where appropriate so copied text stays useful.
