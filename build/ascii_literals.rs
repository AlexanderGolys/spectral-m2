//! Keep non-ASCII string literals in the glyphs module.
//!
//! Every symbol beyond ASCII that the shell draws belongs in the glyphs module
//! (`src/ui/glyphs.rs` and the files below `src/ui/glyphs/`), where a limited
//! terminal can be given another spelling. The build script scans the crate's
//! sources with [`violations`] and fails the build on any string literal whose
//! value leaves ASCII, written directly or through a `\u{..}` escape, anywhere
//! else. Unit tests (`#[cfg(test)]` and `#[test]` items, and modules declared
//! under `#[cfg(test)]`), comments and doc attributes are exempt.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

/// The glyphs module, the one place allowed to spell non-ASCII string literals.
pub const GLYPHS: [&str; 2] = ["src/ui/glyphs.rs", "src/ui/glyphs"];

/// Each non-ASCII string literal outside the glyphs module and unit tests, as
/// `path:line: literal`, for the crate whose manifest is in `root`.
pub fn violations(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_sources(&root.join("src"), &mut files);
    files.sort();

    let mut test_files = HashSet::new();
    let mut scans = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        let scan = scan(&source);
        for name in &scan.test_modules {
            test_files.extend(child_module_paths(&path, name));
        }
        scans.push((path, scan));
    }

    let mut violations = Vec::new();
    for (path, scan) in scans {
        let relative = path.strip_prefix(root).unwrap();
        if GLYPHS.iter().any(|glyphs| relative.starts_with(glyphs))
            || is_under_test_module(&path, &test_files)
        {
            continue;
        }
        for (line, literal) in scan.violations {
            violations.push(format!("{}:{line}: {literal}", relative.display()));
        }
    }
    violations
}

fn collect_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// The files that a `mod name;` declared in `parent` may live in.
fn child_module_paths(parent: &Path, name: &str) -> [PathBuf; 2] {
    let stem = parent.file_stem().unwrap().to_str().unwrap();
    let dir = parent.parent().unwrap();
    let base = if matches!(stem, "main" | "lib" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    [
        base.join(format!("{name}.rs")),
        base.join(name).join("mod.rs"),
    ]
}

/// Whether `path` is a test-only module file or sits below one.
fn is_under_test_module(path: &Path, test_files: &HashSet<PathBuf>) -> bool {
    test_files.iter().any(|test| {
        path == test
            || test
                .file_stem()
                .and_then(|stem| (stem != "mod").then(|| test.with_extension("")))
                .or_else(|| test.parent().map(Path::to_path_buf))
                .is_some_and(|dir| path.starts_with(dir))
    })
}

/// What one source file holds.
#[derive(Default)]
pub struct Scan {
    /// Line and source text of each non-ASCII literal outside test code.
    pub violations: Vec<(usize, String)>,
    /// Names of `mod name;` declarations made under `#[cfg(test)]`.
    pub test_modules: Vec<String>,
}

/// A minimal Rust lexer: enough to tell string literals from comments, char
/// literals and lifetimes, and to follow which braces hold test-only items.
pub fn scan(source: &str) -> Scan {
    let chars: Vec<char> = source.chars().collect();
    let mut scan = Scan::default();
    let mut i = 0;
    let mut line = 1;
    // Brace depth, and the depths at which test-only blocks opened.
    let mut depth = 0usize;
    let mut test_blocks: Vec<usize> = Vec::new();
    // A test attribute was seen and its item's body or `;` is still ahead;
    // the nesting of `(`/`[` inside the item's header so far.
    let mut pending_test = false;
    let mut header_nesting = 0usize;
    let mut header_idents: Vec<String> = Vec::new();

    let at = |j: usize| chars.get(j).copied().unwrap_or('\0');

    while i < chars.len() {
        let c = chars[i];
        let in_test = pending_test || !test_blocks.is_empty();
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if at(i + 1) == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if at(i + 1) == '*' => {
                let mut nesting = 0;
                while i < chars.len() {
                    if chars[i] == '/' && at(i + 1) == '*' {
                        nesting += 1;
                        i += 2;
                    } else if chars[i] == '*' && at(i + 1) == '/' {
                        nesting -= 1;
                        i += 2;
                        if nesting == 0 {
                            break;
                        }
                    } else {
                        if chars[i] == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            '#' if at(i + 1) == '[' || (at(i + 1) == '!' && at(i + 2) == '[') => {
                let start = i;
                let doc = chars[i..]
                    .iter()
                    .filter(|c| !matches!(c, '#' | '!' | '[') && !c.is_whitespace())
                    .take(4)
                    .collect::<String>()
                    .strip_prefix("doc")
                    .is_some_and(|rest| {
                        !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')
                    });
                let mut nesting = 0;
                while i < chars.len() {
                    match chars[i] {
                        '[' => nesting += 1,
                        ']' => {
                            nesting -= 1;
                            if nesting == 0 {
                                i += 1;
                                break;
                            }
                        }
                        '"' => {
                            // Doc text is not drawn; other attribute strings, such as
                            // error messages, may be.
                            let (end, newlines) = skip_string(&chars, i, 0);
                            if !doc {
                                check_literal(&chars[i..end], false, line, in_test, &mut scan);
                            }
                            line += newlines;
                            i = end;
                            continue;
                        }
                        '\n' => line += 1,
                        _ => {}
                    }
                    i += 1;
                }
                let attr: String = chars[start..i]
                    .iter()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                if matches!(attr.as_str(), "#[cfg(test)]" | "#[test]") {
                    pending_test = true;
                    header_nesting = 0;
                    header_idents.clear();
                }
            }
            '\'' => {
                // A char literal, or a lifetime or label.
                if at(i + 1) == '\\' {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if at(i + 2) == '\'' {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            '"' => {
                let (end, newlines) = skip_string(&chars, i, 0);
                check_literal(&chars[i..end], false, line, in_test, &mut scan);
                line += newlines;
                i = end;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                let raw = matches!(ident.as_str(), "r" | "br" | "cr");
                let hashes = (i..).take_while(|&j| at(j) == '#').count();
                if raw && at(i + hashes) == '"' {
                    let (end, newlines) = skip_string(&chars, i + hashes, hashes);
                    let is_byte = ident == "br";
                    check_literal(&chars[i..end], true, line, in_test || is_byte, &mut scan);
                    line += newlines;
                    i = end;
                } else if matches!(ident.as_str(), "b" | "c") && at(i) == '"' {
                    let (end, newlines) = skip_string(&chars, i, 0);
                    check_literal(
                        &chars[i..end],
                        false,
                        line,
                        in_test || ident == "b",
                        &mut scan,
                    );
                    line += newlines;
                    i = end;
                } else if ident == "b" && at(i) == '\'' {
                    i += if at(i + 1) == '\\' { 4 } else { 3 };
                    while at(i - 1) != '\'' {
                        i += 1;
                    }
                } else if pending_test {
                    header_idents.push(ident);
                }
            }
            '(' | '[' if pending_test => {
                header_nesting += 1;
                i += 1;
            }
            ')' | ']' if pending_test => {
                header_nesting = header_nesting.saturating_sub(1);
                i += 1;
            }
            ';' if pending_test && header_nesting == 0 => {
                if header_idents.iter().any(|ident| ident == "mod") {
                    if let Some(name) = header_idents.last() {
                        scan.test_modules.push(name.clone());
                    }
                }
                pending_test = false;
                i += 1;
            }
            '{' => {
                if pending_test && header_nesting == 0 {
                    test_blocks.push(depth);
                    pending_test = false;
                }
                depth += 1;
                i += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if test_blocks.last() == Some(&depth) {
                    test_blocks.pop();
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    scan
}

/// The index just past the string literal whose opening quote is at `start`,
/// and how many newlines it spans. A raw string closes on `"` and `hashes` `#`.
fn skip_string(chars: &[char], start: usize, hashes: usize) -> (usize, usize) {
    let raw = hashes > 0 || (start > 0 && chars[start - 1] == 'r');
    let mut i = start + 1;
    let mut newlines = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if !raw => i += 1,
            '"' if chars[i + 1..]
                .iter()
                .take(hashes)
                .filter(|&&c| c == '#')
                .count()
                == hashes =>
            {
                return (i + 1 + hashes, newlines);
            }
            '\n' => newlines += 1,
            _ => {}
        }
        i += 1;
    }
    (i, newlines)
}

/// Record `literal` (opening quote or raw hashes to closing quote) if its value
/// leaves ASCII and it is not test code.
fn check_literal(literal: &[char], raw: bool, line: usize, in_test: bool, scan: &mut Scan) {
    if in_test {
        return;
    }
    let text: String = literal.iter().collect();
    let escaped = !raw && unicode_escapes(&text).any(|value| value > 0x7f);
    if !text.is_ascii() || escaped {
        let shown: String = text.chars().take(60).collect();
        scan.violations.push((line, shown));
    }
}

/// The code points of each `\u{..}` escape in a string literal's source.
fn unicode_escapes(text: &str) -> impl Iterator<Item = u32> + '_ {
    let mut rest = text;
    std::iter::from_fn(move || loop {
        let at = rest.find('\\')?;
        let after = &rest[at + 1..];
        let next = after.chars().next()?;
        rest = &after[next.len_utf8()..];
        if next == 'u' && rest.starts_with('{') {
            let end = rest.find('}')?;
            let digits: String = rest[1..end].chars().filter(|&c| c != '_').collect();
            rest = &rest[end + 1..];
            if let Ok(value) = u32::from_str_radix(&digits, 16) {
                return Some(value);
            }
        }
    })
}
