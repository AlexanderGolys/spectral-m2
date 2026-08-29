//! Conversion of M2's structured evaluation reply into feed-ready printed,
//! value, type, and diagnostic text.

use std::sync::LazyLock;

use regex::Regex;

/// Printed text, an optional returned value, and warnings from one evaluation.
pub struct CapturedOutput {
    pub printed: String,
    pub value: Option<CapturedValue>,
    pub warnings: Vec<String>,
}

/// The rendered value and optional M2 type annotation of an output cell.
pub struct CapturedValue {
    pub text: String,
    pub type_info: Option<String>,
}

/// Lines a successful cell's transcript carries that are diagnostics rather than
/// printed text. Errors are not among them: an errored cell never reaches here,
/// its message travelling through [`captured_error`] instead.
/// A line of M2 diagnostic rather than printed text. Errors are not among them:
/// an errored cell never reaches here, its message travelling through
/// [`captured_error`] instead.
static DIAGNOSTIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r": warning:|--backtrace:").expect("valid pattern"));

/// The position M2 stamps on a diagnostic, as in
/// `currentString:1:6:(3):[8]: warning: …`. The leading name is sometimes
/// truncated by M2 itself, so it is matched loosely; the digits and brackets
/// after it are what identify the prefix.
static LOCATION_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s:]*:\d+:\d+:\(\d+\):\[\d+\]: ").expect("valid pattern"));

/// The `warning: ` / `error: ` keyword introducing a diagnostic's message.
static DIAGNOSTIC_KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+: ").expect("valid pattern"));

fn is_diagnostic(s: &str) -> bool {
    DIAGNOSTIC.is_match(s)
}

/// Strip M2's cryptic source-location prefix (`currentString:1:6:(3):[8]: `),
/// keeping only the message (`warning: …` / `error: …`). The prefix is slated
/// for removal upstream, so we drop it rather than render it. Returns the input
/// unchanged when no such prefix is present (e.g. an ordinary value line).
pub fn strip_m2_location(s: &str) -> &str {
    LOCATION_PREFIX
        .find(s)
        .map_or(s, |location| &s[location.end()..])
}

/// Drop the introducing keyword, leaving the message itself.
fn diagnostic_message(s: &str) -> &str {
    DIAGNOSTIC_KEYWORD
        .find(s)
        .map_or(s, |keyword| &s[keyword.end()..])
}

/// The text `capture` produced for `code`.
///
/// The shape is fixed by the shell's own `topLevelMode`, which installs no input
/// prompt and prints no results: the capture is the submitted code, a newline,
/// whatever the code wrote, and a closing newline. Verified against values,
/// printed text, errors, stderr, `<<` without its own newline, and empty input.
/// Anything not of that shape is handed back untouched rather than guessed at.
fn captured_text<'a>(raw: &'a str, code: &str) -> &'a str {
    raw.strip_prefix(code)
        .and_then(|text| text.strip_prefix('\n'))
        .and_then(|text| text.strip_suffix('\n'))
        .unwrap_or(raw)
}

/// Separate the printed text, value, and type annotation in a successful capture.
pub fn captured_output(
    raw: &str,
    code: &str,
    value: Option<&str>,
    type_info: Option<&str>,
) -> CapturedOutput {
    let printed = captured_text(raw, code);
    let mut warnings = Vec::new();
    CapturedOutput {
        printed: take_warnings(printed, &mut warnings),
        value: value.map(|text| CapturedValue {
            text: text.to_string(),
            type_info: type_info.map(str::to_string),
        }),
        warnings,
    }
}

fn take_warnings(text: &str, warnings: &mut Vec<String>) -> String {
    text.split('\n')
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
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove the echoed input from a failed capture while preserving its diagnostic.
pub fn captured_error(raw: &str, code: &str) -> String {
    captured_text(raw, code).to_string()
}

#[cfg(test)]
mod tests {
    use super::{captured_error, captured_output, strip_m2_location};

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

    // Text that merely contains the prefix's punctuation is not a location. The
    // pattern demands the whole shape; matching the pieces separately used to
    // swallow everything before the first `]: `.
    #[test]
    fn punctuation_alone_is_not_a_location_prefix() {
        for line in [
            "map:(x) -> R:[i]: value",
            "see foo:(3) and bar:[8]: done",
            "o3 = HashTable{a:(1), b:[2]: c}",
        ] {
            assert_eq!(strip_m2_location(line), line, "mangled {line:?}");
        }
    }

    #[test]
    fn strips_echo_and_prompt_simple_value() {
        let raw = "2 + 2\n\n";
        let output = captured_output(raw, "2 + 2", Some("4"), None);
        let value = output.value.expect("returned value");
        assert_eq!(output.printed, "");
        assert_eq!(value.text, "4");
        assert_eq!(value.type_info, None);
    }

    #[test]
    fn extracts_type_annotation() {
        let raw = "parse \"2...2\"\n\n";
        let output = captured_output(
            raw,
            "parse \"2...2\"",
            Some("{{Binary, {Token, 2}}}"),
            Some("List"),
        );
        let value = output.value.expect("returned value");
        assert_eq!(value.text, "{{Binary, {Token, 2}}}");
        assert_eq!(value.type_info.as_deref(), Some("List"));
    }

    #[test]
    fn preserves_2d_net_alignment() {
        let raw = "R = QQ[x,y]; x^2 + y\n\n";
        let output = captured_output(raw, "R = QQ[x,y]; x^2 + y", Some(" 2\nx  + y"), Some("R"));
        let value = output.value.expect("returned value");
        assert_eq!(value.text, " 2\nx  + y");
        assert_eq!(value.type_info.as_deref(), Some("R"));
    }

    #[test]
    fn preserves_printed_output_before_returned_value() {
        let raw = "(print \"before\"; 2 + 2)\nbefore\n\n";
        let output = captured_output(raw, "(print \"before\"; 2 + 2)", Some("4"), None);
        let value = output.value.expect("returned value");
        assert_eq!(output.printed, "before\n");
        assert_eq!(value.text, "4");
        assert_eq!(value.type_info, None);
    }

    #[test]
    fn printed_output_marker_does_not_hide_returned_value() {
        let raw = "(print \"o2 = fake\"; 2 + 2)\no2 = fake\n\n";
        let output = captured_output(raw, "(print \"o2 = fake\"; 2 + 2)", Some("4"), None);
        let value = output.value.expect("returned value");
        assert_eq!(output.printed, "o2 = fake\n");
        assert_eq!(value.text, "4");
    }

    #[test]
    fn null_result_has_printed_output_without_output_binding() {
        let raw = "print \"hello\"\nhello\n\n";
        let output = captured_output(raw, "print \"hello\"", Some("null"), Some("Nothing"));
        assert_eq!(output.printed, "hello\n");
        let value = output.value.expect("null result");
        assert_eq!(value.text, "null");
        assert_eq!(value.type_info.as_deref(), Some("Nothing"));
    }

    #[test]
    fn printed_output_marker_does_not_become_a_value() {
        let code = "print \"\\no184 = 11\"";
        let raw = "print \"\\no184 = 11\"\n\no184 = 11\n\n";
        let output = captured_output(raw, code, Some("null"), Some("Nothing"));
        assert_eq!(output.printed, "\no184 = 11\n");
        let value = output.value.expect("null result");
        assert_eq!(value.text, "null");
        assert_eq!(value.type_info.as_deref(), Some("Nothing"));
    }

    #[test]
    fn multiline_input_echo_fully_stripped() {
        let raw = "a = 1 +\n2 + 3\n\n";
        let output = captured_output(raw, "a = 1 +\n2 + 3", Some("6"), None);
        let value = output.value.expect("returned value");
        assert_eq!(value.text, "6");
        assert_eq!(value.type_info, None);
    }

    #[test]
    fn suppressed_assignment_is_empty() {
        let raw = "z = 7;\n\n";
        let output = captured_output(raw, "z = 7;", None, None);
        assert_eq!(output.printed, "");
        assert!(output.value.is_none());
    }

    #[test]
    fn error_keeps_message_drops_echo() {
        let raw = "1/0\ncurrentString:1:1:(0):[4]: error: division by zero\n";
        let output = captured_error(raw, "1/0");
        assert_eq!(output, "currentString:1:1:(0):[4]: error: division by zero");
    }

    #[test]
    fn extracts_distinct_warnings_without_changing_other_lines() {
        let raw = "currentString:1:1:(3):[8]: warning: first\nvalue\ncurrentString:1:1:(3):[8]: warning: first\ncurrentString:1:1:(3):[8]: warning: second";
        let output = captured_output(raw, "code", None, None);
        assert_eq!(output.printed, "value");
        assert!(output.value.is_none());
        assert_eq!(output.warnings, vec!["first", "second"]);
    }
}
