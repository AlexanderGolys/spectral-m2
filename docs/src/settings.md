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
| Complete after | Characters of a name, or of a file name in a string, typed before it is completed. |
| Status line | Below input, above input, or off. |
| Fractions | Stacked, underlined, or inline. |
| Math fonts | Off by default, which draws a ring by the name it has in code, `ZZ`; on draws its blackboard glyph, `ℤ`, and sets `\mathcal` and `\mathfrak` capitals as script and fraktur glyphs. |
| Max output height | Rows a returned value shows before its middle folds away. |
| Compute examples | How long each documentation example may run when it is computed again, so its value is drawn from its structure as a live value is; `off` keeps the output the documentation recorded. Each page runs in a fresh Macaulay2 with its package loaded, and stops at the first example that runs out of time or answers differently from the record. |
| Restore default | Resets settings, keymaps, and appearance. |

## Appearance

On a limited terminal, set `SPECTRAL_M2_BASIC_GLYPHS=1` to use simple one-column prompt, divider, separator, and spinner symbols. `TERM=dumb` selects these symbols automatically.

Transparent background lets the terminal show through the canvas: otherwise empty feed or preview cells, and the space inside laid-out values. Code bands, output boxes, selections, pickers, and other explicitly styled UI keep their backgrounds.

The installed families are Catppuccin, Tokyo Night, Gruvbox, Gruvbox Material, Everforest, Rosé Pine, and Kanagawa. Each supplies the same eight hues: red, green, yellow, blue, purple, aqua, orange, and gray. Every hue has a base, light, and dark shade; every variant also has five foregrounds (`fg0`–`fg4`) and five backgrounds (`bg0`–`bg4`). Existing dark/light modes and named variants remain available.

Scheme TOML files supply a single hue color or native shades. A mode-wide value applies to all its variants; a variant table is needed only when values differ:

```toml
[colors.red]
dark = { base = "#e67e80", dark = "#b85f60" }
light = "#f85552"

[colors.fg]
dark = ["#d3c6aa", "#859289", "#7a8478"]
light = ["#5c6a72", "#939f91", "#a6b0a0"]
```

Hue arrays are also accepted in base, light, dark order. Missing light/dark shades blend 25% toward the brighter/darker of the default foreground and background. Foreground and background arrays accept one to five colors, starting with the default color. Missing steps are inserted at the midpoints of the widest RGB gaps, preserving supplied colors and their order. With one anchor, a second is derived toward the opposite default color (65% for foregrounds, 20% for backgrounds).

The palette groups are fixed: `Red`, `RedLight`, `RedDark`, and the corresponding names for every hue; `Fg0`–`Fg4`; and `Bg0`–`Bg4`. Colored background groups such as `BgRed` use 5% hue and 95% default background; styles can refer to those colors as `bg_red`, `bg_green`, and so on. These groups expose every resolved color in its intended foreground or background channel, without scheme-specific group tables or overrides.

## Highlight debug view

Press `Ctrl+D` to open the one-column inspector at the lower right. It lists all active UI, Macaulay2, and palette highlight groups, including foreground and background samples. Opening it only shows the panel, without submitting code or adding messages to the feed. Scroll the inspector with arrow or page keys. `Esc` closes only the inspector; `Ctrl+D` toggles it from any shell, preview, or picker.

## Keymaps

Run `/keymaps` to inspect and edit every command shortcut plus `clear input` and `copy selection`. Selecting one and pressing a new modified key updates both dispatch and displayed hints. Movement, submission, and other form-editing keys remain fixed.
