# When things go wrong

An error costs you the failed cell and nothing else: everything you built before it is still there, and the shell is ready for the next line.

```m2
R = QQ[x, y]
I = ideal(x^2, y)
I_5          -- index out of range
gens I       -- R and I are still here
```

## Errors

The failed cell and any cells queued after it sit on a red background; the code that never ran is muted, without syntax colours, and is discarded. The error box names the error in its title and shows the message in red. Variables and rings from before the error stay available.

## The debugger

Macaulay2's automatic debugger entry is off by default: an ordinary error returns you to normal input. Start with `sp2 --debugger` when you want M2 to enter its debugger on errors. `--debug N` only turns warning `N` into an error and does not enable the debugger.

## Cancelling

`Ctrl+C` interrupts a computation and discards queued work while keeping the session, including programs Macaulay2 launched. Output arriving late from a cancelled cell is dropped rather than filed under the next one. See [A first session](../start/first-session.md#cancel).

## If the shell itself stops

While the screen is drawn, anything written to stderr is discarded so it cannot garble the display. A crash is therefore recorded with its backtrace in `crash.log` under the state directory, and the shell prints the one-line cause and that path once it has restored your terminal. Include the file when you report a problem.

> [!NOTE]
> After such an exit Macaulay2 may print "mutable unexported unset symbol(s) in package User". That is M2's own check as its input closes, listing names you used without assigning them; it is unrelated to the crash.
