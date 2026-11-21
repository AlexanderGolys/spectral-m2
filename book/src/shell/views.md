# Views beside the session

Open a second look at the same session beside it: just your code, just its output, or the syntax tree. The view follows the session as you keep working, and scrolling it never moves your feed. Documentation pages and files you open land in the same pane, each in a tab of its own.

<!-- capture: screenshot of the shell with the code view (F2) open beside it, its two header rows visible. -->

## The three views

`/code`, `F2`
: Your committed input as one highlighted file, without pending or rejected code. The place to select setup lines worth [keeping](sessions.md#keep-a-starting-point).

`/output`, `F5`
: What the session answered, without the code. The *output view* setting in the [side panel settings](../reference/settings.md#side-panel) chooses what: `everything` (printed text, values and messages, the default), only returned `values` with their types, or only `printed` text, each piece numbered by the cell that printed it.

`/ast`, `F6`
: The syntax tree of every committed cell, opening at the newest: highlighted source between two rules, then its tree. Node types come before field labels, brackets enclose what they hold, and source ranges line up in one column.

Code wider than the pane wraps as it does in the shell, or is cut at the right edge when *code overflow* in the [side panel settings](../reference/settings.md#side-panel) is `trim`.

A view opens to the right of the shell and focus stays in the shell, so you can keep evaluating while it follows. Each new input or result brings every view back to its newest content. Opening the view already showing closes it again.

## Tabs { #tabs }

The pane has two header rows. The first names what is showing, centred, with **‹** (back) and **☆** (bookmark) at its start. The second holds one icon per tab:

- On the left, the tabs you opened: documentation pages, files, and descriptions of names from the [language server](language-server.md#jump-from-a-name). They rotate through five slots, so the next one replaces the oldest once all five are used. Click **☆** to move the current one into the **★** bookmark tab, which never rotates out until you bookmark something else.
- On the right, the three views above, always there, each with its own tab that never takes one of your five.

Every tab keeps its scroll position and folds. Click an icon to go to it.

`Left` and `Right` step through the tabs on the same side as the one showing when the pane has focus; `Ctrl+PageUp` and `Ctrl+PageDown` do it from either pane, so the shell's arrows stay free for the input. `Ctrl+Shift+B` returns to the previous page or file, and `Ctrl+Q` closes the view from either pane.

## Focus and size

Click a pane to focus it, and drag the divider to resize. Pickers appear only in the focused pane, and the status line names the view and its tab.

Each pane keeps at least 40 columns, so a split needs 81. In a narrower terminal both panes stay open but show one at a time: the view takes the full width when it opens, and closing it brings the shell back.

> [!NOTE]
> Views show the one session they sit beside. For a second, independent session, open another terminal pane, a tmux split or a new window, and start `sp2` there. History is shared between them.

## Notes { #notes }

Keep a list of ideas or computations beside your work with `/notes`. Click inside the notebook to edit it; `Enter` inserts a line, the input's editing shortcuts provide selection and undo, and paste inserts text. `Ctrl+Y` copies and `Ctrl+X` cuts the selection. `Esc` returns focus to the shell; `Ctrl+Q` closes the pane.

Changes are saved automatically to `notes.md` in the active session storage directory. The *local session files* setting gives each repository its own notebook. Notes are text and are never evaluated by Macaulay2.
