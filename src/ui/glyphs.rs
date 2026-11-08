//! Symbols the shell's chrome draws beyond ASCII, each with the spelling a
//! limited terminal uses instead.

use std::sync::LazyLock;

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
}

/// UI symbols that have a simpler spelling on limited terminals.
#[derive(Clone, Copy)]
pub struct Glyphs {
    pub prompt: Glyph,
    pub prompt_typing: Glyph,
    pub split_divider: Glyph,
    pub cell_separator: Glyph,
    pub pending_separator: Glyph,
    pub fold_rule: Glyph,
    pub unevaluated_cell: Glyph,
    pub rejected_cell: Glyph,
    /// Marks which renderer laid out an output cell's value.
    pub renderer_markup: Glyph,
    pub renderer_typeset: Glyph,
    pub renderer_ascii: Glyph,
    pub renderer_unicode: Glyph,
    pub renderer_plain: Glyph,
    /// Stands in for the leading part of text trimmed to fit.
    pub trimmed: Glyph,
    /// Either side of a picker value that steps with the horizontal arrows.
    pub step_back: Glyph,
    pub step_forward: Glyph,
    /// Divides a picker's rows from what it shows beneath them.
    pub picker_rule: Glyph,
}

impl Glyphs {
    /// `TERM=dumb` and an explicit override use plain Unicode characters.
    pub fn basic_terminal() -> bool {
        static BASIC: LazyLock<bool> = LazyLock::new(|| {
            std::env::var("TERM").is_ok_and(|term| term == "dumb")
                || std::env::var("SPECTRAL_M2_BASIC_GLYPHS").is_ok_and(|value| value == "1")
        });
        *BASIC
    }
}

/// The symbols of the shell's chrome.
pub const GLYPHS: Glyphs = Glyphs {
    prompt: Glyph {
        preferred: "",
        basic: ">",
    },
    prompt_typing: Glyph {
        preferred: " ",
        basic: ">>",
    },
    split_divider: Glyph {
        preferred: "│",
        basic: "|",
    },
    cell_separator: Glyph {
        preferred: "│",
        basic: "|",
    },
    pending_separator: Glyph {
        preferred: "┈",
        basic: "-",
    },
    fold_rule: Glyph {
        preferred: "─",
        basic: "-",
    },
    unevaluated_cell: Glyph {
        preferred: " ",
        basic: ".",
    },
    rejected_cell: Glyph {
        preferred: " ",
        basic: "x",
    },
    renderer_markup: Glyph {
        preferred: "\u{f0219}",
        basic: "D",
    },
    renderer_typeset: Glyph {
        preferred: "\u{f02e9}",
        basic: "P",
    },
    renderer_ascii: Glyph {
        preferred: "\u{f120}",
        basic: "A",
    },
    renderer_unicode: Glyph {
        preferred: "∑",
        basic: "U",
    },
    renderer_plain: Glyph {
        preferred: "\u{f036}",
        basic: "T",
    },
    trimmed: Glyph {
        preferred: "󰨾 ",
        basic: "...",
    },
    step_back: Glyph {
        preferred: "‹",
        basic: "<",
    },
    step_forward: Glyph {
        preferred: "›",
        basic: ">",
    },
    picker_rule: Glyph {
        preferred: "─",
        basic: "-",
    },
};

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
}
