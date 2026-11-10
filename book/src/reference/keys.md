# Default keys

Every key below can be [rebound](../custom/keymaps.md) unless marked *fixed*. The groups are the contexts of the keymaps picker.

## Global

| Key | Action |
| --- | --- |
| `Enter` | Evaluate the input, or add a line to an unfinished construct. *Fixed.* |
| `Ctrl+C` | Cancel the computation and queued work; when idle, discard a continuation or draft, then quit. |
| `Ctrl+/` | Open the commands. |
| `Ctrl+O` | Open the settings. |
| `Ctrl+K` | Open the keymaps. |
| `Alt+L` | Clear the feed. |
| `F2` … `F6` | Show code, values, printed output, all output, or the syntax tree beside the session. |
| `F7` | Browse saved sessions. |
| `F8` | Restart and run the viewed file. |
| `Ctrl+N` | Restart Macaulay2. |
| `Ctrl+R` | Run the previous session's code. |
| `Ctrl+Q` | Close the view. |
| `Ctrl+Shift+B` | Return to the previous tab of the view. |
| `Ctrl+Shift+S` | Save the selected code to a file. |
| `Ctrl+Y` | Copy the selection. |
| `Alt+Up`, `Alt+Down` | Select the previous or next cell or result. |
| `Alt+K` | Tell what the name at the cursor is, in a panel beside it. |
| `Alt+Shift+K` | Describe the name at the cursor beside the session. |
| `Ctrl+]` (`Ctrl+5`) | Go to where the name at the cursor is defined. |
| `Alt+Shift+F` | Format the input. |
| `PageUp`, `PageDown` | Scroll the feed. *Fixed.* |
| `Ctrl+Shift+D` | Toggle the highlight inspector. *Fixed.* |

## Movement

| Key | Action |
| --- | --- |
| `Left`, `Right` | Move left or right. |
| `Up`, `Down` | Move up or down; at the first or last line, recall older or newer input. |
| `Ctrl+Left`, `Ctrl+Right` | Move by a word. |
| `Home`, `End` | Move to the start or end of the line. |
| `Ctrl+Home`, `Ctrl+End` | Move to the start or end of the input. |

## Selection

| Key | Action |
| --- | --- |
| `Shift+Left`, `Shift+Right` | Extend the selection. |
| `Ctrl+Shift+Left`, `Ctrl+Shift+Right` | Extend the selection by a word. |
| `Shift+Home`, `Shift+End` | Extend the selection to the line's start or end. |
| `Ctrl+Shift+Home`, `Ctrl+Shift+End` | Extend the selection to the input's start or end. |
| `Ctrl+A` | Select the whole input. |

## Editing

| Key | Action |
| --- | --- |
| `Shift+Enter` | Insert a line break. `Alt+Enter`, `Shift+Space` and `Ctrl+J` do the same, for terminals that cannot report `Shift+Enter`. |
| `Alt+Tab` | Insert a tab. |
| `Backspace`, `Delete` | Delete before or under the cursor. |
| `Ctrl+Backspace`, `Ctrl+Delete` | Delete back or on by a word. `Alt` works too, and so does `Ctrl+H`, which some terminals send for `Ctrl+Backspace`. |
| `Ctrl+U` | Delete the current line. |
| `Ctrl+L` | Clear the input. |
| `Ctrl+Z`, `Ctrl+Shift+Z` | Undo, redo. |

## Recall

| Key | Action |
| --- | --- |
| `Tab` | Show completion candidates, take the hint, or take the next candidate. |
| `Shift+Tab`, `Ctrl+Tab` | Take the previous candidate. |
| `Ctrl+Up`, `Ctrl+Down` | Recall older or newer input, regardless of the draft. |

## In views and pickers

These follow the pane or picker that has focus and are not in the keymaps:

| Key | Where | Action |
| --- | --- | --- |
| `Up`, `Down`, `PageUp`, `PageDown` | view | Scroll. |
| `Left`, `Right` | view | Switch to the previous or next tab. |
| `Ctrl+PageUp`, `Ctrl+PageDown` | either pane | Switch the view's tab. |
| `Up`, `Down` | picker | Select a row. |
| `Left`, `Right` | settings | Change a value; on the row of tabs, switch tab. |
| `Tab`, `Shift+Tab` | settings | Switch tab. |
| `0` … `5` | statusline row | Hide the component, or put it in segment 1 to 5. |
| `Enter` | picker | Run or open the selected row. |
| `Esc` | picker, inspector | Close it. |

## Mouse

| Action | Effect |
| --- | --- |
| Drag | Select; releasing copies. |
| Double-click | Select a word, a bracketed block or a formula; outside text, a whole cell or result; on a folded value, expand it. |
| Triple-click | Select a paragraph; outside text, a whole submission. |
| Middle-click a link | Open it in the view beside the session. |
| `Ctrl`+click a link | Submit `help` or `get` for it to the session. |
| Click an output's icon | Draw that value the next available way. |
| Click a pane | Focus it; drag the divider to resize. |
