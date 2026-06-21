//! Tree-sitter cell splitting over a complete buffer. Completeness is decided by the
//! live M2 oracle ([`crate::m2_backend::M2Backend::check`]), NOT here -- tree-sitter's
//! error recovery does not reliably localize incompleteness. Used only after the oracle
//! has classified the buffer: [`split_cells`] returns the byte ranges of the top-level
//! cells, and [`has_syntax_error`] catches the strict errors `parse` is lenient about
//! (e.g. the empty-statement `;;`).

use std::ops::Range;

use tree_sitter::Parser;

fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .expect("the compiled-in Macaulay2 grammar always loads");
    parser
}

/// Byte ranges of each top-level `cell` node (the depth-0 `;`-separated units) in a
/// complete buffer. Ranges index into `source`; the caller slices both the text and the
/// matching highlight spans from them, so nothing is cloned here. Only valid on a buffer
/// the M2 oracle has confirmed parses cleanly, so the tree carries no recovery artifacts.
pub fn split_cells(source: &str) -> Vec<Range<usize>> {
    let mut parser = parser();
    let tree = parser
        .parse(source, None)
        .expect("parsing without a timeout always yields a tree");
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|n| n.kind() == "cell")
        .map(|n| n.byte_range())
        .collect()
}

/// Whether tree-sitter finds a syntax error in `source`. Consulted only when the M2
/// `parse` oracle reports COMPLETE, to catch the strict errors `parse` waves through --
/// notably the empty-statement `;;`, which `parse` accepts but the real reader rejects.
pub fn has_syntax_error(source: &str) -> bool {
    let mut parser = parser();
    let tree = parser
        .parse(source, None)
        .expect("parsing without a timeout always yields a tree");
    tree.root_node().has_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_newline_delimited_cells() {
        let source = "a = 5\nb = 6";
        let cells: Vec<&str> = split_cells(source)
            .into_iter()
            .map(|r| &source[r])
            .collect();
        assert_eq!(cells, vec!["a = 5", "b = 6"]);
    }

    #[test]
    fn single_cell_is_one_range() {
        assert_eq!(split_cells("1 + 1").len(), 1);
    }

    #[test]
    fn clean_code_has_no_error() {
        assert!(!has_syntax_error("a = 5\nb = 6"));
    }
}
