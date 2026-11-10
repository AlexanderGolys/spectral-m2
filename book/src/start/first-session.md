# A first session

Five minutes with this page and you know how a session moves: what `Enter` does, what appears in the feed, and how to stop or restart a computation.

<!-- capture: screenshot of the feed after the block below, showing pending cells, a folded Betti table and output labels. -->

## Submit some code

Type this and press `Enter` after each line:

```m2
R = QQ[x, y, z];
I = ideal(x^2 - y*z, y^2 - x*z, z^2 - x*y)
betti res I
```

Each line you submit becomes a **cell** in the **feed** above the input, with its number in the gutter. The cell appears at once, with a spinner while Macaulay2 works on it; its results follow underneath: printed text, the value with its `oN` label, the value's type at the right edge, and any warnings or errors.

A semicolon stays part of its cell, so M2 still decides whether a value is shown: `R = QQ[x, y, z];` prints nothing, as it would in a plain M2 shell.

## Several lines at once

`Enter` submits only complete code. If you press it inside an open bracket, the input simply gets another line:

```m2
f = x -> (
    x^2 + 1)
```

When the code is complete, `Enter` sends it all. Code you submit together is one **submission**: its cells run in order, and their outputs share one border. `Shift+Enter` (or `Alt+Enter`, or `Shift+Space`) always inserts a newline, complete or not.

## The commands

Type `/` into an empty input for the command picker, filtered as you type. `Ctrl+/` opens it with everything listed, and `Ctrl+O` opens the settings. Every command is in the [commands reference](../reference/commands.md).

## Cancel and restart { #cancel }

`Ctrl+C` during a computation interrupts it and discards whatever was queued behind it. The session survives: rings and variables you made before are still there. Programs Macaulay2 started are interrupted too.

When nothing is running, `Ctrl+C` first throws away an unfinished continuation or draft, and only quits once the input is empty. After three seconds of computing, the status line shows the cancel key, in case you rebound it.

`Ctrl+N` restarts Macaulay2 with the same startup options, and `Alt+L` clears the feed without touching the session.

> [!TIP]
> Only one submission runs at a time. Submitting another while M2 is busy shows a diagnostic in the input instead of queueing it, so nothing runs in an order you did not expect.

Next: [write code](../shell/write.md) with completion and history, or jump to [reading results](../shell/results.md).
