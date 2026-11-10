# Command-line options

```text
sp2 [OPTIONS] [FILE]
```

`FILE`
: Run the Macaulay2 code in `FILE` cell by cell as the session starts, then hand over the input. A name without an extension gets `.m2`.

`-r`, `--resume`
: Run the code of the previous session as the session starts.

`-h`, `--help`
: Print help.

`-V`, `--version`
: Print the version.

## Macaulay2 startup

These are passed to M2 and kept for every restart, including the fresh session `/load` starts. They combine with a file or `--resume`:

```sh
sp2 --print-width 100 -e 'R = QQ[x]' ring.m2
```

`-q`
: Skip user initialization (both M2's `init.m2` and Spectral's) and the packages in your home directory.

`-E <EXPR>`
: Evaluate an expression before M2 initializes. Repeatable.

`-e <EXPR>`
: Evaluate an expression after M2 initializes, before Spectral's `init.m2`. Repeatable. These run without making input cells.

`--no-preload`
: Skip preloaded packages.

`--prefix <DIR>`
: Set M2's prefix directory.

`--srcdir <DIR>`
: Add an M2 source or build tree to the initial path.

`--no-backtrace`
: Suppress error backtraces.

`--no-debug`
: Keep automatic debugger entry off, which is already the default.

`--debugger`
: Enter the Macaulay2 debugger on errors. Conflicts with `--no-debug`.

`--debug <N>`
: Turn M2 warning `N` into an error. Does not enable the debugger.

`--notify`
: Report files loaded and command-line expressions evaluated.

`--print-width <N>`
: Set M2's output width.

`--no-randomize`
: Use M2's fixed initial random seed.

`--no-threads`
: Disable additional M2 threads and tasks.

`--no-time`
: Make M2's current time always zero.

Documentation examples, when [computed](../shell/documentation.md#examples-computed-for-real), run in their own Macaulay2 with its default configuration, not these options.

## Environment

`SPECTRAL_M2_BASIC_GLYPHS=1`
: Use plain prompt, divider, separator and spinner symbols. `TERM=dumb` does the same.

`XDG_CONFIG_HOME`, `XDG_STATE_HOME`
: Where Spectral keeps its files. See [Files and directories](files.md).
