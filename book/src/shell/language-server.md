# What the language server knows

With the optional language server `m2-ls` running, the input knows more than its colours: what a name is, where it was defined, what is wrong with a line before you run it, and how the code should be laid out. None of it evaluates anything; the server only reads your session's source.

```macaulay2
g = x -> x + 1
ideal(g 2, 3
```

Put the cursor on `ideal` and press `Alt+K`: a small panel above the cursor shows its signatures and the line saying what it does. Press `Alt+Shift+K` for everything the server knows, shown beside the session.

## Ask what a name is

`Alt+K`
: A small panel floats above the cursor with the start of the description: the signature or type and its headline. Any key or click puts it away.

`Alt+Shift+K`
: The whole description opens beside the session, its links leading to the documentation. It takes no tab of its own: choose any tab and it is gone, while the tab it covered is kept as it was.

Middle-click
: A middle click on code in the input asks the same as `Alt+K` about the name under the pointer, the panel floating beside the click.

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

A file opened beside the session, by `/view` or by going to a definition, marks every stretch of code the server can fold, such as a function body, with ▾ in its gutter. Double-click beside it to fold the stretch to its first line, which then ends with how many lines it hides; double-click the ▸ again to unfold it. A copy of the file's text skips what is folded.

## When the server is off

Turn the server off in [settings](../reference/settings.md) (*language server*) and all of the above goes quiet: the keys say that the language server is not running, underlines and shading go away, and files open without folds.
