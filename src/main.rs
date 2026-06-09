mod m2_backend;
mod repl;

fn main() {
    match m2_backend::M2Backend::spawn() {
        Ok(backend) => {
            if let Err(e) = repl::run(backend) {
                eprintln!("Error: {}", e);
            }
        }
        Err(e) => {
            eprintln!("Failed to start Macaulay2: {}", e);
            eprintln!("Make sure M2 is installed and on your PATH.");
            std::process::exit(1);
        }
    }
}