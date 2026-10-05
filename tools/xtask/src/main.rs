//! `cargo xtask`: the command line over the checks in the library.
//!
//! Findings go to standard output, one `path: message` line each, and nothing else does, so
//! the output can be fed to a problem matcher. Progress and summaries go to standard error.
//! The exit status is 0 for no findings, 1 for findings and 2 when a check could not run.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use lactose_xtask::policy::Policy;
use lactose_xtask::repo::Files;
use lactose_xtask::sour::{self, Dirs, Selection, stand_in};
use lactose_xtask::tiers::TierFile;
use lactose_xtask::workspace::{Workspace, load_metadata};
use lactose_xtask::{Finding, POLICY_FILE, TIER_FILE, boundary, features, tiers};

const USAGE: &str = "\
cargo xtask <command>

  tiers                      check every crate dependency against crate-tiers.toml
  boundary                   check boundary lint files, unsafe policy, READMEs, the boundary
                             dependency allowlist and the x86-64-v3 pin
  features [--package <p>]…  require identical boundary feature trees across the shipped
                             binaries and the SOUR test crate (or across the named packages)
  sour matrix                print the SOUR mode matrix
  sour run <selection> [--emit <file>]
                             run simulation tests under SOUR and compare world hashes;
                             <selection> is --pr, --nightly or --mode <name>…;
                             --emit writes the baseline hashes for `sour compare`
  sour compare <ref> <file>  compare two emitted hash files (architecture mode)
  sour self-test             prove the SOUR runner on its stand-in simulation
  help                       print this text

Findings are printed as `path: message`. Exit status: 0 clean, 1 findings, 2 could not run.";

/// The package that holds the stand-in tests of `sour self-test`: this one.
const STAND_IN_PACKAGE: &str = env!("CARGO_PKG_NAME");
/// The integration test target that holds them.
const STAND_IN_TARGET: &str = "sour_stand_in";

/// What a command produced: findings, or the reason it could not run.
type Checked = Result<Vec<Finding>, String>;

/// Everything a command reads, gathered once.
struct Context {
    cargo: String,
    root: PathBuf,
    target_dir: PathBuf,
    workspace: Workspace,
    tiers: TierFile,
    policy: Policy,
}

impl Context {
    /// Loads the workspace that contains the working directory and makes its root the
    /// working directory, so every relative path below is relative to the repository.
    fn load() -> Result<Self, String> {
        // Cargo sets `CARGO` for the binaries it runs.
        let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let metadata = load_metadata(&cargo, Path::new("."))?;
        let root = PathBuf::from(&metadata.root);
        env::set_current_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        Ok(Self {
            cargo,
            target_dir: PathBuf::from(metadata.target_dir),
            workspace: metadata.workspace,
            tiers: TierFile::parse(&read(TIER_FILE)?)?,
            policy: Policy::parse(&read(POLICY_FILE)?)?,
            root,
        })
    }

    fn sour_dirs(&self) -> Dirs {
        Dirs { root: self.root.clone(), out: self.target_dir.join("sour") }
    }

    /// Runs `cargo nextest run` over `packages` with `vars` set. Its output goes to standard
    /// error, which keeps standard output for findings.
    fn nextest(
        &self,
        packages: &[String],
        extra: &[&str],
        vars: &[(&'static str, String)],
    ) -> Result<(), String> {
        let mut command = Command::new(&self.cargo);
        command.args(["nextest", "run", "--locked", "--no-tests=fail"]);
        for package in packages {
            command.args(["--package", package]);
        }
        let status = command
            .args(extra)
            .envs(vars.iter().map(|(name, value)| (name, value)))
            .stdout(Stdio::from(io::stderr()))
            .status()
            .map_err(|e| format!("cannot run `cargo nextest run`: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`cargo nextest run` ended with {status}"))
        }
    }
}

fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (name, checked) = match args[..] {
        [] | ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        ["tiers"] => ("tiers", check_tiers()),
        ["boundary"] => ("boundary", check_boundary()),
        ["features", ref rest @ ..] => ("features", check_features(rest)),
        ["sour", "matrix"] => {
            print_matrix();
            return ExitCode::SUCCESS;
        }
        ["sour", "run", ref rest @ ..] => ("sour", sour_run(rest)),
        ["sour", "compare", reference, file] => ("sour", sour_compare(reference, file)),
        ["sour", "self-test"] => ("sour", sour_self_test()),
        _ => ("xtask", Err(format!("unrecognised arguments `{}`\n\n{USAGE}", args.join(" ")))),
    };
    match checked {
        Ok(findings) if findings.is_empty() => {
            eprintln!("{name}: ok");
            ExitCode::SUCCESS
        }
        Ok(findings) => {
            for finding in &findings {
                println!("{finding}");
            }
            eprintln!("{name}: {} finding(s)", findings.len());
            ExitCode::from(1)
        }
        Err(message) => {
            eprintln!("{name}: {message}");
            ExitCode::from(2)
        }
    }
}

fn check_tiers() -> Checked {
    let context = Context::load()?;
    Ok(tiers::check(&context.tiers, &context.workspace))
}

fn check_boundary() -> Checked {
    let context = Context::load()?;
    let files = Files::load(&context.root)?;
    Ok(boundary::check(&context.tiers, &context.policy, &context.workspace, &files))
}

fn check_features(args: &[&str]) -> Checked {
    let context = Context::load()?;
    let mut roots: Vec<String> = Vec::new();
    let mut rest = args;
    while let [first, ..] = rest {
        let ["--package", package, tail @ ..] = rest else {
            return Err(format!("unexpected `{first}`; expected `--package <name>`"));
        };
        roots.push((*package).to_owned());
        rest = tail;
    }
    if roots.is_empty() {
        roots.clone_from(&context.policy.feature_roots);
    }
    if roots.is_empty() {
        eprintln!(
            "features: [feature_roots] in {POLICY_FILE} lists no packages yet (no shipped binary \
             or SOUR test crate exists), so there is nothing to compare"
        );
        return Ok(Vec::new());
    }
    let mut trees = Vec::new();
    for root in &roots {
        let package = context
            .workspace
            .package(root)
            .ok_or_else(|| format!("`{root}` is not a workspace package"))?;
        let dump = features::dump(&context.cargo, &context.root, root)?;
        let tree = features::boundary_tree(&dump, &|name| context.tiers.in_boundary(name))?;
        trees.push((package.manifest(), tree));
    }
    let packages = trees.first().map_or(0, |(_, tree)| tree.len());
    eprintln!(
        "features: compared {} roots; the first has {packages} package(s) in its boundary tree",
        roots.len()
    );
    Ok(features::compare(&trees))
}

fn print_matrix() {
    for mode in sour::matrix() {
        println!("{}: {}", mode.name, mode.varies);
        for variant in &mode.variants {
            let vars: Vec<String> = variant
                .config
                .to_vars()
                .iter()
                .skip(1)
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            let when = if variant.pull_request { "pull request" } else { "nightly" };
            println!("  {:<12} {:<13} {}", variant.label, when, vars.join(" "));
        }
    }
}

fn sour_run(args: &[&str]) -> Checked {
    let mut selection: Option<Selection> = None;
    let mut emit: Option<&str> = None;
    let mut rest = args;
    loop {
        rest = match (rest, &mut selection) {
            ([], _) => break,
            (["--pr", tail @ ..], None) => {
                selection = Some(Selection::PullRequest);
                tail
            }
            (["--nightly", tail @ ..], None) => {
                selection = Some(Selection::Nightly);
                tail
            }
            (["--mode", name, tail @ ..], None) => {
                selection = Some(Selection::Modes(vec![(*name).to_owned()]));
                tail
            }
            (["--mode", name, tail @ ..], Some(Selection::Modes(names))) => {
                names.push((*name).to_owned());
                tail
            }
            (["--emit", file, tail @ ..], _) if emit.is_none() => {
                emit = Some(file);
                tail
            }
            ([other, ..], _) => {
                return Err(format!(
                    "unexpected `{other}`; give one of --pr, --nightly or --mode <name>…, and \
                     at most one --emit <file>"
                ));
            }
        };
    }
    let selection = selection.ok_or("give one of --pr, --nightly or --mode <name>…")?;
    // Resolved now: loading the context moves the working directory to the repository root.
    let emit = match emit {
        Some(file) => Some(std::path::absolute(file).map_err(|e| format!("{file}: {e}"))?),
        None => None,
    };
    let matrix = sour::matrix();
    let plans = sour::plan(&matrix, &selection)?;

    let context = Context::load()?;
    let packages = &context.policy.sour_packages;
    let outcome = if packages.is_empty() {
        eprintln!(
            "sour: [sour] in {POLICY_FILE} lists no packages yet (no simulation exists), so \
             there is nothing to run; `cargo xtask sour self-test` proves the runner"
        );
        sour::Outcome::default()
    } else {
        let mut launch = |vars: &[(&'static str, String)]| context.nextest(packages, &[], vars);
        let outcome = sour::run(&plans, &context.sour_dirs(), &mut launch);
        eprintln!("sour: {} mode(s), {} test run(s)", plans.len(), outcome.launches);
        outcome
    };
    if let Some(file) = emit {
        let hashes = outcome.baseline.clone().unwrap_or_default();
        fs::write(&file, sour::emit(&hashes)).map_err(|e| format!("{}: {e}", file.display()))?;
        eprintln!("sour: wrote {} baseline hash(es) to {}", hashes.len(), file.display());
    }
    Ok(outcome.findings)
}

fn sour_compare(reference: &str, file: &str) -> Checked {
    let (findings, records) =
        sour::compare_files(reference, &read(reference)?, file, &read(file)?)?;
    eprintln!("sour: compared {file} against {records} record(s) of {reference}");
    Ok(findings)
}

fn sour_self_test() -> Checked {
    let context = Context::load()?;
    let packages = [STAND_IN_PACKAGE.to_owned()];
    let mut launch = |test: &str, vars: &[(&'static str, String)]| {
        debug_assert!(test == stand_in::SOUND_TEST || test == stand_in::FLAWED_TEST);
        let filter = format!("test(={test})");
        context.nextest(&packages, &["--test", STAND_IN_TARGET, "-E", &filter], vars)
    };
    let findings = sour::self_test(&context.sour_dirs(), &mut launch);
    if findings.is_empty() {
        eprintln!(
            "sour: the runner passed the sound stand-in in the pull-request modes and reported \
             the order-dependent one at tick {}",
            stand_in::FLAW_STARTS_AT
        );
    }
    Ok(findings)
}
