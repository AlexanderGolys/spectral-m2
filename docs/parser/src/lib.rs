//! Cargo-built Macaulay2 parser entry point for `mdbook-tsitter`.

unsafe extern "C" {
    #[link_name = "tree_sitter_macaulay2"]
    fn macaulay2() -> *const ();
}

/// Return the Macaulay2 Tree-sitter language used by the documentation.
///
/// # Safety
///
/// The returned pointer is owned by the linked grammar and must remain loaded.
#[no_mangle]
pub unsafe extern "C" fn tree_sitter_macaulay2_docs() -> *const () {
    drop(tree_sitter_macaulay2::language());
    unsafe { macaulay2() }
}
