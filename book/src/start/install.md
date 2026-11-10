# Install and first run

Two commands get you from nothing to a running session:

```sh
cargo install spectral-m2 --locked
sp2
```

The package is called `spectral-m2`; the program it installs is `sp2`. It starts a Macaulay2 process straight away. If `M2` cannot be found or started, `sp2` says why and exits instead of opening an interface that cannot work.

## What you need

Macaulay2
: Available as `M2` on your `PATH`. Spectral runs your normal M2, so your `~/.Macaulay2/init.m2` and the packages in your home directory work as they always do.

A terminal with mouse events and OSC 52
: Mouse events for clicking and selecting; OSC 52 so a copy lands on your system clipboard. Most modern terminals have both.

A terminal that shows pictures, optionally
: For [typeset maths](../shell/results.md#typeset), a terminal speaking the kitty graphics protocol: kitty or Ghostty, inside tmux too. Elsewhere maths is laid out in Unicode characters.

`m2-ls`, optionally
: The Macaulay2 language server, 2.0 or later, adds [semantic colours](../custom/colours.md#semantic-colours): `cargo install m2-ls`. Without it everything else works the same.

## Start from code you already have

Give `sp2` a file and it runs the file cell by cell as the session starts, as if you had typed it, then hands the input over to you:

```sh
sp2 ring.m2      # run ring.m2, then keep going
sp2 --resume     # run the code of the session you last closed
```

A name without an extension gets `.m2`. Comments are left out of the replayed code, so a file of comments only leaves the session empty.

## Pass options to Macaulay2

Every M2 startup option `sp2` knows is passed through and kept when the session restarts:

```sh
sp2 --print-width 100 -e 'R = QQ[x]' ring.m2
sp2 -q --no-randomize --resume
```

`-E` runs an expression before M2 initialises and `-e` after; both repeat. The full list is in [Command-line options](../reference/cli.md).

## Leave

`Ctrl+C` quits when nothing is running and the input is empty. During a computation it cancels the computation instead and keeps the session; see [A first session](first-session.md#cancel). Macaulay2's own `exit`, `quit` and `end` close the shell too.

Next: [a first session](first-session.md).
