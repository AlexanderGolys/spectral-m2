# Documentation without leaving

`help` pages are drawn right in the shell, laid out like the rest of your session: the examples look like your own cells, maths is drawn the way you chose, and every link opens with one click, without disturbing your input or your variables.

<!-- capture: screenshot of `help ideal` in the feed with a page open beside it in the reference pane. -->

```m2
help ideal
```

## A page, drawn natively

A page shows its title over a rule, its name apart from its one-line brief, its usage, inputs and outputs as headers, and its examples as session blocks without their numbers. Tables keep their columns where they fit and stack their cells where they do not. Paths to source files are shortened to the file and its directory; the link still leads to the full path.

## Follow links

A link keeps the colours of its text, prose or code, with only a soft underline; under the pointer its colours brighten and the underline is drawn in full.

`Ctrl`+click
: Opens the page, or the source file, in the tab showing in the **reference pane** beside the session, replacing what was there; `Ctrl+O` goes back to it.

Middle-click
: Opens it in a new tab and leaves you where you are; `Ctrl`+middle-click goes to the new tab.

Nothing is submitted either way: your feed, input numbers and variables stay as they were. To have a page in the feed, type `help "page"`.

A value that is a source position, such as the result of `locate`, is drawn as the same kind of link: it shows the file and the position, and opens the source there.

`Ctrl+O` and `Ctrl+I` go back and forward through the pages, and `Ctrl+Q` closes the tab. See [Views beside the session](views.md#tabs). `viewHelp` still opens your browser.

## Examples computed for real

Documentation examples carry the output M2 recorded when the docs were built, as plain text. Turn on *compute examples* in the [side panel settings](../reference/settings.md#side-panel) to have each page's examples run again, each for at most the time you choose, so their values are drawn from their structure like your own results.

Each page runs in a fresh Macaulay2 of its own with its package loaded, never in your session, and stops at the first example that runs out of time or answers differently from the record.

## Read the source

A file opened from a link, or with `/view file.m2`, is shown highlighted without being run. `F8` restarts Macaulay2 and runs the file you are looking at; see [Sessions and files](sessions.md).
