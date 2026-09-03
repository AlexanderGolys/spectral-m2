//! m2-syn highlighting as source ranges tagged with semantic syntax roles.

use std::ops::{Add, Range};

use crate::ui::controller::settings::theme::groups::HLGroup;
use crate::ui::element::HLSpan;
use m2_syn::nodes::visit::{self, Visit};
use m2_syn::nodes::{
    AdjacentExpr, BinaryExpr, Cell, Collection, Component, EscapeSequence, Expr, ExprPack,
    FixedArity, GlobalBinding, Literal, LocalBinding, MethodInstallation, NewInstallation,
    NewStatement, OperatorExpr, OperatorInstallation, SourceFile, StringLiteral, Symbol,
    TypeAnnotation, Variadic,
};
use m2_syn::{
    lex_str, lex_str_recovering, parse_native_recovering, CommentTrivia, Lex, LexError,
    LexErrorKind, LiteralKind, Span, Spanned, TokenStream, TokenTree, Trivia,
};

impl Add<u32> for HLSpan {
    type Output = Self;

    fn add(self, offset: u32) -> Self::Output {
        let offset = offset as usize;
        Self {
            content: self.content.start + offset..self.content.end + offset,
            kind: self.kind,
        }
    }
}

/// A source-to-highlight projection consumed by the renderer.
pub trait Highlighter {
    fn highlight(&mut self, source: &str) -> Vec<HLSpan>;
}

/// A bytewise paint layer that resolves nested syntax roles by priority.
struct Painter {
    bytes: Vec<Option<(u8, HLGroup)>>,
}

impl Painter {
    fn new(len: usize) -> Self {
        Self {
            bytes: vec![None; len],
        }
    }

    fn range(&mut self, range: Range<usize>, kind: HLGroup, priority: u8) {
        let end = range.end.min(self.bytes.len());
        for byte in self.bytes[range.start.min(end)..end].iter_mut() {
            if byte.is_none_or(|(current, _)| priority >= current) {
                *byte = Some((priority, kind));
            }
        }
    }

    fn span(&mut self, span: &Span, kind: HLGroup, priority: u8) {
        if let Some((start, end)) = span.bounds() {
            self.range(start..end, kind, priority);
        }
    }

    fn syntax<T: Spanned + ?Sized>(&mut self, node: &T, kind: HLGroup, priority: u8) {
        self.span(&node.span(), kind, priority);
    }

    fn comments(&mut self, comments: &CommentTrivia) {
        for comment in comments.outer().iter().chain(comments.inner()) {
            self.syntax(comment, HLGroup::Comment, 0);
        }
    }

    fn trivia(&mut self, trivia: &[Trivia]) {
        for comment in trivia {
            self.syntax(comment, HLGroup::Comment, 0);
        }
    }

    fn tokens(&mut self, stream: &TokenStream) {
        for token in stream.iter() {
            self.trivia(token.leading_trivia().as_slice());
            match token {
                TokenTree::Group(group) => {
                    self.span(&group.delimiter().opening_span(), HLGroup::Punctuation, 0);
                    if let Some(close) = group.delimiter().closing_span() {
                        self.span(&close, HLGroup::Punctuation, 0);
                    }
                    self.tokens(group.stream());
                }
                TokenTree::Literal(literal) => self.syntax(
                    literal,
                    match literal.kind {
                        LiteralKind::Integer => HLGroup::Number,
                        LiteralKind::Float => HLGroup::Number,
                        LiteralKind::String | LiteralKind::RawString => HLGroup::String,
                    },
                    0,
                ),
                TokenTree::Punct(punctuation) => self.syntax(
                    punctuation,
                    match punctuation.text() {
                        "," | ";" => HLGroup::Delimiter,
                        _ => HLGroup::Operator,
                    },
                    0,
                ),
                TokenTree::Ident(identifier) => {
                    self.syntax(identifier, word_kind(identifier.text()), 0);
                }
            }
            self.trivia(token.trailing_trivia().as_slice());
        }
    }

    fn lex(&mut self, source: &str) {
        match lex_str_recovering(source) {
            Ok(recovered) => self.tokens(&recovered.value),
            Err(error) => {
                if let Some(start) = error.span().start() {
                    if let Ok(prefix) = lex_str(&source[..start.min(source.len())]) {
                        self.tokens(&prefix);
                    }
                }
                self.lex_error(&error);
            }
        }
    }

    fn lex_error(&mut self, error: &LexError) {
        let kind = match error.kind() {
            LexErrorKind::UnterminatedString | LexErrorKind::UnterminatedRawString => {
                HLGroup::String
            }
            LexErrorKind::UnterminatedBlockComment => HLGroup::Comment,
            _ => HLGroup::Error,
        };
        self.syntax(error, kind, 1);
    }

    fn finish(self) -> Vec<HLSpan> {
        let mut spans = Vec::new();
        let mut start = 0;
        while start < self.bytes.len() {
            let Some((_, kind)) = self.bytes[start] else {
                start += 1;
                continue;
            };
            let mut end = start + 1;
            while self
                .bytes
                .get(end)
                .is_some_and(|paint| paint.is_some_and(|(_, next)| next == kind))
            {
                end += 1;
            }
            spans.push(HLSpan::new(start..end, kind));
            start = end;
        }
        spans
    }
}

fn word_kind(word: &str) -> HLGroup {
    let keyword = word.strip_prefix("Core$").unwrap_or(word);
    match keyword {
        "if" | "else" | "then" | "when" | "list" | "do" => HLGroup::Keyword,
        "for" | "while" | "in" | "from" | "to" => HLGroup::Keyword,
        "return" | "break" | "continue" => HLGroup::Keyword,
        "try" | "catch" | "throw" | "trap" | "except" | "shield" => HLGroup::Keyword,
        "global" | "local" | "symbol" | "threadVariable" | "threadLocal" => HLGroup::Keyword,
        "time" | "timing" | "elapsedTime" | "elapsedTiming" | "profile" | "TEST" | "breakpoint"
        | "finish" | "step" => HLGroup::Keyword,
        "and" | "or" | "xor" | "not" | "SPACE" => HLGroup::Operator,
        "new" | "of" => HLGroup::Keyword,
        "true" | "false" => HLGroup::Boolean,
        "CatalanConstant" | "EulerConstant" | "ii" | "pi" | "null" | "infinity" => {
            HLGroup::Constant
        }
        "error" | "stderr" => HLGroup::Error,
        word if output_symbol(word) || BUILTIN_VARIABLES.contains(&word) => HLGroup::AlgVariable,
        _ => HLGroup::Variable,
    }
}

const BUILTIN_VARIABLES: &[&str] = &[
    "allowableThreads",
    "debugLevel",
    "defaultPrecision",
    "engineDebugLevel",
    "errorDepth",
    "gbTrace",
    "interpreterDepth",
    "lineNumber",
    "loadDepth",
    "maxAllowableThreads",
    "maxExponent",
    "minExponent",
    "numTBBThreads",
    "printingAccuracy",
    "printingLeadLimit",
    "printingPrecision",
    "printingTimeLimit",
    "printingTrailLimit",
    "version",
    "printWidth",
    "recursionLimit",
];

fn output_symbol(word: &str) -> bool {
    matches!(word, "oo" | "ooo" | "oooo")
        || word.strip_prefix('o').is_some_and(|digits| {
            digits
                .bytes()
                .next()
                .is_some_and(|digit| matches!(digit, b'1'..=b'9'))
                && digits.bytes().all(|digit| digit.is_ascii_digit())
        })
}

fn symbol(expr: &Expr) -> Option<&Symbol> {
    match expr {
        Expr::Symbol(symbol) => Some(symbol),
        _ => None,
    }
}

fn first_expr(pack: &ExprPack) -> Option<&Expr> {
    pack.pack.iter().find_map(|component| match component {
        Component::Expr(expr) => Some(expr.as_ref()),
        Component::Empty(_) => None,
    })
}

fn argument(expr: &Expr) -> &Expr {
    match expr {
        Expr::Holder(holder) => holder.value.as_deref().map_or(expr, argument),
        Expr::Collection(Collection::Sequence(sequence)) => sequence
            .elements
            .as_ref()
            .and_then(first_expr)
            .map_or(expr, argument),
        _ => expr,
    }
}

fn is_lambda(expr: &Expr) -> bool {
    matches!(expr, Expr::LambdaExpression(_))
}

fn is_number(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Literal(Literal::IntegerLiteral(_) | Literal::FloatLiteral(_))
    )
}

fn method_name(symbol: &Symbol) -> bool {
    symbol
        .text()
        .rsplit('$')
        .next()
        .and_then(|name| name.chars().next())
        .is_some_and(char::is_lowercase)
}

fn is_url(text: &str) -> bool {
    let text = text.trim_matches(['"', '/']);
    text.starts_with("http://") || text.starts_with("https://") || text.starts_with("www.")
}

/// Typed semantic overrides layered over m2-syn's lexical token roles.
struct SemanticPainter<'a> {
    painter: &'a mut Painter,
}

impl SemanticPainter<'_> {
    fn paint<T: Spanned + ?Sized>(&mut self, node: &T, kind: HLGroup) {
        self.painter.syntax(node, kind, 1);
    }

    fn paint_argument(&mut self, expr: &Expr, kind: HLGroup) {
        self.paint(argument(expr), kind);
    }

    fn operator_signature(&mut self, operator: &OperatorExpr) {
        match operator {
            OperatorExpr::AdjacentExpr(node) => {
                self.paint(node.left_operand.as_ref(), HLGroup::Type);
                self.paint(node.right_operand.as_ref(), HLGroup::Type);
            }
            OperatorExpr::BinaryExpr(node) => {
                self.paint(node.left_operand.as_ref(), HLGroup::Type);
                self.paint(&node.operator, HLGroup::Installation);
                self.paint(node.right_operand.as_ref(), HLGroup::Type);
            }
            OperatorExpr::PrefixExpr(node) => {
                self.paint(&node.operator, HLGroup::Installation);
                self.paint(node.operand.as_ref(), HLGroup::Type);
            }
            OperatorExpr::PostfixExpr(node) => {
                self.paint(node.operand.as_ref(), HLGroup::Type);
                self.paint(&node.operator, HLGroup::Installation);
            }
        }
    }

    fn assignment_method(&mut self, right: &Expr) {
        let Expr::Collection(Collection::Sequence(sequence)) = argument(right) else {
            return;
        };
        let Some(elements) = sequence.elements.as_ref() else {
            return;
        };
        let mut arguments = elements
            .pack
            .iter()
            .filter_map(|component| match component {
                Component::Expr(expr) => Some(expr.as_ref()),
                Component::Empty(_) => None,
            });
        if let Some(Expr::QuoteExpression(quoted)) = arguments.next() {
            self.paint(&quoted.symbol, HLGroup::Installation);
        }
        for domain in arguments.take(2) {
            self.paint(domain, HLGroup::Type);
        }
    }
}

impl<'ast> Visit<'ast> for SemanticPainter<'_> {
    fn visit_source_file(&mut self, node: &'ast SourceFile) {
        self.painter.comments(&node.comments);
        visit::visit_source_file(self, node);
    }

    fn visit_cell(&mut self, node: &'ast Cell) {
        self.painter.comments(&node.comments);
        visit::visit_cell(self, node);
    }

    fn visit_escape_sequence(&mut self, node: &'ast EscapeSequence) {
        self.paint(node, HLGroup::Quote);
    }

    fn visit_string_literal(&mut self, node: &'ast StringLiteral) {
        if node.span().text().is_some_and(is_url) {
            self.paint(node, HLGroup::Documentation);
        }
        visit::visit_string_literal(self, node);
    }

    fn visit_variadic(&mut self, node: &'ast Variadic) {
        self.paint(&node.0, HLGroup::Parameter);
    }

    fn visit_fixed_arity(&mut self, node: &'ast FixedArity) {
        if let Some(parameters) = node.0.contents.as_ref() {
            for parameter in parameters.iter() {
                self.paint(parameter, HLGroup::Parameter);
            }
        }
    }

    fn visit_global_binding(&mut self, node: &'ast GlobalBinding) {
        if is_lambda(&node.value) {
            self.paint(&node.variable, HLGroup::Function);
        }
        visit::visit_global_binding(self, node);
    }

    fn visit_local_binding(&mut self, node: &'ast LocalBinding) {
        if is_lambda(&node.value) {
            self.paint(&node.variable, HLGroup::Function);
        }
        visit::visit_local_binding(self, node);
    }

    fn visit_binary_expr(&mut self, node: &'ast BinaryExpr) {
        let operator = node.operator.spelling();
        if matches!(operator, "." | ".?" | "#" | "#?" | "_")
            && (symbol(&node.right_operand).is_some() || is_number(&node.right_operand))
        {
            self.paint(node.right_operand.as_ref(), HLGroup::Property);
        }
        if operator == "_" {
            if let Some(function) = symbol(&node.left_operand) {
                if matches!(function.text(), "importFrom" | "exportFrom") {
                    self.paint(function, HLGroup::Function);
                    self.paint(node.right_operand.as_ref(), HLGroup::Package);
                }
            }
        }
        if operator == "_" && is_number(&node.left_operand) && symbol(&node.right_operand).is_some()
        {
            self.painter.syntax(node, HLGroup::Number, 2);
        }
        visit::visit_binary_expr(self, node);
    }

    fn visit_adjacent_expr(&mut self, node: &'ast AdjacentExpr) {
        if let Some(function) = symbol(&node.left_operand) {
            self.paint(function, HLGroup::Function);
            match function.text() {
                "loadPackage" | "installPackage" | "uninstallPackage" | "needsPackage"
                | "endPackage" | "newPackage" => {
                    self.paint(function, HLGroup::Function);
                    self.paint_argument(&node.right_operand, HLGroup::Package);
                }
                "load" => {
                    self.paint(function, HLGroup::Function);
                    self.paint_argument(&node.right_operand, HLGroup::Namespace);
                }
                "splitWWW" | "getWWW" | "urlEncode" => {
                    self.paint(function, HLGroup::CompiledFunction);
                    self.paint_argument(&node.right_operand, HLGroup::Documentation);
                }
                "installAssignmentMethod" => {
                    self.paint(function, HLGroup::CompiledFunction);
                    self.assignment_method(&node.right_operand);
                }
                _ => {}
            }
        }
        visit::visit_adjacent_expr(self, node);
    }

    fn visit_new_statement(&mut self, node: &'ast NewStatement) {
        self.paint(node.class.as_ref(), HLGroup::Type);
        if let Some(parent) = node.parent_class.as_ref() {
            self.paint(parent.parent.as_ref(), HLGroup::Type);
        }
        if let Some(instance) = node.instance.as_ref() {
            self.paint(&instance.from_token, HLGroup::Keyword);
        }
        visit::visit_new_statement(self, node);
    }

    fn visit_method_installation(&mut self, node: &'ast MethodInstallation) {
        if let Some(method) = symbol(&node.method) {
            self.paint(
                method,
                if method_name(method) {
                    HLGroup::Installation
                } else {
                    HLGroup::Type
                },
            );
        }
        self.paint(node.signature.as_ref(), HLGroup::Type);
        self.paint(&node.assign_op, HLGroup::Operator);
        visit::visit_method_installation(self, node);
    }

    fn visit_operator_installation(&mut self, node: &'ast OperatorInstallation) {
        self.operator_signature(&node.domain_expr);
        self.paint(&node.assign_op, HLGroup::Operator);
        visit::visit_operator_installation(self, node);
    }

    fn visit_new_installation(&mut self, node: &'ast NewInstallation) {
        self.paint(&node.operator, HLGroup::Operator);
        visit::visit_new_installation(self, node);
    }

    fn visit_type_annotation(&mut self, node: &'ast TypeAnnotation) {
        self.paint(node.typical_value.as_ref(), HLGroup::Type);
        self.paint(&node.operator, HLGroup::Operator);
        visit::visit_type_annotation(self, node);
    }
}

/// A reusable native m2-syn syntax highlighter.
#[derive(Default)]
pub struct SyntaxHighlighter;

impl SyntaxHighlighter {
    pub const fn new() -> Self {
        Self
    }
}

impl Highlighter for SyntaxHighlighter {
    fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
        let mut painter = Painter::new(source.len());
        match parse_native_recovering(source) {
            Ok(recovered) => {
                painter.tokens(&recovered.value.to_token_stream());
                SemanticPainter {
                    painter: &mut painter,
                }
                .visit_source_file(&recovered.value);
            }
            Err(_) => painter.lex(source),
        }
        painter.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_at(source: &str, needle: &str) -> Option<HLGroup> {
        let offset = source.find(needle)?;
        SyntaxHighlighter::new()
            .highlight(source)
            .into_iter()
            .find(|span| span.content.contains(&offset))
            .map(|span| span.kind)
    }

    #[test]
    fn m2_syn_highlights_lexical_and_typed_roles() {
        for (source, needle, expected) in [
            ("if x then 1 else 0", "if", HLGroup::Keyword),
            ("1 + 2", "1", HLGroup::Number),
            ("f 1", "f", HLGroup::Function),
            ("loadPackage \"Foo\"", "\"Foo\"", HLGroup::Package),
            ("f ZZ := x -> x", "f", HLGroup::Installation),
            ("f ZZ := x -> x", "ZZ", HLGroup::Type),
            ("g = x -> x", "g", HLGroup::Function),
            ("g = x -> x", "x ->", HLGroup::Parameter),
            ("x.foo", "foo", HLGroup::Property),
        ] {
            assert_eq!(kind_at(source, needle), Some(expected), "{source:?}");
        }
    }

    #[test]
    fn incomplete_source_keeps_lexical_highlights() {
        let mut highlighter = SyntaxHighlighter::new();
        let spans = highlighter.highlight("if x then \"unfinished");

        assert!(spans.iter().any(|span| span.kind == HLGroup::Keyword));
        assert!(spans.iter().any(|span| span.kind == HLGroup::String));
    }

    #[test]
    fn spans_are_ordered_non_overlapping_utf8_boundaries() {
        let source = "-- λ\nf = x -> x + 1";
        let mut highlighter = SyntaxHighlighter::new();
        let spans = highlighter.highlight(source);

        for pair in spans.windows(2) {
            assert!(pair[0].content.end <= pair[1].content.start);
        }
        for span in spans {
            assert!(source.is_char_boundary(span.content.start));
            assert!(source.is_char_boundary(span.content.end));
        }
    }
}
