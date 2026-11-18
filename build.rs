//! Generate the Rust highlight names from the shared TOML declarations, and the
//! list of the `Spectral` package's source files the shell carries.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// The pinned `Spectral` submodule: its main file and the directory of the
/// auxiliary files it loads, as `installPackage` expects them side by side.
const SPECTRAL: &str = "Spectral";
const SPECTRAL_MAIN: &str = "Spectral.m2";
/// How `newPackage` names the package's version: `Version => "0.2.0",`.
const SPECTRAL_VERSION_FIELD: &str = "Version =>";

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    highlight_groups(&out);
    spectral_sources(&out);
}

fn highlight_groups(out: &Path) {
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
    fs::write(
        out.join("highlight_groups.rs"),
        format!("highlight_groups! {{ {names} }}"),
    )
    .unwrap();
}

/// `spectral_package.rs`: an array of `EmbeddedSource`s, one per `.m2` file of
/// the package, each named by its path relative to the package's directory.
fn spectral_sources(out: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(SPECTRAL);
    let auxiliary = root.join(SPECTRAL);
    println!("cargo:rerun-if-changed={}", root.join(SPECTRAL_MAIN).display());
    println!("cargo:rerun-if-changed={}", auxiliary.display());
    let main = fs::read_to_string(root.join(SPECTRAL_MAIN)).unwrap();
    let version = main
        .lines()
        .find_map(|line| line.trim().strip_prefix(SPECTRAL_VERSION_FIELD))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("the Spectral package declares its Version");
    println!("cargo:rustc-env=SPECTRAL_VERSION={version}");
    let mut files = vec![root.join(SPECTRAL_MAIN)];
    files.extend(
        fs::read_dir(&auxiliary)
            .expect("the Spectral submodule is checked out")
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "m2")),
    );
    files.sort();
    let entries = files
        .iter()
        .map(|file| {
            let name = file.strip_prefix(&root).unwrap().display();
            format!(
                "EmbeddedSource {{ file_name: {:?}, contents: include_str!({:?}) }}",
                name.to_string(),
                file.display().to_string()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    fs::write(out.join("spectral_package.rs"), format!("[{entries}]")).unwrap();
}
