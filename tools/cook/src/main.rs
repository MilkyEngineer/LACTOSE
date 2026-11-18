//! RICOTTA, the asset cooker's binary: `ricotta [--slangc SLANGC] [--code DIR] [--shaders
//! DIR] SOURCES PACK` cooks every `.gltf` file under the directory `SOURCES` to a skeleton and every Slang
//! shader to its cooked shaders, and writes them as one pack at `PACK` (`README.md`, "The
//! command line"). Prints one line per asset written, its name, ID and version, in name
//! order; exits 1 with the error on stderr when the cook fails, 2 with the usage when the
//! arguments are wrong.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use lactose_cook::{Shaders, Slang};
use lactose_core::{Error, ErrorKind, Result};
use lactose_io::{Io, IoConfig};
use lactose_task::Executor;

const USAGE: &str = "usage: ricotta [--slangc SLANGC] [--code DIR] [--shaders DIR] SOURCES PACK";

/// The command line, read.
struct Arguments {
    slangc: Option<PathBuf>,
    code: Option<PathBuf>,
    shaders: Option<PathBuf>,
    sources: PathBuf,
    pack: PathBuf,
}

fn main() -> ExitCode {
    install_crash_hook();
    let Some(arguments) = arguments(std::env::args_os().skip(1)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ricotta: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The options, each at most once and before the two paths, then the two paths.
fn arguments(mut args: impl Iterator<Item = OsString>) -> Option<Arguments> {
    let (mut slangc, mut code, mut shaders) = (None, None, None);
    let sources = loop {
        let arg = args.next()?;
        let slot = match arg.to_str() {
            Some("--slangc") => &mut slangc,
            Some("--code") => &mut code,
            Some("--shaders") => &mut shaders,
            _ => break arg,
        };
        if slot.replace(PathBuf::from(args.next()?)).is_some() {
            return None;
        }
    };
    let (Some(pack), None) = (args.next(), args.next()) else { return None };
    Some(Arguments { slangc, code, shaders, sources: sources.into(), pack: pack.into() })
}

/// The cook, its futures driven on this thread by `Executor::block_on` (D130): one worker
/// of each class, since the cook spawns nothing, and the platform's pool.
fn run(arguments: &Arguments) -> Result<()> {
    if arguments.slangc.is_none() && arguments.code.is_some() {
        return Err(Error::new(ErrorKind::InvalidData, "--code is given without --slangc"));
    }
    if arguments.slangc.is_none() && arguments.shaders.is_some() {
        return Err(Error::new(ErrorKind::InvalidData, "--shaders is given without --slangc"));
    }
    let slang = arguments.slangc.as_deref().map(Slang::new).transpose()?;
    let shaders = slang.as_ref().map(|slang| Shaders {
        slang,
        code: arguments.code.as_deref(),
        cooked: arguments.shaders.as_deref(),
    });
    let executor = Executor::builder().frame_workers(1).background_workers(1).build()?;
    let platform = executor.platform().as_ref();
    let io = Io::start(platform, IoConfig::for_platform(platform))?;
    let cooked =
        executor.block_on(lactose_cook::cook(&io, &arguments.sources, &arguments.pack, shaders));
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
