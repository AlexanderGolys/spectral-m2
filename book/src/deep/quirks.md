# Limitations and quirks

Things that may surprise you, and why they are the way they are.

One submission at a time
: While M2 is busy, submitting more code shows a diagnostic instead of queueing it. Cells you submit together do queue, and run in order.

Errors discard what came after
: When a cell fails, the cells submitted after it in the same submission are not run. They stay visible, muted, so you can see what was skipped.

Typeset maths needs the right terminal
: Pictures need the kitty graphics protocol with Unicode placeholders, which kitty and Ghostty provide. Elsewhere values are drawn in Unicode, and the *maths* setting marks `typeset` unavailable. Inside tmux it needs [passthrough](../troubleshooting.md#typeset-tmux).

The terminal chooses the fonts
: With *math fonts* on, `ℤ` is a character; how it looks depends on which font your terminal takes it from. See [Troubleshooting](../troubleshooting.md#blackboard).

Not every value has a picture
: Strings, names, `null` and nets stay terminal text even with typeset maths, and a formula RaTeX cannot set is drawn in Unicode. The icon before the output's label says what actually drew it.

Big values are previewed
: Plain-text values beyond 256 KiB or 262,144 cells show their beginning and end only. The whole value is still in M2.

Narrow terminals show one pane
: A view beside the session needs 81 columns; below that both panes stay open but show one at a time.

Semantic colours read source only
: `m2-ls` analyses the code you ran without evaluating it, so names created dynamically may keep their syntax colour.

stderr is hidden while the screen is drawn
: So it cannot garble the display. A crash is written to `crash.log` instead; see [When things go wrong](../shell/errors.md#if-the-shell-itself-stops).
