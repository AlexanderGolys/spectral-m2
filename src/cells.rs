//! Tree-sitter helpers over the input buffer. Completeness is decided by the live
//! M2 oracle ([`crate::m2_backend::M2Backend::check`]), NOT here -- tree-sitter's
//! error recovery does not reliably localize incompleteness. This module is used
//! only after the oracle has classified the buffer: [`split_cells`] slices a
//! known-complete buffer into top-level cells, and [`has_syntax_error`] catches
//! the strict errors `parse` is lenient about (e.g. the empty-statement `;;`).

use tree_sitter::Parser;

fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .expect("the compiled-in Macaulay2 grammar always loads");
    parser
}

/// Split a complete buffer into one source string per top-level `cell` node. Only
/// valid on a buffer the M2 oracle has confirmed parses cleanly, so the tree never
/// carries error-recovery artifacts.
pub fn split_cells(source: &str) -> Vec<String> {
    let mut parser = parser();
    let tree = parser
        .parse(source, None)
        .expect("parsing without a timeout always yields a tree");
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|n| n.kind() == "cell")
        .map(|n| source[n.byte_range()].to_string())
        .collect()
}

/// Whether tree-sitter finds a syntax error in `source`. Consulted only when the
/// M2 `parse` oracle reports COMPLETE, to catch the strict errors `parse` waves
/// through -- notably the empty-statement `;;`, which `parse` accepts but the real
/// reader rejects -- which tree-sitter captures correctly.
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
        assert_eq!(
            split_cells("a = 5\nb = 6"),
            vec!["a = 5".to_string(), "b = 6".to_string()]
        );
    }

    #[test]
    fn single_cell_is_one_element() {
        assert_eq!(split_cells("1 + 1"), vec!["1 + 1".to_string()]);
    }

    #[test]
    fn clean_code_has_no_error() {
        assert!(!has_syntax_error("a = 5\nb = 6"));
    }
}
