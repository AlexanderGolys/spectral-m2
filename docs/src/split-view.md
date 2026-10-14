# Split view

Spectral runs one Macaulay2 session. For a second, independent session, open another terminal pane — tmux, a terminal split, or a new window — and start `spectral-m2` there.

Split view instead shows another view of the current session beside it.

## Open a view beside the session

Choose `code`, `output`, or `syntax tree` in the command picker, or press its key: the view opens in a pane to the right of the shell, and focus stays in the shell so you can keep evaluating while the view follows.

- Opening a different view replaces the one beside the session.
- Opening the view that is already shown closes the split.
- `Alt+V` on a selected view row opens it in place of the focused pane instead.

Every view shows the session's committed cells with its own scroll position, so scrolling a view never moves the shell feed. Each new input or result brings every view back to its newest content.

Both panes share one status line along the bottom: the session's state on the left, the view beside it in the middle, and the keys of the focused pane on the right.

## Focus, resize, and close

Use `Ctrl+Left` and `Ctrl+Right` to focus the shell or the view, or click a pane. On a wide terminal, drag the divider or use `Ctrl+Shift+Left` and `Ctrl+Shift+Right` to resize it one column at a time.

Each visible pane keeps at least 40 columns, so the split needs 81 columns including the divider. Below that width Spectral keeps both panes but displays one at a time: the view takes the full width when it opens or the window narrows, and `Ctrl+Left` and `Ctrl+Right` switch between it and the shell.

`Ctrl+Q` closes the view from either pane: the view beside the session closes, and a view shown in place gives way to the shell. In a view, `Ctrl+C` leaves it the same way instead of quitting Spectral.
