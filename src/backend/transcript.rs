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
    #[cfg(test)]
    pub text: String,
    pub type_info: Option<String>,
}

/// Ordinary output emitted before a failed evaluation and its diagnostic.
pub struct CapturedError {
    pub printed: String,
    /// Warnings raised before the error, each with its continuation lines.
    pub warnings: Vec<String>,
    pub diagnostic: String,
}

/// A warning's opening line, carried beside otherwise successful output: stamped
/// with M2's location, or bare as Core's `warningMessage` writes it.
static WARNING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^(?:{LOCATION})?warning: ")).expect("valid pattern"));

/// A line continuing the warning above it, indented to the keyword's colon:
/// `       : debug with expression   debug 365   or with command line option …`.
static CONTINUATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s+: ").expect("valid pattern"));

/// Debugger trace markers emitted after the useful error diagnostic.
static BACKTRACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^--back ?trace(?:--|:)").expect("valid pattern"));

/// The position M2 stamps on a diagnostic, as in
/// `currentString:1:6:(3):[8]: warning: …`. The leading name is sometimes
/// truncated by M2 itself, so it is matched loosely; the digits and brackets
/// after it are what identify the prefix.
static LOCATION_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{LOCATION}")).expect("valid pattern"));
const LOCATION: &str = r"[^\s:]*:\d+:\d+:\(\d+\):\[\d+\]: ";

/// The `warning: ` / `error: ` keyword introducing a diagnostic's message.
static DIAGNOSTIC_KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+: ").expect("valid pattern"));

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
    raw.strip_prefix(code.trim_end())
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
    let (printed, warnings) = take_warnings(captured_text(raw, code));
    CapturedOutput {
        printed,
        value: value.map(|_text| CapturedValue {
            #[cfg(test)]
            text: _text.to_string(),
            type_info: type_info.map(str::to_string),
        }),
        warnings,
    }
}

/// Split `text` into what stays printed and its warnings, one per `warning:`
/// line with the lines continuing it, each keeping its own block.
fn take_warnings(text: &str) -> (String, Vec<String>) {
    let mut printed = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut continuing = false;
    for line in text.split('\n') {
        if BACKTRACE.is_match(line.trim()) {
            continuing = false;
        } else if WARNING.is_match(line) {
            warnings.push(diagnostic_message(strip_m2_location(line)).to_string());
            continuing = true;
        } else if let Some(more) = CONTINUATION.find(line).filter(|_| continuing) {
            let warning = warnings
                .last_mut()
                .expect("a continuation follows its warning");
            warning.push('\n');
            warning.push_str(&line[more.end()..]);
        } else {
            continuing = false;
            printed.push(line);
        }
    }
    (printed.join("\n"), warnings)
}

/// Remove the echoed input and keep pre-error output out of the diagnostic.
pub fn captured_error(raw: &str, code: &str) -> CapturedError {
    let lines = captured_text(raw, code)
        .lines()
        .filter(|line| !BACKTRACE.is_match(line.trim()))
        .collect::<Vec<_>>();
    let diagnostic = lines
        .iter()
        .position(|line| strip_m2_location(line).starts_with("error: "))
        .unwrap_or_default();
    let (printed, warnings) = take_warnings(&lines[..diagnostic].join("\n"));
    CapturedError {
        printed,
        warnings,
        diagnostic: lines[diagnostic..].join("\n"),
    }
}
