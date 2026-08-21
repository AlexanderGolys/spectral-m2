//! Parsing that turns raw `capture` output into clean value/type/diagnostic
//! pieces ready for the typed feed blocks. [`clean_m2_output`] strips the session
//! scaffolding `capture(.., UserMode => true)` wraps around a result; the smaller
//! helpers lift warnings out of a value and tidy M2's location-prefixed messages.

pub struct CapturedOutput {
    pub value: String,
    pub type_info: Option<String>,
    pub warnings: Vec<String>,
}

/// Match a Macaulay2 session prefix like `i12 = ` or `o3 : `, returning the
/// byte length of the prefix (tag, digits, space, separator, space) if present.
fn io_prefix(s: &str, tag: u8, sep: u8) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || b[0] != tag {
        return None;
    }
    let mut i = 1;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 1 {
        return None; // no digits after the tag
    }
    if i + 3 <= b.len() && b[i] == b' ' && b[i + 1] == sep && b[i + 2] == b' ' {
        Some(i + 3)
    } else {
        None
    }
}

/// A trailing `iN : ` prompt with nothing meaningful after it.
fn is_trailing_prompt(s: &str) -> bool {
    io_prefix(s, b'i', b':').is_some_and(|p| s[p..].trim().is_empty())
}

pub fn is_diagnostic(s: &str) -> bool {
    s.contains(": warning:") || s.contains(": error:") || s.contains("--backtrace:")
}

/// Strip M2's cryptic source-location prefix (`currentString:1:6:(3):[8]: `),
/// keeping only the message (`warning: …` / `error: …`). The prefix is slated
/// for removal upstream, so we drop it rather than render it. Returns the input
/// unchanged when no such prefix is present (e.g. an ordinary value line).
pub fn strip_m2_location(s: &str) -> &str {
    match s.find("]: ") {
        Some(i) if s[..i].contains(":(") && s[..i].contains(":[") => &s[i + 3..],
        _ => s,
    }
}

/// Drop a diagnostic's leading keyword (`warning: character '.' …` → `character
/// '.' …`); the keyword becomes the box's border title instead. Returns the
/// whole string when there's no `keyword: ` lead.
pub fn diagnostic_message(s: &str) -> &str {
    match s.split_once(": ") {
        Some((_keyword, message)) => message,
        None => s,
    }
}

pub fn trim_leading_blank(lines: &mut Vec<String>) {
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
}

pub fn trim_trailing_blank(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

/// Strip the session scaffolding that `capture(.., UserMode => true)` wraps
/// around a result: the echoed input prompt, the trailing next prompt, and the
/// `oN = ` / `oN : Type` labels. Returns `(value, type)`.
fn clean_m2_output(raw: &str, code: &str, is_error: bool) -> (String, Option<String>) {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();

    trim_leading_blank(&mut lines);

    // Drop the echoed input: one transcript line per submitted input line.
    if lines
        .first()
        .is_some_and(|l| io_prefix(l, b'i', b':').is_some())
    {
        let echo_n = code.lines().count().max(1);
        for _ in 0..echo_n {
            if lines.is_empty() {
                break;
            }
            lines.remove(0);
        }
    }
    trim_leading_blank(&mut lines);

    if !is_error {
        trim_trailing_blank(&mut lines);
        if lines.last().is_some_and(|l| is_trailing_prompt(l)) {
            lines.pop();
        }
        trim_trailing_blank(&mut lines);
    }

    // Pull off a trailing `oN : Type` annotation.
    let mut type_info = None;
    if !is_error {
        if let Some((line, p)) = lines
            .last()
            .and_then(|line| io_prefix(line, b'o', b':').map(|p| (line, p)))
        {
            type_info = Some(line[p..].trim().to_string());
            lines.pop();
            trim_trailing_blank(&mut lines);
        }
    }

    // Drop the `oN = ` value prefix, preserving 2-D net alignment by removing
    // the same number of leading columns from every value line.
    if !is_error {
        if let Some(plen) = lines.iter().find_map(|l| io_prefix(l, b'o', b'=')) {
            for l in lines.iter_mut() {
                *l = l.chars().skip(plen).collect();
            }
        }
    }
    trim_trailing_blank(&mut lines);

    (lines.join("\n"), type_info)
}

pub fn captured_output(raw: &str, code: &str) -> CapturedOutput {
    let (value, type_info) = clean_m2_output(raw, code, false);
    let mut warnings = Vec::new();
    let value_lines: Vec<&str> = value
        .lines()
        .filter(|line| {
            if !is_diagnostic(line) {
                return true;
            }
            let warning = diagnostic_message(strip_m2_location(line)).to_string();
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
            false
        })
        .collect();
    let first = value_lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(value_lines.len());
    let last = value_lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(first, |index| index + 1);

    CapturedOutput {
        value: value_lines[first..last].join("\n"),
        type_info,
        warnings,
    }
}

pub fn captured_error(raw: &str, code: &str) -> String {
    clean_m2_output(raw, code, true).0
}

#[cfg(test)]
mod tests {
    use super::{captured_output, clean_m2_output, strip_m2_location};

    #[test]
    fn strips_m2_location_prefix() {
        assert_eq!(
            strip_m2_location("ntString:1:12:(3):[8]: warning: character '.' follows"),
            "warning: character '.' follows"
        );
        assert_eq!(
            strip_m2_location("currentString:1:6:(3):[8]: error: exponent missing"),
            "error: exponent missing"
        );
        // An ordinary value line (no location prefix) is returned unchanged.
        assert_eq!(
            strip_m2_location("{{Binary, {Token, 2}}}"),
            "{{Binary, {Token, 2}}}"
        );
    }

    #[test]
    fn strips_echo_and_prompt_simple_value() {
        let raw = "\ni1 : 2 + 2\n\no1 = 4\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "2 + 2", false);
        assert_eq!(val, "4");
        assert_eq!(ty, None);
    }

    #[test]
    fn extracts_type_annotation() {
        let raw = "\ni1 : parse \"2...2\"\n\no1 = {{Binary, {Token, 2}}}\n\no1 : List\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "parse \"2...2\"", false);
        assert_eq!(val, "{{Binary, {Token, 2}}}");
        assert_eq!(ty.as_deref(), Some("List"));
    }

    #[test]
    fn preserves_2d_net_alignment() {
        let raw = "\ni1 : R = QQ[x,y]; x^2 + y\n\n      2\no2 = x  + y\n\no2 : R\n\ni3 : ";
        let (val, ty) = clean_m2_output(raw, "R = QQ[x,y]; x^2 + y", false);
        assert_eq!(val, " 2\nx  + y");
        assert_eq!(ty.as_deref(), Some("R"));
    }

    #[test]
    fn multiline_input_echo_fully_stripped() {
        let raw = "\ni1 : a = 1 +\n     2 + 3\n\no1 = 6\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "a = 1 +\n2 + 3", false);
        assert_eq!(val, "6");
        assert_eq!(ty, None);
    }

    #[test]
    fn suppressed_assignment_is_empty() {
        let raw = "\ni1 : z = 7;\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "z = 7;", false);
        assert_eq!(val, "");
        assert_eq!(ty, None);
    }

    #[test]
    fn error_keeps_message_drops_echo() {
        let raw = "\ni1 : 1/0\ncurrentString:1:1:(3):[9]: error: division by zero";
        let (val, ty) = clean_m2_output(raw, "1/0", true);
        assert_eq!(val, "currentString:1:1:(3):[9]: error: division by zero");
        assert_eq!(ty, None);
    }

    #[test]
    fn extracts_distinct_warnings_from_a_value() {
        let raw = "currentString:1:1:(3):[8]: warning: first\nvalue\ncurrentString:1:1:(3):[8]: warning: first\ncurrentString:1:1:(3):[8]: warning: second";
        let output = captured_output(raw, "code");
        assert_eq!(output.value, "value");
        assert_eq!(output.warnings, vec!["first", "second"]);
    }
}
