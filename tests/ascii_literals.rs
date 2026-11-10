//! The lexer the build script uses to keep non-ASCII string literals in the
//! glyphs module.

#[path = "../build/ascii_literals.rs"]
#[allow(dead_code)]
mod ascii_literals;

use ascii_literals::scan;

fn flagged(source: &str) -> Vec<usize> {
    scan(source)
        .violations
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}

#[test]
fn flags_direct_and_escaped_non_ascii_strings() {
    let source = "fn f() {\n    let a = \"plain\";\n    let b = \"\u{2500}\";\n    let c = \"\\u{2500}\";\n    let d = r#\"\u{2026}\"#;\n}\n";
    assert_eq!(flagged(source), [3, 4, 5]);
}

#[test]
fn ignores_comments_chars_bytes_and_ascii_escapes() {
    let source = "// \u{2500}\n/* \u{2500} /* \u{2500} */ */\n/// \u{2500}\nconst A: char = '\u{2500}';\nconst B: &str = \"\\u{41}\\n\";\nfn f<'a>(x: &'a str) {}\nconst C: &[u8] = b\"\\xff\";\n";
    assert!(flagged(source).is_empty());
}

#[test]
fn checks_attribute_strings_but_not_doc_text() {
    let source = "#[doc = \"\u{2500}\"]\nstruct A;\n#[error(\"\u{2500}\")]\nstruct B;\n";
    assert_eq!(flagged(source), [3]);
}

#[test]
fn exempts_unit_tests() {
    let source = "#[cfg(test)]\nmod tests {\n    const A: &str = \"\u{2500}\";\n}\n#[test]\nfn t() { let _ = \"\u{2500}\"; }\nconst B: &str = \"\u{2500}\";\n#[cfg(test)]\nmod more;\n";
    let scan = scan(source);
    assert_eq!(
        scan.violations
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [7]
    );
    assert_eq!(scan.test_modules, ["more"]);
}
