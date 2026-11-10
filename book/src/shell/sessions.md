# Sessions and files

A session's code is never lost: it is kept every time you quit or restart, so you can resume it, save it as an `.m2` file, or pick an older one from a browser and run it again.

<!-- capture: screenshot of the session browser (F7) with a source preview open. -->

## Pick up where you left off

`/resume`, or `Ctrl+R`, runs the code the previous session left, one submission at a time, so the feed reads as it did when you typed it. From the command line, `sp2 --resume` does the same as the shell starts.

The code is kept on every exit and restart, in `previous-session.m2` under the state directory; see [Files and directories](../reference/files.md). Cells you entered together stay together: submissions are divided by a form-feed line, which Macaulay2 reads as whitespace. A session with no committed code leaves the kept one alone.

## Save and load

`/save notes.m2`
: Writes the code of every committed cell to `notes.m2`. A bare `/save` keeps it where quitting would, for `/resume`.

`/load notes.m2`
: Restarts Macaulay2 and runs the file as if you had typed it, cell by cell. A file without form-feed lines, such as one you wrote by hand, runs one cell at a time.

`/view notes.m2`
: Shows the file highlighted beside the shell, without running it. `F8` then restarts and runs it.

A file name without an extension gets `.m2`, and a leading `~` is your home directory. `Tab` after `/save `, `/load ` or `/view ` completes directories and `.m2` files; hidden ones appear once you type a `.`.

## Keep a starting point { #keep-a-starting-point }

Most sessions begin with the same few lines: load a package, define a ring, set some options. Keep them once and start from them whenever you like:

1. Open the code view with `F2`. It holds only cells that ran successfully, so errors and output stay out.
2. Select the setup lines.
3. Press `Ctrl+Shift+S`.

That saves exactly the selection as a session. To give it a file name, type `/save-selection setup.m2` first, then select and press `Enter`.

## Browse earlier sessions

`/sessions`, or `F7`, lists saved sessions, newest first, each by its first lines. `Enter` shows its highlighted source; `F8` restarts Macaulay2 and runs it.

Sessions are archived on exit, on restart and when you open the browser, under `sessions/` in the state directory. An unchanged session is archived once per run.

> [!NOTE]
> A saved session is source code, not a snapshot: running it computes everything again.
