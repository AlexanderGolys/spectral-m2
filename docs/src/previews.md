# Preview modes

Session previews contain committed cells only. All previews share five cyclic tabs and a separate bookmark tab, as described in [Split view](split-view.md). They update from the workspace they reference while keeping an independent scroll position. Use `Up`, `Down`, `PageUp`, `PageDown`, or the mouse wheel to navigate, and drag to select text.

`Ctrl+Q` leaves a preview. The command and settings pickers remain available with `Ctrl+/` and `Ctrl+O`.

## Code

`/code` combines committed input into one syntax-highlighted view. Pending and rejected input is not included.

```m2
R = ZZ/101[x, y, z]
I = ideal(x^2-y*z, y^2-x*z, z^2-x*y)
dim I
```

## Output

`/output` shows printed text and returned values without input cells. `/values` shows returned values and their types, while `/stdout` shows printed text alone. Each command opens a preview tab. Use `Left` and `Right` to switch between occupied tabs.

Previewing does not change or remove the original feed.

## Syntax tree

`/ast` renders the `m2-syn` pretty tree for every committed cell and opens at the newest tree. Each section has a centered cell-number separator, highlighted source, and its structural tree.

The presentation keeps semantic structure compact:

- node types precede optional field labels;
- nonterminal rows include trimmed source text when it fits;
- delimiter rows enclose and indent their contents;
- `Token![…]` displays its source character only inside the brackets;
- virtual tokens are named without pretending that their marker text occurred in the source;
- absent optionals and empty or purely mechanical collection wrappers are hidden;
- source ranges line up in one column after the widest row.

Code and syntax-tree views sit on one code background throughout, and the output view on the background of printed text, so no seams separate their blocks. Each view names itself at the left of its status line.
