//! Kitty graphics: pictures sent to the terminal and drawn by Unicode
//! placeholder cells.
//!
//! A placeholder is an ordinary character whose foreground colour names the picture
//! and whose combining marks name the row and column of the piece it shows. Being
//! text, it scrolls, folds and clips with the lines around it, and tmux moves it
//! between panes as it moves any other text.

use std::fmt::Write;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ratatui::layout::Size;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// The character kitty reserves for picture placeholders.
pub const PLACEHOLDER: char = '\u{10EEEE}';
/// Payload bytes the protocol admits in one escape sequence.
const CHUNK_BYTES: usize = 4096;
/// The largest id a 24-bit foreground colour can carry.
const MAX_ID: u32 = 0xFF_FFFF;
/// Combining marks numbering placeholder rows and columns, in kitty's order.
const DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

/// How escape sequences reach the terminal drawing the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Passthrough {
    /// Written straight to the terminal.
    #[default]
    Direct,
    /// Wrapped so tmux forwards them, unread, to the terminal it runs in.
    Tmux,
}

impl Passthrough {
    fn wrap(self, sequence: &str) -> String {
        match self {
            Self::Direct => sequence.to_string(),
            Self::Tmux => format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b")),
        }
    }
}

/// The number the terminal stores a picture under, small enough to be a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageId(u32);

impl ImageId {
    /// The id after this one, wrapping before it outgrows a colour. Zero is never
    /// issued: the protocol reads it as no id at all.
    pub fn next(self) -> Self {
        Self(self.0 % MAX_ID + 1)
    }

    fn colour(self) -> Color {
        let [_, r, g, b] = self.0.to_be_bytes();
        Color::Rgb(r, g, b)
    }
}

/// What one placeholder cell shows: a picture, and the row and column of its piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub image: ImageId,
    pub row: u16,
    pub column: u16,
}

impl Piece {
    /// The piece a placeholder drawn as `symbol` in `colour` shows.
    pub fn placed(symbol: &str, colour: Color) -> Option<Self> {
        let Color::Rgb(r, g, b) = colour else {
            return None;
        };
        let mut marks = symbol.strip_prefix(PLACEHOLDER)?.chars().map(|mark| {
            DIACRITICS
                .iter()
                .position(|&diacritic| diacritic == mark)
                .and_then(|index| u16::try_from(index).ok())
        });
        Some(Self {
            image: ImageId(u32::from_be_bytes([0, r, g, b])),
            row: marks.next()??,
            column: marks.next()??,
        })
    }
}

impl ImageId {
    /// The escape sequences telling the terminal to forget this picture and free
    /// what it stored; the terminal is asked not to answer.
    pub fn deletion(self, passthrough: Passthrough) -> String {
        passthrough.wrap(&format!("\x1b_Ga=d,d=I,i={},q=2\x1b\\", self.0))
    }
}

impl From<u32> for ImageId {
    fn from(id: u32) -> Self {
        Self(id % MAX_ID + 1)
    }
}

/// A picture too large for placeholders to number its pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Oversized(pub Size);

/// A picture the terminal holds under one id, covering a grid of cells. Its pixels
/// are not kept here: they are sent as PNG when the terminal needs them.
#[derive(Debug)]
pub struct KittyImage {
    id: ImageId,
    cells: Size,
}

impl KittyImage {
    /// A picture drawn across `cells`, refused when they outnumber the marks that
    /// address them.
    pub fn new(id: ImageId, cells: Size) -> Result<Self, Oversized> {
        let limit = DIACRITICS.len() as u16;
        match cells.width <= limit && cells.height <= limit {
            true => Ok(Self { id, cells }),
            false => Err(Oversized(cells)),
        }
    }

    pub fn id(&self) -> ImageId {
        self.id
    }

    /// The escape sequences that store `png` as this picture and place it virtually,
    /// so placeholders can show it; the terminal is asked not to answer.
    pub fn transmission(&self, png: &[u8], passthrough: Passthrough) -> String {
        let payload = STANDARD.encode(png);
        let chunks = payload.as_bytes().chunks(CHUNK_BYTES).collect::<Vec<_>>();
        let mut sequences = String::new();
        for (index, chunk) in chunks.iter().enumerate() {
            let mut control = String::new();
            if index == 0 {
                let Size { width, height } = self.cells;
                let _ = write!(
                    control,
                    "a=T,U=1,f=100,t=d,q=2,i={},c={width},r={height},",
                    self.id.0
                );
            }
            let more = u8::from(index + 1 < chunks.len());
            let chunk = std::str::from_utf8(chunk).expect("base64 is ASCII");
            let sequence = format!("\x1b_G{control}m={more};{chunk}\x1b\\");
            sequences.push_str(&passthrough.wrap(&sequence));
        }
        sequences
    }

    /// One line of placeholders per row of cells, each cell naming its own row and
    /// column so any slice of the lines still shows the right pieces. The colour
    /// naming the picture is on the span, so it stays when the row is set beside
    /// other text.
    pub fn lines(&self) -> Vec<Line<'static>> {
        let style = Style::new().fg(self.id.colour());
        (0..usize::from(self.cells.height))
            .map(|row| {
                let text = (0..usize::from(self.cells.width))
                    .flat_map(|column| [PLACEHOLDER, DIACRITICS[row], DIACRITICS[column]])
                    .collect::<String>();
                Line::from(Span::styled(text, style))
            })
            .collect()
    }
}
