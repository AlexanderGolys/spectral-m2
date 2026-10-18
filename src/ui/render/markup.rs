//! Documentation markup, as M2 sends it in HTML, drawn as coloured rows.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::LazyLock;

use html2text::render::{RichAnnotation, RichDecorator, TaggedLine, TextDecorator};
use html2text::{Element, Handle, RcDom};
use m2_syn::native::parse_native;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

use super::maths::{MathStyle, Shift};
use crate::code::highlight::{Highlighter, SyntaxHighlighter};
use crate::ui::block::{Link, Text, TextSpan};
use crate::ui::controller::settings::theme::groups::HLGroup;
use crate::ui::present::{wrap, SpanLine};
use crate::ui::render::RenderContext;
use crate::ui::wrap::WordWrap;

mod examples;

pub use self::examples::ArchivedExample;
use crate::backend::M2Response;
use crate::ui::controller::settings::theme::groups::Groups;

/// A private prefix supplied by our decorator, never inferred from prose or code.
const HEADING_MARK: char = '\u{1}';
/// The heading a page's property labels -- `Usage:`, `Inputs:` -- become, a
/// level of their own so they can be told from the page's other headings.
const SYNOPSIS_LABEL: &str = "h5";
const SYNOPSIS_LEVEL: usize = 5;
/// What M2's documentation sets between a name and its description.
const DESCRIPTION_MARK: &str = " -- ";

/// Where a documentation node was written, `Package/file.m2:line:column`: a
/// location to read, not code.
static SOURCE_LOCATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\S+\.m2(:[0-9]+)*$").expect("valid source location pattern"));

/// The typewriter spans M2 sets locations and code in, as its HTML spells them.
static TYPEWRITER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span class="tt">([^<]*)</span>"#).expect("valid typewriter span pattern")
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
    let tagged = layout(html, width)?;
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

/// Retain a page's layout for measurement, painting, folding and pointer checks.
/// Entries are bounded because the transcript itself can grow without limit.
struct Document {
    html: String,
    example_width: usize,
    width: u16,
    math: MathStyle,
    groups: &'static Groups,
    numbers_printed: bool,
    links: Vec<Vec<(Rect, Rc<Link>)>>,
    lines: Rc<Vec<Line<'static>>>,
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
                && std::ptr::eq(entry.groups, context.groups())
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
    let rows = rows(html, example_width, context)?;
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
        links: placed_links(&rows),
        lines,
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
    let Some(tagged) = layout(html, usize::from(u16::MAX)) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut index = 0;
    while index < tagged.len() {
        match preformatted(&tagged[index]) {
            Some(_) => {
                let block = Preformatted::starting(&tagged, index);
                found.extend(examples::archived(&block.source, example_width));
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

fn rows(html: &str, example_width: usize, context: &RenderContext) -> Option<Vec<Row>> {
    let width = usize::from(context.width().max(1));
    let tagged = layout(html, width)?;
    let page = page(html);
    let mut result = Vec::new();
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
            let inner = context.with_width(context.width().saturating_sub(prefix.width() as u16));
            if let Some(example) = can_embed
                .then(|| {
                    examples::render(&source, example_width, page, &inner)
                        .or_else(|| examples::render_code(&source, example_width, &inner))
                })
                .flatten()
            {
                for (row, mut line) in example.into_iter().enumerate() {
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
            for line in start..index {
                result.extend(prose(&tagged, line, width).into_iter().map(Row::Prose));
            }
        } else {
            result.extend(prose(&tagged, index, width).into_iter().map(Row::Prose));
            index += 1;
        }
    }
    Some(result)
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
    let mut rows = fitted_row(&tagged[index], width);
    let is_title = |line: &HtmlLine| heading_level(line) == 1;
    let title_ends = is_title(&tagged[index]) && !tagged.get(index + 1).is_some_and(is_title);
    if title_ends {
        rows.push(vec![TextSpan::new(
            "─".repeat(width),
            HLGroup::MarkupHeading1,
        )]);
    }
    rows
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

fn layout(html: &str, width: usize) -> Option<Vec<HtmlLine>> {
    let config = html2text::config::with_decorator(Decorator(RichDecorator::new()))
        .link_footnotes(false)
        .unicode_strikeout(false)
        .min_wrap_width(1)
        .allow_width_overflow();
    let dom = config.parse_html(html.as_bytes()).ok()?;
    normalize(&dom.document, false);
    let tree = config.dom_to_render_tree(&dom).ok()?;
    config.render_to_lines(tree, width).ok()
}

/// Normalize M2's presentation wrappers before layout. Example tables describe
/// consecutive code blocks; data tables keep their rows, columns and borders.
fn normalize(node: &Handle, in_examples: bool) -> Vec<Handle> {
    let (replacement, subscript, in_examples, flatten) = match &node.data {
        Element { name, attrs, .. } => {
            let attrs = attrs.borrow();
            let class = |class: &str| {
                attrs.iter().any(|attr| {
                    attr.name.local.as_ref() == "class"
                        && attr
                            .value
                            .split_ascii_whitespace()
                            .any(|value| value == class)
                })
            };
            let tag = name.local.as_ref();
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
            )
        }
        _ => (None, false, in_examples, false),
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
        let escaped = label
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        vec![element("span", &escaped)]
    } else if let Some(tag) = replacement {
        // Use the same HTML parser to construct a code element, preserving the
        // original children (including links) instead of matching serialized HTML.
        vec![adopt(tag, node.children.take())]
    } else {
        vec![Rc::clone(node)]
    }
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
        .is_some_and(|parent| tag_of(&parent).as_deref() == Some(tag));
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
    let wrapper = element(tag, "");
    for child in &children {
        child.parent.set(Some(Rc::downgrade(&wrapper)));
    }
    wrapper.children.replace(children);
    wrapper
}

fn element(tag: &str, body: &str) -> Handle {
    let html = format!("<{tag}>{body}</{tag}>");
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
    if matches!(&node.data, Element { name, .. } if name.local.as_ref() == tag) {
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
        let linked = run
            .tag
            .iter()
            .any(|tag| matches!(tag, RichAnnotation::Link(_)));
        // A link to another documentation node carries its key as an M2 string,
        // as the shell's startup rewrites it; any other link only reads as one.
        let target = run.tag.iter().find_map(|tag| match tag {
            RichAnnotation::Link(url) if url.starts_with('"') => {
                Some(Arc::<str>::from(url.as_str()))
            }
            _ => None,
        });
        let mut painted = if is_code {
            example(text, preformatted)
        } else {
            let group = heading.unwrap_or(if linked {
                HLGroup::Documentation
            } else if described {
                HLGroup::Muted
            } else {
                HLGroup::Value
            });
            // The title sets its name apart from its brief as prose sets a
            // name apart from its description.
            let described_here = !described && (heading.is_none() || level == 1);
            match text.split_once(DESCRIPTION_MARK).filter(|_| described_here) {
                Some((name, description)) => {
                    described = true;
                    match heading {
                        Some(_) => vec![
                            TextSpan::new(name, group),
                            TextSpan::new(DESCRIPTION_MARK, HLGroup::Muted),
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
        let mark = if run.tag.contains(&RichAnnotation::Strong) && emphasis {
            Some(HLGroup::MarkupStrongEmphasis)
        } else if run.tag.contains(&RichAnnotation::Strong) {
            Some(HLGroup::MarkupStrong)
        } else if emphasis {
            Some(HLGroup::MarkupEmphasis)
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
    let mut spans = Vec::new();
    let mut at = 0;
    for mark in &marked {
        if mark.content.start < at {
            continue;
        }
        if at < mark.content.start {
            spans.push(TextSpan::new(
                &source[at..mark.content.start],
                HLGroup::Value,
            ));
        }
        spans.push(TextSpan::new(&source[mark.content.clone()], mark.kind));
        at = mark.content.end;
    }
    if at < source.len() {
        spans.push(TextSpan::new(&source[at..], HLGroup::Value));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::controller::settings::theme::groups::Groups;
    use ratatui::style::Modifier;

    /// The print width M2 runs documentation examples at.
    const EXAMPLE_WIDTH: usize = 77;

    #[test]
    fn document_layout_is_shared_by_painting_and_pointer_checks() {
        let html = "<p><a href='&quot;ZZ&quot;'>integers</a></p><pre>i1 : 2+2\n\no1 = 4</pre>";
        let context = RenderContext::new(60, 0);
        let first = lines(html, EXAMPLE_WIDTH, &context).unwrap();
        let row = first
            .iter()
            .position(|line| line.to_string().contains("integers"))
            .unwrap();
        assert!(link_at(html, EXAMPLE_WIDTH, &context, Position::new(0, row as u16)).is_some());
        assert!(Rc::ptr_eq(
            &first,
            &lines(html, EXAMPLE_WIDTH, &context).unwrap()
        ));
        for changed in [
            context.with_width(30),
            context.with_blackboard(true),
            context.with_printed_numbers(),
        ] {
            assert!(!Rc::ptr_eq(
                &first,
                &lines(html, EXAMPLE_WIDTH, &changed).unwrap()
            ));
        }
        assert!(!Rc::ptr_eq(&first, &lines(html, 40, &context).unwrap()));
        crate::ui::controller::settings::theme::groups::publish(Groups::default());
        assert!(!Rc::ptr_eq(
            &first,
            &lines(html, EXAMPLE_WIDTH, &context).unwrap()
        ));
    }

    #[test]
    fn a_computed_example_invalidates_only_its_own_page() {
        let html = "<pre>i1 : 2+2\n\no1 = 4</pre>";
        let other = "<p>An unrelated page</p>";
        let context = RenderContext::new(60, 0);
        let before = lines(html, EXAMPLE_WIDTH, &context).unwrap();
        let retained = lines(other, EXAMPLE_WIDTH, &context).unwrap();
        remember_example(
            page(html),
            1,
            M2Response {
                number: 1,
                is_error: false,
                transcript: String::new(),
                value: Some(crate::backend::M2Value::Net("updated answer".into())),
                type_info: None,
                type_parts: Vec::new(),
                has_value: true,
                depth: 1,
                names: Vec::new(),
            },
        );
        let after = lines(html, EXAMPLE_WIDTH, &context).unwrap();
        assert!(!Rc::ptr_eq(&before, &after));
        assert!(after
            .iter()
            .any(|line| line.to_string().contains("updated answer")));
        assert!(Rc::ptr_eq(
            &retained,
            &lines(other, EXAMPLE_WIDTH, &context).unwrap()
        ));
    }

    #[test]
    fn prose_and_usage_code_fit_narrow_documentation() {
        let html = "<h1>arithmetic with complex maps</h1><p>The set of complex maps forms a module over the underlying ring. These methods implement arithmetic operations.</p><dl><dt>Usage:</dt><dd><code class=\"language-macaulay2\">f + g</code></dd><dd><code class=\"language-macaulay2\">a * f</code></dd></dl>";
        for width in [24, 40, 80] {
            let context = RenderContext::new(width, 0);
            let lines = lines(html, 77, &context).unwrap();
            assert!(
                lines.iter().all(|line| line.width() <= usize::from(width)),
                "{lines:?}"
            );
            let content = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(content.contains("underlying ring"), "{content}");
            assert!(content.contains("f + g"), "{content}");
        }
    }

    #[test]
    fn long_help_pages_reuse_layout_for_repeated_hover_checks() {
        let mut backend = crate::backend::M2Backend::spawn_for_examples(None).unwrap();
        let context = RenderContext::new(80, 0);
        for source in ["help ZZ", "help \"arithmetic with complex maps\""] {
            let response = backend.evaluate(source, Some(1)).unwrap();
            let Some(crate::backend::M2Value::Markup {
                html,
                example_width,
                ..
            }) = response.value
            else {
                panic!("expected documentation");
            };
            let start = std::time::Instant::now();
            let first = lines(&html, example_width, &context).unwrap();
            let cold = start.elapsed();
            let start = std::time::Instant::now();
            for row in 0..100 {
                let _ = link_at(&html, example_width, &context, Position::new(10, row));
                assert!(Rc::ptr_eq(
                    &first,
                    &lines(&html, example_width, &context).unwrap()
                ));
            }
            eprintln!(
                "{source}: initial layout {cold:?}; 100 cached hover checks {:?}",
                start.elapsed()
            );
        }
    }

    fn text(lines: &[SpanLine]) -> String {
        lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn literal_hashes_in_prose_and_code_are_preserved() {
        let lines = spans("<p># ordinary text</p><pre>## a comment\n#x</pre>", 40).unwrap();
        let flat = text(&lines);
        assert!(flat.contains("# ordinary text"), "{flat}");
        assert!(flat.contains("## a comment\n#x"), "{flat}");
        assert!(!flat.contains(HEADING_MARK));
        assert!(!lines.iter().flatten().any(|span| matches!(
            span.kind,
            HLGroup::MarkupHeading1 | HLGroup::MarkupHeading2 | HLGroup::MarkupHeading3
        )));
    }

    #[test]
    fn wrapped_headings_keep_their_colour_and_inline_emphasis() {
        let lines = spans(
            "<h2>A long <em>section heading</em> with <a href='&quot;x&quot;'>a link</a></h2>",
            16,
        )
        .unwrap();
        let groups = Groups::default();
        for span in lines.iter().flatten() {
            assert_eq!(span.kind, HLGroup::MarkupHeading2);
            assert!(span.style(&groups).add_modifier.contains(Modifier::BOLD));
        }
        assert!(lines
            .iter()
            .flatten()
            .any(|span| span.mark == Some(HLGroup::MarkupEmphasis)));
        assert!(lines
            .iter()
            .flatten()
            .any(|span| span.target.as_deref() == Some("\"x\"")));
        assert!(!text(&lines).contains(HEADING_MARK));
    }

    #[test]
    fn nested_typewriter_spans_preserve_links_and_syntax_colours() {
        let lines = spans(
            "<span title='example' class='extra tt'><a href='&quot;isPrime(ZZ)&quot;'>isPrime(ZZ)</a></span>",
            60,
        )
        .unwrap();
        assert_eq!(text(&lines), "isPrime(ZZ)");
        let function = lines
            .iter()
            .flatten()
            .find(|span| span.content == "isPrime")
            .unwrap();
        assert_eq!(function.kind, HLGroup::Function);
        assert_eq!(function.target.as_deref(), Some("\"isPrime(ZZ)\""));
        let groups = Groups::default();
        assert_eq!(function.style(&groups).fg, groups[HLGroup::Function].fg);
        assert!(
            !function
                .style(&groups)
                .add_modifier
                .contains(Modifier::UNDERLINED),
            "a link is underlined only under the pointer"
        );
    }

    #[test]
    fn emphasis_and_strikeout_add_modifiers_to_text() {
        let lines = spans("<p><strong>bold</strong> <em>italic</em> <strong><em>both</em></strong> <del>old</del></p>", 60).unwrap();
        let groups = Groups::default();
        for (word, modifiers) in [
            ("bold", Modifier::BOLD),
            ("italic", Modifier::ITALIC),
            ("both", Modifier::BOLD | Modifier::ITALIC),
            ("old", Modifier::CROSSED_OUT),
        ] {
            let span = lines
                .iter()
                .flatten()
                .find(|span| span.content.trim() == word)
                .unwrap();
            assert!(
                span.style(&groups).add_modifier.contains(modifiers),
                "{word}: {span:?}"
            );
        }
        assert!(text(&lines).contains("old"));
    }

    #[test]
    fn documentation_examples_are_code_blocks_without_table_rules() {
        let lines = spans("<table class='extra examples'><tr><td><pre><code>i1 : R = QQ[x]\n\no1 = | x  1 |\n     | 0  x |</code></pre></td></tr><tr><td><pre><code>i2 : 1+1\n\no2 = 2</code></pre></td></tr></table>", 40).unwrap();
        let flat = text(&lines);
        assert!(flat.contains("o1 = | x  1 |\n     | 0  x |"), "{flat}");
        assert!(flat.contains("i2 : 1+1"), "{flat}");
        assert!(!flat.contains(['─', '│', '┬', '┼']), "{flat}");
        assert!(lines
            .iter()
            .flatten()
            .any(|span| span.content == "i1 :" && span.kind == HLGroup::OutputNumber));
        assert!(lines
            .iter()
            .flatten()
            .any(|span| span.content == "o1 =" && span.kind == HLGroup::Muted));
    }

    #[test]
    fn nested_and_numbered_list_markers_are_punctuation() {
        let lines = spans(
            "<ul><li>outer<ul><li>inner</li></ul></li></ul><ol start='3'><li>third</li></ol>",
            30,
        )
        .unwrap();
        for marker in ["* ", "  * ", "3. "] {
            assert!(
                lines
                    .iter()
                    .flatten()
                    .any(|span| span.content == marker && span.kind == HLGroup::Punctuation),
                "{lines:?}"
            );
        }
    }

    #[test]
    fn tables_and_narrow_documents_fit_without_losing_content() {
        let html = "<h1>Documentation</h1><ul><li>nested<ul><li>longidentifier</li></ul></li></ul><table><tr><td>Name</td><td>Value</td></tr><tr><td>αβ</td><td>42</td></tr></table>";
        for width in [4, 8, 16, 40] {
            let lines = spans(html, width).expect("narrow markup renders");
            let flat = text(&lines);
            for line in flat.lines() {
                assert!(line.width() <= width, "width {width}: {line:?}");
            }
            let mut letters: Vec<_> = flat.chars().filter(|ch| ch.is_alphanumeric()).collect();
            let mut expected: Vec<_> = "DocumentationnestedlongidentifierNameValueαβ42"
                .chars()
                .collect();
            letters.sort_unstable();
            expected.sort_unstable();
            assert_eq!(letters, expected, "width {width}: {flat}");
            assert!(flat.contains('─'), "data table lost its rules: {flat}");
        }
    }

    #[test]
    fn subscript_superscript_and_variable_nodes_keep_their_meaning() {
        let lines = spans(
            "<p>x<sub>12</sub> + y<sup>2</sup>, a<sub>index</sub> and <var>v</var></p>",
            60,
        )
        .unwrap();
        assert_eq!(text(&lines), "x₁₂ + y², a_{index} and v");
        assert!(lines
            .iter()
            .flatten()
            .any(|span| span.content == "v" && span.mark == Some(HLGroup::MarkupEmphasis)));
    }

    #[test]
    fn examples_use_native_blocks_after_html_wrapping() {
        let html = "<h1>Example</h1><table class='examples'><tr><td><pre><code>i1 : veryLongVariableName = 123456789\n\no1 = 123456789\n\no1 : ZZ</code></pre></td></tr></table><p>After the example.</p>";
        for width in [20, 40, 80] {
            let lines = super::lines(html, EXAMPLE_WIDTH, &RenderContext::new(width, 0)).unwrap();
            let flat = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!flat.contains("i1 :"), "{flat}");
            assert!(flat.contains("o1"), "{flat}");
            assert!(flat.contains("ZZ"), "{flat}");
            assert!(flat.contains('─'), "{flat}");
            assert!(flat.contains("After the example."), "{flat}");
            assert!(
                lines.iter().all(|line| line.width() <= usize::from(width)),
                "{flat}"
            );
        }
    }

    #[test]
    fn nested_headings_and_examples_do_not_leak_layout_markers() {
        let html = "<ul><li><h2>Section</h2><pre><code>i1 : 2+2\n\no1 = 4\n\no1 : ZZ</code></pre></li></ul>";
        let lines = super::lines(html, EXAMPLE_WIDTH, &RenderContext::new(40, 0)).unwrap();
        let flat = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(flat.contains("Section"), "{flat}");
        assert!(!flat.contains(HEADING_MARK), "{flat}");
        assert!(!flat.contains("i1 :"), "{flat}");
        assert!(flat.contains("o1"), "{flat}");
        assert!(flat.contains("ZZ"), "{flat}");
    }

    fn kinds(html: &str) -> Vec<(String, HLGroup)> {
        spans(html, 60)
            .expect("reads")
            .into_iter()
            .flatten()
            .map(|span| (span.content, span.kind))
            .collect()
    }

    #[test]
    fn headings_and_links_have_semantic_colours() {
        let painted = kinds(
            r#"<div><h1>det</h1><p>see <a href="x">matrix</a>, <b>bold</b> and <em>slanted</em></p></div>"#,
        );
        for (text, group) in [
            ("det", HLGroup::MarkupHeading1),
            ("matrix", HLGroup::Documentation),
            ("bold", HLGroup::Value),
            ("slanted", HLGroup::Value),
        ] {
            assert!(
                painted.contains(&(text.to_string(), group)),
                "{text} should be {group:?} in {painted:?}"
            );
        }
    }

    #[test]
    fn code_is_coloured_as_it_would_be_typed() {
        let painted = kinds(r#"<pre><code>if x then 1 else 0</code></pre>"#);
        assert!(painted.contains(&("if".to_string(), HLGroup::Conditional)));
        assert!(painted.contains(&("1".to_string(), HLGroup::Number)));
    }

    #[test]
    fn a_menu_entry_reads_as_code_a_link_and_a_muted_description() {
        let painted = kinds(
            r#"<ul><li><span><kbd>isPrime(ZZ)</kbd> -- see <span><a href="x">isPrime</a> -- whether prime</span></span></li></ul>"#,
        );
        assert!(painted.contains(&("* ".to_string(), HLGroup::Punctuation)));
        assert!(painted.contains(&("isPrime".to_string(), HLGroup::Function)));
        assert!(painted.contains(&("isPrime".to_string(), HLGroup::Documentation)));
        assert!(painted
            .iter()
            .any(|(text, group)| text.starts_with(" -- see") && *group == HLGroup::Muted));
    }

    #[test]
    fn deeper_headings_recede() {
        let painted = kinds("<div><h1>One</h1><h2>Two</h2><h3>Three</h3></div>");
        assert_eq!(
            painted,
            [
                ("One".to_string(), HLGroup::MarkupHeading1),
                ("─".repeat(60), HLGroup::MarkupHeading1),
                ("Two".to_string(), HLGroup::MarkupHeading2),
                ("Three".to_string(), HLGroup::MarkupHeading3),
            ]
        );
    }

    #[test]
    fn no_text_is_lost_to_colouring() {
        let flat = spans("<div>hello <b>there</b></div>", 60)
            .expect("reads")
            .iter()
            .map(|row| {
                row.iter()
                    .map(|span| span.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(flat, ["hello there"]);
    }

    /// The text of each row `lines` draws.
    fn drawn(html: &str, width: u16) -> Vec<String> {
        super::lines(html, EXAMPLE_WIDTH, &RenderContext::new(width, 0))
            .unwrap()
            .iter()
            .map(|line| line.to_string().trim_end().to_string())
            .collect()
    }

    #[test]
    fn the_title_is_ruled_off_with_its_name_apart_from_its_brief() {
        let html = "<div><h1>ancestors -- the ancestors of something</h1><p>Body.</p></div>";
        let rows = drawn(html, 60);
        let title = "ancestors -- the ancestors of something";
        assert_eq!(rows[0], title);
        assert_eq!(rows[1], "─".repeat(60));
        let painted = kinds(html);
        assert!(painted.contains(&("ancestors".to_string(), HLGroup::MarkupHeading1)));
        assert!(painted.contains(&(
            "the ancestors of something".to_string(),
            HLGroup::MarkupBrief
        )));
    }

    #[test]
    fn the_properties_under_the_title_read_as_headers_over_their_entries() {
        let html = "<div><h1>f -- a function</h1><ul>\
            <li><dl class='element'><dt>Usage: </dt><dd><code>f x</code></dd></dl></li>\
            <li>Inputs:<ul><li><span class='tt'>input</span></li></ul></li></ul>\
            <ul><li>Unrelated:</li></ul></div>";
        let rows = drawn(html, 60);
        let row = |text: &str| {
            rows.iter()
                .find(|row| row.contains(text))
                .unwrap_or_else(|| panic!("{text} is drawn: {rows:?}"))
                .clone()
        };
        assert_eq!(row("Usage:").trim(), "Usage:");
        assert_eq!(row("Inputs:").trim(), "Inputs:");
        assert_eq!(row("input").trim(), "* input", "entries keep their bullets");
        assert_eq!(row("f x").trim(), "f x");
        assert_eq!(
            row("Unrelated:").trim(),
            "* Unrelated:",
            "only the title's list"
        );
        let painted = kinds(html);
        for header in ["Usage:", "Inputs:"] {
            assert!(painted
                .iter()
                .any(|(text, kind)| text.trim() == header && *kind == HLGroup::MarkupHeading3));
        }
    }

    #[test]
    fn a_source_location_is_text_not_code() {
        let html = "<p>The source is in <span class='tt'>Macaulay2Doc/ov_types.m2:495:0</span>, \
            beside <span class='tt'>ancestors x</span>.</p>";
        let painted = kinds(html);
        assert!(painted
            .iter()
            .any(|(text, kind)| text.contains("ov_types.m2:495:0") && *kind == HLGroup::Value));
        assert!(
            painted
                .iter()
                .any(|(text, kind)| text == "ancestors" && *kind != HLGroup::Value),
            "{painted:?}"
        );
    }

    #[test]
    fn a_documentation_link_is_found_over_every_row_it_wraps_onto() {
        let html = "<p>See <a href='&quot;ancestor(Type,Type)&quot;'>whether one type \
            is an ancestor</a> or <a href='https://macaulay2.com'>the site</a>.</p>";
        let context = RenderContext::new(20, 0);
        let rows = drawn(html, 20);
        let link = super::link_at(html, EXAMPLE_WIDTH, &context, Position::new(5, 0))
            .expect("the link starts on the first row");
        assert_eq!(&*link.target, "\"ancestor(Type,Type)\"");
        let covered: String = link
            .cells
            .iter()
            .map(|cells| {
                rows[usize::from(cells.y)]
                    .chars()
                    .skip(usize::from(cells.x))
                    .take(usize::from(cells.width))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            covered.split_whitespace().collect::<Vec<_>>().join(" "),
            "whether one type is an ancestor"
        );
        assert!(
            link.cells.iter().map(|cells| cells.y).max() > Some(0),
            "{rows:?}"
        );

        let site = rows.iter().position(|row| row.contains("site")).unwrap();
        let column = rows[site].find("site").unwrap() as u16;
        assert!(
            super::link_at(
                html,
                EXAMPLE_WIDTH,
                &context,
                Position::new(column, site as u16)
            )
            .is_none(),
            "a link outside the documentation is not followed"
        );
        assert!(super::link_at(html, EXAMPLE_WIDTH, &context, Position::new(0, 0)).is_none());
    }

    #[test]
    fn a_page_lists_its_examples_in_order_and_names_its_package() {
        let html = "<div><h1>graph -- a graph</h1><table class='examples'>\
            <tr><td><pre><code>i1 : G = graph {{1,2}}\n\no1 = G</code></pre></td></tr>\
            <tr><td><pre><code>i2 : G#nothing\nstdio:2:2:(3):[1]: error: key not found in hash table</code></pre></td></tr>\
            </table><p>The source of this document is in <span class=\"tt\">Graphs.m2:1868:0</span>.</p></div>";
        let found = examples(html, EXAMPLE_WIDTH);
        let summary: Vec<_> = found
            .iter()
            .map(|example| (example.number, example.source.as_str(), example.failed))
            .collect();
        assert_eq!(
            summary,
            [(1, "G = graph {{1,2}}", false), (2, "G#nothing", true)]
        );
        assert_eq!(source_package(html).as_deref(), Some("Graphs"));
        assert_eq!(
            source_package("<span class=\"tt\">Macaulay2Doc/functions/det-doc.m2:25:0</span>")
                .as_deref(),
            Some("Macaulay2Doc")
        );
        assert_eq!(source_package("<span class=\"tt\">det M</span>"), None);
    }

    #[test]
    fn typewriter_text_is_coloured_as_code_only_when_it_is_code() {
        let html = "<p><span class=\"tt\">ancestors class 3.</span> but \
            <span class=\"tt\">Here is a long line.  Here is a long line.</span></p>";
        let painted = kinds(html);
        assert!(
            painted
                .iter()
                .any(|(text, kind)| text == "ancestors" && *kind != HLGroup::Value),
            "{painted:?}"
        );
        assert!(
            painted
                .iter()
                .filter(|(text, _)| text.contains("Here") || text.contains("line"))
                .all(|(_, kind)| *kind == HLGroup::Value),
            "{painted:?}"
        );
    }

    #[test]
    fn empty_paragraphs_still_separate_the_text_between_them() {
        let html =
            "<div><h2>Description</h2>First part.    <p></p>\nSecond part.    <p></p>\n</div>";
        let rows = drawn(html, 60);
        let first = rows.iter().position(|row| row.contains("First")).unwrap();
        let second = rows.iter().position(|row| row.contains("Second")).unwrap();
        assert!(second > first + 1, "a blank row between: {rows:?}");
        assert!(rows[first + 1..second]
            .iter()
            .all(|row| row.trim().is_empty()));
    }

    #[test]
    fn displayed_code_is_a_code_block_and_other_preformatted_text_is_not() {
        let context = RenderContext::new(60, 0);
        let code_rows = |html: &str| {
            super::lines(html, EXAMPLE_WIDTH, &context)
                .unwrap()
                .iter()
                .filter(|line| {
                    line.spans
                        .iter()
                        .any(|span| span.style.bg == context.groups().code().bg)
                })
                .map(|line| line.to_string().trim().to_string())
                .collect::<Vec<_>>()
        };
        let usage = code_rows(
            "<dl class='element'><dt>Usage: </dt><dd><code class='language-macaulay2'>det M</code></dd></dl>",
        );
        assert!(usage.contains(&"det M".to_string()), "{usage:?}");
        assert!(
            usage.len() > 1,
            "the block is padded as an example's input is: {usage:?}"
        );
        assert!(code_rows("<pre>+---+\n| a |\n+---+</pre>").is_empty());
    }
}
