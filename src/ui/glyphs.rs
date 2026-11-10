//! Symbols the shell's chrome draws beyond ASCII, each with the spelling a
//! limited terminal uses instead.

#![allow(clippy::non_ascii_literal)]

use std::sync::LazyLock;

use crate::env::ENV;
use these_macros_should_be_illegal::forall;

pub mod maths;

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
    renderer_typeset,
    renderer_ascii,
    renderer_unicode,
    renderer_plain,
    // Stands in for the leading part of text trimmed to fit.
    trimmed,
    // Either side of a picker value that steps with the horizontal arrows.
    step_back,
    step_forward,
    // Divides a picker's rows from what it shows beneath them.
    picker_rule,
    // A full-width rule: under a title, and around the syntax tree.
    rule,
    // What a table too wide to fit draws between the cells it stacks in a row.
    dashed_rule,
    // The syntax tree's branches: to a child, to the last child, and the trunk
    // carried down past a child that has siblings below it.
    tree_branch,
    tree_last_branch,
    tree_trunk,
    // Between the parts of a line of hints or figures.
    separator,
    // Stands in for text left out, inside a sentence.
    ellipsis,
    // Sets apart a clause added to a label, and stands in for a missing label.
    dash,
    // The keys that move a selection or scroll a list up and down.
    vertical_arrows,
    // A swatch of colour the width of two cells.
    swatch,
    // Bookmarks the open preview, and labels the bookmarked tab.
    bookmark_add,
    bookmark,
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
    renderer_markup: Glyph::new("\u{f0219}", "D"),
    renderer_typeset: Glyph::new("\u{f02e9}", "P"),
    renderer_ascii: Glyph::new("\u{f120}", "A"),
    renderer_unicode: Glyph::new("∑", "U"),
    renderer_plain: Glyph::new("\u{f036}", "T"),
    trimmed: Glyph::new("󰨾 ", "..."),
    step_back: Glyph::new("‹", "<"),
    step_forward: Glyph::new("›", ">"),
    picker_rule: Glyph::new("─", "-"),
    rule: Glyph::new("─", "-"),
    dashed_rule: Glyph::new("┄", "-"),
    tree_branch: Glyph::new("├─", "|-"),
    tree_last_branch: Glyph::new("╰─", "`-"),
    tree_trunk: Glyph::new("│", "|"),
    separator: Glyph::new("·", "|"),
    ellipsis: Glyph::new("…", "..."),
    dash: Glyph::new("—", "-"),
    vertical_arrows: Glyph::new("↑↓", "^v"),
    swatch: Glyph::new("██", "##"),
    bookmark_add: Glyph::new("☆", "+"),
    bookmark: Glyph::new("★", "*"),
};

/// The pending-cell spinner's frames on a terminal that draws nerd-font icons.
pub const SPINNER_FRAMES: &[&str] = &["󰋙 ", "󰫃 ", "󰫄 ", "󰫅 ", "󰫆 ", "󰫇 ", "󰫈 ", "󰋘 ", " "];

/// Hints that name an M2 operator spelled beyond ASCII.
pub const OPERATOR_HINTS: [&str; 2] = [
    "Use the operator ⧢ for shuffle products",
    "Use the operator ⊠ for exterior products",
];

#[cfg(test)]
mod tests {
    use super::GLYPHS;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn basic_glyphs_fit_one_cell() {
        let glyphs = GLYPHS;
        for glyph in [
            glyphs.prompt,
            glyphs.split_divider,
            glyphs.cell_separator,
            glyphs.pending_separator,
            glyphs.fold_rule,
            glyphs.unevaluated_cell,
            glyphs.rejected_cell,
            glyphs.renderer_markup,
            glyphs.renderer_typeset,
            glyphs.renderer_ascii,
            glyphs.renderer_unicode,
            glyphs.renderer_plain,
            glyphs.step_back,
            glyphs.step_forward,
            glyphs.picker_rule,
            glyphs.rule,
            glyphs.dashed_rule,
            glyphs.tree_trunk,
            glyphs.separator,
            glyphs.dash,
            glyphs.bookmark_add,
            glyphs.bookmark,
        ] {
            assert_eq!(UnicodeWidthStr::width(glyph.for_basic(true)), 1);
            assert_eq!(UnicodeWidthStr::width_cjk(glyph.for_basic(true)), 1);
        }
    }

    #[test]
    fn prompts_and_divider_fall_back_to_plain_ascii() {
        let glyphs = GLYPHS;
        for glyph in [glyphs.prompt, glyphs.prompt_typing, glyphs.split_divider] {
            assert!(glyph.for_basic(true).is_ascii());
        }
    }

    #[test]
    fn basic_spellings_keep_their_glyphs_width() {
        let glyphs = GLYPHS;
        for glyph in [
            glyphs.tree_branch,
            glyphs.tree_last_branch,
            glyphs.vertical_arrows,
            glyphs.swatch,
        ] {
            assert!(glyph.for_basic(true).is_ascii());
            assert_eq!(
                UnicodeWidthStr::width(glyph.for_basic(true)),
                UnicodeWidthStr::width(glyph.for_basic(false))
            );
        }
    }
}
