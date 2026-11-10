# Commands

Commands are handled by the shell, never sent to Macaulay2. Type `/` into an empty input for the picker, filtered as you type, or press `Ctrl+/` to list them all without touching your draft. A command typed out in full and submitted works too, also when recalled from history.

`/settings`, `Ctrl+O`
: Open the settings. See [Settings](settings.md).

`/keymaps`, `Ctrl+K`
: Open the settings on the keymaps tab. See [Keymaps](../custom/keymaps.md).

`/clear`, `Alt+L`
: Clear the feed. The session and its variables are untouched.

`/code`, `F2`
: Show your committed code beside the session.

`/values`, `F3`
: Show returned values beside the session.

`/stdout`, `F4`
: Show printed text beside the session.

`/output`, `F5`
: Show printed text and values, without the code, beside the session.

`/ast`, `F6`
: Show the syntax tree of the committed code beside the session.

`/save [file]`
: Write the code of every committed cell to `file`. Without a file, keep it where quitting would, for `/resume`.

`/load <file>`
: Restart Macaulay2, then run the code in `file` as if it had been typed.

`/resume`, `Ctrl+R`
: Run the code the previous session left, one submission at a time.

`/sessions`, `F7`
: Browse saved sessions, newest first. `Enter` shows one; `F8` restarts and runs it.

`/view <file>`
: Show a file highlighted beside the session, without running it.

`/save-selection [file]`, `Ctrl+Shift+S`
: Save the selected code as a session, to `file` or to the session archive.

## Using the picker

`Up` and `Down` select and `Enter` runs; `Esc` closes the picker and keeps what you typed. `Tab` completes the selected command's name, followed by a space when it takes a file, and `Shift+Tab` and `Ctrl+Tab` select the next and previous command. The part of each name typed so far is bold.

After a file command, `Tab` lists directories and `.m2` files starting with what is typed, hidden ones once you type a `.`. A file name without an extension gets `.m2`, and a leading `~` is your home directory.

A picker opened from the command picker returns to it: `Esc` from the settings goes back to the commands before going back to the shell.

## Actions on keys only

Some actions have a key but no command:

`Ctrl+N`
: Restart Macaulay2, keeping the startup options.

`F8`
: Restart Macaulay2 and run the file shown beside the session.

`Ctrl+Q`
: Close the view beside the session.

`Ctrl+Shift+B`
: Return to the previous tab of the view.

`Alt+Up`, `Alt+Down`
: Select the previous or next cell or result. See [Select and copy](../shell/select.md#whole-cells-and-results).

`Ctrl+Y`
: Copy the selection.

`Ctrl+Shift+D`
: Toggle the highlight inspector.

`Ctrl+C`
: Cancel the computation and queued work; when idle, discard a continuation or draft, then quit.
