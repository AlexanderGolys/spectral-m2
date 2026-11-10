# Views beside the session

Open a second look at the same session beside it: just your code, just your values, just what was printed, or the syntax tree. The view follows the session as you keep working, and scrolling it never moves your feed.

<!-- capture: screenshot of the shell with the code view (F2) open beside it. -->

## The five views

`/code`, `F2`
: Your committed input as one highlighted file, without pending or rejected code. The place to select setup lines worth [keeping](sessions.md#keep-a-starting-point).

`/values`, `F3`
: Returned values with their types.

`/stdout`, `F4`
: Printed text alone, each piece numbered by the cell that printed it.

`/output`, `F5`
: Printed text and values together, without the code.

`/ast`, `F6`
: The syntax tree of every committed cell, opening at the newest: highlighted source between two rules, then its tree. Node types come before field labels, brackets enclose what they hold, and source ranges line up in one column.

A view opens to the right of the shell and focus stays in the shell, so you can keep evaluating while it follows. Each new input or result brings every view back to its newest content.

## Tabs { #tabs }

The pane keeps five tabs that rotate: each newly opened view, documentation page or file takes the next one, replacing the oldest once all five are used. Each tab keeps its scroll position and folds. Click a tab to go back to it.

Click **☆** to move the current tab into the **★** bookmark tab, which never rotates out until you bookmark something else.

`Left` and `Right` switch tabs when the pane has focus; `Ctrl+PageUp` and `Ctrl+PageDown` do it from either pane, so the shell's arrows stay free for the input. `Ctrl+Shift+B` returns to the previous tab, and `Ctrl+Q` closes the view from either pane.

## Focus and size

Click a pane to focus it, and drag the divider to resize. Pickers appear only in the focused pane, and the status line names the view and its tab.

Each pane keeps at least 40 columns, so a split needs 81. In a narrower terminal both panes stay open but show one at a time: the view takes the full width when it opens, and closing it brings the shell back.

> [!NOTE]
> Views show the one session they sit beside. For a second, independent session, open another terminal pane, a tmux split or a new window, and start `sp2` there. History is shared between them.
