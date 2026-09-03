# Workspaces and split view

Spectral supports two panes and up to two independent workspaces.

## Open a second pane

Choose `new`, `code`, `output`, or `syntax tree` in the command picker with `Alt+V`:

- `new` starts another Macaulay2 process with its own state, transcript, draft, history, and committed syntax trees;
- the three previews remain linked to the focused workspace's committed cells but have their own view and scroll position.

Ordinary `Enter` opens the chosen item in the focused pane. In a preview, `Ctrl+Q` or `Ctrl+C` returns to the shell; if the preview is the second pane, leaving it closes that pane.

## Focus and resize

Use `Ctrl+Left` and `Ctrl+Right` to focus the left or right pane. On a wide terminal, drag the divider or use `Ctrl+Shift+Left` and `Ctrl+Shift+Right` to resize it one column at a time.

Each visible pane keeps at least 40 columns. Two side-by-side panes therefore require 81 columns including the divider. Below that width Spectral keeps both panes alive but displays only the focused one. `Ctrl+Left` and `Ctrl+Right` then toggle which workspace or view is visible.

Settings, keymaps, and style are application-wide: changing them in one workspace updates the other. Evaluation state and feed contents remain independent.

## A practical split

One pane can develop an object while the other tests a separate idea:

```m2
R = QQ[a..d]
I = minors(2, genericMatrix(R, 2, 2))
```

Or keep the shell on the left and open its syntax tree or collected outputs on the right. Because preview navigation is independent, scrolling the preview never moves the shell feed.
