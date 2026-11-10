//! Generate the Rust highlight names from the shared TOML declarations.

use std::{env, fs, path::PathBuf};

/// Where Cargo asks build scripts to write what they generate.
const OUT_DIR: &str = "OUT_DIR";

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
    let out = PathBuf::from(env::var_os(OUT_DIR).unwrap());
    fs::write(
        out.join("highlight_groups.rs"),
        format!("highlight_groups! {{ {names} }}"),
    )
    .unwrap();
}
