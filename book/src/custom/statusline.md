# Status line

The status line at the bottom tells you what the shell is doing and which keys answer right now. You decide what it shows and where.

<!-- capture: the status line with the default components, then with versions and debug added. -->

## Five segments

The line has five fixed anchors: the left edge, a quarter in, the centre, three quarters in, and the right edge. Each component goes to one of them or is hidden. By default the theme sits on the left, the directory at left-centre, the view in the centre, the session state at right-centre and the key hints on the right.

Open the *statusline* tab of the settings with `Ctrl+O`. Each component has a row; `Left` and `Right` move it between `off`, `L`, `LC`, `C`, `RC` and `R`, and `0` to `5` choose directly, `0` hiding it. The *position* row puts the whole line below the input, above it, or turns it off.

Several components can share a segment: they appear side by side in the order of the list, however you assigned them. An empty segment gives its neighbours more room without moving their anchors, and so does a component with nothing to report.

## Components

theme
: The colour scheme and its variant.

math renderer
: How maths is drawn; it names the Unicode fallback when typeset maths is unavailable in this terminal.

language server
: Whether `m2-ls` is starting, ready, off or unavailable.

rendering settings
: Theme, math renderer and language server together.

directory
: The working directory.

view
: The view beside the session, its active tab highlighted.

session state
: `Processing`, or `Waiting for …` while M2 is busy.

key hints
: The keys that answer now: idle commands, picker controls, the cancel key during a long computation, or continuation controls.

m2-ls version, sp2 version, M2 version
: Each process's version, `unknown` when a process did not report it. Put all three in one segment for a version line.

versions
: The three versions at once.

debug
: The `sp2` version with the memory it holds, the M2 process's memory (`n/a` outside Linux), the pictures the terminal holds and their memory, and the frame rate. Measured only while it is shown.

Placements are saved by component name, under `statusline.components` in `settings.json`.
