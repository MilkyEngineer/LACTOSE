//! RICOTTA, the asset cooker's binary: `ricotta SOURCES PACK` cooks every `.gltf` file under
//! the directory `SOURCES` to a skeleton and writes them as one pack at `PACK`
//! (`README.md`, "The command line"). Prints one line per asset written, its name, ID and
//! version, in name order; exits 1 with the error on stderr when the cook fails, 2 with the
//! usage when the arguments are wrong.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use lactose_core::Result;
use lactose_io::{Io, IoConfig};
use lactose_task::Executor;

const USAGE: &str = "usage: ricotta SOURCES PACK";

fn main() -> ExitCode {
    install_crash_hook();
    let mut args = std::env::args_os().skip(1);
    let (Some(sources), Some(pack), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(Path::new(&sources), Path::new(&pack)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ricotta: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The cook, its futures driven on this thread by `Executor::block_on` (D130): one worker
/// of each class, since the cook spawns nothing, and the platform's pool.
fn run(sources: &Path, pack: &Path) -> Result<()> {
    let executor = Executor::builder().frame_workers(1).background_workers(1).build()?;
    let platform = executor.platform().as_ref();
    let io = Io::start(platform, IoConfig::for_platform(platform))?;
    let cooked = executor.block_on(lactose_cook::cook(&io, sources, pack));
    io.shutdown();
    executor.shutdown();
    for asset in cooked? {
        println!("{} {:#018x} {:#018x}", asset.name, asset.id.to_bits(), asset.version.to_bits());
    }
    Ok(())
}

/// Architecture section 5.1: a panic writes a crash dump and aborts. At milestone 1 the
/// dump is a text report on stderr, as `lactose-app-headless` writes it.
fn install_crash_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let mut stderr = std::io::stderr().lock();
        // Nothing is left to report a failed write to: the process aborts either way.
        let _ = writeln!(
            stderr,
            "ricotta crashed\nthread: {}\n{info}\nbacktrace:\n{backtrace}",
            thread.name().unwrap_or("<unnamed>"),
        );
        let _ = stderr.flush();
        std::process::abort();
    }));
}
