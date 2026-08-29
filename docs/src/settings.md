# Settings and style

Open settings with `Ctrl+O` or `/settings`. Use `Up` and `Down` to choose a row and `Left` or `Right` to adjust it.

| Setting | Choices |
| --- | --- |
| Max input height | Percentage of the pane available to growing multiline input, in 5% steps. |
| Scrolloff | Context rows retained above and below the input cursor. |
| Status line | Below input, above input, or off. |
| Style | Opens the separate style picker. |
| Restore default | Resets settings, keymaps, and appearance. |

## Style

`/style` opens the same style picker directly. Its three ordered axes are:

- **colorscheme** — cycles every scheme variant of the current brightness, keeping each scheme's variants adjacent and deterministic;
- **brightness** — switches between the dark and light variants available for the current scheme;
- **background** — chooses an opaque or transparent empty canvas.

Transparent background affects only otherwise empty feed or preview canvas cells. Code bands, output boxes, selections, pickers, and other explicitly styled UI keep their backgrounds.

The installed families are Catppuccin, Tokyo Night, Gruvbox, Gruvbox Material, Everforest, Rosé Pine, and Kanagawa. Each variant uses its own source palette names internally rather than being flattened into one universal color namespace.

## Highlight debug view

Press `Ctrl+D` or run `/debug` to open the one-column inspector at the lower right. It lists all active UI and Macaulay2 highlight groups, followed by the current variant's native palette names and colors. Scroll it with arrow or page keys. `Esc` closes it; `Ctrl+D` toggles it from any shell, preview, or picker.

## Keymaps

Run `/keymaps` to inspect bindings. The current editor exposes modified-key rebinding for `clear input` and `copy selection`; selecting one and pressing a new modified key updates both the dispatcher and displayed shortcut hints. A replacement cannot steal a fixed binding.
