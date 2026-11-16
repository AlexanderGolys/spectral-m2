# Files and directories

Spectral keeps configuration in one directory and session state in another, following the XDG convention.

## Configuration

`$XDG_CONFIG_HOME/spectral-m2/`, or `~/.config/spectral-m2/` when the variable is unset.

`init.m2`
: Your own startup code for Spectral sessions, run after Macaulay2's usual `~/.Macaulay2/init.m2` once Spectral is loaded, so it can [draw your own types](../deep/how-it-works.md#drawing-your-own-types). An error in it is reported and the session starts anyway. Optional; skipped with `-q`.

`backend.m2`
: The Macaulay2 side of the shell, written out from the program so M2's `code` can show you Spectral's hooks. Replaced whenever a new version brings a different one; don't edit it.

`spectral-package.m2`
: Loads the [`Spectral`](https://github.com/AlexanderGolys/Spectral) package the shell draws with. When Macaulay2 has no `Spectral` installed, the first start fetches it with `git` and installs it, documentation and source files included, so `help`, `viewHelp`, `code` and `m2-ls` see it like any package; later starts only load it. Replaced like `backend.m2`; don't edit it.

`settings.json`
: Editable settings and rebound keys, created on first launch. Each tab of the settings keeps its own section, named after it: `rendering`, `lsp`, `editor` (with the selection under `editor.selection`) `statusline` and `session`; files saved by older versions are read into this shape. Restart the shell after editing.

`appearance.json`
: The chosen scheme, variant, continuous contrast position, mode and background.

`completion-usage.json`
: How often you used each symbol M2 knows at startup, for [completion order](../shell/write.md#how-often-used-is-counted).

`themes/`
: `groups.toml` and one file per colour scheme, written on first use. See [Colour schemes](../custom/colours.md#edit-any-colour).

## State

`$XDG_STATE_HOME/spectral-m2/`, or `~/.local/state/spectral-m2/` when the variable is unset.

With *local session files* enabled, history, the previous session, archives and notes instead live in `.spectral-m2/` at the repository root (or the current directory outside a repository). Configuration and crash logs remain in their global directories.

`notes.md`
: The automatically saved [notebook](../shell/views.md#notes).

`history.jsonl`
: Input history, shared by shells using the same storage directory.

`previous-session.m2`
: The code of the last session, kept on every exit and restart, for `/resume` and `--resume`.

`sessions/`
: Archived sessions for the [session browser](../shell/sessions.md#browse-earlier-sessions).

`crash.log`
: Written if the shell crashes, with the cause and backtrace. See [When things go wrong](../shell/errors.md#if-the-shell-itself-stops).
