//! Documentation markup, as M2 sends it in HTML, drawn as coloured rows.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use html2text::render::{RichAnnotation, RichDecorator, TaggedLine, TextDecorator};
use html2text::{Element, Handle, RcDom};
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

use super::maths::Shift;
use crate::code::highlight::{Highlighter, SyntaxHighlighter};
use crate::ui::block::{Text, TextSpan};
use crate::ui::controller::settings::theme::groups::HLGroup;
use crate::ui::present::{wrap, SpanLine};
use crate::ui::render::RenderContext;
use crate::ui::wrap::WordWrap;

mod examples;

/// A private prefix supplied by our decorator, never inferred from prose or code.
const HEADING_MARK: char = '\u{1}';
/// What M2's documentation sets between a name and its description.
const DESCRIPTION_MARK: &str = " -- ";

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
    Some(
        layout(html, width)?
            .iter()
            .flat_map(|line| fitted_row(line, width))
            .collect(),
    )
}

/// Render Text-package markup, composing archived examples from the same blocks
/// as live inputs and outputs while prose retains its HTML layout.
pub fn lines(html: &str, context: &RenderContext) -> Option<Vec<Line<'static>>> {
    let width = usize::from(context.width().max(1));
    let tagged = layout(html, width)?;
    let mut result = Vec::new();
    let mut index = 0;
    while index < tagged.len() {
        if preformatted(&tagged[index]).is_some() {
            let start = index;
            let mut source = String::new();
            let mut prefix = String::new();
            for run in tagged[index].tagged_strings() {
                if run
                    .tag
                    .iter()
                    .any(|tag| matches!(tag, RichAnnotation::Preformat(_)))
                {
                    break;
                }
                prefix.push_str(&run.s);
            }
            while index < tagged.len() {
                let line = &tagged[index];
                match preformatted(line) {
                    Some(continuation) => {
                        if index > start && !continuation {
                            source.push('\n');
                        }
                        for run in line.tagged_strings().filter(|run| {
                            run.tag
                                .iter()
                                .any(|tag| matches!(tag, RichAnnotation::Preformat(_)))
                        }) {
                            source.push_str(&run.s);
                        }
                    }
                    None if line.tagged_strings().all(|run| run.s.trim().is_empty()) => {
                        source.push('\n')
                    }
                    None => break,
                }
                index += 1;
            }
            // Table columns need their original layout. List and quote prefixes
            // can instead sit beside a native example's own gutter.
            let can_embed = prefix.chars().all(|ch| {
                ch.is_whitespace() || ch.is_ascii_digit() || matches!(ch, '*' | '>' | '.')
            });
            let inner = context.with_width(context.width().saturating_sub(prefix.width() as u16));
            if let Some(example) = can_embed
                .then(|| examples::render(&source, &inner))
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
                    result.push(line);
                }
                continue;
            }
            for line in &tagged[start..index] {
                result.extend(
                    fitted_row(line, width)
                        .into_iter()
                        .map(|row| Line::from(row.spans(context.groups()))),
                );
            }
        } else {
            result.extend(
                fitted_row(&tagged[index], width)
                    .into_iter()
                    .map(|row| Line::from(row.spans(context.groups()))),
            );
            index += 1;
        }
    }
    Some(result)
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

fn fitted_row(line: &HtmlLine, width: usize) -> Vec<SpanLine> {
    let spans = row(line);
    if spans.iter().map(|span| span.content.width()).sum::<usize>() > width {
        wrap(&spans, width as u16, &WordWrap)
    } else {
        vec![spans]
    }
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
                    "kbd" | "tt" | "samp" => Some("code"),
                    "span" if class("tt") => Some("code"),
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
    let children: Vec<_> = children
        .iter()
        .flat_map(|child| normalize(child, in_examples))
        .collect();
    for child in &children {
        child.parent.set(Some(Rc::downgrade(node)));
    }
    node.children.replace(children);
    if flatten {
        node.children.take()
    } else if subscript {
        let config = html2text::config::plain().link_footnotes(false);
        let fragment = RcDom {
            document: Rc::clone(node),
            ..RcDom::default()
        };
        let Ok(label) = config
            .dom_to_render_tree(&fragment)
            .and_then(|tree| config.render_to_string(tree, 1024))
        else {
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
        let wrapper = element(tag, "");
        let children = node.children.take();
        for child in &children {
            child.parent.set(Some(Rc::downgrade(&wrapper)));
        }
        wrapper.children.replace(children);
        vec![wrapper]
    } else {
        vec![Rc::clone(node)]
    }
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
    let level = line
        .tagged_strings()
        .find_map(|run| {
            run.s.find(HEADING_MARK).map(|at| {
                run.s[at..]
                    .chars()
                    .take_while(|mark| *mark == HEADING_MARK)
                    .count()
            })
        })
        .unwrap_or(0);
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
                spans.push(TextSpan::new(&text[..end], HLGroup::Punctuation));
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
            if heading.is_none() && !described {
                if let Some((name, description)) = text.split_once(DESCRIPTION_MARK) {
                    described = true;
                    vec![
                        TextSpan::new(name, group),
                        TextSpan::new(format!("{DESCRIPTION_MARK}{description}"), HLGroup::Muted),
                    ]
                } else {
                    vec![TextSpan::new(text, group)]
                }
            } else {
                vec![TextSpan::new(text, group)]
            }
        };
        let mark = if linked {
            Some(HLGroup::MarkupLink)
        } else if run.tag.contains(&RichAnnotation::Strong)
            && run.tag.contains(&RichAnnotation::Emphasis)
        {
            Some(HLGroup::MarkupStrongEmphasis)
        } else if run.tag.contains(&RichAnnotation::Strong) {
            Some(HLGroup::MarkupStrong)
        } else if run.tag.contains(&RichAnnotation::Emphasis) {
            Some(HLGroup::MarkupEmphasis)
        } else if run.tag.contains(&RichAnnotation::Strikeout) {
            Some(HLGroup::MarkupStrike)
        } else if is_code {
            Some(HLGroup::MarkupCode)
        } else {
            None
        };
        for span in &mut painted {
            span.mark = mark;
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
            "<h2>A long <em>section heading</em> with <a href='x'>a link</a></h2>",
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
            .any(|span| span.mark == Some(HLGroup::MarkupLink)));
        assert!(!text(&lines).contains(HEADING_MARK));
    }

    #[test]
    fn nested_typewriter_spans_preserve_links_and_syntax_colours() {
        let lines = spans(
            "<span title='example' class='extra tt'><a href='x'>isPrime(ZZ)</a></span>",
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
        assert_eq!(function.mark, Some(HLGroup::MarkupLink));
        let groups = Groups::default();
        assert_eq!(function.style(&groups).fg, groups[HLGroup::Function].fg);
        assert!(function
            .style(&groups)
            .add_modifier
            .contains(Modifier::UNDERLINED));
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
            let lines = super::lines(html, &RenderContext::new(width, 0)).unwrap();
            let flat = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!flat.contains("i1 :"), "{flat}");
            assert!(!flat.contains("o1 ="), "{flat}");
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
        let lines = super::lines(html, &RenderContext::new(40, 0)).unwrap();
        let flat = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(flat.contains("Section"), "{flat}");
        assert!(!flat.contains(HEADING_MARK), "{flat}");
        assert!(!flat.contains("i1 :"), "{flat}");
        assert!(flat.contains("o1"), "{flat}");
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
}
