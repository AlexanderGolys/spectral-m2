//! The parts of a feed block a selection can take whole, and the text each copies as.

use std::ops::Range;

use super::settings::selection::CopyFormat;

/// What a selectable part is, which decides the prompt it copies with in a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemKind {
    Input,
    /// Text a cell printed, or a message that belongs to no prompt.
    Note,
    Output,
}

/// One code cell or one result of a block, with the rows it takes there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The block's own rows the item covers.
    pub rows: Range<u16>,
    pub kind: ItemKind,
    pub number: Option<u32>,
    pub text: String,
    /// The type Macaulay2 gave a value, copied on its own line in a session.
    pub ty: Option<String>,
}

impl Item {
    pub fn new(kind: ItemKind, rows: Range<u16>, number: Option<u32>, text: String) -> Self {
        Self {
            rows,
            kind,
            number,
            text,
            ty: None,
        }
    }

    pub fn with_type(mut self, ty: Option<String>) -> Self {
        self.ty = ty;
        self
    }

    /// The same item in a block placed `rows` lower.
    pub fn moved(mut self, rows: u16) -> Self {
        self.rows = self.rows.start.saturating_add(rows)..self.rows.end.saturating_add(rows);
        self
    }

    /// The text the item copies as. A session sets it after the prompt Macaulay2
    /// gave it, later lines lined up beneath the first; an item without a number
    /// had none, and copies as its text.
    pub fn copied(&self, format: CopyFormat) -> String {
        let (CopyFormat::Session, Some(number)) = (format, self.number) else {
            return self.text.clone();
        };
        let prompt = match self.kind {
            ItemKind::Input => format!("i{number} : "),
            ItemKind::Output => format!("o{number} = "),
            ItemKind::Note => return self.text.clone(),
        };
        let indent = " ".repeat(prompt.chars().count());
        let mut copied = self
            .text
            .lines()
            .enumerate()
            .map(|(row, line)| match row {
                0 => format!("{prompt}{line}"),
                _ => format!("{indent}{line}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(ty) = self.ty.as_deref().filter(|_| self.kind == ItemKind::Output) {
            copied.push_str(&format!("\n\no{number} : {ty}"));
        }
        copied
    }
}

/// Several items copied as one, a blank line apart. A session reads them as Macaulay2
/// ran them, each input followed by what it printed and returned; raw text keeps the
/// order they are drawn in.
pub fn copied_together(items: &[Item], format: CopyFormat) -> String {
    let mut ordered: Vec<&Item> = items.iter().collect();
    if format == CopyFormat::Session {
        ordered.sort_by_key(|item| (item.number.unwrap_or(u32::MAX), item.kind));
    }
    ordered
        .into_iter()
        .map(|item| item.copied(format))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(number: u32, text: &str) -> Item {
        Item::new(ItemKind::Input, 0..1, Some(number), text.into())
    }

    fn output(number: u32, text: &str, ty: &str) -> Item {
        Item::new(ItemKind::Output, 0..1, Some(number), text.into()).with_type(Some(ty.into()))
    }

    #[test]
    fn raw_text_copies_without_prompts() {
        assert_eq!(input(1, "1+1").copied(CopyFormat::Raw), "1+1");
        assert_eq!(output(1, "2", "ZZ").copied(CopyFormat::Raw), "2");
    }

    #[test]
    fn a_session_prompts_inputs_and_values_and_lines_up_what_follows() {
        assert_eq!(
            input(3, "f = x ->\n  x").copied(CopyFormat::Session),
            "i3 : f = x ->\n       x"
        );
        assert_eq!(
            output(3, "2", "ZZ").copied(CopyFormat::Session),
            "o3 = 2\n\no3 : ZZ"
        );
    }

    #[test]
    fn what_ran_without_a_number_copies_as_its_text() {
        let note = Item::new(ItemKind::Note, 0..1, None, "oops".into());
        assert_eq!(note.copied(CopyFormat::Session), "oops");
    }

    #[test]
    fn a_session_reads_each_input_before_its_value() {
        let items = [
            input(1, "a"),
            input(2, "b"),
            output(1, "A", "T"),
            output(2, "B", "T"),
        ];
        assert_eq!(
            copied_together(&items, CopyFormat::Session),
            "i1 : a\n\no1 = A\n\no1 : T\n\ni2 : b\n\no2 = B\n\no2 : T"
        );
        assert_eq!(copied_together(&items, CopyFormat::Raw), "a\n\nb\n\nA\n\nB");
    }

    #[test]
    fn an_item_moves_with_its_block() {
        assert_eq!(input(1, "a").moved(4).rows, 4..5);
    }
}
