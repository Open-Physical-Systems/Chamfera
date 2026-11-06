//! The Chamfera desktop app.
//!
//! ```text
//! chamfera [part.stl ...] [--sample] [--theme light|dark] [--screenshot shot.png]
//! ```

use std::process::ExitCode;

const USAGE: &str =
    "usage: chamfera [part.stl ...] [--sample] [--theme light|dark] [--screenshot shot.png]";

fn main() -> ExitCode {
    let mut options = chamfera_ui::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sample" => options.sample = true,
            "--theme" => match args.next().as_deref() {
                Some("light") => options.theme = Some(chamfera_render::ThemeMode::Light),
                Some("dark") => options.theme = Some(chamfera_render::ThemeMode::Dark),
                _ => return fail(USAGE),
            },
            "--screenshot" => match args.next() {
                Some(p) => options.screenshot = Some(p.into()),
                None => return fail(USAGE),
            },
            "--version" => {
                println!("chamfera {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => return fail(USAGE),
            file => options.files.push(file.into()),
        }
    }
    match chamfera_ui::run_native(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("chamfera: {msg}");
    ExitCode::FAILURE
}
