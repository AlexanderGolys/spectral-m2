# Settings

Open settings with `Ctrl+O` or `/settings`. They are grouped into three tabs, named along the top of the picker: `Tab` and `Shift+Tab` switch between them. `Up` and `Down` choose a row and `Left` or `Right` adjust it. Each row shows its current value between arrows. Beneath the rows, the *restore defaults* button resets settings, keymaps, and appearance, and under it the selected row describes itself. While the picker is open, the status line names the keys it answers.

Changes are kept between sessions. The appearance is saved to `appearance.json` and the other settings, along with keymap changes, to `settings.json`, both in `$XDG_CONFIG_HOME/spectral-m2`, falling back to `~/.config/spectral-m2`.

**Appearance**

| Setting | Choices |
| --- | --- |
| Theme | Every scheme offering the current mode, keeping the contrast name when the scheme has it and otherwise its position. |
| Contrast | The current theme's variants for the current mode, such as hard, medium, and soft. |
| Mode | The dark and light palettes available for the current theme. |
| Background | An opaque or transparent empty canvas. |

**Layout**

| Setting | Choices |
| --- | --- |
| Status line | Below input, above input, or off. |
| Max input height | Percentage of the pane available to growing multiline input, in 5% steps. |
| Max output height | Rows a returned value shows before its middle folds away. |
| Scrolloff | Context rows retained above and below the input cursor. |
| Complete after | Characters of a name, or of a file name in a string, typed before it is completed. |

**Rendering**

| Setting | Choices |
| --- | --- |
| Semantic highlighting | On by default: use an installed `m2-ls` for semantic source colors, with immediate syntax colors while results arrive. |
| Maths | `unicode` by default: values laid out in characters, with stacked fractions and tall brackets. `typeset` draws them as [typeset pictures](#typeset-maths), marked unavailable where the terminal cannot show them; `ascii` uses Macaulay2's own plain-text spelling. |
| Fractions | Stacked, underlined, or inline. |
| Math fonts | Off by default, which draws a ring by the name it has in code, `ZZ`; on draws its blackboard glyph, `ℤ`, and sets `\mathcal` and `\mathfrak` capitals as script and fraktur glyphs. How they look is up to the terminal's fonts; see [Troubleshooting](troubleshooting.md#blackboard). |
| Compute examples | How long each documentation example may run when it is computed again, so its value is drawn from its structure as a live value is; `off` keeps the output the documentation recorded. Each page runs in a fresh Macaulay2 with its package loaded, and stops at the first example that runs out of time or answers differently from the record. |

## Semantic highlighting

Install the optional language server with `cargo install m2-ls --locked` and restart Spectral. Semantic colors apply to live input, submitted cells, code previews, and the source shown with syntax trees. Analysis includes earlier session source, queued cells, and the current continuation; it does not evaluate code or inspect runtime values. Dynamically created bindings and side effects may remain unknown.

Without the server, or if it stops responding, source keeps its syntax colors. Switch *semantic highlighting* off and on to retry. Completion and syntax diagnostics continue to use the shell's existing providers.

The theme groups `Method`, `TypeParameter`, `SemanticCommand`, `RingGenerator`, `Option`, `File`, and `Unresolved` expose semantic roles. They inherit existing theme colors by default and can be customized in `themes/groups.toml` like other linked groups.

## Typeset maths { #typeset-maths }

With *maths* set to `typeset`, mathematical values are drawn as TeX pictures using the theme's math colours, with matrices and fractions at their full height. Strings, `null`, names and Nets remain terminal text. Collections compose their entries individually, so a Net does not change how neighbouring mathematics is rendered. Type annotations also keep ordinary words in the terminal font and typeset their mathematical parts, in the type colour. Pictures scroll, fold and split with the panes, and a theme change repaints them.

Formulas are set by the built-in [RaTeX](https://github.com/erweixin/RaTeX), with KaTeX's layout and Computer Modern fonts, so nothing else needs installing.

It needs a terminal that speaks the kitty graphics protocol with Unicode placeholders: kitty and Ghostty do, inside tmux too (see [Troubleshooting](troubleshooting.md#typeset-tmux)). Elsewhere the choice is still offered, struck through and marked unavailable in that terminal, and values are drawn as Unicode text until Spectral runs in such a terminal. A value too wide for the pane shrinks to fit while it stays readable; past that, or when its TeX cannot be set, it is drawn as Unicode text.

## Appearance

On a limited terminal, set `SPECTRAL_M2_BASIC_GLYPHS=1` to use simple ASCII prompt, divider, separator, and spinner symbols. `TERM=dumb` selects these symbols automatically.

Transparent background lets the terminal show through the canvas: otherwise empty feed or preview cells, and the space inside laid-out values. Code bands, output boxes, selections, pickers, and other explicitly styled UI keep their backgrounds.

The installed families are Catppuccin, Tokyo Night, Gruvbox, Gruvbox Material, Everforest, Rosé Pine, and Kanagawa. Each scheme keeps its upstream palette names and RGB values. Names are the same across a scheme's variants, but can differ between schemes: Catppuccin has `mauve`, Rosé Pine has `iris`, and Kanagawa has `oniViolet`. There are no generated palette shades, ramps, or universal hue aliases.

### Theme configuration

On first use, Spectral provides editable files in `$XDG_CONFIG_HOME/spectral-m2/themes/` (or `~/.config/spectral-m2/themes/`). Restart Spectral after editing them:

- `groups.toml` declares every highlight group once, for code and interface alike: required groups, default links, and the Tree-sitter captures each group draws.
- Each scheme's TOML file supplies its native `[palette]`, required `[groups]` styles, and optional group overrides.

Existing files are preserved. Newly introduced linked groups inherit the bundled defaults even when an older `groups.toml` does not list them. Invalid configurations report their error and fall back to bundled defaults. Removing a config file restores its bundled version on the next start.

The source default is `src/ui/controller/settings/theme/groups.toml`. Rust's group enum, style fields, reflection, and indexing are generated from that file at build time; adding a group requires one declaration there. Rebuilding is required to add a group to the application, but users can edit existing groups and palettes without rebuilding.

### Required groups and default links

A required entry must be supplied in every scheme's `[groups]` table. Linked entries inherit a resolved group and can add modifiers or copy one of its color channels:

```toml
# groups.toml
Text = { required = true }
Bg0 = { required = true }
Canvas = { link = "Bg0" }
Value = { link = "Text" }
RecentOutput = { link = "Accent", bold = true }
DiagnosticError = { sp_from = "Error.fg", underline = true }
Conditional = { link = "Keyword", captures = ["keyword.conditional"] }
```

`Bg0`–`Bg4` are shared surface roles, assigned native colors separately by each scheme. Syntax, text, muted text, borders, accents, and severities also have required base groups. A scheme can set any linked group in `[groups]` to replace its default style entirely. A style with `link` inherits that group's final style before applying its own fields. Dependents follow scheme and variant overrides, including overrides of required groups.

`captures` lists the Tree-sitter query captures, such as `keyword.conditional`, that code highlighting draws with the group. Each capture belongs to exactly one group; moving a capture to another group recolours it everywhere code appears. An entry without `captures` keeps the bundled ones, so older files keep highlighting code. Captures are declared only in `groups.toml`, never in a scheme.

### Native palettes and scheme overrides

A named palette color has one RGB value per mode, or explicit values per variant when they differ. Every declared color must exist in every supported variant:

```toml
# everforest.toml
[palette.red]
dark = "#e67e80"
light = "#f85552"

[palette.bg0]
dark = { hard = "#272e33", medium = "#2d353b", soft = "#333c43" }
light = { hard = "#fffbef", medium = "#fdf6e3", soft = "#f3ead3" }
```

To change a style, edit the scheme's `[groups]` table. For a single variant, use a mode-qualified override, such as this example in `kanagawa.toml`:

```toml
[variant_overrides.light.lotus]
MarkupHeading3 = { fg = "lotusViolet1", bold = true }
CompletionHint = { link = "Muted", italic = true }
PlaceholderHint = { fg = "lotusGray" }
```

Mode qualification keeps dark and light variants with the same name independent. Overrides replace the whole style, including modifiers. `fg`, `bg`, and `sp` refer to native palette names; `"none"` clears a channel. `fg_from`, `bg_from`, and `sp_from` copy `Group.fg`, `Group.bg`, or `Group.sp`. Modifiers accept `true` and `false` so inherited modifiers can be removed. Unknown names, missing required groups, missing variant colors, and link cycles are errors.

The existing printed-text desaturation, recent-label darkening, and error-surface tint remain explicit style operations in `groups.toml`. `saturation` and `lightening` use percentages; `darken` and `tint` use fractions from 0 to 1, with `tint_from` naming a group channel. A scheme can use `bg_blend` with a native palette color and `bg_blend_amount` to apply an opacity guideline; Catppuccin's selection uses 25% `overlay2` over `base`.

Assignments follow upstream guidance where available: [Catppuccin's style guide](https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md), [Everforest's highlighting semantics](https://github.com/sainnhe/everforest/blob/master/palette.md), and [Rosé Pine's color roles](https://rosepinetheme.com/palette/). Other assignments use the upstream Neovim highlights to select existing native colors for Spectral's existing groups. Gruvbox's palette comes from [the original Gruvbox declaration](https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim). Each scheme file records its palette source.

Input hints, picker parts, cell markers, status-line parts, settings labels, and syntax-tree markers have separate named groups even when their default colors match. Open the highlight inspector with `Ctrl+Shift+D` to find the name. Kanagawa's picker retains its separate native popup, selection, and border colors, independent of printed output.

## Highlight debug view

Press `Ctrl+Shift+D` to open the one-column inspector at the lower right. It lists all active UI, Macaulay2, and base highlight groups, including foreground and background samples. Opening it only shows the panel, without submitting code or adding messages to the feed. Scroll the inspector with arrow or page keys. `Esc` closes only the inspector; `Ctrl+Shift+D` toggles it from any shell, preview, or picker.

## Keymaps

Run `/keymaps` to inspect and edit command shortcuts. Select a row and press `Enter`; its shortcut changes to “Type keybind…”. Press the replacement shortcut, or `Esc` to cancel. Shortcuts require Ctrl or Alt for characters, or Ctrl, Alt, or Shift for other keys; `F2` through `F24` also work without modifiers. Ordinary typing, `F1`, terminal control keys such as `Ctrl+I`, and existing editing keys cannot be assigned. Invalid keys leave capture active. The *cancel execution* action defaults to `Ctrl+C` and can be remapped. It cancels running computations and queued work, or quits when idle. After three seconds of processing, the status line shows its current shortcut. Movement, submission, and other form-editing keys remain fixed.
