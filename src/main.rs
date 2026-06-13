mod app;
mod cells;
mod commands;
mod editor;
mod highlight;
mod m2_backend;
mod session;
mod theme;
mod transcript;
mod ui;

use std::process::exit;

use m2_backend::M2Backend;

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
