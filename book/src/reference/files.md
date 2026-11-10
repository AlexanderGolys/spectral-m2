# Files and directories

Spectral keeps configuration in one directory and session state in another, following the XDG convention.

## Configuration

`$XDG_CONFIG_HOME/spectral-m2/`, or `~/.config/spectral-m2/` when the variable is unset.

`init.m2`
: Your own startup code for Spectral sessions, run after Macaulay2's usual `~/.Macaulay2/init.m2`. Optional; skipped with `-q`.

`backend.m2`
: The Macaulay2 side of the shell, written out from the program so M2's `code` can show you Spectral's hooks. Replaced whenever a new version brings a different one; don't edit it.

`settings.json`
: Settings and rebound keys, in a section per settings tab.

`appearance.json`
: The chosen scheme, variant, mode and background.

`completion-usage.json`
: How often you used each symbol M2 knows at startup, for [completion order](../shell/write.md#how-often-used-is-counted).

`themes/`
: `groups.toml` and one file per colour scheme, written on first use. See [Colour schemes](../custom/colours.md#edit-any-colour).

## State

`$XDG_STATE_HOME/spectral-m2/`, or `~/.local/state/spectral-m2/` when the variable is unset.

`history.jsonl`
: Input history, shared by every running shell.

`previous-session.m2`
: The code of the last session, kept on every exit and restart, for `/resume` and `--resume`.

`sessions/`
: Archived sessions for the [session browser](../shell/sessions.md#browse-earlier-sessions).

`crash.log`
: Written if the shell crashes, with the cause and backtrace. See [When things go wrong](../shell/errors.md#if-the-shell-itself-stops).
