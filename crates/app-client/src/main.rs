//! The client binary (architecture section 2, "app-client").
//!
//! Usage: `lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] [--seconds N]` runs
//! paced until the window is closed or `N` seconds have passed;
//! `lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] --fixed FRAMES DUMP` runs
//! decision 5.1's fixed-frame mode for `FRAMES` steps, writes the last frame's dump into the
//! directory `DUMP`, and holds the last frame to the CPU reference. `DIR` for `--shaders` is
//! the cooked shaders' directory, which `cargo xtask gpu` writes under `target/gpu/shaders`.
//! The cache file is `pipeline-cache.bin` in the working directory unless named.
//!
//! Prints the outcome and exits 0; exits 1 with the error, or 1 where a validator sent any
//! message. A panic aborts through `lactose-app`'s crash hook, which writes its report to
//! stderr and to the directory `LACTOSE_CRASH_DIR` names, `crash` in the working directory
//! where it is unset or empty.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use lactose_app::install_crash_hook;
use lactose_app_client::{Mode, Options, run};

fn usage() -> ExitCode {
    eprintln!(
        "usage: lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] \
         [--seconds N | --fixed FRAMES DUMP]"
    );
    ExitCode::from(2)
}

fn parse(mut args: impl Iterator<Item = String>) -> Option<Options> {
    let mut shaders = None;
    let mut packs = None;
    let mut cache = PathBuf::from("pipeline-cache.bin");
    let mut mode = Mode::Paced { seconds: None };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--shaders" => shaders = Some(PathBuf::from(args.next()?)),
            "--packs" => packs = Some(PathBuf::from(args.next()?)),
            "--cache" => cache = PathBuf::from(args.next()?),
            "--seconds" => mode = Mode::Paced { seconds: Some(args.next()?.parse().ok()?) },
            "--fixed" => {
                let frames = args.next()?.parse().ok()?;
                mode = Mode::Fixed { frames, dump: PathBuf::from(args.next()?) };
            }
            _ => return None,
        }
    }
    Some(Options { shaders: shaders?, packs, cache, mode })
}

fn crash_dir() -> PathBuf {
    std::env::var_os("LACTOSE_CRASH_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| PathBuf::from("crash"), PathBuf::from)
}

fn main() -> ExitCode {
    install_crash_hook("lactose-app-client", Some(crash_dir()));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing_subscriber::filter::LevelFilter::INFO)
        .init();
    let Some(options) = parse(std::env::args().skip(1)) else {
        return usage();
    };
    match run(options) {
        Ok(outcome) => {
            println!("{outcome:?}");
            if outcome.messages == 0 {
                ExitCode::SUCCESS
            } else {
                eprintln!("lactose-app-client: the validators sent {} messages", outcome.messages);
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("lactose-app-client: {error}");
            ExitCode::FAILURE
        }
    }
}
