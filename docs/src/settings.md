# Settings

Open settings with `Ctrl+O` or `/settings`. Use `Up` and `Down` to choose a row and `Left` or `Right` to adjust it. Each row describes itself in a column after its value.

Changes are kept between sessions. The appearance is saved to `appearance.json` and the other settings, along with keymap changes, to `settings.json`, both in `$XDG_CONFIG_HOME/spectral-m2`, falling back to `~/.config/spectral-m2`.

| Setting | Choices |
| --- | --- |
| Theme | Every scheme offering the current mode, keeping the contrast name when the scheme has it and otherwise its position. |
| Contrast | The current theme's variants for the current mode, such as hard, medium, and soft. |
| Mode | The dark and light palettes available for the current theme. |
| Background | An opaque or transparent empty canvas. |
| Max input height | Percentage of the pane available to growing multiline input, in 5% steps. |
| Scrolloff | Context rows retained above and below the input cursor. |
| Status line | Below input, above input, or off. |
| Fractions | Stacked, underlined, or inline. |
| Max output height | Rows a returned value shows before its middle folds away. |
| Restore default | Resets settings, keymaps, and appearance. |

## Appearance

Transparent background lets the terminal show through the canvas: otherwise empty feed or preview cells, and the space inside laid-out values. Code bands, output boxes, selections, pickers, and other explicitly styled UI keep their backgrounds.

The installed families are Catppuccin, Tokyo Night, Gruvbox, Gruvbox Material, Everforest, Rosé Pine, and Kanagawa. Each variant uses its own source palette names internally rather than being flattened into one universal color namespace.

## Highlight debug view

Press `Ctrl+D` or run `/debug` to open the one-column inspector at the lower right. It lists all active UI and Macaulay2 highlight groups, followed by the current variant's native palette names and colors. Opening it submits a real three-cell value/muted/null probe through the active Macaulay2 session and leaves that post in the feed. Scroll the inspector with arrow or page keys. `Esc` closes only the inspector; `Ctrl+D` toggles it from any shell, preview, or picker.

## Keymaps

Run `/keymaps` to inspect and edit every command shortcut plus `clear input` and `copy selection`. Selecting one and pressing a new modified key updates both dispatch and displayed hints. Movement, submission, and other form-editing keys remain fixed.
