//! Splitting the live input buffer into M2 *cells* and deciding when the buffer
//! is still mid-edit. The grammar's `source_file` is a newline-delimited list of
//! `cell` nodes; an unfinished construct (unclosed bracket/string, dangling
//! operator, half-typed `if`/`for`) surfaces as a `MISSING` node in the tree.

use tree_sitter::{Node, Parser};

/// The outcome of parsing the current input buffer on Enter.
pub enum CellParse {
    /// The buffer is still being typed (e.g. an unclosed delimiter); Enter
    /// should insert a newline and keep editing rather than submit.
    Incomplete,
    /// The buffer is syntactically complete; each element is one top-level
    /// cell's source, to be evaluated in order. A continuation typed across
    /// several lines collapses to a single cell here; a multi-statement paste
    /// yields one entry per statement.
    Complete(Vec<String>),
}

fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .expect("the compiled-in Macaulay2 grammar always loads");
    parser
}

pub fn parse_cells(source: &str) -> CellParse {
    let mut parser = parser();
    let tree = parser
        .parse(source, None)
        .expect("parsing without a timeout always yields a tree");
    let root = tree.root_node();

    if is_incomplete(&root) {
        return CellParse::Incomplete;
    }

    let mut cursor = root.walk();
    let cells = root
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "cell")
        .map(|n| source[n.byte_range()].to_string())
        .collect();
    CellParse::Complete(cells)
}

/// Whether `root`'s subtree contains an unfinished construct — input the user is
/// still typing — so Enter should continue the line instead of submitting.
///
/// From the grammar probe: every unclosed delimiter, unterminated string,
/// dangling binary operator, and half-typed `if`/`for`/`list` produces a node
/// for which `Node::is_missing()` is true (the parser's error-recovery inserting
/// the token it still expects). Syntactically complete input — including things
/// that only fail at *runtime*, like `1/0` — has no missing node. A stray,
/// already-wrong token (e.g. a lone `)`) instead yields an ERROR node, not a
/// missing one, and should submit so M2 reports the error.
fn is_incomplete(root: &Node) -> bool {
    // TODO(human): walk `root`'s subtree and return true if any descendant
    // (or the node itself) satisfies `node.is_missing()`. A `tree_sitter::Node`
    // exposes `child_count()` and `child(i: u32) -> Option<Node>` for the walk
    // (note the `u32` index), plus `is_missing()`.
    unimplemented!()
}
