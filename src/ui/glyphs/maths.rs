//! The symbols maths is drawn with in a terminal: M2's TeX commands as the
//! characters they stand for, and the pieces tall fences are built from.
//!
//! These have no plain spelling: the Unicode renderer exists to draw them, and a
//! limited terminal is given M2's own Net spelling instead.

use crate::backend::OPTION_ARROW;

/// A placeholder box, TeX's `\square`, which ends an interval field's name.
pub const SQUARE: &str = "\u{25A1}";
/// Arrows appear in the types of maps between modules. Both the short and long
/// spellings render long, because U+2190/U+2192 are East Asian Ambiguous while
/// U+27F5/U+27F6 measure one column everywhere.
pub const LONG_LEFT_ARROW: &str = "\u{27F5}";
/// TeX's `\to` and `\rightarrow`, drawn long as [`LONG_LEFT_ARROW`] is.
pub const LONG_RIGHT_ARROW: &str = "\u{27F6}";
/// TeX's `\mapsto`.
pub const LONG_MAPSTO: &str = "\u{27FC}";
/// TeX's `\setminus`.
pub const SETMINUS: &str = "\u{2216}";
/// The superscript forms of the digits 0 to 9, in order.
pub const SUPERSCRIPT_DIGITS: &str =
    "\u{2070}\u{00B9}\u{00B2}\u{00B3}\u{2074}\u{2075}\u{2076}\u{2077}\u{2078}\u{2079}";
/// The rule a fraction is drawn over, repeated to its width.
pub const FRACTION_RULE: &str = "\u{2500}";
/// M2's `<|` and `|>` as drawn in a terminal, where a font may join them into a
/// triangle.
pub const ANGLE_BAR_DELIMITERS: (&str, &str) = ("\u{27E8}|", "|\u{27E9}");
/// A left curly brace's pieces: top, cusp, bottom, and the upper and lower halves
/// of the cusp an even height splits across two rows.
pub const LEFT_CURLY: [&str; 5] = ["\u{23A7}", "\u{23A8}", "\u{23A9}", "\u{23B0}", "\u{23B1}"];
/// A right curly brace's pieces, in the order of [`LEFT_CURLY`].
pub const RIGHT_CURLY: [&str; 5] = ["\u{23AB}", "\u{23AC}", "\u{23AD}", "\u{23B1}", "\u{23B0}"];
/// What extends a curly brace between its pieces.
pub const CURLY_EXTENSION: &str = "\u{23AA}";

/// U+22C5 DOT OPERATOR, whose width is the same under every terminal convention --
/// unlike U+00B7 MIDDLE DOT, which is East Asian Ambiguous and would shift a border.
pub const PRODUCT: &str = "\u{22C5}";
/// M2's constants TeX spells as a command, with the glyph a terminal draws for each.
pub const CONSTANT_COMMANDS: [(&str, &str); 3] = [
    ("pi", "\u{03C0}"),
    ("gamma", "\u{03B3}"),
    ("infty", "\u{221E}"),
];

/// Operators TeX spells as a command, with the glyph a terminal draws for each.
pub const OPERATOR_COMMANDS: [(&str, &str); 19] = [
    ("wedge", "\u{2227}"),
    ("vee", "\u{2228}"),
    ("otimes", "\u{2297}"),
    ("oplus", "\u{2295}"),
    ("circ", "\u{2218}"),
    ("cap", "\u{2229}"),
    ("cup", "\u{222A}"),
    ("times", "\u{00D7}"),
    ("ast", "\u{2217}"),
    ("mid", "|"),
    ("Rightarrow", OPTION_ARROW),
    ("Leftarrow", "\u{21D0}"),
    ("Leftrightarrow", "\u{21D4}"),
    ("Longrightarrow", "\u{27F9}"),
    ("Longleftarrow", "\u{27F8}"),
    ("leq", "\u{2264}"),
    ("geq", "\u{2265}"),
    ("cdots", "\u{22EF}"),
    // U+2026 is East Asian Ambiguous, so the low ellipsis is three full stops.
    ("ldots", "..."),
];

/// The pieces a fence grows from when taller than a row: top, extension, bottom,
/// and the tip a brace points at its baseline with. These families measure one
/// column under every terminal convention, unlike the box-drawing set.
pub const TALL_FENCES: [(&str, [&str; 4]); 7] = [
    ("(", ["\u{239B}", "\u{239C}", "\u{239D}", "\u{239C}"]),
    (")", ["\u{239E}", "\u{239F}", "\u{23A0}", "\u{239F}"]),
    ("[", ["\u{23A1}", "\u{23A2}", "\u{23A3}", "\u{23A2}"]),
    ("]", ["\u{23A4}", "\u{23A5}", "\u{23A6}", "\u{23A5}"]),
    ("{", ["\u{23A7}", "\u{23AA}", "\u{23A9}", "\u{23A8}"]),
    ("}", ["\u{23AB}", "\u{23AA}", "\u{23AD}", "\u{23AC}"]),
    ("|", ["\u{23AA}"; 4]),
];
/// Braces two rows high, which leave no room for a tip and so are drawn from
/// their two halves; without either they would read as parentheses.
pub const SHORT_BRACES: [(&str, [&str; 4]); 2] = [
    ("{", ["\u{23B0}", "", "\u{23B1}", ""]),
    ("}", ["\u{23B1}", "", "\u{23B0}", ""]),
];
