mod m2_backend;
mod repl;

use std::process::exit;

use m2_backend::M2Backend;

fn main() {
    match M2Backend::spawn() {
        Ok(backend) => {
            if let Err(e) = repl::run(backend) {
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
