//! A syntax-highlighted terminal interface for an interactive Macaulay2 session.

// Unit tests may spell non-ASCII text; everything else takes it from `ui::glyphs`.
#![cfg_attr(test, allow(clippy::non_ascii_literal))]

mod app;
mod backend;
mod cli;
mod code;
mod crash;
mod env;
mod errors;
mod input;
mod ui;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::exit;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use backend::M2Backend;
use cli::Options;

fn main() {
    crash::install();
    let options = Options::parse();
    let startup = options
        .startup_code()
        .unwrap_or_else(|message| Options::command().error(ErrorKind::Io, message).exit());
    match M2Backend::spawn_with_options(&options.m2) {
        Ok(backend) => {
            // Unwinding restores the terminal, so what stopped the program can be
            // told once stderr shows again.
            match catch_unwind(AssertUnwindSafe(|| app::run(backend, startup))) {
                Ok(Ok(())) => crash::report(),
                Ok(Err(e)) => {
                    eprintln!("Error: {}", e);
                    crash::report();
                }
                Err(_) => {
                    crash::report();
                    exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to start Macaulay2: {}", e);
            eprintln!("Make sure M2 is installed and on your PATH.");
            exit(1);
        }
    }
}
