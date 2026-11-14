//! Markdown from the language server, as the HTML the documentation renderer reads.

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};
use pulldown_cmark_escape::escape_html;

/// How a link that leaves the documentation begins: it is kept as it is.
const WEB_SCHEMES: [&str; 2] = ["http://", "https://"];

/// `markdown` as HTML, each link to another documentation page or section made a
/// link to the documentation of the name it shows, as the renderer opens it.
pub fn html(markdown: &str) -> String {
    let mut events: Vec<Event> = Parser::new_ext(markdown, Options::ENABLE_TABLES).collect();
    let mut at = 0;
    while at < events.len() {
        if let Event::Start(Tag::Link { dest_url, .. }) = &events[at] {
            if !WEB_SCHEMES
                .iter()
                .any(|scheme| dest_url.starts_with(scheme))
            {
                let shown: String = events[at + 1..]
                    .iter()
                    .take_while(|event| !matches!(event, Event::End(TagEnd::Link)))
                    .filter_map(|event| match event {
                        Event::Text(text) | Event::Code(text) => Some(text.as_ref()),
                        _ => None,
                    })
                    .collect();
                let key = serde_json::to_string(shown.trim()).expect("text serializes");
                let mut href = String::new();
                escape_html(&mut href, &key).expect("writing to a string");
                // Written out, as a link's destination would be percent-encoded.
                events[at] = Event::InlineHtml(CowStr::from(format!(r#"<a href="{href}">"#)));
            }
        }
        at += 1;
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

/// The lead of `markdown`: what it says before its first heading or list, the
/// part that tells what it is about, without a rule left trailing.
pub fn lead(markdown: &str) -> &str {
    let mut depth = 0usize;
    let mut end = markdown.len();
    for (event, range) in Parser::new_ext(markdown, Options::ENABLE_TABLES).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. } | Tag::List(_)) if depth == 0 => {
                end = range.start;
                break;
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let mut lead = markdown[..end].trim_end();
    while let Some(before) = ["---", "***", "___"]
        .iter()
        .find_map(|rule| lead.strip_suffix(rule))
    {
        lead = before.trim_end();
    }
    lead
}
