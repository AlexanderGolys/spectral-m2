//! Text the shell draws in a spelling for each [`Charset`]: the symbols any font
//! has, Unicode, or Nerd Font icons.
//!
//! A [`Glyph`] spells itself; anything drawn from glyphs implements [`Spelled`]
//! by spelling its pieces with the charset it was given, so one choice in the
//! settings respells the whole interface.
#![allow(clippy::non_ascii_literal)]
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use ratatui::symbols::border::{self, Set};
use serde::{Deserialize, Serialize};

use crate::ui::cycle::cyclic;

cyclic! {
    /// The symbols the terminal's font is trusted with, each charset adding to the
    /// one before it.
    #[non_exhaustive]
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Charset {
        /// Plain ASCII: `>`, `|`, `...`.
        Ascii = "ascii",
        /// Unicode symbols most fonts have: `❯`, `│`, `…`.
        Unicode = "unicode",
        /// Unicode and Nerd Font icons.
        #[default]
        Nerd = "nerd font",
    }
}

/// The charset the shell draws with now, as the position of its variant.
static CHARSET: AtomicU8 = AtomicU8::new(Charset::Nerd as u8);

impl Charset {
    pub fn current() -> Self {
        Self::CHOICES[usize::from(CHARSET.load(Relaxed))]
    }

    pub fn choose(self) {
        CHARSET.store(self as u8, Relaxed);
    }
}

/// Anything drawn whose text depends on the charset.
pub trait Spelled {
    type Spelling;

    fn spelled(&self, charset: Charset) -> Self::Spelling;

    fn current(&self) -> Self::Spelling {
        self.spelled(Charset::current())
    }
}

impl<A: Spelled, B: Spelled> Spelled for (A, B) {
    type Spelling = (A::Spelling, B::Spelling);

    fn spelled(&self, charset: Charset) -> Self::Spelling {
        (self.0.spelled(charset), self.1.spelled(charset))
    }
}

/// One thing spelled for every charset, a richer charset keeping the plainer
/// spelling unless given its own:
/// `Spellings::new("⋯", "...").nerd("󰇘")` is `...` in ASCII, `⋯` in Unicode and
/// `󰇘` with a Nerd Font, while `Spellings::plain("d").nerd("󰗚")` is `d` until
/// Nerd Font icons are there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Spellings<T> {
    ascii: T,
    unicode: T,
    nerd: T,
}

impl<T: Copy> Spellings<T> {
    pub const fn plain(ascii: T) -> Self {
        Self::new(ascii, ascii)
    }

    pub const fn new(unicode: T, ascii: T) -> Self {
        Self {
            ascii,
            unicode,
            nerd: unicode,
        }
    }

    pub const fn nerd(self, nerd: T) -> Self {
        Self { nerd, ..self }
    }
}

impl<T: Copy> Spelled for Spellings<T> {
    type Spelling = T;

    fn spelled(&self, charset: Charset) -> T {
        match charset {
            Charset::Ascii => self.ascii,
            Charset::Unicode => self.unicode,
            Charset::Nerd => self.nerd,
        }
    }
}

/// A symbol in its spelling for each charset.
pub type Glyph = Spellings<&'static str>;

/// The symbols of the shell's chrome.
#[enum_fn(pub const glyph -> Glyph)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Glyphs {
    Prompt = Glyph::new("❯", ">").nerd("\u{f105}"),
    PromptTyping = Glyph::new("»", ">>").nerd("\u{f101} "),
    SplitDivider = Glyph::new("│", "|"),
    CellSeparator = Glyph::new("│", "|"),
    PendingSeparator = Glyph::new("⋯", "-").nerd("󰇘"),
    /// Tree rows are spelled three columns wide, so a child's branch starts under
    /// its parent's label.
    TreeBranch = Glyph::new("├─ ", "+- "),
    TreeLast = Glyph::new("╰─ ", "`- "),
    TreeContinue = Glyph::new("│  ", "|  "),
    TreeIndent = Glyph::plain("   "),
    TreeRule = Glyph::new("─", "-"),
    FoldRule = Glyph::new("─", "-"),
    FoldOpen = Glyph::new("▾", "v"),
    FoldClosed = Glyph::new("▸", ">"),
    FoldedLines = Glyph::new("⋯", "..."),
    Ellipsis = Glyph::new("…", "..."),
    /// A line break in code laid on one row, one column wide like any character.
    LineBreak = Glyph::new("↵", " "),
    HintSeparator = Glyph::new("·", "-"),
    UpDown = Glyph::new("↑↓", "^v"),
    LeftRight = Glyph::new("←→", "<>"),
    Range = Glyph::new("–", "-"),
    Dash = Glyph::new("—", "--"),
    UnevaluatedCell = Glyph::new("○", ".").nerd("\u{f28b}"),
    RejectedCell = Glyph::new("✗", "x").nerd("\u{f530}"),
    RendererMarkup = Glyph::plain("m").nerd("󱚊"),
    RendererExternal = Glyph::plain("e").nerd("\u{eac4}"),
    RendererNet = Glyph::plain("n").nerd("󰊉"),
    RendererRod = Glyph::plain("*").nerd("󰎃"),
    RendererString = Glyph::plain("s").nerd("\u{eb69}"),
    RendererSpeek = Glyph::plain("P").nerd("󰛐"),
    Trimmed = Glyph::new("[…]", "[..]").nerd("..󰴏"),
    StepBack = Glyph::new("‹", "<").nerd("\u{f438}"),
    StepForward = Glyph::new("›", ">").nerd("\u{f44a}"),
    PickerRule = Glyph::new("─", "-"),
    /// The rule under the side panel's header, and where it meets the panel's side.
    PanelRule = Glyph::new("─", "-"),
    PanelRuleJoint = Glyph::new("├", "+"),
    HeadingRule = Glyph::new("─", "="),
    StackedCellRule = Glyph::new("┄", "-"),
    TabDocumentation = Glyph::plain("d").nerd("󰗚"),
    TabHover = Glyph::plain("i").nerd("󰋽"),
    TabSource = Glyph::plain("f").nerd("󰈮"),
    TabCode = Glyph::plain("c").nerd("󰅩 "),
    TabOutput = Glyph::plain("o").nerd("󰞷 "),
    TabSyntaxTree = Glyph::plain("t").nerd("󱁉 "),
    TabNotes = Glyph::plain("n").nerd("󱓧 "),
    TabBack = Glyph::new("‹", "<"),
    TabForward = Glyph::new("›", ">"),
}

impl Spelled for Glyphs {
    type Spelling = &'static str;

    fn spelled(&self, charset: Charset) -> &'static str {
        self.glyph().spelled(charset)
    }
}

/// The corners and sides of a box in their spelling for each charset.
pub type Border = Spellings<Set<'static>>;

/// A remark standing in for content, set off by ellipses: `… laying out …`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Remark(pub &'static str);

impl Spelled for Remark {
    type Spelling = String;

    fn spelled(&self, charset: Charset) -> String {
        let ellipsis = Glyphs::Ellipsis.spelled(charset);
        format!("{ellipsis} {} {ellipsis}", self.0)
    }
}

impl Glyphs {
    pub const SQUARE_BORDER: Border = Border::new(border::PLAIN, Self::ASCII_BORDER);
    pub const ROUNDED_BORDER: Border = Border::new(border::ROUNDED, Self::ASCII_BORDER);
    const ASCII_BORDER: Set<'static> = Set {
        top_left: "+",
        top_right: "+",
        bottom_left: "+",
        bottom_right: "+",
        vertical_left: "|",
        vertical_right: "|",
        horizontal_top: "-",
        horizontal_bottom: "-",
    };

    /// The branch leading to a row of a drawn tree, and the trunk carried past it
    /// to its children's rows.
    pub const fn tree_limb(last: bool) -> (Self, Self) {
        match last {
            true => (Self::TreeLast, Self::TreeIndent),
            false => (Self::TreeBranch, Self::TreeContinue),
        }
    }
}
