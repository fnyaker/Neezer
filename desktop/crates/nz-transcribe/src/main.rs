//! `neezer-transcribe`: the standalone half of local transcription.
//!
//! The app only ever starts this binary; it can run on its own on a machine
//! with no Neezer window (lot T2 adds `file`/`engine`/`probe`, T3 adds
//! `worker`/`local`). D0 ships the one subcommand CI needs to prove the
//! whisper.cpp build it links, Vulkan included.

use std::process::ExitCode;

const USAGE: &str = "\
neezer-transcribe — transcription locale des épisodes, sur le GPU via Vulkan.

USAGE:
    neezer-transcribe --version
    neezer-transcribe --help
";

/// `--version`: what this binary is, which whisper.cpp it links and what that
/// build was compiled with. No GPU is touched here on purpose (see the crate
/// doc): a `--version` that aborts on a machine without a Vulkan driver would
/// be the first thing a new user meets.
fn version() -> String {
    format!(
        "neezer-transcribe {}\nwhisper.cpp {} (backend vulkan compilé)\n{}",
        env!("CARGO_PKG_VERSION"),
        nz_transcribe::WHISPER_VERSION,
        nz_transcribe::system_info(),
    )
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version" | "-V") | None => {
            println!("{}", version());
            ExitCode::SUCCESS
        }
        Some("-h" | "--help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("neezer-transcribe : argument inconnu : {other}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
