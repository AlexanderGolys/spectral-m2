# How sp2 talks to Macaulay2

Spectral is a front end to one real, long-running Macaulay2 process. Knowing a little about the connection helps you predict what the shell will and will not do.

## One session, one reader

When `sp2` starts, it launches `M2` with your startup options, loads its backend, and lets M2 enter its normal top level, reading the whole session from standard input, exactly as an interactive M2 does. So `x := 1` in one cell is visible in the next, `exit` closes the shell, and your packages and `init.m2` behave as usual.

What the backend changes is only how results come back: instead of printing `oN =` and `oN :` lines, M2 hands each value over with its structure (TeX where the class has one, parts of containers one by one, its type) so the shell can draw it. Printed text and errors arrive as M2 produced them, in order.

You can read all of it: the backend is written out as `backend.m2` in the [configuration directory](../reference/files.md), so `code` shows Spectral's hooks like any other M2 code.

## Your code runs once

Each cell is sent to Macaulay2 exactly once and is never replayed behind your back. Deciding whether input is complete, splitting it into cells, highlighting and diagnostics are all done locally, by parsing the text, before anything is sent. An hour-long computation is never started twice.

Drawing a value calls the class's own formatting methods (`net`, `tex` and the like) once, at the moment M2 prints it; switching how an output is drawn afterwards only picks among what already arrived. A trailing `;` skips all of that, as it does in plain M2.

## Other processes

Documentation examples, when you [compute them](../shell/documentation.md#examples-computed-for-real), run in a separate Macaulay2 per page, never in your session. The optional language server `m2-ls` reads your session's source to colour it and never evaluates anything.

## Pictures

Typeset maths is set inside `sp2` by the built-in RaTeX engine, encoded off the main thread, and sent to the terminal through the kitty graphics protocol. The terminal keeps only the pictures near what is on screen: those no longer shown are deleted, and past a memory budget the furthest go first. Inside tmux, pictures tmux dropped while the pane was hidden are sent again when it regains focus.
