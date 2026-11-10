//! Documentation markup, as M2 sends it in HTML, drawn as coloured rows.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem;
use std::ptr;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::LazyLock;

use html2text::config::Config;
use html2text::render::{RichAnnotation, RichDecorator, TaggedLine, TextDecorator};
use html2text::{Element, Handle, RcDom};
use m2_syn::native::parse_native;
use ratatui::layout::{Offset, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use regex::{Captures, Regex};
use unicode_width::UnicodeWidthStr;

use super::maths::{inline, MathStyle, Shift};
use crate::code::highlight::{Highlighter, SyntaxHighlighter};
use crate::code::semantic::PageColors;
use crate::ui::block::{HLSpan, Link, Text, TextSpan};
use crate::ui::present::{wrap, SpanLine};
use crate::ui::render::selection::Inclusions;
use crate::ui::render::RenderContext;
use crate::ui::theme::groups::HLGroup;
use crate::ui::wrap::WordWrap;

mod examples;

pub use self::examples::highlighted as source_highlights;
pub use self::examples::ArchivedExample;
use crate::backend::M2Response;
use crate::ui::config::UI;
use crate::ui::theme::groups::Groups;

/// A private prefix supplied by our decorator, never inferred from prose or code.
const HEADING_MARK: char = '\u{1}';
/// The heading a page's property labels -- `Usage:`, `Inputs:` -- become, a
/// level of their own so they can be told from the page's other headings.
const SYNOPSIS_LABEL: &str = "h5";
const SYNOPSIS_LEVEL: usize = 5;
/// What M2's documentation sets between a name and its description.
const DESCRIPTION_MARK: &str = " -- ";
/// What M2's menus set between a method and the page that documents it.
const ALIAS_POINTER: &str = "-- see";
/// What M2 sets before an optional input's default, which it spells as code.
const DEFAULT_VALUE: &str = "default value ";
/// How the URL of a link to a source position begins.
const FILE_LINK_SCHEME: &str = "file://";
/// How the URL of a link that only carries a highlight group through layout
/// begins: it is painted in that group and is no link to the reader.
const GROUP_LINK_SCHEME: &str = "group:";
/// How the URL of a link that lays a highlight group over its text begins,
/// keeping the text's own colours beneath.
const MARK_LINK_SCHEME: &str = "mark:";
/// What M2 sets after a method listed in a page that has no documentation.
const MISSING_DOCUMENTATION: &str = "(missing documentation)";
/// The attribute typeset maths names each piece's highlight group by, until
/// the document is parsed and the piece can be told to sit inside a link.
const GROUP_ATTRIBUTE: &str = "data-group";
/// The groups of the `token` classes M2 sets a value's text in.
const TOKEN_GROUPS: [(&str, HLGroup); 4] = [
    ("class-name", HLGroup::Type),
    ("function", HLGroup::Function),
    ("constant", HLGroup::Boolean),
    ("string", HLGroup::String),
];
/// Elements whose text cannot carry a highlight group: a link's text reads as
/// that link, preformatted text is coloured as code, and a subscript is read
/// back as plain text.
const UNCOLOURED_WITHIN: [&str; 3] = ["a", "pre", "sub"];
/// The narrowest a table column may be drawn before its text has to wrap, so a
/// type's name stays whole beside a long value. Text longer than this may wrap.
const TABLE_COLUMN_MIN_WIDTH: usize = 24;
/// What a table too wide to fit draws between the cells it stacks in a row,
/// replaced by the glyphs' dashed rule when drawn.
const STACKED_CELL_BORDER: char = '/';

/// Where a documentation node was written, `Package/file.m2:line:column`: a
/// location to read, not code.
static SOURCE_LOCATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\S+\.m2(:[0-9]+)*$").expect("valid source location pattern"));

/// The typewriter spans M2 sets locations and code in, as its HTML spells them.
static TYPEWRITER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span class="tt">([^<]*)</span>"#).expect("valid typewriter span pattern")
});

/// What M2 sets maths in: TeX between `$` signs, for a web page's MathJax to typeset.
/// Code is matched first so a `$` inside it is left as written.
static MATHS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<pre\b.*?</pre>|<code\b.*?</code>|<tt\b.*?</tt>|\$(?<tex>[^$<>]+)\$")
        .expect("valid maths pattern")
});

static PROMPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([io][0-9]+\s*[:=])").expect("valid M2 example prompt pattern"));

thread_local! {
    /// Built once per thread: a highlight configuration compiles the whole query.
    static CODE: RefCell<SyntaxHighlighter> = RefCell::new(SyntaxHighlighter::new());
}

/// Keep html2text's semantic annotations, with heading markers private to layout.
struct Decorator(RichDecorator);

macro_rules! rich_methods {
    ($($method:ident($($arg:ident: $ty:ty),*) -> $result:ty;)+) => {$(
        fn $method(&self, $($arg: $ty),*) -> $result {
            self.0.$method($($arg),*)
        }
    )+};
}

impl TextDecorator for Decorator {
    type Annotation = RichAnnotation;

    rich_methods! {
        decorate_em_start() -> (String, RichAnnotation);
        decorate_em_end() -> String;
        decorate_strong_start() -> (String, RichAnnotation);
        decorate_strong_end() -> String;
        decorate_strikeout_start() -> (String, RichAnnotation);
        decorate_strikeout_end() -> String;
        decorate_code_start() -> (String, RichAnnotation);
        decorate_code_end() -> String;
        decorate_preformat_first() -> RichAnnotation;
        decorate_preformat_cont() -> RichAnnotation;
        quote_prefix() -> String;
        unordered_item_prefix() -> String;
        ordered_item_prefix(i: i64) -> String;
    }

    fn decorate_link_start(&mut self, url: &str) -> (String, RichAnnotation) {
        self.0.decorate_link_start(url)
    }

    fn decorate_link_end(&mut self) -> String {
        self.0.decorate_link_end()
    }

    fn decorate_image(&mut self, src: &str, title: &str) -> (String, RichAnnotation) {
        self.0.decorate_image(src, title)
    }

    fn header_prefix(&self, level: usize) -> String {
        HEADING_MARK.to_string().repeat(level)
    }

    fn make_subblock_decorator(&self) -> Self {
        Self(self.0.make_subblock_decorator())
    }
}

type HtmlLine = TaggedLine<Vec<RichAnnotation>>;

/// Lay HTML out at the available width, retaining document and code semantics.
pub fn spans(html: &str, width: usize) -> Option<Vec<SpanLine>> {
    let width = width.clamp(1, usize::from(u16::MAX));
    let tagged = layout(html, width, MathStyle::default().blackboard)?;
    Some(
        (0..tagged.len())
            .flat_map(|index| prose(&tagged, index, width))
            .collect(),
    )
}

/// Render Text-package markup, composing archived examples from the same blocks
/// as live inputs and outputs while prose retains its HTML layout. The examples
/// were run at `example_width`, M2's print width for examples.
pub fn lines(
    html: &str,
    example_width: usize,
    context: &RenderContext,
) -> Option<Rc<Vec<Line<'static>>>> {
    Some(document(html, example_width, context)?.lines.clone())
}

/// The text among [`lines`]: all of prose, of an example only its code and values,
/// without the gutter and frames it was composed with.
pub fn inclusions(html: &str, example_width: usize, context: &RenderContext) -> Option<Inclusions> {
    Some(document(html, example_width, context)?.inclusions.clone())
}

/// Retain a page's layout for measurement, painting, folding and pointer checks.
/// Entries are bounded because the transcript itself can grow without limit.
struct Document {
    html: String,
    example_width: usize,
    width: u16,
    math: MathStyle,
    groups: &'static Groups,
    numbers_printed: bool,
    code_style: Style,
    links: Vec<Vec<(Rect, Rc<Link>)>>,
    lines: Rc<Vec<Line<'static>>>,
    inclusions: Inclusions,
}

thread_local! {
    static DOCUMENTS: RefCell<VecDeque<Rc<Document>>> = RefCell::default();
}

fn document(html: &str, example_width: usize, context: &RenderContext) -> Option<Rc<Document>> {
    let cached = DOCUMENTS.with_borrow_mut(|documents| {
        let index = documents.iter().position(|entry| {
            entry.example_width == example_width
                && entry.width == context.width()
                && entry.math == context.math()
                && entry.numbers_printed == context.numbers_printed()
                && entry.code_style == context.code_style()
                && ptr::eq(entry.groups, context.groups())
                && entry.html == html
        })?;
        let entry = documents.remove(index)?;
        documents.push_front(entry.clone());
        Some(entry)
    });
    if cached.is_some() {
        return cached;
    }
    // Rendering examples can recursively render markup. Do not borrow the cache
    // while constructing the page.
    let (rows, inclusions, nested_links) = rows(html, example_width, context)?;
    let mut links = placed_links(&rows);
    for link in nested_links {
        let link = Rc::new(link);
        for cells in &link.cells {
            if let Some(row) = links.get_mut(usize::from(cells.y)) {
                row.push((*cells, link.clone()));
            }
        }
    }
    let lines = Rc::new(
        rows.iter()
            .map(|row| match row {
                Row::Prose(spans) => Line::from(spans.spans(context.groups())),
                Row::Example(line) => line.clone(),
            })
            .collect(),
    );
    let entry = Rc::new(Document {
        html: html.to_string(),
        example_width,
        width: context.width(),
        math: context.math(),
        groups: context.groups(),
        numbers_printed: context.numbers_printed(),
        code_style: context.code_style(),
        links,
        lines,
        inclusions,
    });
    DOCUMENTS.with_borrow_mut(|documents| {
        documents.push_front(entry.clone());
        documents.truncate(8);
    });
    Some(entry)
}

/// Update a page's examples and discard only layouts containing its old answers.
pub fn remember_example(page_id: u64, number: u32, response: M2Response) {
    examples::remember(page_id, number, response);
    DOCUMENTS.with_borrow_mut(|documents| documents.retain(|entry| page(&entry.html) != page_id));
}

/// Colour a page's code by what its analysis found.
pub fn remember_colors(colors: PageColors) {
    examples::remember_colors(colors);
    // An example's computed answer can embed another page's code. Its semantic
    // colours affect every enclosing document that retained those rendered rows.
    DOCUMENTS.with_borrow_mut(VecDeque::clear);
}

/// Return every page's code to its syntax colours.
pub fn forget_colors() {
    examples::forget_colors();
    DOCUMENTS.with_borrow_mut(VecDeque::clear);
}

/// The documentation link drawn at `position` among [`lines`], with every cell
/// it covers.
pub fn link_at(
    html: &str,
    example_width: usize,
    context: &RenderContext,
    position: Position,
) -> Option<Link> {
    let document = document(html, example_width, context)?;
    document
        .links
        .get(usize::from(position.y))?
        .iter()
        .find(|(cells, _)| cells.contains(position))
        .map(|(_, link)| link.as_ref().clone())
}

/// Every link in a laid-out document, including links in embedded results.
pub fn links(html: &str, example_width: usize, context: &RenderContext) -> Vec<Link> {
    let Some(document) = document(html, example_width, context) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    document
        .links
        .iter()
        .flatten()
        .filter(|(_, link)| seen.insert(Rc::as_ptr(link)))
        .map(|(_, link)| link.as_ref().clone())
        .collect()
}

/// Index link geometry once during layout. Hover inspects only the pointer's row.
fn placed_links(rows: &[Row]) -> Vec<Vec<(Rect, Rc<Link>)>> {
    let placed: Vec<(Rect, &TextSpan)> = rows
        .iter()
        .enumerate()
        .filter_map(|(y, row)| match row {
            Row::Prose(spans) => Some((y, spans)),
            Row::Example(_) => None,
        })
        .flat_map(|(y, spans)| {
            let mut x = 0;
            spans.iter().map(move |span| {
                let start = x;
                x += span.content.width();
                (
                    Rect::new(start as u16, y as u16, (x - start) as u16, 1),
                    span,
                )
            })
        })
        .collect();
    // HTML layout wraps a link into a run per row, read across bare indentation.
    let mut result = vec![Vec::new(); rows.len()];
    let mut index = 0;
    while index < placed.len() {
        let Some(target) = placed[index].1.target.clone() else {
            index += 1;
            continue;
        };
        let start = index;
        while index < placed.len()
            && (placed[index].1.target.as_deref() == Some(&*target)
                || (placed[index].1.target.is_none() && placed[index].1.content.trim().is_empty()))
        {
            index += 1;
        }
        let cells: Vec<_> = placed[start..index]
            .iter()
            .filter(|(_, span)| span.target.is_some())
            .map(|(cells, _)| *cells)
            .collect();
        let link = Rc::new(Link { target, cells });
        for cells in &link.cells {
            result[usize::from(cells.y)].push((*cells, link.clone()));
        }
    }
    result
}

/// One drawn row: prose, still as the spans it was set in, or an example
/// already painted by the session's blocks.
enum Row {
    Prose(SpanLine),
    Example(Line<'static>),
}

/// Which page of documentation `html` is, to keep what its examples answer
/// by.
pub fn page(html: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    html.hash(&mut hasher);
    hasher.finish()
}

/// Every example the page records, in the order M2 ran them.
pub fn examples(html: &str, example_width: usize) -> Vec<ArchivedExample> {
    preformatted_sources(html)
        .iter()
        .flat_map(|source| examples::archived(source, example_width))
        .collect()
}

/// The M2 code of every block the page draws as code, in reading order.
pub fn code_sources(html: &str, example_width: usize) -> Vec<String> {
    preformatted_sources(html)
        .iter()
        .flat_map(|source| examples::code_sources(source, example_width))
        .chain(inline_code_sources(html))
        .collect()
}

fn preformatted_sources(html: &str) -> Vec<String> {
    let Some(tagged) = layout(html, usize::from(u16::MAX), MathStyle::default().blackboard) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut index = 0;
    while index < tagged.len() {
        match preformatted(&tagged[index]) {
            Some(_) => {
                let block = Preformatted::starting(&tagged, index);
                found.push(block.source);
                index = block.end;
            }
            None => index += 1,
        }
    }
    found
}

/// The package a page documents, which its source location names first:
/// `Graphs.m2:1868:0`, or `Macaulay2Doc/functions/det-doc.m2:25:0`.
pub fn source_package(html: &str) -> Option<String> {
    let location = TYPEWRITER
        .captures_iter(html)
        .map(|captures| captures[1].to_string())
        .find(|text| SOURCE_LOCATION.is_match(text))?;
    let file = location.split(':').next()?;
    let first = file.split('/').next()?;
    Some(first.strip_suffix(".m2").unwrap_or(first).to_string())
}

/// A preformatted block among laid-out lines: the list or quote prefix it sits
/// beside, its text, and the line after it.
struct Preformatted {
    prefix: String,
    source: String,
    end: usize,
}

impl Preformatted {
    fn starting(tagged: &[HtmlLine], start: usize) -> Self {
        let is_code = |tags: &[RichAnnotation]| {
            tags.iter()
                .any(|tag| matches!(tag, RichAnnotation::Preformat(_)))
        };
        let prefix = tagged[start]
            .tagged_strings()
            .take_while(|run| !is_code(&run.tag))
            .map(|run| run.s.as_str())
            .collect();
        let mut source = String::new();
        let mut index = start;
        while index < tagged.len() {
            let line = &tagged[index];
            match preformatted(line) {
                Some(continuation) => {
                    if index > start && !continuation {
                        source.push('\n');
                    }
                    for run in line.tagged_strings().filter(|run| is_code(&run.tag)) {
                        source.push_str(&run.s);
                    }
                }
                None if is_blank(line) => source.push('\n'),
                None => break,
            }
            index += 1;
        }
        // Blank lines after the example separate it from what follows, so they
        // stay with the prose.
        while index > start + 1
            && preformatted(&tagged[index - 1]).is_none()
            && is_blank(&tagged[index - 1])
        {
            index -= 1;
        }
        Self {
            prefix,
            source,
            end: index,
        }
    }
}

fn rows(
    html: &str,
    example_width: usize,
    context: &RenderContext,
) -> Option<(Vec<Row>, Inclusions, Vec<Link>)> {
    let width = usize::from(context.width().max(1));
    let tagged = layout(html, width, context.math().blackboard)?;
    let page = page(html);
    let mut result = Vec::new();
    let mut inclusions = Inclusions::default();
    let mut links = Vec::new();
    let push_prose =
        |result: &mut Vec<Row>, inclusions: &mut Inclusions, rows: Vec<SpanLine>, indent: u16| {
            for row in rows {
                let y = u16::try_from(result.len()).unwrap_or(u16::MAX);
                inclusions.push(Rect::new(
                    indent,
                    y,
                    context.width().saturating_sub(indent),
                    1,
                ));
                result.push(Row::Prose(row));
            }
        };
    let mut index = 0;
    while index < tagged.len() {
        if preformatted(&tagged[index]).is_some() {
            let start = index;
            let Preformatted {
                prefix,
                source,
                end,
            } = Preformatted::starting(&tagged, start);
            index = end;
            // Table columns need their original layout. List and quote prefixes
            // can instead sit beside a native example's own gutter.
            let can_embed = prefix.chars().all(|ch| {
                ch.is_whitespace() || ch.is_ascii_digit() || matches!(ch, '*' | '>' | '.')
            });
            let indent = prefix.width() as u16;
            let inner = context.with_width(context.width().saturating_sub(indent));
            if let Some(example) = can_embed
                .then(|| {
                    examples::render(&source, example_width, page, &inner)
                        .or_else(|| examples::render_code(&source, page, &inner))
                })
                .flatten()
            {
                let top = i32::try_from(result.len()).unwrap_or(i32::MAX);
                inclusions.merge(example.inclusions + Offset::new(i32::from(indent), top));
                links.extend(
                    example
                        .links
                        .into_iter()
                        .map(|link| link.moved(Offset::new(i32::from(indent), top))),
                );
                for (row, mut line) in example.lines.into_iter().enumerate() {
                    if !prefix.is_empty() {
                        line.spans.insert(
                            0,
                            Span::styled(
                                if row == 0 {
                                    prefix.clone()
                                } else {
                                    " ".repeat(prefix.width())
                                },
                                context.groups().style(HLGroup::Punctuation),
                            ),
                        );
                    }
                    result.push(Row::Example(line));
                }
                continue;
            }
            // The prefix only places the block, unless it is a table's own column.
            let indent = if can_embed { indent } else { 0 };
            for line in start..index {
                let rows = prose(&tagged, line, width);
                push_prose(&mut result, &mut inclusions, rows, indent);
            }
        } else {
            let rows = prose(&tagged, index, width);
            push_prose(&mut result, &mut inclusions, rows, 0);
            index += 1;
        }
    }
    Some((result, inclusions, links))
}

fn is_blank(line: &HtmlLine) -> bool {
    line.tagged_strings().all(|run| run.s.trim().is_empty())
}

fn preformatted(line: &HtmlLine) -> Option<bool> {
    line.tagged_strings().find_map(|run| {
        run.tag.iter().find_map(|tag| {
            if let RichAnnotation::Preformat(continuation) = tag {
                Some(*continuation)
            } else {
                None
            }
        })
    })
}

/// Laid-out line `index` fitted to `width`. The title's rule spans the full
/// documentation body, alongside the output blocks below it.
fn prose(tagged: &[HtmlLine], index: usize, width: usize) -> Vec<SpanLine> {
    if let Some(rule) = stacked_cell_rule(&tagged[index], width) {
        return vec![vec![TextSpan::new(rule, HLGroup::Value)]];
    }
    let mut rows = fitted_row(&tagged[index], width);
    let is_title = |line: &HtmlLine| heading_level(line) == 1;
    let title_ends = is_title(&tagged[index]) && !tagged.get(index + 1).is_some_and(is_title);
    if title_ends {
        rows.push(vec![TextSpan::new(
            UI.glyphs.rule.current().repeat(width),
            HLGroup::MarkupHeading1,
        )]);
    }
    rows
}

/// The rule to draw for `line` when it is the border between stacked table cells,
/// which runs the whole `width` where a line of prose would not.
fn stacked_cell_rule(line: &HtmlLine, width: usize) -> Option<String> {
    let text: String = line.tagged_strings().map(|run| run.s.as_str()).collect();
    let border = text.trim_start();
    (text.width() == width
        && !border.is_empty()
        && border.chars().all(|ch| ch == STACKED_CELL_BORDER))
    .then(|| text.replace(STACKED_CELL_BORDER, UI.glyphs.dashed_rule.current()))
}

fn fitted_row(line: &HtmlLine, width: usize) -> Vec<SpanLine> {
    let spans = row(line);
    if spans.iter().map(|span| span.content.width()).sum::<usize>() > width {
        wrap(&spans, width as u16, &WordWrap)
    } else {
        vec![spans]
    }
}

/// How deep the heading `line` belongs to is, or 0 outside one.
fn heading_level(line: &HtmlLine) -> usize {
    line.tagged_strings()
        .find_map(|run| {
            run.s.find(HEADING_MARK).map(|at| {
                run.s[at..]
                    .chars()
                    .take_while(|mark| *mark == HEADING_MARK)
                    .count()
            })
        })
        .unwrap_or(0)
}

/// `html` with each run of TeX outside code set as the text it reads as, where
/// this renderer reads it. Anything it does not is left for the reader as TeX.
fn typeset_maths(html: &str, blackboard: bool) -> Cow<'_, str> {
    MATHS.replace_all(html, |captures: &Captures| {
        captures
            .name("tex")
            .and_then(|tex| inline(&unescape_text(tex.as_str()), blackboard))
            .map_or_else(
                || captures[0].to_string(),
                |spans| spans.iter().map(grouped_html).collect(),
            )
    })
}

/// `span` as an element naming its highlight group.
fn grouped_html(span: &TextSpan) -> String {
    let group = span.kind.description();
    let text = escape_text(&span.content);
    format!(r#"<span {GROUP_ATTRIBUTE}="{group}">{text}</span>"#)
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape_text(html: &str) -> String {
    html.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn html_config() -> Config<Decorator> {
    html2text::config::with_decorator(Decorator(RichDecorator::new()))
        .link_footnotes(false)
        .unicode_strikeout(false)
        .min_wrap_width(TABLE_COLUMN_MIN_WIDTH)
        .allow_width_overflow()
}

/// `html` parsed, its maths typeset and its presentation wrappers normalized.
fn normalized(config: &Config<Decorator>, html: &str, blackboard: bool) -> Option<RcDom> {
    let html = typeset_maths(html, blackboard);
    let dom = config.parse_html(html.as_bytes()).ok()?;
    normalize(&dom.document, false);
    Some(dom)
}

fn layout(html: &str, width: usize, blackboard: bool) -> Option<Vec<HtmlLine>> {
    let config = html_config();
    let dom = normalized(&config, html, blackboard)?;
    decorate_inline(&dom.document, &mut false, page(html));
    let tree = config.dom_to_render_tree(&dom).ok()?;
    config.render_to_lines(tree, width).ok()
}

/// Normalize M2's presentation wrappers before layout. Example tables describe
/// consecutive code blocks; data tables keep their rows, columns and borders.
fn normalize(node: &Handle, in_examples: bool) -> Vec<Handle> {
    match tag_of(node).as_deref() {
        Some("li") => link_signature(node),
        Some("span") => default_as_code(node),
        _ => {}
    }
    let (replacement, subscript, in_examples, flatten, group) = match &node.data {
        Element { name, attrs, .. } => {
            let attrs = attrs.borrow();
            let class = |class: &str| {
                attrs.iter().any(|attr| {
                    &*attr.name.local == "class"
                        && attr
                            .value
                            .split_ascii_whitespace()
                            .any(|value| value == class)
                })
            };
            let tag: &str = &name.local;
            let examples = in_examples || (tag == "table" && class("examples"));
            (
                match tag {
                    // M2 marks the code it displays, a usage say, by its language,
                    // and shows it apart from the text around it.
                    "code" if class("language-macaulay2") && !within("pre", node) => Some("pre"),
                    "kbd" | "samp" => Some("code"),
                    "tt" if reads_as_code(node) => Some("code"),
                    "span" if class("tt") && reads_as_code(node) => Some("code"),
                    "var" => Some("em"),
                    _ => None,
                },
                tag == "sub",
                examples,
                examples
                    && matches!(
                        tag,
                        "table" | "tbody" | "thead" | "tfoot" | "tr" | "td" | "th"
                    ),
                match tag {
                    "samp" if class("token") => TOKEN_GROUPS
                        .iter()
                        .find(|(token, _)| class(token))
                        .map(|(_, group)| *group),
                    "span" => attrs
                        .iter()
                        .find(|attr| &*attr.name.local == GROUP_ATTRIBUTE)
                        .and_then(|attr| HLGroup::named(&attr.value).ok()),
                    _ => None,
                },
            )
        }
        _ => (None, false, in_examples, false, None),
    };
    let children = node.children.take();
    for (previous, list) in element_siblings(&children) {
        if tag_of(&previous).as_deref() == Some("h1") && tag_of(&list).as_deref() == Some("ul") {
            head_synopsis(&list);
        }
    }
    let children: Vec<_> = children
        .iter()
        .flat_map(|child| normalize(child, in_examples))
        .collect();
    let children = paragraphs_between_separators(children);
    for child in &children {
        child.parent.set(Some(Rc::downgrade(node)));
    }
    node.children.replace(children);
    if flatten {
        node.children.take()
    } else if subscript {
        let Some(label) = plain_text(node) else {
            return vec![Rc::clone(node)];
        };
        let label = label.trim();
        let label = Shift::Lowered
            .form(label)
            .unwrap_or_else(|| format!("_{{{label}}}"));
        vec![element("span", &escape_text(&label))]
    } else if let Some(group) = group.filter(|_| can_carry_group(node)) {
        vec![group_link(group, node.children.take())]
    } else if let Some(tag) = replacement {
        // Use the same HTML parser to construct a code element, preserving the
        // original children (including links) instead of matching serialized HTML.
        vec![adopt(tag, node.children.take())]
    } else {
        vec![Rc::clone(node)]
    }
}

/// Set an optional input's `default value null` with its default as code, so the
/// default is coloured as the value it is.
fn default_as_code(span: &Handle) {
    let Some(text) = span.children.borrow().first().and_then(text_leaf) else {
        return;
    };
    if span.children.borrow().len() != 1 {
        return;
    }
    let Some(default) = text.strip_prefix(DEFAULT_VALUE) else {
        return;
    };
    if default.trim().is_empty() || parse_native(default).is_err() {
        return;
    }
    let label = element("span", &escape_text(DEFAULT_VALUE));
    let code = element("code", &escape_text(default));
    adopted_by(Rc::clone(span), vec![label, code]);
}

/// Give an unlinked method signature its entry's documentation destination.
fn link_signature(item: &Handle) {
    let signature = find_signature(item);
    let Some(signature) = signature.filter(|node| !within("a", node)) else {
        return;
    };
    let Some(text) = plain_text(&signature) else {
        return;
    };
    if !reads_as_code(&signature) {
        return;
    }
    let note = taken_note(&signature);
    let mut children = signature.children.take();
    let alias = match note {
        EntryNote::Alias(alias) => alias,
        EntryNote::MissingDocumentation => {
            children = vec![carrying(
                MARK_LINK_SCHEME,
                HLGroup::MarkupMissingDoc,
                children,
            )];
            None
        }
        EntryNote::None => None,
    };
    if alias.is_some() || text.contains('(') {
        let target =
            alias.unwrap_or_else(|| serde_json::to_string(text.trim()).expect("text serializes"));
        let link = parsed_element(
            &format!(
                "<a href=\"{}\"></a>",
                escape_text(&target).replace('"', "&quot;")
            ),
            "a",
        );
        children = vec![adopted_by(link, children)];
    }
    adopted_by(signature, children);
}

/// What M2 sets after a method in a page's menu, which the method's own drawing
/// replaces.
#[derive(Debug)]
enum EntryNote {
    /// ` -- see page -- headline`: documented on that page, which the signature
    /// links to instead, as every such entry repeated the pointer.
    Alias(Option<String>),
    /// ` (missing documentation)`: shown by a tint rather than in words.
    MissingDocumentation,
    None,
}

/// The note after `signature`, taken out of its entry. Other links in an entry,
/// such as an option's type, are not the signature's.
fn taken_note(signature: &Handle) -> EntryNote {
    let Some(parent) = signature.parent.take() else {
        return EntryNote::None;
    };
    signature.parent.set(Some(Weak::clone(&parent)));
    let Some(parent) = parent.upgrade() else {
        return EntryNote::None;
    };
    let mut children = parent.children.borrow_mut();
    let Some(at) = children
        .iter()
        .position(|child| Rc::ptr_eq(child, signature))
    else {
        return EntryNote::None;
    };
    match children
        .get(at + 1)
        .and_then(text_leaf)
        .as_deref()
        .map(str::trim)
    {
        Some(ALIAS_POINTER) => {
            let alias = children[at + 2..]
                .iter()
                .find_map(|child| find_element(child, "a"))
                .and_then(|link| attribute(&link, "href"));
            children.truncate(at + 1);
            EntryNote::Alias(alias)
        }
        Some(MISSING_DOCUMENTATION) => {
            children.remove(at + 1);
            EntryNote::MissingDocumentation
        }
        _ => EntryNote::None,
    }
}

fn find_signature(node: &Handle) -> Option<Handle> {
    let tag = tag_of(node);
    if tag.as_deref() == Some("a") {
        return None;
    }
    if matches!(tag.as_deref(), Some("kbd" | "code" | "tt"))
        || (tag.as_deref() == Some("span")
            && attribute(node, "class")
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|class| class == "tt")))
    {
        return Some(Rc::clone(node));
    }
    node.children.borrow().iter().find_map(find_signature)
}

fn attribute(node: &Handle, key: &str) -> Option<String> {
    match &node.data {
        Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|attribute| &*attribute.name.local == key)
            .map(|attribute| attribute.value.to_string()),
        _ => None,
    }
}

/// Attach inline semantics before width-dependent layout splits the text.
fn decorate_inline(node: &Handle, described: &mut bool, page: u64) {
    let tag = tag_of(node);
    if tag
        .as_deref()
        .is_some_and(|tag| matches!(tag, "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
    {
        return;
    }
    if tag.as_deref() == Some("a") && is_code_link(node) {
        let children = node.children.take();
        node.children.replace(vec![adopt("code", children)]);
    }
    if tag.as_deref() == Some("code") {
        let source = inline_text(node);
        paint_inline_code(node, &analysed_code(page, &source), &mut 0);
        return;
    }
    let mut local = false;
    let described = if tag.as_deref().is_some_and(is_block) {
        &mut local
    } else {
        described
    };
    let children = node.children.take();
    let mut decorated = Vec::new();
    for child in children {
        if let Some(text) = text_leaf(&child) {
            if *described {
                decorated.push(group_link(HLGroup::Muted, vec![child]));
            } else if !tag.as_deref().is_some_and(|tag| tag.starts_with('h')) {
                if let Some((lead, tail)) = text.split_once(DESCRIPTION_MARK) {
                    decorated.push(element("span", &escape_text(lead)));
                    decorated.push(group_link(
                        HLGroup::Muted,
                        vec![element(
                            "span",
                            &escape_text(&format!("{DESCRIPTION_MARK}{tail}")),
                        )],
                    ));
                    *described = true;
                } else {
                    decorated.push(child);
                }
            } else {
                decorated.push(child);
            }
        } else {
            decorate_inline(&child, described, page);
            decorated.push(child);
        }
    }
    adopted_by(Rc::clone(node), decorated);
}

/// Whether a link is set as code: a method's signature, or the name of the node
/// it opens, which a page's analysis can tell the kind of.
fn is_code_link(node: &Handle) -> bool {
    let text = inline_text(node);
    let text = text.trim();
    (text.contains('(') || names_its_target(node))
        && !SOURCE_LOCATION.is_match(text)
        && parse_native(text).is_ok()
}

/// Whether `node` links to the documentation node its own text names.
fn names_its_target(node: &Handle) -> bool {
    let key =
        attribute(node, "href").and_then(|target| serde_json::from_str::<String>(&target).ok());
    key.is_some_and(|key| inline_text(node).trim() == key)
}

/// The code a page sets inline, as `decorate_inline` reads it: every source that
/// is to be coloured by its analysis.
fn inline_code_sources(html: &str) -> Vec<String> {
    let config = html_config();
    let Some(dom) = normalized(&config, html, MathStyle::default().blackboard) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    collect_inline_code(&dom.document, &mut found);
    found
}

fn collect_inline_code(node: &Handle, found: &mut Vec<String>) {
    let tag = tag_of(node);
    if tag
        .as_deref()
        .is_some_and(|tag| matches!(tag, "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
    {
        return;
    }
    if tag.as_deref() == Some("code") || (tag.as_deref() == Some("a") && is_code_link(node)) {
        let source = inline_text(node);
        if parse_native(&source).is_ok() && !found.contains(&source) {
            found.push(source);
        }
        return;
    }
    for child in node.children.borrow().iter() {
        collect_inline_code(child, found);
    }
}

/// Text leaves serialized without introducing layout whitespace.
fn text_leaf(node: &Handle) -> Option<String> {
    if !node.children.borrow().is_empty()
        || matches!(
            &node.data,
            Element { .. } | html2text::Comment { .. } | html2text::Document
        )
    {
        return None;
    }
    let mut serialized = Vec::new();
    node.serialize(&mut serialized).ok()?;
    let serialized = String::from_utf8(serialized).ok()?;
    (!serialized.starts_with('<')).then(|| unescape_text(&serialized))
}

/// The original inline source, without introducing presentation whitespace.
fn inline_text(node: &Handle) -> String {
    if let Some(text) = text_leaf(node) {
        return text;
    }
    node.children.borrow().iter().map(inline_text).collect()
}

/// Distribute whole-source highlights over text leaves while preserving links.
fn paint_inline_code(node: &Handle, spans: &[TextSpan], offset: &mut usize) {
    let mut children = Vec::new();
    for child in node.children.take() {
        if let Some(text) = text_leaf(&child) {
            let end = *offset + text.len();
            let mut start = 0;
            for span in spans {
                let next = start + span.content.len();
                let from = start.max(*offset);
                let to = next.min(end);
                if from < to {
                    children.push(group_link(
                        span.kind,
                        vec![element(
                            "span",
                            &escape_text(&text[from - *offset..to - *offset]),
                        )],
                    ));
                }
                start = next;
            }
            *offset = end;
        } else {
            if attribute(&child, "href").is_some_and(|target| target.starts_with(FILE_LINK_SCHEME))
            {
                *offset += inline_text(&child).len();
            } else {
                paint_inline_code(&child, spans, offset);
            }
            children.push(child);
        }
    }
    adopted_by(Rc::clone(node), children);
}

/// Whether `node` is text alone, set where a highlight group would be seen. A
/// value M2 builds of other values leaves their colours to them.
fn can_carry_group(node: &Handle) -> bool {
    node.children
        .borrow()
        .iter()
        .all(|child| tag_of(child).is_none())
        && !UNCOLOURED_WITHIN.iter().any(|tag| within(tag, node))
}

/// What `node` reads as, without markup.
fn plain_text(node: &Handle) -> Option<String> {
    let config = html2text::config::plain().link_footnotes(false);
    let fragment = RcDom {
        document: Rc::clone(node),
        ..RcDom::default()
    };
    config
        .dom_to_render_tree(&fragment)
        .and_then(|tree| config.render_to_string(tree, 1024))
        .ok()
}

/// Whether typewriter text is M2 code, to be coloured as such. Typewriter face
/// also sets prose apart -- a sentence shown as typed, a file's location -- so
/// only text that parses whole as M2 counts.
fn reads_as_code(node: &Handle) -> bool {
    plain_text(node).is_some_and(|text| {
        let text = text.trim();
        !SOURCE_LOCATION.is_match(text) && parse_native(text).is_ok()
    })
}

/// Whether `node` is a paragraph with nothing in it. Older documentation
/// separates its paragraphs with these rather than enclosing them.
fn is_empty_paragraph(node: &Handle) -> bool {
    tag_of(node).as_deref() == Some("p")
        && plain_text(node).is_none_or(|text| text.trim().is_empty())
}

/// `children` with each run of inline content between empty paragraphs made a
/// paragraph of its own, so the breaks the empty ones stand for are kept.
fn paragraphs_between_separators(children: Vec<Handle>) -> Vec<Handle> {
    if !children.iter().any(is_empty_paragraph) {
        return children;
    }
    let mut grouped = Vec::new();
    let mut run = Vec::new();
    let flush = |run: &mut Vec<Handle>, grouped: &mut Vec<Handle>| {
        let has_content = run.iter().any(|node| {
            tag_of(node).is_some() || plain_text(node).is_some_and(|text| !text.trim().is_empty())
        });
        let run = mem::take(run);
        match has_content {
            true => grouped.push(adopt("p", run)),
            false => grouped.extend(run),
        }
    };
    for child in children {
        let inline = tag_of(&child).is_none_or(|tag| !is_block(&tag));
        if inline {
            run.push(child);
        } else {
            flush(&mut run, &mut grouped);
            if !is_empty_paragraph(&child) {
                grouped.push(child);
            }
        }
    }
    flush(&mut run, &mut grouped);
    grouped
}

/// Whether `node`'s parent is a `tag` element.
fn within(tag: &str, node: &Handle) -> bool {
    let parent = node.parent.take();
    let enclosed = parent
        .as_ref()
        .and_then(|parent| parent.upgrade())
        .is_some_and(|parent| tag_of(&parent).as_deref() == Some(tag) || within(tag, &parent));
    node.parent.set(parent);
    enclosed
}

fn tag_of(node: &Handle) -> Option<String> {
    match &node.data {
        Element { name, .. } => Some(name.local.to_string()),
        _ => None,
    }
}

/// Each element among `nodes` paired with the element before it, the text
/// between them skipped.
fn element_siblings(nodes: &[Handle]) -> Vec<(Handle, Handle)> {
    let elements: Vec<_> = nodes.iter().filter(|node| tag_of(node).is_some()).collect();
    elements
        .windows(2)
        .map(|pair| (Rc::clone(pair[0]), Rc::clone(pair[1])))
        .collect()
}

/// The list under a page's title names its properties -- `Usage:`, `Inputs:`
/// and the rest -- each over its own entries. Its labels become headings, so
/// they read as headers rather than as items.
fn head_synopsis(list: &Handle) {
    for item in list.children.borrow().iter() {
        if tag_of(item).as_deref() != Some("li") {
            continue;
        }
        let first = item
            .children
            .borrow()
            .iter()
            .find(|child| tag_of(child).is_some())
            .cloned();
        // `Usage:` is the term of a definition list, the others lead their item.
        let term = first
            .filter(|first| tag_of(first).as_deref() == Some("dl"))
            .and_then(|list| {
                list.children
                    .borrow()
                    .iter()
                    .find(|child| tag_of(child).as_deref() == Some("dt"))
                    .cloned()
            });
        match term {
            Some(term) => {
                let label = term.children.take();
                term.children.replace(vec![adopt(SYNOPSIS_LABEL, label)]);
            }
            None => {
                let mut children = item.children.take();
                let lead = children
                    .iter()
                    .position(|child| tag_of(child).is_some_and(|tag| is_block(&tag)))
                    .unwrap_or(children.len());
                let label: Vec<_> = children.drain(..lead).collect();
                children.insert(0, adopt(SYNOPSIS_LABEL, label));
                item.children.replace(children);
            }
        }
        for child in item.children.borrow().iter() {
            child.parent.set(Some(Rc::downgrade(item)));
        }
    }
}

/// Whether an element of `tag` starts a block of its own rather than running
/// on in a line, as a label's text and links do.
fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "ul" | "ol"
            | "dl"
            | "div"
            | "p"
            | "pre"
            | "table"
            | "hr"
            | "blockquote"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

/// A new `tag` element holding `children`.
fn adopt(tag: &str, children: Vec<Handle>) -> Handle {
    adopted_by(element(tag, ""), children)
}

/// A link holding `children` that only carries `group` through layout.
fn group_link(group: HLGroup, children: Vec<Handle>) -> Handle {
    carrying(GROUP_LINK_SCHEME, group, children)
}

/// A link holding `children` that carries `group` through layout under `scheme`:
/// [`GROUP_LINK_SCHEME`] paints them in it, [`MARK_LINK_SCHEME`] lays it over them.
fn carrying(scheme: &str, group: HLGroup, children: Vec<Handle>) -> Handle {
    let group = group.description();
    let wrapper = parsed_element(&format!(r#"<a href="{scheme}{group}"></a>"#), "a");
    adopted_by(wrapper, children)
}

fn adopted_by(wrapper: Handle, children: Vec<Handle>) -> Handle {
    for child in &children {
        child.parent.set(Some(Rc::downgrade(&wrapper)));
    }
    wrapper.children.replace(children);
    wrapper
}

fn element(tag: &str, body: &str) -> Handle {
    parsed_element(&format!("<{tag}>{body}</{tag}>"), tag)
}

/// The `tag` element `html` spells.
fn parsed_element(html: &str, tag: &str) -> Handle {
    let dom = html2text::config::rich()
        .parse_html(html.as_bytes())
        .expect("normalized element parses");
    let wrapper = find_element(&dom.document, tag).expect("normalized element exists");
    // Detach before the temporary DOM drops: its destructor drains even
    // externally referenced descendants.
    if let Some(parent) = wrapper.parent.take().and_then(|parent| parent.upgrade()) {
        parent
            .children
            .borrow_mut()
            .retain(|child| !Rc::ptr_eq(child, &wrapper));
    }
    wrapper
}

fn find_element(node: &Handle, tag: &str) -> Option<Handle> {
    if matches!(&node.data, Element { name, .. } if &*name.local == tag) {
        return Some(Rc::clone(node));
    }
    node.children
        .borrow()
        .iter()
        .find_map(|child| find_element(child, tag))
}

fn row(line: &TaggedLine<Vec<RichAnnotation>>) -> SpanLine {
    let Some(_) = line.tagged_strings().next() else {
        return SpanLine::new();
    };
    let level = heading_level(line);
    let heading = (level > 0).then(|| heading(level));
    let mut spans = SpanLine::new();
    let mut described = false;
    for run in line.tagged_strings() {
        let clean = run.s.replace(HEADING_MARK, "");
        let mut text = clean.as_str();
        let preformatted = run
            .tag
            .iter()
            .any(|tag| matches!(tag, RichAnnotation::Preformat(_)));
        let is_code = run.tag.contains(&RichAnnotation::Code) || preformatted;
        if spans.is_empty() && !is_code {
            let indent = text.len() - text.trim_start_matches(' ').len();
            let rest = &text[indent..];
            let marker = if rest.starts_with("* ") || rest.starts_with("> ") {
                Some(2)
            } else {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                (digits > 0 && rest[digits..].starts_with(". ")).then_some(digits + 2)
            };
            if let Some(marker) = marker {
                let end = indent + marker;
                // A heading in a list reads as a header over its entries, so it
                // keeps its indent but not its bullet.
                let kept = if heading.is_some() { indent } else { end };
                if kept > 0 {
                    spans.push(TextSpan::new(&text[..kept], HLGroup::Punctuation));
                }
                text = &text[end..];
            }
        }
        if text.is_empty() {
            continue;
        }
        let group = run.tag.iter().find_map(|tag| match tag {
            RichAnnotation::Link(url) => url
                .strip_prefix(GROUP_LINK_SCHEME)
                .and_then(|name| HLGroup::named(name).ok()),
            _ => None,
        });
        let carried_mark = run.tag.iter().find_map(|tag| match tag {
            RichAnnotation::Link(url) => url
                .strip_prefix(MARK_LINK_SCHEME)
                .and_then(|name| HLGroup::named(name).ok()),
            _ => None,
        });
        let linked = run.tag.iter().any(|tag| {
            matches!(tag, RichAnnotation::Link(url)
                if !url.starts_with(GROUP_LINK_SCHEME) && !url.starts_with(MARK_LINK_SCHEME))
        });
        // A link to another documentation node carries its key as an M2 string,
        // as the shell's startup rewrites it; any other link only reads as one.
        let target = run.tag.iter().find_map(|tag| match tag {
            RichAnnotation::Link(url)
                if url.starts_with('"') || url.starts_with(FILE_LINK_SCHEME) =>
            {
                Some(Arc::<str>::from(url.as_str()))
            }
            _ => None,
        });
        // A source position is set as a link, in the link colour, rather than as
        // the code its typewriter wrapper would make of it.
        let file_link = run.tag.iter().any(
            |tag| matches!(tag, RichAnnotation::Link(url) if url.starts_with(FILE_LINK_SCHEME)),
        );
        // Headings and muted descriptions keep their own colour throughout.
        let mut painted = if let Some(group) = group.filter(|_| heading.is_none() && !described) {
            vec![TextSpan::new(text, group)]
        } else if is_code && !file_link {
            example(text, preformatted)
        } else {
            let group = heading.unwrap_or(if described {
                HLGroup::Muted
            } else {
                HLGroup::Value
            });
            // The title sets its name apart from its brief by weight and slant
            // alone, where prose mutes a name's description.
            let described_here = !described && (heading.is_none() || level == 1);
            match text.split_once(DESCRIPTION_MARK).filter(|_| described_here) {
                Some((name, description)) => {
                    described = true;
                    match heading {
                        Some(_) => vec![
                            TextSpan::new(name, group),
                            TextSpan::new(DESCRIPTION_MARK, group),
                            TextSpan::new(description, HLGroup::MarkupBrief),
                        ],
                        None => vec![
                            TextSpan::new(name, group),
                            TextSpan::new(
                                format!("{DESCRIPTION_MARK}{description}"),
                                HLGroup::Muted,
                            ),
                        ],
                    }
                }
                None => vec![TextSpan::new(text, group)],
            }
        };
        // A definition term is always set in emphasis, which would set `Usage:`
        // apart from the other property labels.
        let emphasis = run.tag.contains(&RichAnnotation::Emphasis) && level != SYNOPSIS_LEVEL;
        // A link keeps the colours of its text, prose or code, and only has a soft
        // underline laid over them.
        let strong = run.tag.contains(&RichAnnotation::Strong);
        let mark = if carried_mark.is_some() {
            carried_mark
        } else if strong && emphasis {
            Some(HLGroup::MarkupStrongEmphasis)
        } else if strong {
            Some(HLGroup::MarkupStrong)
        } else if emphasis {
            Some(HLGroup::MarkupEmphasis)
        } else if linked {
            Some(HLGroup::MarkupDocLink)
        } else if run.tag.contains(&RichAnnotation::Strikeout) {
            Some(HLGroup::MarkupStrike)
        } else {
            None
        };
        for span in &mut painted {
            span.mark = mark;
            span.target.clone_from(&target);
        }
        spans.extend(painted);
    }
    spans
}

/// The title and sections use distinct colours and retain boldness on every wrap.
fn heading(level: usize) -> HLGroup {
    match level {
        1 => HLGroup::MarkupHeading1,
        2 => HLGroup::MarkupHeading2,
        _ => HLGroup::MarkupHeading3,
    }
}

fn example(text: &str, preformatted: bool) -> SpanLine {
    if preformatted {
        if let Some(prompt) = PROMPT.find(text) {
            let group = if text.starts_with('i') {
                HLGroup::OutputNumber
            } else {
                HLGroup::Muted
            };
            let mut spans = vec![TextSpan::new(prompt.as_str(), group)];
            spans.extend(code(&text[prompt.end()..]));
            return spans;
        }
    }
    code(text)
}

/// `source` coloured as M2 code, the stretches the grammar leaves unmarked kept as
/// plain value text.
fn code(source: &str) -> Vec<TextSpan> {
    let marked = CODE.with_borrow_mut(|highlighter| highlighter.highlight(source));
    filled(source, &marked)
}

/// `source` coloured as M2 code under what the analysis of `page` found in it.
fn analysed_code(page: u64, source: &str) -> Vec<TextSpan> {
    filled(source, &examples::highlighted(page, source))
}

fn filled(source: &str, marked: &[HLSpan]) -> Vec<TextSpan> {
    let mut spans = Vec::new();
    let mut at = 0;
    for mark in marked {
        if mark.content.start < at {
            continue;
        }
        if at < mark.content.start {
            spans.push(TextSpan::new(
                &source[at..mark.content.start],
                HLGroup::Value,
            ));
        }
        spans.push(TextSpan::new(
            &source[mark.content.clone()],
            mark.flattened_kind(),
        ));
        at = mark.content.end;
    }
    if at < source.len() {
        spans.push(TextSpan::new(&source[at..], HLGroup::Value));
    }
    spans
}

#[cfg(test)]
mod tests;
