//! A syntax-highlighted terminal interface for an interactive Macaulay2 session.

mod app;
mod backend;
mod cli;
mod code;
mod errors;
mod input;
mod ui;

use std::process::exit;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use backend::M2Backend;
use cli::Options;

fn main() {
    let options = Options::parse();
    let startup = options
        .startup_code()
        .unwrap_or_else(|message| Options::command().error(ErrorKind::Io, message).exit());
    match M2Backend::spawn() {
        Ok(backend) => {
            if let Err(e) = app::run(backend, startup) {
                eprintln!("Error: {}", e);
            }
        }
        Err(e) => {
            eprintln!("Failed to start Macaulay2: {}", e);
            eprintln!("Make sure M2 is installed and on your PATH.");
            exit(1);
        }
    }
}
