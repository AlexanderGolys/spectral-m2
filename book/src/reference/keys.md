# Default keys

Every key below can be [rebound](../custom/keymaps.md) unless marked *fixed*. The groups are the contexts of the keymaps picker; fixed keys, the arrows in every combination and the keys every editor or shell gives the same meaning, stay out of it so the shortcuts worth learning are easy to find.

## Global

| Key | Action |
| --- | --- |
| `Enter` | Evaluate the input, or add a line to an unfinished construct. *Fixed.* |
| `Ctrl+C` | Cancel the computation and queued work; when idle, discard a continuation or draft, then quit. *Fixed.* |
| `Ctrl+/` | Open the commands. |
| `Ctrl+O` | Open the settings. |
| `Ctrl+Shift+K` | Open the keymaps. |
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
| `Ctrl+X` | Cut the input's selection, or its current line when nothing is selected. *Fixed.* |
| `Ctrl+E` | Move the cursor into the feed or the focused view, and back; see [the view cursor](#the-view-cursor). |
| `Alt+Up`, `Alt+Down` | Select the previous or next cell or result. *Fixed.* |
| `Alt+K` | Tell what the name at the cursor is, in a panel beside it. |
| `Ctrl+]` (`Ctrl+5`) | Go to where the name at the cursor is defined. |
| `Alt+]` | Describe the name at the cursor beside the session, in a tab of its own. |
| `Ctrl+K` | Open the documentation of the name at the cursor beside the session. |
| `Alt+Shift+F` | Format the input. |
| `PageUp`, `PageDown` | Scroll the feed a page, keeping *scrolloff* rows of the last one in sight. *Fixed.* |
| `Alt+PageUp` | Scroll to the start of the block at the top of the feed or view; again, to the start of the one before. |
| `Alt+PageDown` | Scroll to the end of the block at the bottom; again, to the end of the one after. |
| `Ctrl+Shift+D` | Toggle the highlight inspector. *Fixed.* |

## Movement

| Key | Action |
| --- | --- |
| `Left`, `Right` | Move left or right. *Fixed.* |
| `Up`, `Down` | Move up or down; at the first or last line, recall older or newer input. *Fixed.* |
| `Ctrl+Left`, `Ctrl+Right` | Move by a word. *Fixed.* |
| `Alt+Left`, `Alt+Right` | Move past text up to whitespace, brackets and punctuation included. *Fixed.* |
| `Home`, `End` | Move to the start or end of the line. *Fixed.* |
| `Ctrl+Alt+Up`, `Ctrl+Alt+Down` | Move to the blank line before or after the paragraph. *Fixed.* |
| `Ctrl+Alt+Left`, `Ctrl+Alt+Right` | Move to the bracket opening the block the cursor is in, or past the one closing it; again, to the block around that. *Fixed.* |
| `Alt+M` | Jump to the partner of the bracket at the cursor. |
| `Ctrl+Home`, `Ctrl+End` | Move to the start or end of the input. *Fixed.* |

## Selection

| Key | Action |
| --- | --- |
| `Shift` with any fixed movement key | Extend the selection the same way. *Fixed.* |
| `Alt+Shift+Left`, `Alt+Shift+Right` | Extend the selection up to whitespace. *Fixed.* |
| `Ctrl+Alt+Shift+Up`, `Ctrl+Alt+Shift+Down` | Extend the selection by a paragraph. *Fixed.* |
| `Alt+Shift+Up` | Grow the selection: the word, then what the brackets around it hold, the brackets too, the line, the paragraph, everything. *Fixed.* |
| `Alt+Shift+Down` | Shrink it back the way it grew. *Fixed.* |
| `Ctrl+A` | Select the whole input. *Fixed.* |

## Editing

| Key | Action |
| --- | --- |
| `Shift+Enter` | Insert a line break. `Alt+Enter`, `Shift+Space` and `Ctrl+J` do the same, for terminals that cannot report `Shift+Enter`. *Fixed.* |
| `Alt+Tab` | Insert a tab. |
| `Backspace`, `Delete` | Delete before or under the cursor. *Fixed.* |
| `Ctrl+Backspace`, `Ctrl+Delete` | Delete back or on by a word. `Alt` works too, and so does `Ctrl+H`, which some terminals send for `Ctrl+Backspace`. *Fixed.* |
| `Ctrl+W` | Delete back to the whitespace before the cursor. *Fixed.* |
| `Ctrl+U` | Delete the current line. *Fixed.* |
| `Ctrl+L` | Clear the input. |
| `Ctrl+Z`, `Ctrl+Shift+Z` | Undo, redo. *Fixed.* |

## Recall

| Key | Action |
| --- | --- |
| `Tab` | Show completion candidates, take the hint, or take the next candidate. *Fixed.* |
| `Shift+Tab`, `Ctrl+Tab` | Take the previous candidate. *Fixed.* |
| `Ctrl+Up`, `Ctrl+Down` | Step to the very previous or next entry, whatever was typed and from any line. *Fixed.* |

## In views and pickers

These follow the pane or picker that has focus and are not in the keymaps:

| Key | Where | Action |
| --- | --- | --- |
| `Up`, `Down`, `PageUp`, `PageDown` | view | Scroll a row or a page. |
| `Left`, `Right` | view | Switch to the previous or next tab. |
| `Ctrl+PageUp`, `Ctrl+PageDown` | either pane | Switch the view's tab. |
| `Up`, `Down` | picker | Select a row. |
| `Left`, `Right` | settings | Change a value; on the row of tabs, switch tab. |
| `Tab`, `Shift+Tab` | settings | Switch tab. |
| `0` … `5` | statusline row | Hide the component, or put it in segment 1 to 5. |
| `Enter` | picker | Run or open the selected row. |
| `Esc` | picker, inspector | Close it. |

## The view cursor

After `Ctrl+E` the cursor sits on the last text of the focused pane, the feed or the view beside it, and these keys work there instead of in the input. The movement and selection keys of the input work too.

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` | Move left, down, up, right; past the top or bottom the pane scrolls. |
| `w` `b`, `W` `B` | Move by a word, or up to whitespace. |
| `0`, `$` | Move to the first or last character of the row. |
| `{`, `}` | Move to the blank row before or after the paragraph. |
| `g`, `G` | Move to the start or end of the feed. |
| `%` | Jump to the partner of the bracket under the cursor. |
| `v` | Start or stop selecting as the cursor moves. |
| `y` | Copy the selection. |
| `Alt+Shift+Up` | Select what a double-click would. |
| `Enter` | Open the link under the cursor beside the session; else fold or unfold the value; else switch to the formatter whose icon it is on. |
| `K`, `Alt+K`, `Ctrl+]`, `Alt+]`, `Ctrl+K` | Ask about the name in the session code under the cursor, or open its documentation. |
| `Esc`, `q`, `i`, `Ctrl+E` | Go back to the input. |

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
