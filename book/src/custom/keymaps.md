# Keymaps

Every command and every editing key can be rebound, in the same picker as the other settings. Keys you bind are saved with your settings and kept between sessions.

## Rebind a key

1. Open the keymaps with `Ctrl+K`, or `/keymaps`. It is the last tab of the settings.
2. Select a row and press `Enter`. Its key changes to *Type keybind…*.
3. Press the new shortcut, or `Esc` to keep the old one.

Rows come in five contexts: **global** commands, and the **movement**, **selection**, **editing** and **recall** keys of the input. The selected row names its context below the list.

## What can be a shortcut

- a character with `Ctrl` or `Alt`;
- any other key with `Ctrl`, `Alt` or `Shift`;
- `F2` to `F24` on their own.

Plain typing, `F1`, terminal control characters such as `Ctrl+I`, and keys already used for editing are refused, and capture stays on until you press a valid one. Submitting with `Enter`, scrolling with `PageUp` and `PageDown`, and the highlight inspector `Ctrl+Shift+D` are fixed and not listed.

*Cancel execution*, `Ctrl+C` by default, can be moved too; after three seconds of computing, the status line shows whichever key it has.

## Reset

*Restore keymaps*, at the bottom of the tab, resets the keys alone. *Restore defaults* resets every setting, the keys and the appearance included.

The defaults are listed in [Default keys](../reference/keys.md).
