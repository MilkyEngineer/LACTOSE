//! `cargo xtask`: the command line over the checks in the library.
//!
//! Findings go to standard output, one `path: message` line each, and nothing else does, so
//! the output can be fed to a problem matcher. Progress and summaries go to standard error.
//! The exit status is 0 for no findings, 1 for findings and 2 when a check could not run.
//!
//! Every cargo command this binary runs is `--locked`, as is the `cargo xtask` alias itself:
//! a check never rewrites `Cargo.lock`, and fails if it is out of date. The one exception is
//! the online step of `vendor`, which resolves the lock of a manifest it writes under the
//! target directory, never the workspace's.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use lactose_xtask::harness::{self, Dirs, Selection, stand_in};
use lactose_xtask::policy::Policy;
use lactose_xtask::repo::Files;
use lactose_xtask::tiers::TierFile;
use lactose_xtask::vendor::{self, Repository};
use lactose_xtask::workspace::{Workspace, cargo_metadata, load_metadata};
use lactose_xtask::{Finding, POLICY_FILE, TIER_FILE, boundary, features, tiers};

const USAGE: &str = "\
cargo xtask <command>

  tiers                      check every crate dependency against crate-tiers.toml
  boundary                   check the boundary lint files and everything that could switch
                             them off, the unsafe policy, READMEs, the boundary dependency
                             allowlist and the x86-64-v3 pin
  features [--package <p>]…  require agreeing boundary feature trees, per CI platform, across
                             the shipped binaries and the harness packages (or across the
                             named packages): identical across shipped binaries, the same
                             features for every package a harness package shares with them
  vendor [--offline]         prove each crate built from vendor/ is its patch applied to its
                             record, the patch within the rule of section 8.2, the record
                             the crates.io archive's, and the upstream versions free of
                             advisories
  sour matrix                print the SOUR mode matrix
  sour run <selection> [--emit <file>]
                             run simulation tests under SOUR and compare world hashes;
                             <selection> is --pr, --nightly or --mode <name>…;
                             --emit writes the baseline hashes for `sour compare`
  sour compare <ref> <file>  compare two emitted hash files (architecture mode)
  sour self-test             prove the SOUR runner on its stand-in simulations
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
    let repo = Repository::load(&root, members)?;
    let graph = cargo_metadata(&cargo, &root, if offline { &["--offline"] } else { &[] });
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
    let mut findings = Vec::new();
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
    }
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
    let matrix = harness::matrix();
    let plans = harness::plan(&matrix, &selection)?;

    let context = Context::load()?;
    let (mut findings, nothing_to_run) = context.harness_coverage();
    let mut baseline = harness::Hashes::new();
    if !nothing_to_run {
        let packages = &context.policy.harness_packages;
        let mut launch = |vars: &[(&'static str, String)]| context.nextest(packages, &[], vars);
        let outcome = harness::run(&plans, &context.harness_dirs(), &mut launch);
        eprintln!("sour: {} mode(s), {} test run(s)", plans.len(), outcome.launches);
        findings.extend(outcome.findings);
        baseline = outcome.baseline.unwrap_or_default();
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
