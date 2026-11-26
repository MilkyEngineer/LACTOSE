//! The client binary (architecture section 2, "app-client").
//!
//! Usage: `lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] [--seconds N]
//! [--time-scale THOUSANDTHS] [--minimised] [--stamps FILE]` runs paced until the window is
//! closed or `N` seconds have passed, at the time scale given, the window minimised, and
//! `frame-pace`'s stamps written to `FILE`;
//! `lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] --fixed FRAMES DUMP` runs
//! decision 5.1's fixed-frame mode for `FRAMES` steps, writes the last frame's dump into the
//! directory `DUMP`, and holds the last frame to the CPU reference. `DIR` for `--shaders` is
//! the cooked shaders' directory, which `cargo xtask gpu` writes under `target/gpu/shaders`.
//! The cache file is `pipeline-cache.bin` in the working directory unless named. In either
//! mode `--extra-present-passes N`, a test and measurement knob, makes each present draw its
//! image `N` more times (`Options::extra_present_passes`). A flag given twice, a paced
//! mode's flag with `--fixed`, or a time scale of zero is refused with the usage, as is an
//! unknown flag.
//!
//! Prints the outcome and exits 0; exits 1 with the error, or 1 where a validator sent any
//! message. A panic aborts through `lactose-app`'s crash hook, which writes its report to
//! stderr and to the directory `LACTOSE_CRASH_DIR` names, `crash` in the working directory
//! where it is unset or empty.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use lactose_app::install_crash_hook;
use lactose_app_client::{Mode, Options, install_log, run};
use lactose_sim::TimeScale;

fn usage() -> ExitCode {
    eprintln!(
        "usage: lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] \
         [--extra-present-passes N] [[--seconds N] [--time-scale THOUSANDTHS] [--minimised] \
         [--stamps FILE] | --fixed FRAMES DUMP]"
    );
    ExitCode::from(2)
}

/// The options, or `None` for an unknown flag, a flag without its values, a flag given
/// twice, a paced mode's flag with `--fixed`, or a time scale of zero.
fn parse(mut args: impl Iterator<Item = String>) -> Option<Options> {
    let mut shaders = None;
    let mut packs = None;
    let mut cache = None;
    let mut seconds = None;
    let mut scale = None;
    let mut minimised = None;
    let mut stamps = None;
    let mut fixed = None;
    let mut extra = None;
    // Each flag is set once: a second sets nothing and fails the parse.
    fn once<T>(slot: &mut Option<T>, value: T) -> Option<()> {
        slot.replace(value).is_none().then_some(())
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--shaders" => once(&mut shaders, PathBuf::from(args.next()?))?,
            "--packs" => once(&mut packs, PathBuf::from(args.next()?))?,
            "--cache" => once(&mut cache, PathBuf::from(args.next()?))?,
            "--seconds" => once(&mut seconds, args.next()?.parse().ok()?)?,
            "--time-scale" => {
                let thousandths = args.next()?.parse().ok().filter(|&n: &u32| n > 0)?;
                once(&mut scale, TimeScale { thousandths })?;
            }
            "--minimised" => once(&mut minimised, true)?,
            "--stamps" => once(&mut stamps, PathBuf::from(args.next()?))?,
            "--extra-present-passes" => once(&mut extra, args.next()?.parse().ok()?)?,
            "--fixed" => {
                let frames = args.next()?.parse().ok()?;
                once(&mut fixed, Mode::Fixed { frames, dump: PathBuf::from(args.next()?) })?;
            }
            _ => return None,
        }
    }
    let paced = seconds.is_some() || scale.is_some() || minimised.is_some() || stamps.is_some();
    let mode = match fixed {
        Some(_) if paced => return None,
        Some(fixed) => fixed,
        None => Mode::Paced { seconds, scale, minimised: minimised.is_some(), stamps },
    };
    Some(Options {
        shaders: shaders?,
        packs,
        cache: cache.unwrap_or_else(|| PathBuf::from("pipeline-cache.bin")),
        mode,
        extra_present_passes: extra.unwrap_or(0),
    })
}

fn crash_dir() -> PathBuf {
    std::env::var_os("LACTOSE_CRASH_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| PathBuf::from("crash"), PathBuf::from)
}

fn main() -> ExitCode {
    install_crash_hook("lactose-app-client", Some(crash_dir()));
    let Some(options) = parse(std::env::args().skip(1)) else {
        let _ = install_log(false);
        return usage();
    };
    let stamps = matches!(options.mode, Mode::Paced { stamps: Some(_), .. });
    if !install_log(stamps) && stamps {
        eprintln!("lactose-app-client: a log was installed before `install_log`: no recorders");
        return ExitCode::FAILURE;
    }
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

#[cfg(test)]
mod tests {
    use lactose_sim::TimeScale;

    use super::{Mode, Options, parse};

    fn parsed(args: &[&str]) -> Option<Options> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn a_flag_given_twice_or_both_modes_are_refused() {
        let paced = parsed(&["--shaders", "s", "--seconds", "3"]).expect("paced");
        assert!(matches!(paced.mode, Mode::Paced { seconds: Some(3), .. }));
        let fixed = parsed(&["--fixed", "600", "d", "--shaders", "s"]).expect("fixed");
        assert!(matches!(fixed.mode, Mode::Fixed { frames: 600, .. }));
        let plain = parsed(&["--shaders", "s"]).expect("the defaults");
        assert!(matches!(
            plain.mode,
            Mode::Paced { seconds: None, scale: None, minimised: false, stamps: None }
        ));
        assert!(parsed(&["--shaders", "s", "--shaders", "t"]).is_none());
        assert!(parsed(&["--shaders", "s", "--cache", "a", "--cache", "b"]).is_none());
        assert!(parsed(&["--shaders", "s", "--packs", "a", "--packs", "b"]).is_none());
        assert!(parsed(&["--shaders", "s", "--seconds", "3", "--seconds", "4"]).is_none());
        assert!(parsed(&["--shaders", "s", "--seconds", "3", "--fixed", "6", "d"]).is_none());
        assert!(parsed(&["--seconds", "3"]).is_none(), "no shaders");
        assert_eq!(plain.extra_present_passes, 0, "none unless asked");
        let extra = parsed(&["--shaders", "s", "--extra-present-passes", "40"]).expect("extra");
        assert_eq!(extra.extra_present_passes, 40);
        let fixed = parsed(&["--fixed", "6", "d", "--shaders", "s", "--extra-present-passes", "2"]);
        assert_eq!(fixed.map(|fixed| fixed.extra_present_passes), Some(2), "beside --fixed");
        let twice =
            ["--shaders", "s", "--extra-present-passes", "1", "--extra-present-passes", "2"];
        assert!(parsed(&twice).is_none());
        assert!(parsed(&["--shaders", "s", "--extra-present-passes", "many"]).is_none());
    }

    #[test]
    fn the_paced_flags_are_taken_and_refused_beside_the_fixed_mode() {
        let args = [
            "--shaders",
            "s",
            "--time-scale",
            "499",
            "--minimised",
            "--stamps",
            "f",
            "--seconds",
            "60",
        ];
        let Some(Options { mode: Mode::Paced { seconds, scale, minimised, stamps }, .. }) =
            parsed(&args)
        else {
            panic!("the paced flags were refused");
        };
        assert_eq!(seconds, Some(60));
        assert_eq!(scale, Some(TimeScale { thousandths: 499 }));
        assert!(minimised);
        assert_eq!(stamps.as_deref(), Some(std::path::Path::new("f")));
        for flag in [&["--time-scale", "499"][..], &["--minimised"], &["--stamps", "f"]] {
            let mut args = vec!["--shaders", "s", "--fixed", "6", "d"];
            args.extend(flag);
            assert!(parsed(&args).is_none(), "{flag:?} beside --fixed");
        }
        assert!(parsed(&["--shaders", "s", "--time-scale", "0"]).is_none(), "a scale of zero");
        assert!(parsed(&["--shaders", "s", "--minimised", "--minimised"]).is_none());
    }
}
