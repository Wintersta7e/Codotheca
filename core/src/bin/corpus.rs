//! The corpus generator. Plan 06 Task 12 replaces this with the real CLI.
//!
//! It exists now because `core/Cargo.toml` declares the target and `--all-features` clippy
//! builds every declared target; a missing path is a hard manifest error.

fn main() {
    let mut err = std::io::stderr();
    let _ = std::io::Write::write_all(&mut err, b"codotheca-corpus: not implemented yet\n");
}
