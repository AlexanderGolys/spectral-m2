# Commands and pickers

Type `/` into an empty input to open the filtered command picker. Continue typing to narrow it, use `Up` and `Down` to select, then press `Enter`. `Esc` closes the picker. `Ctrl+/` opens the full picker without changing the current draft; typing from that picker returns to ordinary input.

Commands are handled by Spectral rather than sent to Macaulay2.

| Command | Effect |
| --- | --- |
| `/settings` | Open shell settings. |
| `/style` | Open the separate colorscheme picker. |
| `/keymaps` | Show and edit the remappable bindings. |
| `/clear` | Clear the focused workspace's feed. |
| `/code` | Preview all committed input cells. |
| `/output` | Preview structured output without input cells. |
| `/syntax tree` | Preview the `m2-syn` tree of committed cells. |
| `/new` | Start a fresh, independent M2 workspace. |
| `/restart` | Restart the focused workspace's M2 process and reset its state. |
| `/debug` | Toggle the highlight-group and native-palette inspector. |
| `/quit` | Leave Spectral. |
| `/close focused window` | Close the focused pane; shown only while split. |

The `new`, `code`, `output`, and `syntax tree` rows also accept `Ctrl+Enter` when only one pane is open. That opens the result in a second pane instead of replacing the focused one.

Exact commands submitted from input are opened locally, including commands recalled from history. The bare inputs `exit`, `quit`, and `restart` are also intercepted locally, with an optional trailing semicolon.

Nested pickers preserve their parent. For example, `/settings` can open `style`, and `Esc` returns from style to settings before returning to the shell.
