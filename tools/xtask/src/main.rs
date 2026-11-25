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
use lactose_xtask::vulkan;
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
  measure (--all | <name>…) [--short] [--no-device] [--driver <name>] [--adapter <name>]
          [--out <dir>]
                             run the milestone measurements and write each one's result
                             files to docs/results (or <dir>); --short cuts repetitions
                             and marks the files as short; --no-device leaves out those
                             on a Vulkan device, which run with no layer under --driver
                             (moltenvk on macOS, system elsewhere) on --adapter (the
                             loader's first); the names are in docs/results/README.md
  slang                      install the pinned Slang release under the target directory,
                             from tools/cook/slang.toml, and print its slangc's path
  gpu [--driver <name>] [--adapter <name>] [--only <metal|vulkan>] [--gpu-assisted]
      [--cooked <dir>]       cook the engine shaders, or with --cooked take the code
                             `ricotta --code` wrote to <dir> (Metal step alone); on macOS
                             make a Metal pipeline state from each under Metal's
                             validation; then run every device test under the pinned
                             Vulkan validation layer and the named driver (lavapipe,
                             moltenvk, kosmickrisp or system, whose device --adapter
                             names), with GPU-assisted validation for --gpu-assisted;
                             with no --driver, on macOS, under moltenvk then kosmickrisp,
                             or kosmickrisp alone with --gpu-assisted
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

/// The engine's Slang sources, which `gpu` cooks.
const SHADERS: &str = "shaders";
/// The Swift script of the Metal step.
const METAL_SCRIPT: &str = "tools/xtask/metal/pipelines.swift";
/// The device tests the Vulkan step runs: each package, its test target (`None` for its
/// library's unit tests) and the prefix of the names taken, whose ignored tests are the ones
/// that need a device. The library's are its `canary` module's, the thread-safety canary,
/// whose unlocked driver call no safe call can make, and its `swapchain` module's, which
/// inject what no safe call makes (an acquire semaphore's reuse, a present fence that never
/// signals, a failed `resize`, a present the driver refuses) or watch what none reports (a
/// present's walk left undecided, a driver swapchain kept for a submit an encoder dropped
/// while unwinding left pending). The render graph's and the renderer's are their `device`
/// modules'. The client's are the measurements on a device that `cargo xtask measure` runs
/// with no layer, here run short under the validators, their figures discarded.
const DEVICE_TESTS: [(&str, Option<&str>, &str); 7] = [
    ("lactose-rhi-vulkan", None, "canary::"),
    ("lactose-rhi-vulkan", None, "swapchain::"),
    ("lactose-rhi-vulkan", Some("integration"), ""),
    ("lactose-render-graph", Some("integration"), "device::"),
    ("lactose-renderer", Some("integration"), "device::"),
    ("lactose-app-client", Some("integration"), "extract::"),
    ("lactose-app-client", Some("integration"), "upload::"),
];

/// What `gpu` was asked to run.
struct GpuArgs<'a> {
    /// The drivers of the Vulkan step, one after the other; the Metal step's report is named
    /// for the first.
    drivers: Vec<vulkan::Driver>,
    adapter: Option<&'a str>,
    cooked: Option<&'a str>,
    metal: bool,
    vulkan: bool,
    validators: vulkan::Validators,
}

impl<'a> GpuArgs<'a> {
    fn parse(args: &[&'a str]) -> Result<Self, String> {
        let usage = || {
            "expected `[--driver <lavapipe|moltenvk|kosmickrisp|system>] [--adapter <name>] \
             [--only <metal|vulkan>] [--gpu-assisted] [--cooked <dir>]`"
                .to_owned()
        };
        let mut parsed = Self {
            drivers: Vec::new(),
            adapter: None,
            cooked: None,
            metal: cfg!(target_os = "macos"),
            vulkan: true,
            validators: vulkan::Validators::Plain,
        };
        let mut rest = args;
        while let [flag, ref tail @ ..] = *rest {
            rest = match (flag, tail) {
                ("--driver", [name, tail @ ..]) if parsed.drivers.is_empty() => {
                    parsed.drivers.push(vulkan::Driver::of(name).ok_or_else(usage)?);
                    tail
                }
                ("--adapter", [name, tail @ ..]) => {
                    parsed.adapter = Some(name);
                    tail
                }
                ("--cooked", [dir, tail @ ..]) => {
                    parsed.cooked = Some(dir);
                    tail
                }
                ("--only", ["metal", tail @ ..]) => {
                    parsed.vulkan = false;
                    tail
                }
                ("--only", ["vulkan", tail @ ..]) => {
                    parsed.metal = false;
                    tail
                }
                ("--gpu-assisted", tail) => {
                    parsed.validators = vulkan::Validators::GpuAssisted;
                    tail
                }
                _ => return Err(usage()),
            };
        }
        if parsed.drivers.is_empty() {
            // The M1's gate: both drivers, MoltenVK's run under Metal's validation and
            // KosmicKrisp's without. GPU-assisted validation fails in MoltenVK's SPIRV-Cross,
            // so its form runs under KosmicKrisp alone.
            if !cfg!(target_os = "macos") {
                return Err(format!("{}: on this host, name the driver", usage()));
            }
            parsed.drivers = match parsed.validators {
                vulkan::Validators::Plain => vulkan::MACOS_GATE.to_vec(),
                vulkan::Validators::GpuAssisted => vec![vulkan::Driver::KosmicKrisp],
            };
        }
        if parsed.adapter.is_some() && parsed.drivers != [vulkan::Driver::System] {
            return Err("`--adapter` selects a device under `--driver system` alone".to_owned());
        }
        if parsed.cooked.is_some() && parsed.vulkan {
            // The code another runner cooked is MSL and metallibs; the Vulkan step takes the
            // whole cooked shaders this host's cook writes.
            return Err("`--cooked` takes the Metal step alone: add `--only metal`".to_owned());
        }
        if parsed.metal && !cfg!(target_os = "macos") {
            return Err("the Metal step runs on macOS".to_owned());
        }
        if !parsed.metal && !parsed.vulkan {
            return Err("no step to run".to_owned());
        }
        Ok(parsed)
    }
}

/// `gpu [--driver <name>] [--adapter <name>] [--only <metal|vulkan>] [--gpu-assisted]
/// [--cooked <dir>]`: cooks the engine's shaders with the pinned Slang, or takes the code
/// another runner cooked to `<dir>`, then runs the Metal step on macOS and the Vulkan step
/// under the named driver, or under each of the M1's gate with none (`README.md`, "GPU
/// runs"). Each step writes its report to `target/gpu/` and to standard error, and every
/// step runs before a failure of any fails the run.
fn run_gpu(args: &[&str]) -> Checked {
    let args = GpuArgs::parse(args)?;
    let context = Context::load()?;
    let pin = slang::Pin::parse(&read(slang::PIN_FILE)?)?;
    let out = context.target_dir.join("gpu");
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (code, cooked_by) = match args.cooked {
        Some(dir) => (PathBuf::from(dir), format!("cooked elsewhere, in {dir}")),
        None => (cook_engine_shaders(&context, &pin, &out)?, format!("Slang {}", pin.version)),
    };
    let mut failures = Vec::new();
    if args.metal {
        failures.extend(run_metal(args.drivers[0].name(), &code, &cooked_by, &out).err());
    }
    if args.vulkan {
        for &driver in &args.drivers {
            failures.extend(
                run_vulkan(&context, &args, driver, &cooked_by, &out)
                    .err()
                    .map(|failure| format!("under {}: {failure}", driver.name())),
            );
        }
    }
    if failures.is_empty() { Ok(Vec::new()) } else { Err(failures.join("\n")) }
}

/// The Metal step: makes a Metal pipeline state from each cooked MSL and metallib, through
/// the Swift script run by the command line tools' `swift`, its canaries first. The driver
/// selects nothing here; it is recorded. Its report is `<out>/<driver>-metal.txt`.
fn run_metal(driver: &str, code: &Path, cooked_by: &str, out: &Path) -> Result<(), String> {
    eprintln!("gpu: the Metal step, under driver {driver}, with Metal's validation on");
    // The canaries first, each in a process of its own: a layer that did not load reports
    // nothing. API validation's aborts the process; shader validation's lets it complete.
    // What each said, as it wrote it, is recorded: the wording differs between Xcodes.
    let mut heard = Vec::with_capacity(CANARIES.len());
    for (canary, phrasings, aborts) in CANARIES {
        let run = metal_step(&["--canary", canary])?;
        match said(phrasings, &run.stderr) {
            Some(says) if run.status.success() != aborts => {
                heard.push(format!("{canary} `{says}`"));
            }
            _ => {
                return Err(format!(
                    "Metal {canary} validation's canary yielded none of `{}`, so the layer is \
                     not listening: it ended with {}:\n{}",
                    phrasings.join("`, `"),
                    run.status,
                    run.stderr
                ));
            }
        }
    }
    let step = metal_step(&[&code.to_string_lossy()])?;
    eprint!("{}", step.stdout);
    // Every line on standard error counts (decision 5.10), but the layers' own two.
    let counted: Vec<&str> = step
        .stderr
        .lines()
        .filter(|line| !vulkan::METAL_BANNERS.iter().any(|on| line.ends_with(on)))
        .collect();
    let file = out.join(format!("{driver}-metal.txt"));
    let written = format!(
        "driver: {driver}\ncode: {cooked_by}\nvalidation: api and shader, canaries {}\n{}messages: \
         {}\n{}",
        heard.join(", "),
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
    Ok(())
}

/// The Vulkan step: every device test of [`DEVICE_TESTS`] in a process of its own, under the
/// pinned layer and `driver`, each judged by decision 5.10's rule; the run's validators'
/// canaries first. Its report is `<out>/<driver>-vulkan[-gpu-assisted].txt`, and a failed
/// test fails the run.
fn run_vulkan(
    context: &Context,
    args: &GpuArgs<'_>,
    driver: vulkan::Driver,
    cooked_by: &str,
    out: &Path,
) -> Result<(), String> {
    let pin = vulkan::Pin::parse(&read(vulkan::PIN_FILE)?)?;
    let sdk = vulkan::sdk(&pin, &context.target_dir)?;
    let layers = out.join("layers");
    let no_layers = out.join("no-layers");
    for dir in [&layers, &no_layers] {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let manifest = fs::read_to_string(&sdk.manifest)
        .map_err(|e| format!("{}: {e}", sdk.manifest.display()))?;
    let implicit = layers.join("VkLayer_khronos_validation.json");
    fs::write(&implicit, vulkan::implicit_manifest(&manifest, &sdk.library)?)
        .map_err(|e| format!("{}: {e}", implicit.display()))?;
    let shaders = context.root.join(out).join("shaders");
    let run = vulkan::Run::environment(
        driver,
        &sdk,
        [&context.root.join(&layers), &context.root.join(&no_layers), &shaders],
        args.validators,
        args.adapter,
    )?;
    let form = match args.validators {
        vulkan::Validators::Plain => "",
        vulkan::Validators::GpuAssisted => "-gpu-assisted",
    };
    eprintln!(
        "gpu: the Vulkan step, under driver {}{form}, with the layer {}{}",
        driver.name(),
        pin.version,
        if run.metal { " and Metal's validation" } else { "" }
    );
    let mut lines = Vec::new();
    let mut failed = Vec::new();
    let mut heard = Vec::new();
    let mut answered = Vec::new();
    for (package, target, prefix) in DEVICE_TESTS {
        let binary = test_binary(context, package, target)?;
        let mut tests = ignored_tests(&binary)?;
        tests.retain(|test| test.starts_with(prefix));
        // The run's canaries first: a layer that did not load reports nothing, and every
        // verdict after would pass for clean.
        let canaries = &run.canaries;
        let order = canaries
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| tests.iter().any(|test| test == name))
            .chain(tests.iter().map(String::as_str).filter(|test| !vulkan::is_canary(test)));
        for test in order {
            let expected = canaries.iter().find(|(name, _)| *name == test).map(|(_, ids)| *ids);
            let mut command = Command::new(&binary);
            // A measurement among them runs short: its figures under the validators mean
            // nothing, and the full run's repetitions would only cost time.
            run.apply(&mut command)
                .env(measure::SHORT_VAR, "1")
                .args(["--exact", test, "--ignored", "--nocapture", "--test-threads", "1"])
                .stdin(Stdio::null());
            let ended = vulkan::run_bounded(&mut command, vulkan::DEADLINE)
                .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
            let text = ended.text();
            let verdict = vulkan::judge_ended(test, &ended, expected, run.metal);
            if test.ends_with("device_reports_its_driver_and_sends_nothing_when_clean") {
                lines.extend(
                    text.lines()
                        .filter(|line| {
                            line.starts_with("DeviceInfo") || line.starts_with("Capabilities")
                        })
                        .map(|line| format!("{line}\n")),
                );
            }
            if expected.is_some() && verdict.failure.is_none() {
                answered.push(test.to_owned());
                heard.push(match &verdict.said {
                    Some(said) if said.starts_with("skipped, ") => format!("{test} {said}"),
                    _ => format!("{test} heard {}", verdict.messages.len() + verdict.reports.len()),
                });
            }
            let entry = vulkan::entry(package, test, &verdict, &ended);
            if verdict.failure.is_some() {
                eprint!("gpu: {entry}{}", vulkan::output(test, &ended));
                failed.push(test.to_owned());
            }
            lines.push(entry);
        }
    }
    // Decision 5.10's "one case for each validator" holds of the run, not of the test
    // binary: a listed canary the binaries do not hold, or that ran and was not heard, fails.
    for (canary, _) in &run.canaries {
        if !answered.iter().any(|test| test == canary) && !failed.iter().any(|test| test == canary)
        {
            lines.push(format!("{canary}: FAILED, a listed canary no test binary holds\n"));
            eprintln!("gpu: {canary}: a listed canary no test binary holds");
            failed.push((*canary).to_owned());
        }
    }
    let file = out.join(format!("{}-vulkan{form}.txt", driver.name()));
    let settings: String =
        run.set.iter().map(|(name, value)| format!("  {name}={value}\n")).collect();
    let cleared = if run.cleared.is_empty() { "none".to_owned() } else { run.cleared.join(", ") };
    let written = format!(
        "driver: {}\nshaders: {cooked_by}\nlayer: {} {} of the Vulkan SDK {}, library {}\n\
         inherited and cleared: {cleared}\nenvironment:\n{settings}canaries: {}\nfailed: {}\n{}",
        driver.name(),
        "VK_LAYER_KHRONOS_validation",
        pin.api_version,
        pin.version,
        sdk.library.display(),
        if heard.is_empty() { "none heard".to_owned() } else { heard.join(", ") },
        failed.len(),
        lines.concat()
    );
    fs::write(&file, written).map_err(|e| format!("{}: {e}", file.display()))?;
    eprintln!("gpu: the report is {}", file.display());
    if !failed.is_empty() {
        return Err(format!(
            "{} device test(s) failed decision 5.10's rule: {}",
            failed.len(),
            failed.join(", ")
        ));
    }
    Ok(())
}

/// Builds the test target `target` of `package`, or its library's unit tests for `None`, and
/// returns its binary.
fn test_binary(context: &Context, package: &str, target: Option<&str>) -> Result<PathBuf, String> {
    let selected = target.map_or(vec!["--lib"], |target| vec!["--test", target]);
    let target = target.map_or_else(|| package.replace('-', "_"), str::to_owned);
    let output = Command::new(&context.cargo)
        .args(["test", "--locked", "--no-run", "--message-format", "json-render-diagnostics"])
        .args(["--package", package])
        .args(selected)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "the build of {package}'s {target} tests ended with {}",
            output.status
        ));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["target"]["name"] == target.as_str())
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| format!("cargo built no binary of {package}'s {target} tests"))
}

/// The ignored tests of a libtest binary, by name.
fn ignored_tests(binary: &Path) -> Result<Vec<String>, String> {
    let output = Command::new(binary)
        .args(["--list", "--ignored", "--format", "terse"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    if !output.status.success() {
        return Err(format!("{} --list ended with {}", binary.display(), output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_suffix(": test").map(str::to_owned))
        .collect())
}

/// Cooks the engine's shaders with the pinned Slang: the code into `<out>/code` and each
/// whole cooked shader into `<out>/shaders`, both emptied first. Returns the code's
/// directory.
fn cook_engine_shaders(context: &Context, pin: &slang::Pin, out: &Path) -> Result<PathBuf, String> {
    let slangc = slang::install(pin, &context.target_dir)?;
    let code = out.join("code");
    let shaders = out.join("shaders");
    // A shader cooked by an earlier run and since removed would otherwise be loaded again.
    for dir in [&code, &shaders] {
        match fs::remove_dir_all(dir) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                return Err(format!("{}: {error}", dir.display()));
            }
            _ => {}
        }
    }
    eprintln!("gpu: cooking {SHADERS} with Slang {}", pin.version);
    let status = Command::new(&context.cargo)
        .args(["run", "--locked", "--quiet", "--package", "lactose-cook", "--bin", "ricotta", "--"])
        .arg("--slangc")
        .arg(&slangc)
        .arg("--code")
        .arg(&code)
        .arg("--shaders")
        .arg(&shaders)
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

/// Each validator's canary in the Metal script, every phrasing seen so far of what the
/// validator says of it, and whether that aborts the process. Each phrasing names the
/// canary's own misuse: no buffer bound at index 0, and the `canary` kernel's store at
/// offset 4194304, the script's index `1 << 20` into a buffer of one `uint`.
const CANARIES: [(&str, &[&str], bool); 2] = [
    // Xcode 16.4 writes `buffer`, Xcode 26.6 and 27.2 `Buffer`: `said` ignores ASCII case.
    ("api", &["missing Buffer binding at index 0"], true),
    (
        "shader",
        &[
            // Xcode 27.2.
            "Invalid store in 'device' or 'constant' address space at offset 4194304, executing \
             compute function: \"canary\"",
            // Xcode 26.6.
            "Invalid device store at offset 4194304, executing kernel function: \"canary\"",
        ],
        false,
    ),
];

/// The text of `stderr` that is one of `phrasings`, compared without regard to ASCII case,
/// as the validator wrote it.
fn said<'a>(phrasings: &[&str], stderr: &'a str) -> Option<&'a str> {
    // ASCII lowercase keeps every byte where it was, so a match's offsets hold in `stderr`.
    let lower = stderr.to_ascii_lowercase();
    phrasings.iter().find_map(|phrasing| {
        let at = lower.find(&phrasing.to_ascii_lowercase())?;
        Some(&stderr[at..at + phrasing.len()])
    })
}

/// What one run of the Metal script printed.
struct StepOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Runs the Metal script under Metal API and shader validation
/// ([`vulkan::METAL_VALIDATION`]).
fn metal_step(args: &[&str]) -> Result<StepOutput, String> {
    let output = Command::new("swift")
        .arg(METAL_SCRIPT)
        .args(args)
        .envs(vulkan::METAL_VALIDATION)
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

#[cfg(test)]
mod tests {
    use super::{CANARIES, DEVICE_TESTS, GpuArgs, said, vulkan};

    #[test]
    fn with_no_driver_the_mac_gates_on_both_and_gpu_assisted_on_kosmickrisp() {
        use vulkan::Driver::{KosmicKrisp, MoltenVk, System};
        let drivers = |args: &[&str]| GpuArgs::parse(args).map(|parsed| parsed.drivers);
        if cfg!(target_os = "macos") {
            assert_eq!(drivers(&[]).unwrap(), [MoltenVk, KosmicKrisp]);
            assert_eq!(drivers(&["--only", "vulkan"]).unwrap(), [MoltenVk, KosmicKrisp]);
            assert_eq!(drivers(&["--gpu-assisted"]).unwrap(), [KosmicKrisp]);
        } else {
            assert!(drivers(&["--only", "vulkan"]).unwrap_err().contains("name the driver"));
        }
        let only = ["--only", "vulkan"];
        assert_eq!(drivers(&[&["--driver", "moltenvk"], &only[..]].concat()).unwrap(), [MoltenVk]);
        assert_eq!(
            drivers(&["--only", "vulkan", "--driver", "system", "--adapter", "M1"]).unwrap(),
            [System]
        );
        assert!(drivers(&["--driver", "moltenvk", "--driver", "kosmickrisp"]).is_err());
        assert!(drivers(&["--driver", "metal"]).is_err());
        let adapter = drivers(&["--only", "vulkan", "--adapter", "M1"]);
        if cfg!(target_os = "macos") {
            assert!(adapter.unwrap_err().contains("--driver system"));
        }
    }

    #[test]
    fn the_device_tests_are_rhi_vulkans_canary_and_swapchain_modules_and_five_test_targets() {
        let library: Vec<&str> = DEVICE_TESTS
            .iter()
            .filter(|(_, target, _)| target.is_none())
            .map(|(_, _, prefix)| *prefix)
            .collect();
        assert_eq!(library, ["canary::", "swapchain::"]);
        // Each test target by its name too: another binary's name would run the wrong one.
        let targets: Vec<(&str, &str, &str)> = DEVICE_TESTS
            .iter()
            .filter_map(|(package, target, prefix)| Some((*package, (*target)?, *prefix)))
            .collect();
        assert_eq!(
            targets,
            [
                ("lactose-rhi-vulkan", "integration", ""),
                ("lactose-render-graph", "integration", "device::"),
                ("lactose-renderer", "integration", "device::"),
                ("lactose-app-client", "integration", "extract::"),
                ("lactose-app-client", "integration", "upload::"),
            ]
        );
    }

    #[test]
    fn a_canary_is_heard_in_each_phrasing_seen_and_no_other_misuse_is() {
        let [(_, api, _), (_, shader, _)] = CANARIES;
        for (phrasings, text, heard) in [
            // Xcode 27.2 on the M1; Xcode 26.6 on the hosted runner wrote the same phrase.
            (
                api,
                "validateComputeFunctionArguments:1043: failed assertion `Compute \
                 Function(canary): missing Buffer binding at index 0 for o[0].'",
                Some("missing Buffer binding at index 0"),
            ),
            // Xcode 16.4 on the oldest runner.
            (
                api,
                "validateComputeFunctionArguments:1056: failed assertion `Compute \
                 Function(canary): missing buffer binding at index 0 for o[0].'",
                Some("missing buffer binding at index 0"),
            ),
            // Xcode 27.2 on the M1.
            (
                shader,
                "Invalid Metal usage: Invalid store in 'device' or 'constant' address space at \
                 offset 4194304, executing compute function: \"canary\" \nMTLBuffer: <no label>",
                Some(
                    "Invalid store in 'device' or 'constant' address space at offset 4194304, \
                     executing compute function: \"canary\"",
                ),
            ),
            // Xcode 26.6 on the hosted runner.
            (
                shader,
                "Invalid device store at offset 4194304, executing kernel function: \"canary\" \
                 encoder: \"0\", dispatch: 0",
                Some(
                    "Invalid device store at offset 4194304, executing kernel function: \"canary\"",
                ),
            ),
            // Another binding index, another kernel's store, another offset, a load.
            (
                api,
                "failed assertion `Compute Function(canary): missing buffer binding at index 1 \
                 for at[0].'",
                None,
            ),
            (
                shader,
                "Invalid device store at offset 4194304, executing kernel function: \"cull\" \
                 encoder: \"0\", dispatch: 0",
                None,
            ),
            (
                shader,
                "Invalid store in 'device' or 'constant' address space at offset 64, executing \
                 compute function: \"canary\"",
                None,
            ),
            (
                shader,
                "Invalid device load at offset 4194304, executing kernel function: \"canary\"",
                None,
            ),
        ] {
            assert_eq!(said(phrasings, text), heard, "{text}");
        }
    }
}
