# Commands and pickers

Type `/` into an empty input to open the filtered command picker. Continue typing to narrow it, use `Up` and `Down` to select, then press `Enter`. `Esc` closes the picker. `Ctrl+/` opens the full picker without changing the current draft; typing from that picker returns to ordinary input.

Commands are handled by Spectral rather than sent to Macaulay2.

| Command | Key | Effect |
| --- | --- | --- |
| `/settings` | `Ctrl+O` | Open settings, appearance included. |
| `/keymaps` | `Ctrl+K` | Show and edit the remappable bindings. |
| `/clear` | `Alt+L` | Clear the focused workspace's feed. |
| `/code` | `F2` | Show all committed input cells beside the session. |
| `/values` | `F3` | Show the returned values beside the session. |
| `/stdout` | `F4` | Show the text cells printed beside the session. |
| `/output` | `F5` | Show printed text and returned values, without input cells, beside the session. |
| `/ast` | `F6` | Show the `m2-syn` tree of committed cells beside the session. |
| `/save [file]` | | Write the code of every committed cell to `file`, or, without one, keep it where exit does. |
| `/load <file>` | | Restart Macaulay2, then run the code in `file` as if it had been typed. |
| `/resume` | `Ctrl+R` | Run the code the previous session left, one submission at a time, so the feed reads as it did. |

Views open beside the session.

While a command's name is being typed, `Tab` completes it to the command selected in the picker, closing the picker, or followed by a space when it takes a file; `Shift+Tab` selects the next command. The cursor moves with `Left` and `Right` as in any input, and the completion shows only while it is at the end of the name. The part of each command typed so far is bold in the picker. After `/save ` or `/load `, `Tab` completes the file name: directories and `.m2` files whose names start with what is typed, hidden ones only once a `.` is typed.

A file name without an extension gets `.m2`, and a leading `~` is the home directory. Picking `/load` from the picker types it into the input, ready for the file name; picking `/save` keeps the session at once.

The session's code is kept on every exit and restart, `/load` included, in `$XDG_STATE_HOME/spectral-m2/previous-session.m2` (`~/.local/state` when unset), for `/resume` to run. Cells entered together are kept together, divided from the next submission by a form feed line, which Macaulay2 reads as whitespace; code without one, such as a hand-written file, runs cell by cell. A session with no committed code leaves the kept one alone.

Exact commands submitted from input are opened locally, including commands recalled from history.

Pickers opened from the command picker preserve it: `Esc` from `/settings` returns to the commands before returning to the shell.
