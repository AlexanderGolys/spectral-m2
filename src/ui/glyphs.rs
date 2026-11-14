//! Symbols the shell's chrome draws beyond ASCII, each with the spelling a
//! limited terminal uses instead.

use std::sync::LazyLock;

use crate::env::ENV;
use these_macros_should_be_illegal::forall;

/// A preferred symbol and a plain Unicode fallback for limited terminals.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    preferred: &'static str,
    basic: &'static str,
}

impl Glyph {
    /// Select the symbol for a terminal's glyph capability.
    pub const fn for_basic(self, basic: bool) -> &'static str {
        if basic {
            self.basic
        } else {
            self.preferred
        }
    }

    /// Select the symbol for this process's terminal.
    pub fn current(self) -> &'static str {
        self.for_basic(Glyphs::basic_terminal())
    }

    pub const fn new(preferred: &'static str, basic: &'static str) -> Self {
        Self { preferred, basic }
    }
}

/// UI symbols that have a simpler spelling on limited terminals.
#[forall(pub, Glyph)]
#[derive(Clone, Copy)]
pub struct Glyphs(
    prompt, // 󱃵 󱪂    
    prompt_typing,
    split_divider,
    cell_separator,
    pending_separator,
    fold_rule,
    // Beside the first line of a stretch of code that folds, open or folded.
    fold_open,
    fold_closed,
    // After a folded stretch's first line, before the count of lines it hides.
    folded_lines,
    unevaluated_cell,
    rejected_cell,
    // Marks which renderer laid out an output cell's value.
    renderer_markup,
    renderer_external,
    renderer_tex,
    renderer_net,
    renderer_spec_net,
    renderer_string,
    renderer_peek,
    renderer_speek,
    // Stands in for the leading part of text trimmed to fit.
    trimmed,
    // Either side of a picker value that steps with the horizontal arrows.
    step_back,
    step_forward,
    // Divides a picker's rows from what it shows beneath them.
    picker_rule,
    // One cell for each tab of the pane beside the session.
    tab_documentation,
    tab_hover,
    tab_source,
    tab_bookmark,
    tab_code,
    tab_output,
    tab_syntax_tree,
    // Before the tab's title: back to the previous tab, and bookmark this one.
    tab_back,
    tab_pin,
);

impl Glyphs {
    /// `TERM=dumb` and an explicit override use plain Unicode characters.
    pub fn basic_terminal() -> bool {
        static BASIC: LazyLock<bool> = LazyLock::new(|| ENV.basic_glyphs());
        *BASIC
    }
}

/// The symbols of the shell's chrome.
pub const GLYPHS: Glyphs = Glyphs {
    prompt: Glyph::new("", ">"),
    prompt_typing: Glyph::new(" ", ">>"),
    split_divider: Glyph::new("│", "|"),
    cell_separator: Glyph::new("│", "|"),
    pending_separator: Glyph::new("┈", "-"),
    fold_rule: Glyph::new("─", "-"),
    fold_open: Glyph::new("▾", "v"),
    fold_closed: Glyph::new("▸", ">"),
    folded_lines: Glyph::new("⋯", "..."),
    unevaluated_cell: Glyph::new(" ", "."),
    rejected_cell: Glyph::new(" ", "x"),
    renderer_markup: Glyph::new("󱚊", "m"),
    renderer_external: Glyph::new("󰅩", "e"),
    renderer_tex: Glyph::new("", "t"),
    renderer_net: Glyph::new("󰧭", "n"),
    renderer_spec_net: Glyph::new("", "*"),
    renderer_string: Glyph::new("󰦨", "s"),
    renderer_peek: Glyph::new("󰈈", "p"),
    renderer_speek: Glyph::new("󰛐", "P"),
    trimmed: Glyph::new("󰨾 ", "..."),
    step_back: Glyph::new("‹", "<"),
    step_forward: Glyph::new("›", ">"),
    picker_rule: Glyph::new("─", "-"),
    tab_documentation: Glyph::new("󰗚", "d"),
    tab_hover: Glyph::new("󰋽", "i"),
    tab_source: Glyph::new("󰈮", "f"),
    tab_bookmark: Glyph::new("★", "*"),
    tab_code: Glyph::new("󰅩", "c"),
    tab_output: Glyph::new("󰞷", "o"),
    tab_syntax_tree: Glyph::new("󰙅", "t"),
    tab_back: Glyph::new("‹", "<"),
    tab_pin: Glyph::new("☆", "+"),
};
