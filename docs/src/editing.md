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

While you type, the prompt changes to its typing form, and it returns to the resting prompt shortly after the last keystroke that changed the input. Moving the cursor leaves the prompt at rest, and the text stays in place as the prompt changes.

## Pending and completed cells

Submitted code moves into the feed immediately. As Macaulay2 answers, Spectral appends printed text, values, types, warnings, or errors; consecutive values share one border. A value's type stands at the right edge, and a long value wraps short of it; a type wider than a third of the box goes below the value instead. If a cell fails, its code and the unevaluated cells below it sit on a red background; the unevaluated code is muted without syntax highlighting. The error box puts “Error” in its title and shows the message in light red. Cell numbers remain session metadata but are not displayed until cells become focusable.

Only one submission can evaluate in a workspace at a time. Trying to submit another while it is busy produces an input diagnostic and does not disturb the running evaluation.

Semicolons remain part of their cell, so M2 still controls whether a value is printed:

```m2
R = QQ[x, y];
I = ideal(x^2, x*y, y^2);
degree I
```

## Documentation

Plain-text results such as `net help "arithmetic with complex maps"` use a bounded preview: at most 256 KiB of source text and 262,144 display cells per value. Larger previews retain their beginning and end with a truncation notice. Expanding a result keeps these limits; the full value remains available in Macaulay2.

`help` pages are drawn in the feed: the title over a rule, its name apart from its brief; the usage, inputs and outputs as headers over their entries; and each example as the session draws code and values, without its numbers. A link to another page is set in italics in the colour of printed output (the `MarkupLink` group), while one written as code keeps the colours of its code, and is underlined while the pointer is over it, and Ctrl+click opens that page in a separate reference pane. Links in that pane open there too. Documentation lookups use their own Macaulay2 process, leaving your draft, transcript, input numbers, and variables alone. Close the pane with `Ctrl+Q`.

## History and navigation

`Up` and `Down` move inside multiline input. At the first or last line they recall older or newer submissions. As in a shell's history search, the text before the cursor when you start browsing is a prefix: typing `R =` and pressing `Up` steps only through submissions beginning with `R =`, each distinct one once. Editing a recalled entry starts a new search from what the input then begins with. Stepping past the newest match restores what you had typed. History is shared by every running session: what one submits, the others recall the next time they start browsing. Commands chosen after typing `/` are recorded in the same history, under their whole name however much of it was typed; opening the full command picker with `Ctrl+/` does not add a history entry.

`Ctrl+Up` and `Ctrl+Down` browse all history from any line without prefix filtering. They continue through history after edits to a recalled entry, and stepping past the newest entry restores the original draft and cursor.

The usual editing keys are available for horizontal movement, `Ctrl` with the arrows for words, `Home` and `End` for line boundaries (`Ctrl` for the whole input), and deletion. `Ctrl+Backspace` and `Ctrl+Delete` delete exactly what `Ctrl+Left` and `Ctrl+Right` move over. `Ctrl+U` deletes the whole current line. `Ctrl+L` clears the whole input. Both operations undo as a single edit.

If you submitted an unfinished construct by mistake, press `Ctrl+C` to discard its pending continuation and draft while keeping the session alive. During evaluation, `Ctrl+C` interrupts the computation and cancels queued work. With no computation, continuation, or draft, it quits.

`Ctrl+Z` undoes the last edit to the input and `Ctrl+Shift+Z` redoes it. Typing undoes a word at a time and a run of deletions all at once; clearing, pasting, recalling history and taking a completion each undo as one step. Submitting starts a fresh input with nothing to undo.

## Recovering from errors

An ordinary runtime error reports its diagnostic and returns to normal input. Variables and rings created before the error remain available; code queued after the failing cell is discarded. Automatic debugger entry is disabled by default. Start with `sp2 --debugger` when you deliberately want Macaulay2 to enter its debugger on errors. `--debug N` controls warning diagnostics and does not enable the debugger.

## Completion

As you type, an inline hint shows the rest of the preferred completion only when there are fewer candidates than *hint completion below* (10 by default). `Tab` opens the candidate list above the input; if a hint was visible, it also inserts that completion immediately. Otherwise the next `Tab` inserts the first candidate. Further presses cycle through the list, and `Ctrl+Tab` or `Shift+Tab` cycles backwards. `Esc` dismisses the list. A name Macaulay2 already knows is not completed. `Alt+Tab` inserts a tab. What completes depends on where the cursor is, as Macaulay2 reads the code before it:

- a name: every symbol Macaulay2 knows, the names its own terminal completes. Frequently used symbols come first, with shorter names and then alphabetical order breaking ties. Completion starts once the name is as long as the *complete after* setting (one character by default).
- a string literal: a file or directory, so `load "ri` completes to `load "ring.m2`.
- a comment: nothing.

Macaulay2 reports its names as it starts and after every cell, without evaluating anything extra, so a name bound with `=` or a top-level `:=`, or exported by a package just loaded, completes from the next keystroke on.

Usage counts references to known global symbols in successfully evaluated source. Counts for names introduced during the session, such as a ring named `R`, affect completion order only in that session and are discarded on restart. Counts for symbols known when Macaulay2 starts are saved in `completion-usage.json` beside the settings and retained across restarts. Function parameters, loop variables, exception bindings, and names introduced by `:=` or `local` do not add to a symbol's count. Comments, strings, assignment targets, and choosing or cycling a completion do not count either.

Commands and their files complete too; see [Commands and pickers](commands.md).

## Selection and clipboard

Hold `Shift` with `Left`, `Right`, `Home`, or `End` to extend an input selection, or press `Ctrl+A` to select all of it. `Ctrl+Y` copies it. Typing or deleting replaces the selected range.

Drag over the input, the feed, or a preview to select with the mouse. Releasing the button copies the text through OSC 52, and the selection stays marked until the next click or keystroke; new output in the feed lets it go as well. `Ctrl+Y` copies it again. Drag against the top or bottom edge to scroll on, so a selection can reach past what fits on screen. Only text is read: UI markers, gutters, borders, and picker rows are left out, so copied text stays useful, and code inside a documentation page copies with its own indentation and no more.

A value and its type select apart: a drag that starts on a type annotation reads only types, and one that starts anywhere else reads no types.

Double-click to select and copy a piece at once:

- a word, such as a variable name;
- a bracket, to take everything up to its partner -- `(x_1, x_2)` or a whole `{...}` list, across lines too;
- a typeset formula, which copies as the Macaulay2 code it shows.

Typeset maths always copies as code you can paste back: a matrix shown as a picture copies as `matrix {{x_1, 1/2}}`, not as its TeX. Rows held together by a tall formula read back as one line, its tall brackets as single ones.

To copy by hand instead, or to take any character exactly as it is drawn, see the [selection settings](settings.md).
