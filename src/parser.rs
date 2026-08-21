//! Tree-sitter cell splitting over a complete buffer. Completeness is decided by the
//! live M2 oracle ([`crate::m2_backend::M2Backend::check`]), NOT here -- tree-sitter's
//! error recovery does not reliably localize incompleteness. Used only after the oracle
//! has classified the buffer: [`split_cells`] returns the byte ranges of the top-level
//! cells, and [`has_syntax_error`] catches the strict errors `parse` is lenient about
//! (e.g. the empty-statement `;;`).

use std::ops::Range;

use tree_sitter::{Parser, Tree};

use crate::errors::{M2Error, M2Result};

fn parser() -> M2Result<Parser> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .map_err(|error| M2Error::ParserError(format!("could not load M2 grammar: {error}")))?;
    Ok(parser)
}

fn parse(source: &str) -> M2Result<Tree> {
    let mut parser = parser()?;
    parser
        .parse(source, None)
        .ok_or_else(|| M2Error::ParserError("tree-sitter returned no syntax tree".to_string()))
}

pub fn split_cells(source: &str) -> M2Result<Vec<Range<usize>>> {
    let tree = parse(source)?;
    let root = tree.root_node();
    let mut cursor = root.walk();
    Ok(root
        .named_children(&mut cursor)
        .map(|n| n.byte_range())
        .collect())
}

/// Whether tree-sitter finds a syntax error in `source`. Consulted only when the M2
/// `parse` oracle reports COMPLETE, to catch the strict errors `parse` waves through --
/// notably the empty-statement `;;`, which `parse` accepts but the real reader rejects.
pub fn has_syntax_error(source: &str) -> M2Result<bool> {
    let tree = parse(source)?;
    Ok(tree.root_node().has_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_newline_delimited_cells() {
        let source = "a = 5\nb = 6";
        let cells: Vec<&str> = split_cells(source)
            .expect("split valid cells")
            .into_iter()
            .map(|r| &source[r])
            .collect();
        assert_eq!(cells, vec!["a = 5", "b = 6"]);
    }

    #[test]
    fn single_cell_is_one_range() {
        assert_eq!(split_cells("1 + 1").expect("split one cell").len(), 1);
    }

    #[test]
    fn clean_code_has_no_error() {
        assert!(!has_syntax_error("a = 5\nb = 6").expect("parse clean code"));
    }
}
