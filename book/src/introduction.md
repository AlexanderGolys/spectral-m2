# Spectral M2

Spectral M2 is a terminal shell for [Macaulay2](https://macaulay2.com/) that keeps the familiar numbered `i1`/`o1` flow and adds what a modern editor would: highlighting as you type, values drawn from their structure, maths typeset as pictures, and documentation that opens beside your work instead of in a browser.

<!-- capture: GIF of the headline workflow — type the block below, watch it highlight, submit, see the Betti table and a typeset matrix, then run `help res` and middle-click a link on that page to open it in the reference pane. -->

```m2
R = QQ[x, y, z]
I = ideal(x^2 - y*z, y^2 - x*z, z^2 - x*y)
betti res I
matrix {{x, 1/2}, {y^2, z}}
```

Everything still runs in a real `M2` process: your `init.m2`, your packages, your debugger. Spectral only changes what you see and how you work with it.

## What you get

Input that knows Macaulay2
: Code is highlighted and checked locally as you type, before anything reaches M2. `Enter` submits only complete code; an open bracket just adds a line. See [Write code](shell/write.md).

Results you can read
: Matrices, rings, fractions and subscripts are laid out in two dimensions, or typeset as real TeX pictures on terminals that show images. Types sit beside their values. See [Read results](shell/results.md).

Copy that gives you code back
: Select a typeset matrix and you get `matrix {{x, 1/2}}`, not a picture or its TeX. Whole cells copy as `i1 : code` and `o1 = value` if you like. See [Select and copy](shell/select.md).

Documentation in the shell
: `help` pages are drawn natively, with clickable links and examples coloured like your own cells, in a pane of their own. See [Documentation without leaving](shell/documentation.md).

Views of the same session
: Your code alone, your values alone, printed output alone, or the syntax tree, beside the live session. See [Views beside the session](shell/views.md).

Sessions you can keep
: Save a session as an `.m2` file, browse earlier ones, resume where you left off. See [Sessions and files](shell/sessions.md).

Yours to style
: Seven colour-scheme families with their variants, every highlight group editable, a configurable status line and rebindable keys. See [Make It Yours](custom/colours.md).

## Where to start

New here? [Install it](start/install.md) and take [a first session](start/first-session.md). Already using it? The [reference](reference/commands.md) lists every command, key and setting.
