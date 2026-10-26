//! Session source identities, semantic-token projection, and cached highlighting.

mod client;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend, ServerInfo,
};

use self::client::{Client, Event};
pub use self::client::SERVER_EXECUTABLE;
use super::highlight::{Highlighter, SyntaxHighlighter};
use crate::ui::block::HLSpan;
use crate::ui::controller::settings::theme::groups::HLGroup;

const COMMAND: SemanticTokenType = SemanticTokenType::new("command");
const RING_GENERATOR: SemanticTokenModifier = SemanticTokenModifier::new("ringGenerator");
const OPTION: SemanticTokenModifier = SemanticTokenModifier::new("option");
const FILE: SemanticTokenModifier = SemanticTokenModifier::new("file");
const UNRESOLVED: SemanticTokenModifier = SemanticTokenModifier::new("unresolved");

const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::TYPE,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::COMMENT,
    SemanticTokenType::METHOD,
    SemanticTokenType::MODIFIER,
    SemanticTokenType::TYPE_PARAMETER,
    COMMAND,
];
const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::new("builtin"),
    SemanticTokenModifier::DEFAULT_LIBRARY,
    RING_GENERATOR,
    OPTION,
    FILE,
    UNRESOLVED,
];

/// A source segment whose identity survives evaluation and repeated source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceId {
    /// One cell in a submission, scoped to an interpreter generation.
    Cell {
        /// Interpreter generation.
        generation: u64,
        /// Submission identifier.
        submission: u64,
        /// Position in that submission.
        index: usize,
    },
    /// Retained incomplete source preceding the editable input.
    Pending(u64),
    /// The editable input in an interpreter generation.
    Draft(u64),
}

/// One segment's byte range in the analysis document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Stable source identity.
    pub id: SourceId,
    /// Source bytes, excluding the separator appended by the document.
    pub range: Range<usize>,
}

/// Immutable source sent to the language server, with its projection back to cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Interpreter generation, changed only by a successful restart.
    pub generation: u64,
    /// Client-assigned LSP document version.
    pub version: i32,
    /// Source in evaluation order followed by the live input.
    pub text: String,
    /// Segments in source order.
    pub segments: Vec<Segment>,
}

impl Document {
    /// Start an empty document for an interpreter generation.
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            version: 0,
            text: String::new(),
            segments: Vec::new(),
        }
    }

    /// Append a source segment and a newline separating it from the next one.
    pub fn push(&mut self, id: SourceId, source: &str) {
        let start = self.text.len();
        self.text.push_str(source);
        self.segments.push(Segment {
            id,
            range: start..self.text.len(),
        });
        self.text.push('\n');
    }
}

/// Tree-sitter highlighting enriched by optional, asynchronously computed semantics.
pub struct SemanticHighlighter {
    syntax: SyntaxHighlighter,
    document: Option<Arc<Document>>,
    spans: HashMap<SourceId, (String, Vec<HLSpan>)>,
    revision: u64,
    client: Option<Client>,
    /// Whether the language server should be running.
    server: bool,
    /// Whether its semantic tokens color source.
    semantic: bool,
    version: i32,
}

/// What changed since the highlighter was last polled.
#[derive(Debug, Default, PartialEq)]
pub struct Polled {
    /// Cached colors changed, so source must be highlighted again.
    pub recolored: bool,
    /// The language server that finished starting.
    pub started: Option<ServerInfo>,
    /// Why the language server stopped.
    pub failure: Option<String>,
}

impl SemanticHighlighter {
    /// Create a lexical highlighter before optional language-server startup.
    pub fn new() -> Self {
        Self {
            syntax: SyntaxHighlighter::new(),
            document: None,
            spans: HashMap::new(),
            revision: 0,
            client: None,
            server: false,
            semantic: false,
            version: 0,
        }
    }

    /// Start or stop the optional server and whether its tokens color source,
    /// which they never do without it; whether cached colors changed.
    pub fn configure(&mut self, server: bool, semantic: bool) -> bool {
        let semantic = server && semantic;
        let restarted = self.server != server;
        if restarted {
            self.server = server;
            self.client = server.then(Client::start);
        }
        if !restarted && self.semantic == semantic {
            return false;
        }
        self.semantic = semantic;
        match (&self.client, &self.document) {
            (Some(client), Some(document)) if semantic => client.update(document.clone()),
            _ => {}
        }
        if !semantic {
            self.clear_current();
        }
        restarted || !semantic
    }

    /// Replace the analysis source only when its text or source identities changed.
    pub fn update(&mut self, mut document: Document) {
        if self.document.as_ref().is_some_and(|previous| {
            previous.generation == document.generation
                && previous.text == document.text
                && previous.segments == document.segments
        }) {
            return;
        }
        self.version = self
            .version
            .checked_add(1)
            .expect("LSP document version exhausted");
        document.version = self.version;
        let document = Arc::new(document);
        // Drafts and incomplete fragments are transient; historic cells retain their colors.
        self.spans
            .retain(|id, _| matches!(id, SourceId::Cell { .. }));
        self.document = Some(document.clone());
        self.revision = self.revision.wrapping_add(1);
        if let Some(client) = self.client.as_ref().filter(|_| self.semantic) {
            client.update(document);
        }
    }

    fn clear_current(&mut self) {
        if let Some(document) = &self.document {
            for segment in &document.segments {
                self.spans.remove(&segment.id);
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Apply current responses; report a repaint, the server's startup, and at
    /// most one server failure.
    pub fn poll(&mut self) -> Polled {
        let events = self.client.as_ref().map_or_else(Vec::new, Client::poll);
        let mut polled = Polled::default();
        for event in events {
            match event {
                Event::Started(info) => polled.started = Some(info),
                Event::Tokens {
                    document,
                    legend,
                    tokens,
                } => {
                    if self.semantic
                        && self.document.as_ref().is_some_and(|current| {
                            current.version == document.version
                                && current.generation == document.generation
                        })
                    {
                        self.apply(&document, &legend, &tokens);
                        polled.recolored = true;
                    }
                }
                Event::Failed(message) => {
                    self.client = None;
                    self.clear_current();
                    polled.recolored = true;
                    polled.failure = Some(message);
                }
            }
        }
        polled
    }

    fn apply(
        &mut self,
        document: &Document,
        legend: &SemanticTokensLegend,
        tokens: &[SemanticToken],
    ) {
        let spans = decode(&document.text, legend, tokens);
        for segment in &document.segments {
            let first = spans.partition_point(|span| span.content.end <= segment.range.start);
            let projected = spans[first..]
                .iter()
                .take_while(|span| span.content.start < segment.range.end)
                .filter_map(|span| {
                    let start = span.content.start.max(segment.range.start);
                    let end = span.content.end.min(segment.range.end);
                    (start < end).then(|| {
                        span.with_content(start - segment.range.start..end - segment.range.start)
                    })
                })
                .collect();
            self.spans.insert(
                segment.id,
                (document.text[segment.range.clone()].to_string(), projected),
            );
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Source identities in the current document, used for transcript repainting.
    pub fn segments(&self) -> &[Segment] {
        self.document
            .as_ref()
            .map_or(&[], |document| &document.segments)
    }

    fn overlay(&self, id: SourceId, source: &str, lexical: Vec<HLSpan>) -> Vec<HLSpan> {
        match self.spans.get(&id).filter(|(text, _)| text == source) {
            Some((_, semantic)) => merge(lexical, semantic),
            None => lexical,
        }
    }
}

impl Highlighter for SemanticHighlighter {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
        self.syntax.highlight(source)
    }

    fn highlight_source(&mut self, id: SourceId, source: &str) -> Vec<HLSpan> {
        let lexical = self.syntax.highlight(source);
        self.overlay(id, source, lexical)
    }

    fn highlight_continuation(&mut self, pending: Option<&str>, input: &str) -> Vec<HLSpan> {
        let lexical = self.syntax.highlight_continuation(pending, input);
        match &self.document {
            Some(document) => self.overlay(SourceId::Draft(document.generation), input, lexical),
            None => lexical,
        }
    }
}

/// Decode relative UTF-16 positions without allowing malformed spans into rendering.
fn decode(source: &str, legend: &SemanticTokensLegend, tokens: &[SemanticToken]) -> Vec<HLSpan> {
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(offset, _)| offset + 1));
    let mut line = 0u32;
    let mut column = 0u32;
    let mut spans = Vec::new();
    for token in tokens {
        let Some(next_line) = line.checked_add(token.delta_line) else {
            break;
        };
        line = next_line;
        let Some(next_column) = (if token.delta_line == 0 {
            column.checked_add(token.delta_start)
        } else {
            Some(token.delta_start)
        }) else {
            break;
        };
        column = next_column;
        let Some(&start) = line_starts.get(line as usize) else {
            continue;
        };
        let end = line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(source.len());
        let text = source[start..end].trim_end_matches(['\r', '\n']);
        let Some(end_column) = column.checked_add(token.length) else {
            continue;
        };
        let (Some(left), Some(right)) =
            (utf16_offset(text, column), utf16_offset(text, end_column))
        else {
            continue;
        };
        if left == right
            || spans
                .last()
                .is_some_and(|span: &HLSpan| span.content.end > start + left)
        {
            continue;
        }
        let Some(kind) = classify(legend, token) else {
            continue;
        };
        spans.push(HLSpan::new(start + left..start + right, kind));
    }
    spans
}

fn utf16_offset(text: &str, column: u32) -> Option<usize> {
    let mut units = 0u32;
    for (offset, character) in text.char_indices() {
        if units == column {
            return Some(offset);
        }
        units = units.checked_add(character.len_utf16() as u32)?;
        if units > column {
            return None;
        }
    }
    (units == column).then_some(text.len())
}

fn classify(legend: &SemanticTokensLegend, token: &SemanticToken) -> Option<HLGroup> {
    let token_type = legend.token_types.get(token.token_type as usize)?.as_str();
    let has = |name: &str| {
        legend
            .token_modifiers
            .iter()
            .enumerate()
            .any(|(index, modifier)| {
                index < u32::BITS as usize
                    && token.token_modifiers_bitset & (1u32 << index) != 0
                    && modifier.as_str() == name
            })
    };
    use lsp_types::SemanticTokenType as Token;
    let base = match token_type {
        value if value == Token::TYPE.as_str() => HLGroup::Type,
        value if value == Token::FUNCTION.as_str() => HLGroup::Function,
        value if value == Token::VARIABLE.as_str() => HLGroup::Variable,
        value if value == Token::PARAMETER.as_str() => HLGroup::Parameter,
        value if value == Token::PROPERTY.as_str() => HLGroup::Property,
        value if value == Token::NAMESPACE.as_str() => HLGroup::Namespace,
        value if value == Token::ENUM_MEMBER.as_str() => HLGroup::Symbol,
        value if value == Token::KEYWORD.as_str() => HLGroup::Keyword,
        value if value == Token::STRING.as_str() => HLGroup::String,
        value if value == Token::NUMBER.as_str() => HLGroup::Number,
        value if value == Token::OPERATOR.as_str() => HLGroup::Operator,
        value if value == Token::COMMENT.as_str() => HLGroup::Comment,
        value if value == Token::METHOD.as_str() => HLGroup::Method,
        value if value == Token::MODIFIER.as_str() => HLGroup::StorageClass,
        value if value == Token::TYPE_PARAMETER.as_str() => HLGroup::TypeParameter,
        value if value == COMMAND.as_str() => HLGroup::SemanticCommand,
        _ => return None,
    };
    for (modifier, group) in [
        (RING_GENERATOR.as_str(), HLGroup::RingGenerator),
        (OPTION.as_str(), HLGroup::Option),
        (FILE.as_str(), HLGroup::File),
        (UNRESOLVED.as_str(), HLGroup::Unresolved),
    ] {
        if has(modifier) {
            return Some(group);
        }
    }
    Some(base)
}

/// Overlay semantic colors while retaining lexical spans outside their exact ranges.
fn merge(lexical: Vec<HLSpan>, semantic: &[HLSpan]) -> Vec<HLSpan> {
    let mut result = Vec::new();
    let mut semantic_index = 0;
    for span in lexical {
        let mut start = span.content.start;
        while semantic_index < semantic.len() && semantic[semantic_index].content.end <= start {
            result.push(semantic[semantic_index].clone());
            semantic_index += 1;
        }
        for overlay in &semantic[semantic_index..] {
            if overlay.content.start >= span.content.end {
                break;
            }
            if start < overlay.content.start {
                result.push(span.with_content(start..overlay.content.start.min(span.content.end)));
            }
            start = start.max(overlay.content.end);
        }
        if start < span.content.end {
            result.push(span.with_content(start..span.content.end));
        }
    }
    result.extend(semantic[semantic_index..].iter().cloned());
    result.sort_by_key(|span| span.content.start);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{SemanticTokenModifier, SemanticTokenType};

    fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::FUNCTION,
                SemanticTokenType::TYPE,
                SemanticTokenType::VARIABLE,
            ],
            token_modifiers: vec![
                SemanticTokenModifier::new("unresolved"),
                SemanticTokenModifier::new("ringGenerator"),
            ],
        }
    }

    fn token(line: u32, column: u32, length: u32, kind: u32, modifiers: u32) -> SemanticToken {
        SemanticToken {
            delta_line: line,
            delta_start: column,
            length,
            token_type: kind,
            token_modifiers_bitset: modifiers,
        }
    }

    fn cell(generation: u64, submission: u64, index: usize) -> SourceId {
        SourceId::Cell {
            generation,
            submission,
            index,
        }
    }

    #[test]
    fn relative_utf16_tokens_follow_the_negotiated_legend() {
        let source = "😀λ f\nQQ[x]";
        let spans = decode(
            source,
            &legend(),
            &[
                token(0, 4, 1, 0, 0),
                token(1, 0, 2, 1, 0),
                token(0, 3, 1, 2, 2),
            ],
        );
        assert_eq!(
            spans,
            vec![
                HLSpan::new(7..8, HLGroup::Function),
                HLSpan::new(9..11, HLGroup::Type),
                HLSpan::new(12..13, HLGroup::RingGenerator),
            ]
        );
    }

    #[test]
    fn malformed_positions_and_unknown_types_leave_no_invalid_spans() {
        let source = "😀x\ny";
        let spans = decode(
            source,
            &legend(),
            &[
                token(0, 1, 1, 0, 0), // Inside a surrogate pair.
                token(0, 1, 5, 0, 0), // Beyond the line.
                token(0, 0, 1, 99, 0),
                token(1, 0, 1, 2, 1),
                token(u32::MAX, 0, 1, 0, 0),
            ],
        );
        assert_eq!(spans, vec![HLSpan::new(6..7, HLGroup::Unresolved)]);
    }

    #[test]
    fn overlay_splits_lexical_spans_and_also_colors_unclassified_text() {
        let lexical = vec![
            HLSpan::new(0..5, HLGroup::String),
            HLSpan::new(5..8, HLGroup::String),
        ];
        let semantic = vec![
            HLSpan::new(2..6, HLGroup::Namespace),
            HLSpan::new(10..12, HLGroup::Function),
        ];
        assert_eq!(
            merge(lexical, &semantic),
            vec![
                HLSpan::new(0..2, HLGroup::String),
                HLSpan::new(2..6, HLGroup::Namespace),
                HLSpan::new(6..8, HLGroup::String),
                HLSpan::new(10..12, HLGroup::Function),
            ]
        );
    }

    #[test]
    fn repeated_text_is_projected_by_identity_and_empty_results_clear_old_colors() {
        let mut highlighter = SemanticHighlighter::new();
        let mut document = Document::new(0);
        document.push(cell(0, 1, 0), "f");
        document.push(cell(0, 2, 0), "f");
        highlighter.update(document);
        let document = highlighter.document.clone().unwrap();
        highlighter.apply(
            &document,
            &legend(),
            &[token(0, 0, 1, 0, 0), token(1, 0, 1, 1, 0)],
        );
        assert_eq!(
            highlighter.highlight_source(cell(0, 1, 0), "f")[0].kind,
            HLGroup::Function
        );
        assert_eq!(
            highlighter.highlight_source(cell(0, 2, 0), "f")[0].kind,
            HLGroup::Type
        );
        highlighter.apply(&document, &legend(), &[]);
        assert_eq!(
            highlighter.highlight_source(cell(0, 2, 0), "f"),
            SyntaxHighlighter::new().highlight("f")
        );
    }

    #[test]
    fn changed_drafts_drop_colors_and_restart_preserves_historic_cells() {
        let mut highlighter = SemanticHighlighter::new();
        let mut document = Document::new(0);
        document.push(cell(0, 1, 0), "f");
        document.push(SourceId::Draft(0), "f");
        highlighter.update(document);
        let document = highlighter.document.clone().unwrap();
        highlighter.apply(
            &document,
            &legend(),
            &[token(0, 0, 1, 0, 0), token(1, 0, 1, 0, 0)],
        );
        assert_eq!(
            highlighter.highlight_continuation(None, "f")[0].kind,
            HLGroup::Function
        );
        let mut restarted = Document::new(1);
        restarted.push(SourceId::Draft(1), "g");
        highlighter.update(restarted);
        assert!(!highlighter.spans.contains_key(&SourceId::Draft(0)));
        highlighter.clear_current();
        assert_eq!(
            highlighter.highlight_source(cell(0, 1, 0), "f")[0].kind,
            HLGroup::Function
        );
    }

    #[test]
    fn obsolete_responses_never_recolor_a_newer_document() {
        let mut highlighter = SemanticHighlighter::new();
        let mut document = Document::new(0);
        document.push(SourceId::Draft(0), "f");
        highlighter.update(document.clone());
        let obsolete = highlighter.document.clone().unwrap();
        document.text = document.text.replace('f', "g");
        highlighter.update(document);
        let (client, sender) = Client::test_channel();
        highlighter.client = Some(client);
        highlighter.semantic = true;
        sender
            .send(Event::Tokens {
                document: obsolete,
                legend: legend(),
                tokens: vec![token(0, 0, 1, 0, 0)],
            })
            .unwrap();
        assert_eq!(highlighter.poll(), Polled::default());
        assert!(highlighter.spans.is_empty());
    }
}
