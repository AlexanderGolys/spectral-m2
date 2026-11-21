# What the language server knows

With the optional language server `m2-ls` running, the input knows more than its colours: what a name is, where it was defined, what is wrong with a line before you run it, and how the code should be laid out. None of it evaluates anything; the server only reads your session's source.

```macaulay2
g = x -> x + 1
ideal(g 2, 3
```

Put the cursor on `ideal` and press `Alt+K`: a small panel above the cursor shows its signatures and the line saying what it does. Press `Alt+]` for everything the server knows, shown beside the session, or `Ctrl+K` for the documentation page of `ideal`.

## Ask what a name is

`Alt+K`
: A small panel floats above the cursor with the start of the description: the signature or type and its headline. Any key or click puts it away.

Middle-click
: A middle click on code in the input asks the same as `Alt+K` about the name under the pointer, the panel floating beside the click.

## Jump from a name

Three keys jump from the name under the cursor to a tab beside the session, each tab named after the name:

`Ctrl+]`
: Its source: where it is defined. See [below](#go-to-a-definition).

`Alt+]`
: Its description: everything the server knows about it, its links leading to the documentation.

`Ctrl+K`
: Its documentation: the page `help symbol` shows for it, as for `help symbol ideal`. Operators work too, such as `++`. This one needs no language server, since Macaulay2 itself finds the page.

The tabs sit with the documentation and files you opened, so `Ctrl+O` goes back and `Ctrl+PageUp` cycles through them; see [the tabs](views.md#tabs). Each key can be remapped in the [keymaps](../custom/keymaps.md).

Code you already ran answers too. Press `Ctrl+E` to put a cursor in the feed, walk it onto a name in an earlier cell, and the keys above ask about that name as they do in the input. Cells from before a restart are no longer part of the session, and say so.

## Go to a definition

`Ctrl+]` (or Shift+click on the name) goes to where the name under the cursor is defined:

- defined in the input, the cursor moves there;
- defined by an earlier cell, that cell is selected in the feed and scrolled into view;
- defined in a file, such as a Macaulay2 package, the file opens beside the session at the definition.

Terminals without keyboard enhancement send `Ctrl+]` as `Ctrl+5`, which is bound too.

## Problems before running

What the server finds wrong is underlined in the input in the colour of its severity, and the message for the one under the cursor (else the first error) stands below the input with its code in front, as `E13: …`. Syntax errors are left to Macaulay2's own parser, which reports them when the input runs.

## Occurrences and signatures

Every place the name under the cursor occurs in the input is shaded. Inside a call to a function the server knows, typing shows its signature above the cursor, the parameter you are at set apart:

```macaulay2
matrix({{1}}, 
```

## Completion

Names the server reads as bound in your session complete before Macaulay2 has run them: after `f = x -> x` is typed, `f` completes in the next line straight away.

## Formatting

`Alt+Shift+F` lays out the input as the server formats Macaulay2, as one edit that undo takes back. Nothing happens when the input is already formatted, or when it changed before the answer came.

## Fold a file

A file opened beside the session, by `/view` or by going to a definition, numbers its lines and marks every stretch of code the server can fold, such as a function body, with ▾ in its gutter. Double-click beside it to fold the stretch to its first line, which then ends with how many lines it hides; double-click the ▸ again to unfold it. A copy of the file's text skips what is folded.

## When the server is off

Turn the server off in [settings](../reference/settings.md) (*language server*) and all of the above goes quiet: the keys say that the language server is not running (`Ctrl+K` still opens documentation), underlines and shading go away, and files open without folds.
