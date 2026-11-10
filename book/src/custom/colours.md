# Colour schemes

Pick a scheme in a few keystrokes, or edit any colour of any part of the shell in plain TOML: code, values, pickers, the status line, all from the same set of highlight groups.

<!-- capture: the same session in Everforest, Catppuccin and Gruvbox, side by side. -->

## Choose a scheme

Open the settings with `Ctrl+O`; the *appearance* tab has four rows:

theme
: The scheme family: Catppuccin, Everforest, Gruvbox, Gruvbox Material, Kanagawa, Rosé Pine or Tokyo Night.

contrast
: The family's variants for the current mode, such as hard, medium and soft, or Catppuccin's mocha, macchiato and frappé.

mode
: Dark or light, where the family has both.

background
: An opaque canvas, or a transparent one that lets the terminal show through empty space. Code bands, output boxes, selections and pickers keep their backgrounds either way.

Each scheme uses its upstream palette with its own colour names and RGB values, and assigns them after the scheme's own guidance where there is one, such as [Catppuccin's style guide](https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md) and [Everforest's highlighting semantics](https://github.com/sainnhe/everforest/blob/master/palette.md).

## Semantic colours { #semantic-colours }

With the language server [`m2-ls`](https://crates.io/crates/m2-ls) installed (`cargo install m2-ls`, 2.0 or later), code also takes colours from what the server knows: a method apart from a variable, a ring generator, an option name, the head of a method installation, a library function. They are laid over the syntax colours, the way Neovim lays LSP highlights over tree-sitter ones, and while you type the previous colours stay until new ones arrive.

They apply to the input, submitted cells, code views and documentation examples. Analysis reads your session's source; it never evaluates code, so names created dynamically may stay unknown. Without the server, or if it stops, code keeps its syntax colours. The *language server* and *semantic highlighting* rows of the [rendering settings](../reference/settings.md#rendering) switch it.

## Edit any colour

On first use, Spectral writes editable files to `themes/` in its configuration directory (see [Files and directories](../reference/files.md)). Restart `sp2` after editing them.

`groups.toml`
: Declares every highlight group once, for code and interface alike: which groups each scheme must supply, how the others link to them, and which tree-sitter captures each group draws.

One file per scheme, such as `everforest.toml`
: The scheme's native `[palette]`, its styles for the required groups, and any overrides.

A linked group inherits another group's final style and may add to it:

```toml
# groups.toml
Text = { required = true }
Value = { link = "Text" }
RecentOutput = { link = "Accent", bold = true }
DiagnosticError = { sp_from = "Error.fg", underline = true }
Conditional = { link = "Keyword", captures = ["keyword.conditional"] }
```

A palette colour has one value per mode, or one per variant:

```toml
# everforest.toml
[palette.bg0]
dark = { hard = "#272e33", medium = "#2d353b", soft = "#333c43" }
light = { hard = "#fffbef", medium = "#fdf6e3", soft = "#f3ead3" }
```

To change a style for one variant only, use a mode-qualified override:

```toml
# kanagawa.toml
[variant_overrides.light.lotus]
CompletionHint = { link = "Muted", italic = true }
```

### The rules

- `fg`, `bg` and `sp` name palette colours; `"none"` clears a channel. `fg_from`, `bg_from` and `sp_from` copy another group's channel, as in `Error.fg`.
- An override replaces the whole style, modifiers included; set a modifier to `false` to remove an inherited one.
- `captures` moves a tree-sitter capture to a group, recolouring it everywhere code appears. Each capture belongs to exactly one group, and captures are declared only in `groups.toml`.
- `saturation` and `lightening` take percentages; `darken` and `tint` take fractions from 0 to 1.
- Unknown names, a missing required group, a missing variant colour and link cycles are errors: the shell reports them and falls back to the bundled files. Deleting a file restores the bundled version on the next start, and files you never edited follow new defaults when Spectral updates.

## Find a group's name

`Ctrl+Shift+D` opens the highlight inspector at the lower right: every active group with its foreground and background, for the current theme. Scroll it with the arrow or page keys; `Esc` closes it.

## Plain glyphs for limited terminals

Set `SPECTRAL_M2_BASIC_GLYPHS=1` for simple prompt, divider, separator and spinner symbols. `TERM=dumb` selects them automatically.
