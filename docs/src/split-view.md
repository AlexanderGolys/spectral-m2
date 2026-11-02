# Split view

Spectral runs one Macaulay2 session. For a second, independent session, open another terminal pane — tmux, a terminal split, or a new window — and start `sp2` there.

Split view instead shows another view of the current session beside it.

## Open a view beside the session

Choose `code`, `values`, `stdout`, `output`, or `ast` in the command picker, or press its key: the view opens in a pane to the right of the shell, and focus stays in the shell so you can keep evaluating while the view follows.

- Opening a different view replaces the one beside the session.
- Opening the view that is already shown closes the split.
- In a view, `Left` and `Right` step to the previous or next one, wrapping around: code, values, stdout, output, syntax tree. `Ctrl+Left` and `Ctrl+Right` do the same from either pane, so the view can be changed without leaving the shell. While the view has focus, the middle of the status line lists all five with the current one highlighted.

Every view shows the session's committed cells with its own scroll position, so scrolling a view never moves the shell feed. Each new input or result brings every view back to its newest content.

Each pane has its own status line. The shell's shows where the session works and what it is doing, the view's names the view, and only the pane with focus lists the keys it answers. The pane with focus draws its status line in the theme's status-line colours; the other pane's is drawn in quieter, inactive colours, so you can always tell which pane your keys reach. An open picker appears in the pane with focus.

## Focus, resize, and close

Click a pane to focus it. The shell's input keeps the plain arrows, while `Ctrl+Left` and `Ctrl+Right` change the view from either pane, so there is rarely a reason to leave the shell. On a wide terminal, drag the divider to resize the split.

Each visible pane keeps at least 40 columns, so the split needs 81 columns including the divider. Below that width Spectral keeps both panes but displays one at a time: the view takes the full width when it opens or the window narrows, and closing it brings the shell back.

`Ctrl+Q` closes the view from either pane: the view beside the session closes and focus returns to the shell.
