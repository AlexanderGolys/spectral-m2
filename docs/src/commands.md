# Commands and pickers

Type `/` into an empty input to open the filtered command picker. Continue typing to narrow it, use `Up` and `Down` to select, then press `Enter`. `Esc` closes the picker. `Ctrl+/` opens the full picker without changing the current draft; typing from that picker returns to ordinary input.

Commands are handled by Spectral rather than sent to Macaulay2.

| Command | Key | Effect |
| --- | --- | --- |
| `/settings` | `Ctrl+O` | Open shell settings. |
| `/style` | `Alt+S` | Open the separate colorscheme picker. |
| `/keymaps` | `Alt+K` | Show and edit the remappable bindings. |
| `/clear` | `Alt+L` | Clear the focused workspace's feed. |
| `/code` | `Alt+C` | Preview all committed input cells. |
| `/output` | `Alt+O` | Preview structured output without input cells. |
| `/syntax tree` | `Alt+T` | Preview the `m2-syn` tree of committed cells. |
| `/new` | `Alt+N` | Start a fresh, independent M2 workspace. |
| `/restart` | `Alt+R` | Restart the focused workspace's M2 process and reset its state. |
| `/debug` | `Ctrl+D` | Toggle the highlight inspector; opening it posts a live value/muted/null probe. |
| `/quit` | `Ctrl+C` | Leave Spectral. |
| `/close focused window` | `Alt+W` | Close the focused pane; shown only while split. |

`Alt+V` is not a command keymap. On a selected `new`, `code`, `output`, or `syntax tree` row, it opens that command in an alternative pane instead of replacing the focused one. The command's own keymap remains visible beside it.

Exact commands submitted from input are opened locally, including commands recalled from history. The bare inputs `exit`, `quit`, and `restart` are also intercepted locally, with an optional trailing semicolon.

Nested pickers preserve their parent. For example, `/settings` can open `style`, and `Esc` returns from style to settings before returning to the shell.
