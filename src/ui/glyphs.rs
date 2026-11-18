//! Symbols the shell's chrome draws from a Nerd Font, each with its spelling in
//! *ascii* mode: plain symbols any font has.
#![allow(clippy::non_ascii_literal)]
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// A preferred symbol and its *ascii*-mode spelling.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    preferred: &'static str,
    ascii: &'static str,
}

impl Glyph {
    pub const fn for_ascii(self, ascii: bool) -> &'static str {
        if ascii {
            self.ascii
        } else {
            self.preferred
        }
    }

    /// Select the symbol the shell draws now.
    pub fn current(self) -> &'static str {
        self.for_ascii(Glyphs::ascii())
    }

    pub const fn new(preferred: &'static str, ascii: &'static str) -> Self {
        Self { preferred, ascii }
    }
}

/// UI symbols that *ascii* mode spells differently.
#[enum_fn(pub const glyph -> Glyph)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Glyphs {
    Prompt = Glyph::new("", ">"),
    PromptTyping = Glyph::new(" ", ">>"),
    SplitDivider = Glyph::new("│", "|"),
    CellSeparator = Glyph::new("│", "|"),
    PendingSeparator = Glyph::new("󰇘", "-"),
    TreeBranch = Glyph::new("├─", "+- "),
    TreeLast = Glyph::new("╰─", "`- "),
    TreeContinue = Glyph::new("│  ", "|  "),
    TreeIndent = Glyph::new("  ", "   "),
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
    RendererRod = Glyph::new("󰎃", "*"),
    RendererString = Glyph::new("", "s"),
    RendererPeek = Glyph::new("󰈈", "p"),
    RendererSpeek = Glyph::new("󰛐", "P"),
    Trimmed = Glyph::new("..󰴏", "[..]"),
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
    TabNotes = Glyph::new("󱓧 ", "n"),
    TabBack = Glyph::new("‹", "<"),
    TabPin = Glyph::new(" ", "+"),
}

/// Whether the *ascii* setting is on, published for every drawing.
static ASCII: AtomicBool = AtomicBool::new(false);

impl Glyphs {
    pub fn ascii() -> bool {
        ASCII.load(Relaxed)
    }

    pub fn choose_ascii(ascii: bool) {
        ASCII.store(ascii, Relaxed);
    }
}
