//! The `texrs` driver.
//!
//! Prints the `\message` stream the way `tex` prints it on the terminal —
//! `(./file.tex <messages> )` — which is the comparison `scripts/parity.sh`
//! makes against the real engine.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: texrs FILE.tex");
        return ExitCode::from(1);
    };
    if path == "--version" {
        println!(
            "texrs {} (TeX 3.141592653 mouth+expander)",
            env!("CARGO_PKG_VERSION")
        );
        return ExitCode::SUCCESS;
    }
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("texrs: {path}: {e}");
            return ExitCode::from(1);
        }
    };
    match texrs::run_messages(&src) {
        Ok(msgs) => {
            let body = match msgs.is_empty() {
                true => String::new(),
                false => format!(" {msgs}"),
            };
            println!("(./{path}{body} )");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("! {}.", e.0);
            ExitCode::from(1)
        }
    }
}
