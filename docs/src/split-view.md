# Split view

Spectral runs one Macaulay2 session. For a second, independent session, open another terminal pane — tmux, a terminal split, or a new window — and start `sp2` there.

Split view instead shows another view of the current session beside it.

## Open a view beside the session

Choose `code`, `values`, `stdout`, `output`, or `ast` in the command picker, or press its key: the view opens in a pane to the right of the shell, and focus stays in the shell so you can keep evaluating while the view follows.

The preview pane keeps five cyclic tabs. Every newly opened view occupies the next tab; after all five have been used, the next opening replaces the oldest one. Click a tab at the top to revisit it. The active tab is highlighted, and each retained view keeps its scroll position and folded content.

Click **☆** to move the current view into the separate **★** bookmark tab. That view stays outside cyclic replacement until another view is bookmarked. Any kind of view can be bookmarked: documentation, files, code, values, printed output, or syntax trees.

In the preview pane, `Left` and `Right` switch between occupied recent tabs. `Ctrl+Left` and `Ctrl+Right` do the same from either pane, while the shell's plain arrows move through its input. Click **‹**, or press `Ctrl+Shift+B`, to go to the previous recent tab. The shortcut can be changed under **Settings → keymaps**. `Alt+Left` and `Alt+Right` remain available to the terminal or tmux.

Middle-click a documentation or source-file link in either pane to open it in a new preview tab. A value that is a source position (a `FilePosition`, such as the result of `locate`) is drawn as such a link in the output too: it shows the file's name and position, and clicking it opens the source at that position. A file opened with `/view` also occupies a preview tab and displays highlighted source without running it; `Ctrl+Enter` runs a file shown there.

`Ctrl+Left click` submits code to the live session: `help "key"` for documentation, or `get "path"` for a source file. The command and its result become part of the feed. Opening content without submitting code uses the preview pane instead. `viewHelp` continues to open the browser.

Links underline when the pointer rests on them. Method signatures link to their own documentation or to the alias named beside them, and source locations link to files when M2 can resolve them. Descriptions and inline code retain their highlighting across wrapped lines. The reference view has no enclosing output border. Pickers appear only in the focused pane.

Under **Settings → rendering**, enable **swap reference backgrounds** to use the feed background for reference code. The panel canvas and syntax colours stay the same.

The session previews show the session's committed cells with its own scroll position, so scrolling a view never moves the shell feed. Each new input or result brings every view back to its newest content.

Both panes share one statusline. Assign components to its five fixed segments under **Settings → statusline**; the *view*, *session state*, and *key hints* components follow the focused pane and current interaction. Empty positions give their neighbours more space without moving their anchors.

## Focus, resize, and close

Click a pane to focus it. The shell's input keeps the plain arrows, while `Ctrl+Left` and `Ctrl+Right` change the preview tab from either pane, so there is rarely a reason to leave the shell. On a wide terminal, drag the divider to resize the split.

Each visible pane keeps at least 40 columns, so the split needs 81 columns including the divider. Below that width Spectral keeps both panes but displays one at a time: the view takes the full width when it opens or the window narrows, and closing it brings the shell back.

`Ctrl+Q` closes the view from either pane: the view beside the session closes and focus returns to the shell.
