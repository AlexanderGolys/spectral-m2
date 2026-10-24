# Troubleshooting

## Blackboard letters don't look like LaTeX { #blackboard }

With [Math fonts](settings.md) on, `ZZ` is drawn as `ℤ`. Spectral sends the character; the terminal picks the font that draws it, and no terminal lets an application choose. Programming fonts rarely contain `ℤ`, `ℚ`, `ℝ` or `ℂ`, so the terminal falls back to whatever installed font does. That is usually a thin sans-serif outline, nothing like `\mathbb` in LaTeX.

**Latin Modern Math**, the OpenType form of LaTeX's own Computer Modern, has blackboard capitals at their Unicode code points. Install it (it ships with TeX Live, and most distributions package it, e.g. `otf-latin-modern`) and send the double-struck capitals to it.

Ghostty
: In `~/.config/ghostty/config`:

  ```text
  font-codepoint-map = U+2102,U+210D,U+2115,U+2119-U+211A,U+211D,U+2124,U+1D538-U+1D551=Latin Modern Math
  ```

  Check with `ghostty +show-face --cp=0x2124`, which should name `Latin Modern Math`.

kitty
: In `~/.config/kitty/kitty.conf`:

  ```text
  symbol_map U+2102,U+210D,U+2115,U+2119-U+211A,U+211D,U+2124,U+1D538-U+1D551 Latin Modern Math
  ```

Other terminals
: Without a per-character font mapping, a terminal takes these glyphs from its fallback fonts, so the result depends on which installed font fontconfig ranks first for them.

> [!IMPORTANT]
> A terminal reads the list of installed fonts when it starts. Ghostty keeps one process for all its windows, so neither reloading the configuration nor opening a new window picks up a font installed meanwhile: quit every window (`pkill -x ghostty`) and start it again. tmux sessions survive this.

### Fine print

The mapping above covers blackboard capitals only. Script and fraktur capitals (`\mathcal`, `\mathfrak`) live at other code points and need their own mapping, to Latin Modern Math or STIX Two Math. For maths drawn exactly as LaTeX sets it, use [typeset maths](settings.md#typeset-maths) instead.

## Typeset maths is blank inside tmux { #typeset-tmux }

tmux stands between Spectral and the terminal and drops pictures unless told to pass them on. In `~/.config/tmux/tmux.conf`:

```text
set -g allow-passthrough on
set -g focus-events on
```

The first lets the pictures through, which needs tmux 3.3 or newer; the second tells Spectral when its pane is shown again, so it can resend the pictures tmux discarded while the pane was hidden. Reload with `tmux source-file ~/.config/tmux/tmux.conf`.

tmux keeps the terminal's answer when Spectral asks whether it shows pictures, so inside tmux Spectral goes by the terminal tmux names instead. If *maths* says `typeset` is unavailable, check that name:

```sh
tmux display-message -p '#{client_termtype} #{client_termname}'
```

It should mention `kitty` or `ghostty`. Spectral checks once, as it starts, so restart it after attaching from a different terminal.
