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

The pane has two header rows. The first names what is showing, centred, with **‹** (back) and **›** (forward) at its start. The second holds one icon per tab:

- On the left, the tabs you opened: documentation pages, files, and descriptions of names from the [language server](language-server.md#jump-from-a-name). There is no limit to them. When there are more than fit, the row shows the ones around the tab showing, and the rest are reached through history.
- On the right, the views above, always there.

Click an icon to go to its tab. `Ctrl+Q` closes the tab showing and moves to the one beside it; closing the last one hides the pane.

### Open links

A link opens in one of three ways:

| Click | Opens the link |
| --- | --- |
| `Ctrl`+click | in the tab showing, replacing its page |
| Middle-click | in a new tab to the right, staying where you are |
| `Ctrl`+middle-click | in a new tab to the right, and goes to it |

It works the same on links in the feed and in the pane; from the feed the pane opens if it was hidden. `Enter` on a link under the [view cursor](select.md) opens it in the tab showing.

### History

`Ctrl+O` goes back to what the pane showed before and `Ctrl+I` forward again, across all tabs: a page replaced in its tab comes back in that tab, and a tab you closed is opened again. In a terminal that sends `Ctrl+I` as `Tab`, use `Ctrl+Shift+O` to go forward.

Every page keeps its scroll position and folds, and a page is fetched once: opening the same documentation or file in a second tab shows the same page, scrolled where you left it. Pages far back in history are let go and fetched again when you return to them.

### Move between the panes

`Shift+Tab` moves between the session and the pane, showing the pane where you left it if it was hidden. `Ctrl+B` hides the pane and shows it again, keeping its tabs. While completion candidates or a picker are open, `Shift+Tab` steps back through those instead.

`Left` and `Right` step along the tab row, on the same side as the tab showing, when the pane has focus; `Ctrl+PageUp` and `Ctrl+PageDown` do it from either pane, so the shell's arrows stay free for the input.

## Focus and size

Click a pane to focus it, and drag the divider to resize. Pickers appear only in the focused pane, and the status line names the view and its tab.

Each pane keeps at least 40 columns, so a split needs 81. In a narrower terminal both panes stay open but show one at a time: the view takes the full width when it opens, and closing it brings the shell back.

> [!NOTE]
> Views show the one session they sit beside. For a second, independent session, open another terminal pane, a tmux split or a new window, and start `sp2` there. History is shared between them.

## Notes { #notes }

Keep a list of ideas or computations beside your work with `/notes`. Click inside the notebook to edit it; `Enter` inserts a line, the input's editing shortcuts provide selection and undo, and paste inserts text. `Ctrl+Y` copies and `Ctrl+X` cuts the selection. `Esc` returns focus to the shell; `Ctrl+Q` closes the pane.

Changes are saved automatically to `notes.md` in the active session storage directory. The *local session files* setting gives each repository its own notebook. Notes are text and are never evaluated by Macaulay2.
