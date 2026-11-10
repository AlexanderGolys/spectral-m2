# Select and copy

Whatever you copy comes back as something you can use: a typeset matrix copies as the M2 code that makes it, a value that only wrapped on screen copies as one line, and a whole cell can copy as a transcript ready to paste into a paper or an issue.

<!-- capture: GIF of double-clicking a typeset matrix, then pasting `matrix {{x_1, 1/2}, ...}` into the input. -->

## Drag, double-click, triple-click

Drag over the input, the feed or a view to select. Releasing the button copies the selection to the clipboard (through OSC 52), and it stays marked until your next click or key. Drag against the top or bottom edge to keep scrolling. `Ctrl+Y` copies it again.

Double-click takes a piece at once:

- a word, such as a variable name;
- a bracket, together with everything up to its partner: `(x_1, x_2)` or a whole `{...}` list, across lines too;
- a typeset formula, as the Macaulay2 code it shows.

Triple-click takes the paragraph: the lines around the click up to the blank ones either side.

In the input, `Shift` with the arrows, `Home` or `End` extends a selection, and `Ctrl+A` selects everything. Typing replaces what is selected.

## Only the text you mean

A selection reads text only. Cell numbers, gutters, borders and picker rows are left out, code inside a documentation page copies with its own indentation, and blank lines inside printed text are kept.

A value and its type select apart: a drag that starts on a type reads only types, and one that starts anywhere else reads no types.

Typeset maths copies as code you can paste back: a matrix shown as a picture copies as `matrix {{x_1, 1/2}}`, not as its TeX. Rows held together by a tall formula read back as one line.

## Whole cells and results

Click outside any text, in the margin, a gutter or the blank space after a line: a double-click takes the whole cell or result under it, and a triple-click everything its submission ran and returned. The piece is tinted and copied.

From the keyboard, `Alt+Up` and `Alt+Down` do the same, starting from the last cell or result on screen. While one is selected, `Up` and `Down` step to the previous and next, `y` or `Ctrl+Y` copies, and `Esc` lets go; any other key lets go and types as usual.

The *copy cells as* setting decides what such a piece copies as:

`raw text` (default)
: The code as typed and the value as printed.

`session`
: As a Macaulay2 transcript, with prompts:

  ```m2
  i1 : R = QQ[x, y]

  o1 = R

  o1 : PolynomialRing
  ```

The status line says what each copy took.

## When you want every character

Two [selection settings](../reference/settings.md#selection) change the defaults: *copy on select* off leaves a finished selection marked for `Ctrl+Y` instead of copying it, and *raw selection* on takes every character as drawn, frames, cell numbers and wrapped rows included. Typeset pictures still copy as code.
