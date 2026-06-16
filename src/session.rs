//! The committed session: the ordered cells that successfully evaluated. A cell
//! that errors is never committed, so it consumes no [`CellNumber`] and does not
//! enter the code the LSP sees. The numbering shown as `iN` is owned here, not
//! by the M2 process (whose `capture` labels always restart at 1).

use std::rc::Rc;

/// The 1-based number a committed cell is displayed under (`i1`, `i2`, …).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellNumber(pub usize);

/// One successfully evaluated cell. The source is an [`Rc<str>`] shared with the
/// transcript's echo of the same input: a committed cell's text is immutable and
/// lives in two read-only views, so they share one allocation rather than cloning
/// the bytes.
pub struct Cell {
    pub number: CellNumber,
    pub code: Rc<str>,
}

/// The committed cells, in evaluation order.
pub struct Session {
    cells: Vec<Cell>,
}

impl Session {
    pub fn new() -> Self {
        Session { cells: Vec::new() }
    }

    /// The number the next committed cell will take — also what an
    /// about-to-be-evaluated input is displayed under, so an erroring input and
    /// the successful retry that replaces it share a number.
    pub fn next_number(&self) -> CellNumber {
        CellNumber(self.cells.len() + 1)
    }

    /// Append a successfully evaluated cell, returning its assigned number. Takes
    /// the shared [`Rc<str>`] the transcript already holds, so committing clones a
    /// pointer rather than the source text.
    pub fn commit(&mut self, code: Rc<str>) -> CellNumber {
        let number = self.next_number();
        self.cells.push(Cell { number, code });
        number
    }

    /// The committed cells, in evaluation order — the numbered blocks the
    /// `$code` pane renders (and the same sequence `committed_text` joins).
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The committed code as one document — every cell joined by a newline.
    /// This is the source the language server sees, so earlier definitions
    /// resolve in later cells. (Not yet wired into the app — the LSP virtual doc.)
    #[allow(dead_code)]
    pub fn committed_text(&self) -> String {
        self.cells
            .iter()
            .map(|c| c.code.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
