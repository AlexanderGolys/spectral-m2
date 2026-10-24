# Key reference

## Shell

| Key | Action |
| --- | --- |
| `Enter` | Submit complete input, or continue an incomplete construct. |
| `Shift+Space`, `Shift+Enter`, `Alt+Enter`, `Ctrl+J` | Insert a newline. `Ctrl+J` is the newline character itself, which some terminals send for `Shift+Enter`. |
| `Up`, `Down` | Move vertically or recall history at the input boundary. |
| `PageUp`, `PageDown` | Scroll the feed. |
| `Ctrl+Left`, `Ctrl+Right` | Move the cursor by a word. |
| `Ctrl+Backspace`, `Ctrl+Delete` | Delete what `Ctrl+Left` or `Ctrl+Right` would move over. `Alt` works the same, and so does `Ctrl+H`, which some terminals send for `Ctrl+Backspace`. |
| `Ctrl+Z`, `Ctrl+Shift+Z` | Undo or redo an edit to the input. |
| `Home`, `End` | Move to the start or end of the line. |
| `Ctrl+Home`, `Ctrl+End` | Move to the start or end of the whole input. |
| `Shift+Left`, `Shift+Right` | Extend the input selection; with `Ctrl`, by a word. |
| `Shift+Home`, `Shift+End` | Extend selection to a line boundary; with `Ctrl`, to the start or end of the input. |
| `Ctrl+Y` | Copy selected input or screen text. |
| `Tab` | Take the completion shown after the cursor; again, the next one. |
| `Shift+Tab` | Take the previous completion. |
| `Alt+Tab` | Insert a tab. |
| `Ctrl+L` | Clear the input. |
| `Ctrl+/` | Toggle commands. |
| `Ctrl+O` | Toggle settings. |
| `Ctrl+K` | Toggle keymaps. |
| `Alt+L` | Clear the feed. |
| `F2` … `F6` | Show code, values, printed output, all output, or the syntax tree beside the session. |
| `Ctrl+N` | Restart Macaulay2. |
| `Ctrl+R` | Run the previous session's code again. |
| `Ctrl+Q` | Close the view, from whichever pane has focus. |
| `Ctrl+Shift+D` | Toggle highlight debug. |
| `Ctrl+C` | Quit, from whichever pane has focus. |

## Split view and previews

| Key | Action |
| --- | --- |
| `Alt+Left`, `Alt+Right` | Focus a pane, including in a collapsed narrow split. |
| `Alt+Shift+Left`, `Alt+Shift+Right` | Resize a visible split. |
| `Up`, `Down`, `PageUp`, `PageDown` | Scroll a preview. |
| `Left`, `Right` | Step to the previous or next view: code, values, stdout, output, syntax tree. |

## Pickers

| Key | Action |
| --- | --- |
| `Up`, `Down` | Change the selected row. |
| `Left`, `Right` | Adjust a settings row. |
| `Tab`, `Shift+Tab` | Switch settings tabs. |
| `Enter` | Open or run the selected row; a view opens beside the session. |
| `Esc` | Close the current picker or debug overlay. |
