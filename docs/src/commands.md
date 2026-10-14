# Commands and pickers

Type `/` into an empty input to open the filtered command picker. Continue typing to narrow it, use `Up` and `Down` to select, then press `Enter`. `Esc` closes the picker. `Ctrl+/` opens the full picker without changing the current draft; typing from that picker returns to ordinary input.

Commands are handled by Spectral rather than sent to Macaulay2.

| Command | Key | Effect |
| --- | --- | --- |
| `/settings` | `Ctrl+O` | Open settings, appearance included. |
| `/keymaps` | `Alt+K` | Show and edit the remappable bindings. |
| `/clear` | `Alt+L` | Clear the focused workspace's feed. |
| `/code` | `Alt+C` | Show all committed input cells beside the session. |
| `/output` | `Alt+O` | Show structured output without input cells beside the session. |
| `/ast` | `Alt+T` | Show the `m2-syn` tree of committed cells beside the session. |
| `/new` | `Alt+R` | Restart the M2 process and reset its state. |
| `/debug` | `Ctrl+D` | Toggle the highlight inspector; opening it posts a live value/muted/null probe. |
| `/quit` | `Ctrl+C` | Leave Spectral. |
| `/close split view` | `Alt+W` | Close the view beside the session; shown only while split. |

Views open beside the session. `Alt+V` is not a command keymap: on a selected `code`, `output`, or `ast` row, it opens that view in place of the focused pane instead. The command's own keymap remains visible beside it.

Exact commands submitted from input are opened locally, including commands recalled from history. The bare inputs `exit`, `quit`, and `restart` are also intercepted locally, with an optional trailing semicolon.

Pickers opened from the command picker preserve it: `Esc` from `/settings` returns to the commands before returning to the shell.
