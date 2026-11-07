# Key reference

## Shell

| Key | Action |
| --- | --- |
| `Enter` | Submit complete input, or continue an incomplete construct. |
| `Shift+Space`, `Shift+Enter`, `Alt+Enter`, `Ctrl+J` | Insert a newline. `Ctrl+J` is the newline character itself, which some terminals send for `Shift+Enter`. |
| `Up`, `Down` | Move vertically within multiline input. At its boundaries, browse history only while recalling untouched entries; editing stops boundary history navigation. Single-line input searches history by prefix. |
| `Ctrl+Up`, `Ctrl+Down` | Browse all input history from any line, even after editing a recalled entry. Returning past the newest entry restores the draft and cursor. |
| `PageUp`, `PageDown` | Scroll the feed. |
| `Ctrl+Left`, `Ctrl+Right` | Move the cursor by a word; with a view open beside the session, switch preview tabs instead. |
| `Ctrl+Backspace`, `Ctrl+Delete` | Delete what `Ctrl+Left` or `Ctrl+Right` would move over. `Alt` works the same, and so does `Ctrl+H`, which some terminals send for `Ctrl+Backspace`. |
| `Ctrl+Z`, `Ctrl+Shift+Z` | Undo or redo an edit to the input. |
| `Home`, `End` | Move to the start or end of the line. |
| `Ctrl+Home`, `Ctrl+End` | Move to the start or end of the whole input. |
| `Shift+Left`, `Shift+Right` | Extend the input selection; with `Ctrl`, by a word. |
| `Shift+Home`, `Shift+End` | Extend selection to a line boundary; with `Ctrl`, to the start or end of the input. |
| `Ctrl+A` | Select the whole input. |
| `Ctrl+Y` | Copy the selected text, in the input or from a mouse selection. |
| `Tab` | Open the completion list, inserting a visible hint immediately; otherwise insert on the next press, then cycle candidates. |
| `Ctrl+Tab`, `Shift+Tab` | Cycle completion candidates backwards. |
| `Alt+Tab` | Insert a tab. |
| `Ctrl+U` | Delete the whole current line as one undoable edit. |
| `Ctrl+L` | Clear the whole input as one undoable edit. |
| `Ctrl+Shift+S` | Save selected code as a reusable session. Select setup lines in the code view to keep just that starting point. |
| `F7` | Browse saved sessions; Enter previews the selected session. |
| `Ctrl+Enter` | Restart Macaulay2 and run the file currently shown in the reference pane. |
| `Ctrl+/` | Toggle commands. |
| `Ctrl+O` | Toggle settings. |
| `Ctrl+K` | Open the keymaps tab in Settings; close it when already there. |
| `Alt+L` | Clear the feed. |
| `F2` … `F6` | Show code, values, printed output, all output, or the syntax tree beside the session. |
| `Ctrl+N` | Restart Macaulay2. |
| `Ctrl+R` | Run the previous session's code again. |
| `Ctrl+Q` | Close the view, from whichever pane has focus. |
| `Ctrl+Shift+D` | Toggle highlight debug. |
| `Ctrl+C` | Cancel running computations and queued work. When idle, discard incomplete input or the current draft; quit when both are empty. |

## Split view and previews

| Key | Action |
| --- | --- |
| Click | Focus a pane; drag the divider to resize the split. |
| `Up`, `Down`, `PageUp`, `PageDown` | Scroll a preview. |
| `Left`, `Right` | Switch to the previous or next occupied recent preview tab. |
| `Ctrl+Left`, `Ctrl+Right` | Switch preview tabs from either pane, while the shell's plain arrows move through its input. |

| `Ctrl+Shift+B` | Return to the previous recent preview tab. |

The preview header shows five cyclic tabs and one bookmark tab. Click **☆** to bookmark the active view, then **★** to revisit it after other tabs have been replaced.

## Pickers

| Key | Action |
| --- | --- |
| `Up`, `Down` | Change the selected row. |
| `Left`, `Right` | Adjust a settings row. |
| `0`–`5` on a statusline component | Hide it (`0`) or assign it to one of the five segments (`1`–`5`). |
| `Tab`, `Shift+Tab`, `Ctrl+Left`, `Ctrl+Right` | Switch settings tabs. |
| `Left`, `Right` on the row of tabs | Switch settings tabs. |
| `Enter` | Open or run the selected row; a view opens beside the session. On a settings row, close every picker. |
| `Esc` | Close the current picker or debug overlay. |

## Documentation links

| Mouse action | Action |
| --- | --- |
| Middle click | Open a documentation page or source file in the right reference pane. |
| `Ctrl+Left click` | Submit `help "key"` or `get "path"` to the session, recording the command and result in the feed. |

These actions work on links in both panes. Opening a source file in a preview does not evaluate it; opening it in the feed submits `get` to read its contents without executing the file.
