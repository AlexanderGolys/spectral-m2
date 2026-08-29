//! A syntax-highlighted terminal interface for an interactive Macaulay2 session.

mod app;
mod backend;
mod code;
mod errors;
mod input;
mod ui;

use std::process::exit;

use backend::M2Backend;

fn main() {
    match M2Backend::spawn() {
        Ok(backend) => {
            if let Err(e) = app::run(backend) {
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
