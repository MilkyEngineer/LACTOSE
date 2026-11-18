//! `cargo xtask`: the command line over the checks in the library.
//!
//! Findings go to standard output, one `path: message` line each, and nothing else does, so
//! the output can be fed to a problem matcher. Progress and summaries go to standard error.
//! The exit status is 0 for no findings, 1 for findings and 2 when a check could not run.
//! `heavy` is not a check: it prints only progress to standard error, and exits with the
//! status of the program it runs, or 2 when it could not run it. `group` is not a check
//! either: it prints one line of cargo arguments to standard output, and exits with 2,
//! printing nothing there, when the groups do not divide the workspace.
//!
//! Every cargo command this binary runs is `--locked`, as is the `cargo xtask` alias itself:
//! a check never rewrites `Cargo.lock`, and fails if it is out of date. The one exception is
//! the online step of `vendor`, which resolves the lock of a manifest it writes under the
//! target directory, never the workspace's.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitCode, Stdio};
use std::time::Instant;

use lactose_xtask::harness::{self, Dirs, stand_in};
use lactose_xtask::heavy;
use lactose_xtask::measure;
use lactose_xtask::policy::Policy;
use lactose_xtask::repo::Files;
use lactose_xtask::slang;
use lactose_xtask::tiers::TierFile;
use lactose_xtask::vendor::{self, Repository};
use lactose_xtask::workspace::{Workspace, cargo_metadata, load_metadata};
use lactose_xtask::{
    Finding, POLICY_FILE, TIER_FILE, boundary, check_groups, features, miri, test_targets, tiers,
};

const USAGE: &str = "\
cargo xtask <command>

  tiers                      check every crate dependency against crate-tiers.toml
  boundary                   check the boundary lint files and everything that could switch
                             them off, the unsafe policy, READMEs, the boundary dependency
                             allowlist, the x86-64-v3 pin, that the nightly Miri
                             entries of each split package run its tests once each, that
                             each package builds its integration tests as one binary and
                             is in one check group, and that ci.yml's check job runs
                             every group
  group <name>               print the cargo arguments (`-p <package>` each) that select the
                             packages of a CI check job's group, once the groups divide the
                             workspace
  features [--package <p>]…  require agreeing boundary feature trees, per CI platform, across
                             the shipped binaries and the harness packages (or across the
                             named packages): identical across shipped binaries, the same
                             features for every package a harness package shares with them
  vendor [--offline]         prove each crate built from vendor/ is its patch applied to its
                             record, the patch within the rule of section 8.2, the record
                             the crates.io archive's, and the upstream versions free of
                             advisories
  sour matrix                print the SOUR mode matrix
  sour run <selection> [--emit <file>] [--release]
                             run simulation tests under SOUR and compare world hashes;
                             <selection> is --pr, --nightly or --mode <name>…;
                             --emit writes the baseline hashes for `sour compare`;
                             --release builds the tests with the release profile and
                             writes to target/sour-release
  sour compare <ref> <file>  compare two emitted hash files (architecture mode)
  sour self-test             prove the SOUR runner on its stand-in simulations
  heavy [--slots <n>] -- <program> [<arg>…]
                             run the program once it holds one of <n> machine-wide slots
                             (default 2, the same for every caller on the machine), and
                             exit with its status
  measure (--all | <name>…) [--short] [--out <dir>]
                             run the milestone measurements and write each one's result
                             files to docs/results (or <dir>); --short cuts repetitions
                             and marks the files as short; the names are in
                             docs/results/README.md
  slang                      install the pinned Slang release under the target directory,
                             from tools/cook/slang.toml, and print its slangc's path
  gpu --driver <name> [--cooked <dir>]
                             run the device tests under a named driver (lavapipe, moltenvk,
                             kosmickrisp or system); its one step yet is the Metal step, on
                             macOS: cook the engine shaders, or with --cooked take the code
                             `ricotta --code` wrote to <dir>, and make a Metal pipeline state
                             from each under Metal's validation
  help                       print this text

Findings are printed as `path: message`. Exit status: 0 clean, 1 findings, 2 could not run.";

/// The package that holds the stand-in tests of `sour self-test`: this one.
const STAND_IN_PACKAGE: &str = env!("CARGO_PKG_NAME");
/// The integration test target that holds them.
const STAND_IN_TARGET: &str = "harness_stand_in";
/// The target directory of the self-test's nested build, inside the workspace's own. The
/// nested build compiles this package again, and must not write over the binary that is
/// running it (Windows refuses, and elsewhere it races).
const SELF_TEST_TARGET_DIR: &str = "xtask-self-test";

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

    fn harness_dirs(&self) -> Dirs {
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

    /// The finding for a simulation that no harness package covers, and whether the harness
    /// has nothing to run. With nothing to run and nothing to report, says so.
    fn harness_coverage(&self) -> (Vec<Finding>, bool) {
        let packages = &self.policy.harness_packages;
        let findings = harness::unlisted_simulation(&self.workspace, packages);
        if packages.is_empty() && findings.is_empty() {
            eprintln!(
                "sour: no harness packages are listed under [harness] in {POLICY_FILE} (no \
                 simulation exists yet), so there is nothing to run or compare; \
                 `cargo xtask sour self-test` proves the runner"
            );
        }
        (findings, packages.is_empty())
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
        ["vendor", ref rest @ ..] => ("vendor", check_vendor(rest)),
        ["sour", "matrix"] => {
            print_matrix();
            return ExitCode::SUCCESS;
        }
        ["sour", "run", ref rest @ ..] => ("sour", harness_run(rest)),
        ["sour", "compare", reference, file] => ("sour", harness_compare(reference, file)),
        ["sour", "self-test"] => ("sour", harness_self_test()),
        ["heavy", ref rest @ ..] => match heavy(rest) {
            // The program's own status, which on Windows may not fit an `ExitCode`.
            Ok(code) => process::exit(code),
            Err(message) => ("heavy", Err(message)),
        },
        ["measure", ref rest @ ..] => ("measure", run_measurements(rest)),
        ["slang"] => match install_slang() {
            Ok(slangc) => {
                println!("{}", slangc.display());
                return ExitCode::SUCCESS;
            }
            Err(message) => ("slang", Err(message)),
        },
        ["gpu", ref rest @ ..] => ("gpu", run_gpu(rest)),
        ["group", name] => match group_arguments(name) {
            Ok(arguments) => {
                println!("{arguments}");
                return ExitCode::SUCCESS;
            }
            Err(message) => ("group", Err(message)),
        },
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

/// Runs the measurements; writes files, never findings. Not a check: a failed measurement
/// is reported once the others have run.
fn run_measurements(args: &[&str]) -> Checked {
    let options = measure::parse(args)?;
    let context = Context::load()?;
    let workspace = measure::Workspace {
        cargo: context.cargo,
        root: context.root,
        target_dir: context.target_dir,
    };
    let written = measure::run(&workspace, &options)?;
    eprintln!("measure: {} file(s) written", written.len());
    Ok(Vec::new())
}

fn check_tiers() -> Checked {
    let context = Context::load()?;
    Ok(tiers::check(&context.tiers, &context.workspace))
}

fn check_boundary() -> Checked {
    let context = Context::load()?;
    let files = Files::load(&context.root)?;
    let mut findings = boundary::check(&context.tiers, &context.policy, &context.workspace, &files);
    findings.extend(miri::check(&context.policy, &context.workspace, &files));
    findings.extend(test_targets::check(&context.policy, &context.workspace, &files));
    findings.extend(check_groups::check(&context.policy, &context.workspace));
    findings.extend(check_groups::check_workflow(&context.policy, &files));
    Ok(findings)
}

/// Installs the pinned Slang release under the target directory and returns its `slangc`.
fn install_slang() -> Result<PathBuf, String> {
    let context = Context::load()?;
    let pin = slang::Pin::parse(&read(slang::PIN_FILE)?)?;
    slang::install(&pin, &context.target_dir)
}

/// The drivers `gpu` runs the device tests under (milestone 2 plan, decision 5.10).
const DRIVERS: [&str; 4] = ["lavapipe", "moltenvk", "kosmickrisp", "system"];
/// The engine's Slang sources, which `gpu` cooks.
const SHADERS: &str = "shaders";
/// The Swift script of the Metal step.
const METAL_SCRIPT: &str = "tools/xtask/metal/pipelines.swift";

/// `gpu --driver <name> [--cooked <dir>]`: its one step is the Metal step, which cooks the
/// engine's shaders with the pinned Slang, or takes the code another runner cooked to
/// `<dir>`, and makes a Metal pipeline state from each, through the Swift script run by the
/// command line tools' `swift`. The canaries and the count of messages are the same either
/// way. The driver selects nothing yet, since no step runs Vulkan; it is checked and
/// recorded. The script's report is written to `target/gpu/<driver>-metal.txt` and to
/// standard error.
fn run_gpu(args: &[&str]) -> Checked {
    let (driver, cooked) = match args {
        ["--driver", name] if DRIVERS.contains(name) => (*name, None),
        ["--driver", name, "--cooked", dir] if DRIVERS.contains(name) => (*name, Some(*dir)),
        _ => return Err(format!("expected `--driver <{}> [--cooked <dir>]`", DRIVERS.join("|"))),
    };
    if !cfg!(target_os = "macos") {
        return Err("the one step of `gpu` yet is the Metal step, which runs on macOS".to_owned());
    }
    let context = Context::load()?;
    let pin = slang::Pin::parse(&read(slang::PIN_FILE)?)?;
    let out = context.target_dir.join("gpu");
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (code, cooked_by) = match cooked {
        Some(dir) => (PathBuf::from(dir), format!("cooked elsewhere, in {dir}")),
        None => (cook_engine_shaders(&context, &pin, &out)?, format!("Slang {}", pin.version)),
    };
    eprintln!("gpu: the Metal step, under driver {driver}, with Metal's validation on");
    // The canaries first, each in a process of its own: a layer that did not load reports
    // nothing. API validation's aborts the process; shader validation's lets it complete.
    for (canary, says, aborts) in CANARIES {
        let run = metal_step(&["--canary", canary])?;
        if run.status.success() == aborts || !run.stderr.contains(says) {
            return Err(format!(
                "Metal {canary} validation's canary did not yield `{says}`, so the layer is \
                 not listening: it ended with {}:\n{}",
                run.status, run.stderr
            ));
        }
    }
    let step = metal_step(&[&code.to_string_lossy()])?;
    eprint!("{}", step.stdout);
    // Every line on standard error counts (decision 5.10), but the layers' own two.
    let counted: Vec<&str> = step
        .stderr
        .lines()
        .filter(|line| !VALIDATION_ON.iter().any(|on| line.ends_with(on)))
        .collect();
    let file = out.join(format!("{driver}-metal.txt"));
    let written = format!(
        "driver: {driver}\ncode: {cooked_by}\nvalidation: api and shader, canaries {}\n{}messages: \
         {}\n{}",
        CANARIES.map(|(canary, says, _)| format!("{canary} `{says}`")).join(", "),
        step.stdout,
        counted.len(),
        counted.iter().map(|line| format!("{line}\n")).collect::<String>()
    );
    fs::write(&file, written).map_err(|e| format!("{}: {e}", file.display()))?;
    if !step.status.success() || !counted.is_empty() {
        return Err(format!(
            "the Metal step ended with {} and {} message(s) on standard error:\n{}",
            step.status,
            counted.len(),
            counted.join("\n")
        ));
    }
    eprintln!("gpu: the report is {}", file.display());
    Ok(Vec::new())
}

/// Cooks the engine's shaders with the pinned Slang into `<out>/code`, emptied first, and
/// returns that directory.
fn cook_engine_shaders(context: &Context, pin: &slang::Pin, out: &Path) -> Result<PathBuf, String> {
    let slangc = slang::install(pin, &context.target_dir)?;
    let code = out.join("code");
    // A shader cooked by an earlier run and since removed would otherwise be loaded again.
    match fs::remove_dir_all(&code) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(format!("{}: {error}", code.display()));
        }
        _ => {}
    }
    eprintln!("gpu: cooking {SHADERS} with Slang {}", pin.version);
    let status = Command::new(&context.cargo)
        .args(["run", "--locked", "--quiet", "--package", "lactose-cook", "--bin", "ricotta", "--"])
        .arg("--slangc")
        .arg(&slangc)
        .arg("--code")
        .arg(&code)
        .arg(SHADERS)
        .arg(out.join("shaders.pack"))
        .stdout(Stdio::from(io::stderr()))
        .status()
        .map_err(|e| format!("cannot run ricotta: {e}"))?;
    if !status.success() {
        return Err(format!("ricotta ended with {status}"));
    }
    Ok(code)
}

/// Each validator's canary in the Metal script, what the validator says of it, and whether
/// that aborts the process.
const CANARIES: [(&str, &str, bool); 2] = [
    ("api", "missing Buffer binding at index 0", true),
    ("shader", "Invalid store in 'device' or 'constant' address space", false),
];
/// The lines the two layers write when they load.
const VALIDATION_ON: [&str; 2] = ["Metal API Validation Enabled", "Metal GPU Validation Enabled"];

/// What one run of the Metal script printed.
struct StepOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Runs the Metal script under Metal API and shader validation, set as the Metal spike set
/// them: API validation's assertions abort, its warnings and shader validation's reports go to
/// standard error.
fn metal_step(args: &[&str]) -> Result<StepOutput, String> {
    let output = Command::new("swift")
        .arg(METAL_SCRIPT)
        .args(args)
        .env("MTL_DEBUG_LAYER", "1")
        .env("MTL_DEBUG_LAYER_WARNING_MODE", "nslog")
        .env("MTL_SHADER_VALIDATION", "1")
        .env("MTL_SHADER_VALIDATION_REPORT_TO_STDERR", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run swift: {e}"))?;
    Ok(StepOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// The cargo arguments that select a check group's packages, for CI's check jobs.
fn group_arguments(name: &str) -> Result<String, String> {
    let context = Context::load()?;
    check_groups::arguments(&context.policy, &context.workspace, name)
}

fn check_vendor(args: &[&str]) -> Checked {
    let offline = match args {
        [] => false,
        ["--offline"] => true,
        _ => {
            return Err(format!(
                "unexpected `{}`; expected `--offline` or nothing",
                args.join(" ")
            ));
        }
    };
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    // The members without the graph: cargo refuses to resolve a `[patch]` path that does not
    // exist, which is a finding of step 2 rather than a reason not to run.
    let layout = cargo_metadata(&cargo, Path::new("."), &["--no-deps"])?;
    let root = PathBuf::from(&layout.root);
    env::set_current_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let scratch = PathBuf::from(&layout.target_dir).join("vendor");
    let members = layout.workspace.packages.iter();
    let members = members.map(|p| p.manifest().trim_start_matches('/').to_owned()).collect();
    let repo = Repository::load(&root, &scratch, members)?;
    // Every feature: a second version of a patched crate behind a member's feature is still
    // a version some build compiles.
    let extra: &[&str] =
        if offline { &["--all-features", "--offline"] } else { &["--all-features"] };
    let graph = cargo_metadata(&cargo, &root, extra);
    if repo.copies.is_empty() {
        eprintln!("vendor: nothing is vendored");
    }
    let mut findings = vendor::check(&repo, graph.as_ref().ok().map(|m| &m.workspace));
    for dir in &repo.copies {
        findings.extend(vendor::reverse(&root, &scratch, &repo, dir)?);
    }
    findings.extend(vendor::prose(&repo));
    if let Err(error) = graph {
        if findings.is_empty() {
            return Err(error);
        }
        eprintln!(
            "vendor: the resolved graph is unavailable, so the parts of steps 2 and 4 that read \
             it did not run: {error}"
        );
        return Ok(findings);
    }
    if repo.copies.is_empty() {
        return Ok(findings);
    }
    if !findings.is_empty() {
        eprintln!("vendor: the online step did not run, since the offline steps found something");
        return Ok(findings);
    }
    let online = vendor::online(&cargo, &root, &scratch, &repo, offline)?;
    eprintln!(
        "vendor: {} record(s) re-derived from the crates.io archives{}",
        online.records,
        if offline { " in the cache" } else { "" }
    );
    match &online.advisories_skipped {
        Some(why) => eprintln!(
            "vendor: the advisory check of the upstream versions was skipped (--offline): {why}"
        ),
        None => eprintln!("vendor: the advisory check ran over the upstream versions"),
    }
    Ok(online.findings)
}

fn check_features(args: &[&str]) -> Checked {
    let context = Context::load()?;
    let mut named: Vec<String> = Vec::new();
    let mut rest = args;
    while let [first, ..] = rest {
        let ["--package", package, tail @ ..] = rest else {
            return Err(format!("unexpected `{first}`; expected `--package <name>`"));
        };
        named.push((*package).to_owned());
        rest = tail;
    }
    let policy = &context.policy;
    let mut findings = features::loom_users(&context.workspace);
    let roots = if named.is_empty() {
        // The shipped binaries and the harness packages, each once.
        findings.extend(features::unlisted_binaries(&context.workspace, &policy.feature_roots));
        let mut roots = policy.feature_roots.clone();
        let fresh = |p: &&String| !policy.feature_roots.contains(p);
        roots.extend(policy.harness_packages.iter().filter(fresh).cloned());
        roots
    } else {
        named
    };
    if roots.is_empty() {
        if findings.is_empty() {
            eprintln!(
                "features: no packages are listed under [feature_roots] or [harness] in \
                 {POLICY_FILE} (no shipped binary or harness package exists yet), so there is \
                 nothing to compare"
            );
        }
        return Ok(findings);
    }
    // Every boundary package any root compiles, for the check `fold_defaults` relies on.
    let mut compiled = std::collections::BTreeSet::new();
    for triple in features::TRIPLES {
        let mut trees = Vec::new();
        for root in &roots {
            let package = context
                .workspace
                .package(root)
                .ok_or_else(|| format!("`{root}` is not a workspace package"))?;
            // A harness package is built as tests, which compiles its dev-dependencies.
            let dev = policy.harness_packages.contains(root);
            let dump = features::dump(&context.cargo, &context.root, root, triple, dev)?;
            let tree = features::boundary_tree(&dump, &|name| context.tiers.in_boundary(name))?;
            compiled.extend(tree.keys().map(|(package, _)| package.clone()));
            let tree = features::fold_defaults(tree, &context.workspace.defaults);
            // A shipped binary is compared in full; a harness package that is not one, on
            // the packages it shares with each other root.
            let full = !dev || policy.feature_roots.contains(root);
            trees.push(features::Root { path: package.manifest(), tree, full });
        }
        let packages = trees.first().map_or(0, |root| root.tree.len());
        eprintln!(
            "features: {triple}: compared {} roots; the first has {packages} package(s) in its \
             boundary tree",
            roots.len()
        );
        findings.extend(features::compare(triple, &trees));
        findings.extend(trees.iter().flat_map(|root| features::missing_required(triple, root)));
    }
    findings.extend(features::default_readers(&context.root, &compiled, &context.workspace.dirs)?);
    Ok(findings)
}

fn print_matrix() {
    for mode in harness::matrix() {
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

fn harness_run(args: &[&str]) -> Checked {
    let harness::RunArgs { selection, emit, profile } = harness::run_args(args)?;
    // Resolved now: loading the context moves the working directory to the repository root.
    let emit = match emit {
        Some(file) => Some(std::path::absolute(file).map_err(|e| format!("{file}: {e}"))?),
        None => None,
    };
    let matrix = harness::matrix();
    let plans = harness::plan(&matrix, &selection)?;

    let context = Context::load()?;
    let (mut findings, nothing_to_run) = context.harness_coverage();
    let mut baseline = harness::Hashes::new();
    if !nothing_to_run {
        let packages = &context.policy.harness_packages;
        let mut launch = |vars: &[(&'static str, String)]| {
            context.nextest(packages, profile.nextest_args(), vars)
        };
        let dirs =
            Dirs { out: context.target_dir.join(profile.out_dir()), ..context.harness_dirs() };
        let outcome = harness::run(&plans, &dirs, &mut launch);
        eprintln!("sour: {} mode(s), {} test run(s)", plans.len(), outcome.launches);
        findings.extend(outcome.findings);
        baseline = outcome.baseline.unwrap_or_default();
        findings.extend(harness::recording_tests(&baseline, &context.policy.harness_tests));
    }
    if let Some(file) = emit {
        fs::write(&file, harness::emit(&baseline))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        eprintln!("sour: wrote {} baseline hash(es) to {}", baseline.len(), file.display());
    }
    Ok(findings)
}

fn harness_compare(reference: &str, file: &str) -> Checked {
    // Read before the context moves the working directory. A platform whose file is
    // missing fails here, whatever the files would have held.
    let (reference_text, text) = (read(reference)?, read(file)?);
    let context = Context::load()?;
    let (mut findings, nothing_to_run) = context.harness_coverage();
    if nothing_to_run {
        // The files are still required to be hash files.
        harness::compare_files(reference, &reference_text, file, &text)?;
        return Ok(findings);
    }
    findings.extend(harness::compare_files(reference, &reference_text, file, &text)?);
    eprintln!("sour: compared {file} against {reference}");
    Ok(findings)
}

fn harness_self_test() -> Checked {
    let context = Context::load()?;
    let packages = [STAND_IN_PACKAGE.to_owned()];
    let target_dir = context.target_dir.join(SELF_TEST_TARGET_DIR);
    let target_dir = target_dir.to_string_lossy();
    let mut launches = 0;
    let mut launch = |test: &str, vars: &[(&'static str, String)]| {
        launches += 1;
        let filter = format!("test(={test})");
        let extra = ["--target-dir", &*target_dir, "--test", STAND_IN_TARGET, "-E", &filter];
        context.nextest(&packages, &extra, vars)
    };
    let findings = harness::self_test(&context.harness_dirs(), &mut launch);
    if findings.is_empty() {
        eprintln!(
            "sour: over {launches} nextest runs the runner passed the sound stand-in in every \
             mode and reported each of the {} flawed ones in the mode that exists to catch it",
            stand_in::STAND_INS.len() - 1
        );
    }
    Ok(findings)
}

/// Runs `cargo xtask heavy`: the program once it holds a slot, with this process's standard
/// streams and working directory. The slot is released when the program ends or when this
/// process dies, whichever is first; the program is not killed with it.
fn heavy(args: &[&str]) -> Result<i32, String> {
    let (slots, command) = match args {
        ["--slots", count, "--", command @ ..] => {
            let count = count.parse::<NonZeroUsize>().map_err(|e| {
                format!("`--slots {count}`: {e}; expected a whole number of at least 1")
            })?;
            (count, command)
        }
        ["--", command @ ..] => (heavy::DEFAULT_SLOTS, command),
        _ => {
            return Err(format!(
                "unexpected `{}`; expected `[--slots <n>] -- <program> [<arg>…]`",
                args.join(" ")
            ));
        }
    };
    let [program, args @ ..] = command else {
        return Err("give the program to run after `--`".to_owned());
    };
    let mut command = Command::new(program);
    command.args(args);
    let run = |command: &mut Command| {
        let status = command.status().map_err(|e| format!("cannot run `{program}`: {e}"))?;
        Ok(heavy::exit_code(status))
    };
    let dir = heavy::dir();
    if let Some(held) = env::var_os(heavy::HELD_VAR) {
        let inside = heavy::is_held(&dir, &held);
        let held = held.to_string_lossy();
        if inside {
            eprintln!("heavy: inside slot {held} already");
            return run(&mut command);
        }
        eprintln!("heavy: {}={held} names no slot held for it", heavy::HELD_VAR);
    }
    let start = Instant::now();
    let slot = heavy::acquire(&dir, slots, heavy::RETRY, || {
        let pids: Vec<String> =
            heavy::holders(&dir, slots).iter().map(ToString::to_string).collect();
        let by = if pids.is_empty() {
            String::new()
        } else {
            format!(", held by pid {}", pids.join(", "))
        };
        eprintln!("heavy: waiting for one of {slots} slots{by}");
    })?;
    eprintln!("heavy: slot {} after {}s", slot.index(), start.elapsed().as_secs());
    let code = run(command.env(heavy::HELD_VAR, slot.held_value()));
    drop(slot);
    code
}
