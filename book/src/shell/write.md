# Write code

The input reads Macaulay2 as you type: it colours your code, points at mistakes before M2 sees them, completes names M2 knows, and remembers everything you ran, across every open shell.

<!-- capture: screenshot of a multiline input with an inline completion hint and a local diagnostic underlined. -->

## Highlighting and diagnostics as you type

Your code is parsed locally on every keystroke, so colours and errors appear without asking M2 anything. When you press `Enter`:

- complete code is split into cells and sent;
- code that ended inside an open construct gets another line;
- code that cannot be valid stays in the input, with the error marked at its place in the source.

With the optional language server `m2-ls` running, colours also tell apart what the parser alone cannot, such as a method from a variable; see [semantic colours](../custom/colours.md#semantic-colours). It also describes names, finds definitions and problems, and formats the input: see [what the language server knows](language-server.md).

The prompt takes a typing form while you edit and returns to rest shortly after your last change, without moving the text.

## Completion

A muted hint after the cursor shows the likely completion when there are few candidates: fewer than the *hint below* setting, 10 by default.

`Tab` opens the candidates above the input, and takes the hint at once if one was showing; otherwise the next `Tab` takes the first candidate. Further presses cycle, and `Shift+Tab` or `Ctrl+Tab` cycle back. `Esc` puts the list away; `Alt+Tab` types a real tab.

What completes depends on where the cursor is:

A name
: Every symbol Macaulay2 knows, reported by M2 as it starts and after every cell, so a ring you just defined or a package you just loaded completes from the next keystroke. Names you use often come first.

Inside a string
: Files and directories, so `load "ri` completes to `load "ring.m2`.

Inside a comment
: Nothing.

Completion starts once a name is as long as the *complete after* setting, one character by default.

### How "often used" is counted

Each successfully evaluated cell adds to the counts of the known global symbols it refers to. Counts for names that exist when M2 starts are kept between sessions; counts for names your session created, such as a ring `R`, last only for that session. Assignment targets, function parameters, loop variables, `local` and `:=` bindings, strings, comments, and choosing a completion do not count.

## History

`Up` and `Down` move within multiline input; at its first or last line they recall older or newer submissions. Like a shell's history search, the text before the cursor when you start is a prefix: type `R =`, press `Up`, and you step only through submissions that start with `R =`, each once.

`Ctrl+Up` and `Ctrl+Down` step to the very previous or next entry from any line, with no prefix and even after you edit a recalled entry. Only that step skips the prefix: `Up` afterwards carries on through the entries that start with `R =`. Going past the newest entry gives your draft back.

History is shared by every running `sp2`: what you submit in one shell, the others recall next time they browse.

## Editing

The usual keys move and select: `Ctrl` with the arrows by words, `Home` and `End` to the line ends, `Ctrl+Home` and `Ctrl+End` to the ends of the input, `Shift` to extend a selection, `Ctrl+A` for all of it.

A few more move the way code is built:

`Alt+Left`, `Alt+Right`
: Past everything up to whitespace, so `f(x_1,` is one step.

`Ctrl+Alt+Up`, `Ctrl+Alt+Down`
: To the blank line before or after the paragraph.

`Ctrl+Alt+Left`, `Ctrl+Alt+Right`
: To the bracket opening the block you are in, or past the one closing it. Pressing again climbs to the block around it.

`Alt+M`
: To the partner of the bracket at the cursor. The bracket at the cursor, or just before it, and its partner are always marked, so an unclosed one shows at a glance.

`Alt+Shift+Up`
: Grow the selection step by step: in `ideal(x_1, {y, z})` with the cursor on `y`, it takes `y`, then `y, z`, then `{y, z}`, then `x_1, {y, z}`, the line, the paragraph and everything. `Alt+Shift+Down` shrinks it back the same way.

`Ctrl+Backspace` and `Ctrl+Delete` delete exactly what `Ctrl+Left` and `Ctrl+Right` move over, and `Ctrl+W` back to the whitespace. `Ctrl+X` cuts the selection, or the whole line when nothing is selected. `Ctrl+U` deletes the current line and `Ctrl+L` the whole input.

`Ctrl+Z` undoes and `Ctrl+Shift+Z` redoes. Typing undoes a word at a time and a run of deletions all at once; clearing, pasting, recalling history and taking a completion are one step each. Submitting starts a fresh input with nothing to undo.

Every key here can be rebound; see [Keymaps](../custom/keymaps.md) and the [full key list](../reference/keys.md).
