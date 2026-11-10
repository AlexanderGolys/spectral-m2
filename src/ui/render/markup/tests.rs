use super::*;

use crate::ui::theme::groups::Groups;

/// The print width M2 runs documentation examples at.
const EXAMPLE_WIDTH: usize = 77;

#[test]
fn inline_strings_keep_their_highlights_when_words_wrap() {
    let source = "\"alpha beta gamma delta\"";
    let html = format!("<p><code>{source}</code></p>");
    let rows = spans(&html, 12).unwrap();
    let expected = code(source)
        .into_iter()
        .find(|span| span.content.contains("alpha"))
        .unwrap()
        .kind;
    for word in ["alpha", "beta", "gamma", "delta"] {
        let span = rows
            .iter()
            .flatten()
            .find(|span| span.content.contains(word))
            .unwrap();
        assert_eq!(span.kind, expected, "{word}");
    }
    assert!(rows.len() > 1);
}

#[test]
fn descriptions_keep_their_semantics_across_wrapped_rows() {
    let html = "<ul><li><a href='&quot;probe&quot;'>probe(HashTable,Function)</a> -- every key and value must satisfy the predicate condition</li></ul>";
    let rows = spans(html, 24).unwrap();
    let last = rows
        .iter()
        .flatten()
        .find(|span| span.content.contains("condition"))
        .unwrap();
    assert_eq!(last.kind, HLGroup::Muted);
    assert!(rows.len() > 2);
}

#[test]
fn method_signatures_keep_syntax_and_link_to_their_direct_or_alias_documentation() {
    let html = "<ul><li><a href='&quot;direct&quot;'>probe(HashTable,Function)</a> -- description</li><li><kbd>another(HashTable,Function)</kbd> -- see <a href='&quot;alias&quot;'>alias</a> -- description</li><li><span class='tt'>unknown(HashTable)</span> (missing documentation)</li></ul>";
    for width in [18, 90] {
        let rows = spans(html, width).unwrap();
        for (name, target) in [
            ("probe", "\"direct\""),
            ("another", "\"alias\""),
            ("unknown", "\"unknown(HashTable)\""),
        ] {
            let signature = rows
                .iter()
                .flatten()
                .find(|span| span.content.contains(name))
                .unwrap();
            assert_eq!(signature.target.as_deref(), Some(target));
            assert_ne!(signature.kind, HLGroup::Value);
        }
    }
}

#[test]
fn a_method_documented_elsewhere_drops_the_pointer_its_link_replaces() {
    let html = "<ul><li><span><kbd>probe(ZZ)</kbd> -- see <span><a title='headline' href='&quot;page&quot;'>page</a> -- headline</span></span></li></ul>";
    let rows = spans(html, 90).unwrap();
    let text: String = rows
        .iter()
        .flatten()
        .map(|span| span.content.as_str())
        .collect();
    assert!(!text.contains("see"), "{text}");
    assert!(!text.contains("headline"), "{text}");
    let signature = rows
        .iter()
        .flatten()
        .find(|span| span.content.contains("probe"))
        .unwrap();
    assert_eq!(signature.target.as_deref(), Some("\"page\""));
}

#[test]
fn an_option_name_does_not_link_to_the_type_its_entry_mentions() {
    let html = "<ul><li><span><span class='tt'>Probe</span><span class='tt'> => </span><span>a <a href='&quot;Type&quot;'>type</a></span>, <span>default value null</span></span></li></ul>";
    let rows = spans(html, 90).unwrap();
    let name = rows
        .iter()
        .flatten()
        .find(|span| span.content.contains("Probe"))
        .unwrap();
    assert_eq!(name.target, None);
    let text: String = rows
        .iter()
        .flatten()
        .map(|span| span.content.as_str())
        .collect();
    assert!(text.contains("default value null"), "{text}");
}

#[test]
fn a_method_without_documentation_is_tinted_instead_of_labelled() {
    for signature in ["probe(Matrix)", "Probe ? String"] {
        let html = format!("<ul><li><span><span class='tt'>{}</span> (missing documentation)<!--tag: x--></span></li></ul>", escape_text(signature));
        let rows = spans(&html, 90).unwrap();
        let text: String = rows
            .iter()
            .flatten()
            .map(|span| span.content.as_str())
            .collect();
        assert!(!text.contains("missing"), "{text}");
        let pieces: Vec<_> = rows
            .iter()
            .flatten()
            .filter(|span| !span.content.trim().is_empty() && !span.content.starts_with('*'))
            .collect();
        assert!(!pieces.is_empty(), "{text}");
        assert!(
            pieces
                .iter()
                .all(|span| span.mark == Some(HLGroup::MarkupMissingDoc)),
            "{pieces:?}"
        );
    }
}

#[test]
fn an_option_default_is_coloured_as_code() {
    for default in ["null", "true", "false", "&quot;0.0&quot;"] {
        let html = format!("<ul><li><span><span class='tt'>Probe</span><span class='tt'> => </span><span>default value {default}</span>, prose</span></li></ul>");
        let rows = spans(&html, 90).unwrap();
        let spans: Vec<_> = rows.iter().flatten().collect();
        let label = spans
            .iter()
            .find(|span| span.content.contains("default"))
            .unwrap();
        let value = spans
            .iter()
            .find(|span| span.content.contains(&default.replace("&quot;", "\"")))
            .unwrap();
        assert_ne!(value.kind, label.kind, "{default}");
        let text: String = spans.iter().map(|span| span.content.as_str()).collect();
        assert!(text.contains("default value "), "{text}");
    }
}

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
    crate::ui::theme::groups::publish(Groups::default());
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
    for span in lines.iter().flatten() {
        assert_eq!(span.kind, HLGroup::MarkupHeading2);
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
    let html =
        "<ul><li><h2>Section</h2><pre><code>i1 : 2+2\n\no1 = 4\n\no1 : ZZ</code></pre></li></ul>";
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
        ("matrix", HLGroup::Value),
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
    assert!(
        painted.contains(&(" -- ".to_string(), HLGroup::MarkupHeading1)),
        "the separator wears the title's colour: {painted:?}"
    );
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
    let html = "<div><h2>Description</h2>First part.    <p></p>\nSecond part.    <p></p>\n</div>";
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

fn text_of(html: &str, width: usize) -> String {
    spans(html, width)
        .expect("reads")
        .iter()
        .flatten()
        .map(|span| span.content.as_str())
        .collect()
}

#[test]
fn tex_in_a_table_cell_is_set_as_text() {
    let text = text_of(
        r"<table><tr><td>$\mathit{opts}$</td><td>$\texttt{OptionTable}\left\{\mathit{Algorithm}\ \Rightarrow \ \mathit{Inhomogeneous},\:\mathit{BasisElementLimit}\ \Rightarrow \ \infty,\:\mathit{DegreeLimit}\ \Rightarrow \ \left\{\,\right\},\:\mathit{MaxReductionCount}\ \Rightarrow \ \cdots\right\}$</td></tr></table>",
        200,
    );
    assert!(!text.contains('$') && !text.contains('\\'), "{text}");
    assert!(
        text.contains("opts") && text.contains("OptionTable"),
        "{text}"
    );
    assert!(text.contains("DegreeLimit"), "{text}");
}

/// The text and group of each coloured piece of `html`, in order.
fn groups_of(html: &str, width: usize) -> Vec<(String, HLGroup)> {
    spans(html, width)
        .expect("reads")
        .iter()
        .flatten()
        .map(|span| (span.content.clone(), span.kind))
        .collect()
}

fn group_of(pieces: &[(String, HLGroup)], text: &str) -> HLGroup {
    pieces
        .iter()
        .find(|(content, _)| content == text)
        .unwrap_or_else(|| panic!("{text} in {pieces:?}"))
        .1
}

#[test]
fn tex_keeps_the_colours_of_what_it_names() {
    let pieces = groups_of(
        r"<table><tr><td>$\mathit{opts}$</td><td>$\texttt{OptionTable}\left\{\mathit{Algorithm}\ \Rightarrow \ \texttt{false}\right\}$</td></tr></table>",
        200,
    );
    assert_eq!(group_of(&pieces, "opts"), HLGroup::Variable);
    assert_eq!(group_of(&pieces, "OptionTable"), HLGroup::Type);
    assert_eq!(group_of(&pieces, "false"), HLGroup::Boolean);
}

#[test]
fn tex_reads_the_same_with_or_without_colours() {
    assert_eq!(
        text_of(
            r"<p>so $\mathit{a}\ \Rightarrow \ \mathit{b}$ holds</p>",
            60
        )
        .trim(),
        format!(
            "so {} holds",
            inline(r"\mathit{a}\ \Rightarrow \ \mathit{b}", false)
                .expect("reads")
                .iter()
                .map(|span| span.content.as_str())
                .collect::<String>()
        )
    );
}

#[test]
fn tokens_m2_names_are_painted_as_it_names_them() {
    let pieces = groups_of(
        r#"<table><tr><td><samp class="token class-name">OptionTable</samp></td><td><samp class="token function">gb</samp></td><td><samp class="token constant">true</samp></td></tr></table>"#,
        80,
    );
    assert_eq!(group_of(&pieces, "OptionTable"), HLGroup::Type);
    assert_eq!(group_of(&pieces, "gb"), HLGroup::Function);
    assert_eq!(group_of(&pieces, "true"), HLGroup::Boolean);
}

#[test]
fn a_token_built_of_others_leaves_colours_to_them_and_keeps_its_links() {
    let lines = spans(
        r#"<p><samp class="token function"><span><samp class="token class-name">FunctionClosure</samp></span>[<span><samp><a href="file:///a/b.m2#L1:C2-L3:C4">b.m2:1:2-3:4</a></samp></span>]</samp></p>"#,
        80,
    )
    .expect("reads");
    let spans: Vec<_> = lines.iter().flatten().collect();
    let piece = |text: &str| {
        spans
            .iter()
            .find(|span| span.content.contains(text))
            .unwrap_or_else(|| panic!("{text} in {spans:?}"))
    };
    assert_eq!(piece("FunctionClosure").kind, HLGroup::Type);
    assert_eq!(piece("b.m2").mark, Some(HLGroup::MarkupDocLink));
}

#[test]
fn tex_inside_a_link_stays_part_of_that_link() {
    let lines = spans(
        r#"<p><a href='&quot;ZZ&quot;'>the $\mathit{opts}$ page</a></p>"#,
        60,
    )
    .expect("reads");
    assert_eq!(text(&lines), "the opts page");
    assert!(lines
        .iter()
        .flatten()
        .all(|span| span.target.as_deref() == Some("\"ZZ\"")));
}

#[test]
fn a_heading_keeps_its_colour_over_tex() {
    let lines = spans(r"<h2>On $\mathit{opts}$</h2>", 60).expect("reads");
    let kinds: Vec<_> = lines
        .iter()
        .flatten()
        .filter(|span| span.content.contains("opts"))
        .map(|span| span.kind)
        .collect();
    assert_eq!(kinds, [heading(2)]);
}

#[test]
fn a_short_table_column_stays_whole_beside_a_long_one() {
    let long = "word ".repeat(100);
    let html = format!(
        "<table><tr><th>symbol</th><th>class</th><th>value</th></tr><tr><td>opts</td><td>OptionTable</td><td>{long}</td></tr></table>"
    );
    let flat = text(&spans(&html, 120).expect("reads"));
    for name in ["symbol", "class", "OptionTable"] {
        assert!(flat.contains(name), "{name} was cut: {flat}");
    }
}

#[test]
fn stacked_table_cells_fit_and_are_ruled_apart() {
    let html = "<table><tr><td>first cell here</td><td>second cell here</td><td>third cell here</td></tr></table>";
    let width = 20;
    let flat = text(&spans(html, width).expect("reads"));
    assert!(!flat.contains(STACKED_CELL_BORDER), "{flat}");
    assert!(flat.contains(STACKED_CELL_RULE), "{flat}");
    for line in flat.lines() {
        assert!(line.width() <= width, "{line:?}");
    }
}

#[test]
fn a_line_of_slashes_in_prose_is_left_alone() {
    let text = text_of("<p>/// a raw string ///</p>", 40);
    assert!(text.contains("///"), "{text}");
}

#[test]
fn dollar_signs_in_code_are_left_alone() {
    let text = text_of("<pre><code>f = \"$a$ and $b$\"</code></pre>", 40);
    assert!(text.contains("$a$ and $b$"), "{text}");
}

#[test]
fn tex_that_is_not_read_stays_as_written() {
    let text = text_of(r"<p>costs $\unknowncommand{x}$ now</p>", 60);
    assert!(text.contains(r"$\unknowncommand{x}$"), "{text}");
}

#[test]
fn source_links_keep_their_encoded_path_and_location() {
    let target = "file:///tmp/source%20file.m2#L12:C3-L14:C8";
    let html = format!("<p><a href='{target}'>source location</a></p>");
    let context = RenderContext::new(60, 0);
    let link =
        link_at(&html, 60, &context, Position::new(1, 0)).expect("source location is clickable");
    assert_eq!(&*link.target, target);
}

#[test]
fn semantic_colours_of_computed_source_refresh_the_enclosing_example() {
    let child = "<pre>nestedSemanticProbe = 1</pre>";
    let parent = "<p>Nested source colour probe</p><pre>i1 : code nestedSemanticProbe\n\no1 = recorded source</pre>";
    let context = RenderContext::new(80, 0);
    remember_example(
        page(parent),
        1,
        M2Response {
            number: 1,
            is_error: false,
            transcript: String::new(),
            value: Some(crate::backend::M2Value::Markup {
                html: child.into(),
                fallback: String::new(),
                example_width: 60,
            }),
            type_info: None,
            type_parts: Vec::new(),
            has_value: true,
            depth: 1,
            names: Vec::new(),
        },
    );
    let before = lines(parent, 60, &context).unwrap();
    remember_colors(crate::code::semantic::PageColors {
        page: page(child),
        sources: vec![(
            "nestedSemanticProbe = 1".into(),
            vec![crate::ui::block::HLSpan::new(0..19, HLGroup::Function)],
        )],
    });
    let after = lines(parent, 60, &context).unwrap();
    assert!(
        !Rc::ptr_eq(&before, &after),
        "enclosing rows retained stale semantic colours"
    );
}

#[test]
fn a_documentation_name_takes_the_colour_its_pages_analysis_found_for_it() {
    let html = "<p>See <a href='&quot;HeaderType&quot;'>HeaderType</a> and \
        <a href='&quot;integers&quot;'>the integers</a>.</p>";
    assert_eq!(inline_code_sources(html), ["HeaderType"]);
    let kind_of = |name: &str| {
        spans(html, 80)
            .unwrap()
            .into_iter()
            .flatten()
            .find(|span| span.content.contains(name))
            .map(|span| span.kind)
    };
    let plain = kind_of("HeaderType");
    remember_colors(crate::code::semantic::PageColors {
        page: page(html),
        sources: vec![(
            "HeaderType".to_string(),
            vec![HLSpan::new(0..10, HLGroup::Type)],
        )],
    });
    assert_eq!(kind_of("HeaderType"), Some(HLGroup::Type));
    assert_ne!(plain, Some(HLGroup::Type));
    assert_eq!(
        kind_of("the integers"),
        Some(HLGroup::Value),
        "prose stays prose"
    );
    forget_colors();
}
