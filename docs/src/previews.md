# Preview modes

Previews contain committed cells only. They update from the workspace they reference while keeping an independent scroll position. Use `Up`, `Down`, `PageUp`, `PageDown`, or the mouse wheel to navigate, and drag to select text.

`Ctrl+Q` and `Ctrl+C` leave a preview. The command and settings pickers remain available with `Ctrl+/` and `Ctrl+O`.

## Code

`/code` combines committed input into one syntax-highlighted view. Pending and rejected input is not included.

```m2
R = ZZ/101[x, y, z]
I = ideal(x^2-y*z, y^2-x*z, z^2-x*y)
dim I
```

## Output

`/output` starts with evaluation output: printed text, warnings, and errors. Use `Left` and `Right` to cycle through:

1. evaluation output;
2. values and types;
3. everything except input.

The active projection is shown in the status line. Previewing does not change or remove the original feed.

## Syntax tree

`/syntax tree` renders the `m2-syn` pretty tree for every committed cell and opens at the newest tree. Each section has a centered cell-number separator, highlighted source, and its structural tree.

The presentation keeps semantic structure compact:

- node types precede optional field labels;
- nonterminal rows include trimmed source text when it fits;
- delimiter rows enclose and indent their contents;
- `Token![…]` displays its source character only inside the brackets;
- virtual tokens are named without pretending that their marker text occurred in the source;
- absent optionals and empty or purely mechanical collection wrappers are hidden.

The syntax-tree canvas is centered as a whole, and its status line marks `Syntax Tree Preview Mode` in the middle.
