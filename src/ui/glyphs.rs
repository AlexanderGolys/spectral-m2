//! Symbols the shell's chrome draws beyond ASCII, each with the spelling a
//! limited terminal uses instead.
#![allow(clippy::non_ascii_literal)]
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::LazyLock;

use crate::env::ENV;

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

    /// Select the symbol the shell draws now.
    pub fn current(self) -> &'static str {
        self.for_basic(Glyphs::basic())
    }

    pub const fn new(preferred: &'static str, basic: &'static str) -> Self {
        Self { preferred, basic }
    }
}

/// UI symbols that have a simpler spelling on limited terminals.
#[enum_fn(pub const glyph -> Glyph)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Glyphs {
    Prompt = Glyph::new("", ">"),
    PromptTyping = Glyph::new(" ", ">>"),
    SplitDivider = Glyph::new("│", "|"),
    CellSeparator = Glyph::new("│", "|"),
    PendingSeparator = Glyph::new("󰇘", "-"),
    TreeBranch = Glyph::new("├─ ", "+- "),
    TreeLast = Glyph::new("╰─ ", "`- "),
    TreeContinue = Glyph::new("│  ", "|  "),
    TreeIndent = Glyph::new("   ", "   "),
    FoldRule = Glyph::new("─", "-"),
    FoldOpen = Glyph::new("▾", "v"),
    FoldClosed = Glyph::new("▸", ">"),
    FoldedLines = Glyph::new("⋯", "..."),
    UnevaluatedCell = Glyph::new("", "."),
    RejectedCell = Glyph::new("", "x"),
    RendererMarkup = Glyph::new("󱚊", "m"),
    RendererExternal = Glyph::new("", "e"),
    RendererTex = Glyph::new("", "t"),
    RendererNet = Glyph::new("󰊉", "n"),
    RendererSpecNet = Glyph::new("󰎃", "*"),
    RendererString = Glyph::new("", "s"),
    RendererPeek = Glyph::new("󰈈", "p"),
    RendererSpeek = Glyph::new("󰛐", "P"),
    Trimmed = Glyph::new(" ..󰴏", "[..]"),
    StepBack = Glyph::new("", "<"),
    StepForward = Glyph::new("", ">"),
    PickerRule = Glyph::new("─", "-"),

    TabDocumentation = Glyph::new("󰗚", "d"),
    TabHover = Glyph::new("󰋽", "i"),
    TabSource = Glyph::new("󰈮", "f"),
    TabBookmark = Glyph::new(" ", "*"),
    TabCode = Glyph::new("󰅩 ", "c"),
    TabOutput = Glyph::new("󰞷 ", "o"),
    TabSyntaxTree = Glyph::new("󱁉 ", "t"),
    TabNotes = Glyph::new("󰎞", "n"),
    TabBack = Glyph::new("‹", "<"),
    TabPin = Glyph::new(" ", "+"),
}

/// Whether the *basic glyphs* setting is on, published for every drawing.
static CHOSEN_BASIC: AtomicBool = AtomicBool::new(false);

impl Glyphs {
    /// Whether plain Unicode symbols are drawn: chosen in the settings, or forced.
    pub fn basic() -> bool {
        CHOSEN_BASIC.load(Relaxed) || Self::forced_basic()
    }

    /// Whether the terminal draws nothing but plain symbols, whatever is chosen.
    pub fn forced_basic() -> bool {
        static DUMB: LazyLock<bool> = LazyLock::new(|| ENV.dumb_terminal());
        *DUMB
    }

    pub fn choose_basic(basic: bool) {
        CHOSEN_BASIC.store(basic, Relaxed);
    }
}
