//! Generate the Rust highlight names from the shared TOML declarations, and
//! keep non-ASCII string literals inside the glyphs module.

use std::{env, fs, path::PathBuf};

#[path = "build/ascii_literals.rs"]
mod ascii_literals;

fn main() {
    let path = "src/ui/theme/groups.toml";
    println!("cargo:rerun-if-changed={path}");
    let groups: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    for name in groups.keys() {
        assert!(
            name.starts_with(|c: char| c.is_ascii_uppercase())
                && name.chars().all(|c| c.is_ascii_alphanumeric()),
            "invalid Rust highlight group: {name}"
        );
    }
    let names = groups.keys().cloned().collect::<Vec<_>>().join(",\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("highlight_groups.rs"),
        format!("highlight_groups! {{ {names} }}"),
    )
    .unwrap();

    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build/ascii_literals.rs");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let violations = ascii_literals::violations(&root);
    assert!(
        violations.is_empty(),
        "non-ASCII string literals outside the glyphs module ({}); move each into it:\n{}",
        ascii_literals::GLYPHS.join(", "),
        violations.join("\n")
    );
}
