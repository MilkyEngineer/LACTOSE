//! The shader kind: each entry point of a Slang source cooked by the pinned `slangc` to
//! SPIR-V and MSL, and on macOS by Apple's `metal` to a metallib, and checked on what was
//! emitted (`README.md`, "The shader kind"; milestone 2 plan, decision 5.8).
//!
//! The toolchain runs as subprocesses, which need no `unsafe`. Each reads and writes files
//! of its own, outside the pool: the sources, and a scratch directory of the cook's that
//! holds a link module's compiled form and the MSL `metal` reads, under a name that is the
//! same at every cook.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{CookedShader, ShaderTarget};
use serde::Deserialize;

use crate::{msl, spirv};

/// The pin, which `cargo xtask slang` installs from.
const PIN: &str = include_str!("../slang.toml");

/// The directory below the sources whose Slang modules a shader imports: never cooked alone.
pub(crate) const MODULES: &str = "modules/";
/// The ending of a link module, in place of its shader's `.slang`.
pub(crate) const LINK: &str = ".link.slang";

/// The pinned Slang release's `slangc`, checked, and on macOS Apple's `metal` found.
#[derive(Clone, Debug)]
pub struct Slang {
    slangc: PathBuf,
}

impl Slang {
    /// The `slangc` at `slangc`, once it reports the version `tools/cook/slang.toml` pins.
    /// On macOS it also requires that `xcrun` finds `metal`, which the cook runs for each
    /// metallib.
    ///
    /// # Errors
    /// `NotFound` if `slangc` cannot be run, or on macOS if `metal` is not found;
    /// `VersionMismatch` if it reports another version, the SDK's copy among them.
    pub fn new(slangc: &Path) -> Result<Self> {
        let pinned = pinned_version();
        // Absolute, since the cook runs it in the sources directory, and never looked up on
        // the path, where the SDK's copy may be.
        let slangc = &std::path::absolute(slangc).map_err(|error| {
            Error::new(ErrorKind::NotFound, format!("{}: {error}", slangc.display()))
        })?;
        let output = Command::new(slangc).arg("-version").stdin(Stdio::null()).output();
        let output = output.map_err(|error| {
            Error::new(
                ErrorKind::NotFound,
                format!(
                    "cannot run {}: {error}: `cargo xtask slang` installs Slang {pinned}",
                    slangc.display()
                ),
            )
        })?;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let found = if stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        } else {
            stderr
        };
        if !output.status.success() || found != pinned {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "{} reports `{found}`, and the cook runs Slang {pinned} alone \
                     (tools/cook/slang.toml): `cargo xtask slang` installs it",
                    slangc.display()
                ),
            ));
        }
        if cfg!(target_os = "macos") {
            let found = Command::new("xcrun")
                .args(["-sdk", "macosx", "--find", "metal"])
                .stdin(Stdio::null())
                .output();
            if !found.is_ok_and(|output| output.status.success()) {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    "on macOS the cook runs Apple's `metal` for each metallib, and \
                     `xcrun -sdk macosx --find metal` finds none: install Xcode and its Metal \
                     toolchain",
                ));
            }
        }
        Ok(Self { slangc: slangc.to_owned() })
    }
}

/// The version the pin names.
fn pinned_version() -> String {
    #[derive(Deserialize)]
    struct Pin {
        version: String,
    }
    let pin: Pin = toml::from_str(PIN).expect("tools/cook/slang.toml names a version");
    pin.version
}

/// One shader source: its path below the sources and its link module's, if it has one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Source {
    pub path: String,
    pub link: Option<String>,
}

/// One cooked shader and its asset name.
pub(crate) struct Cooked {
    pub name: String,
    pub shader: CookedShader,
}

/// Cooks every entry point of `source`, in byte order of the entry names: SPIR-V, MSL and,
/// on macOS, a metallib each.
pub(crate) fn cook(slang: &Slang, sources: &Path, source: &Source) -> Result<Vec<Cooked>> {
    let scratch = Scratch::new()?;
    let stem = source.path.strip_suffix(".slang").expect("a shader's name");
    let mut linked = Vec::new();
    if let Some(link) = &source.link {
        let module = scratch.path.join("link.slang-module");
        let mut command = slang.command(sources);
        command.arg(link).arg("-o").arg(&module);
        run(&mut command, link)?;
        linked.push(module);
    }
    // One module of every entry point, read for their names.
    let mut command = slang.command(sources);
    command.arg(&source.path).args(&linked).args(SPIRV);
    let all = run(&mut command, &source.path)?;
    let mut entries = spirv::entry_points(&all).map_err(at(&source.path))?;
    if entries.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{}: a shader with no entry point", source.path),
        ));
    }
    entries.sort_unstable();
    let mut cooked = Vec::new();
    for entry in entries {
        let what = format!("{}: {entry}", source.path);
        let compile = |target: &[&str]| {
            let mut command = slang.command(sources);
            command.arg(&source.path).args(["-entry", &entry]).args(&linked).args(target);
            run(&mut command, &what)
        };
        let code = compile(SPIRV)?;
        let reflection = spirv::reflect(&code).map_err(at(format!("{what} (SPIR-V)")))?;
        let msl = compile(MSL)?;
        let text = String::from_utf8(msl.clone()).map_err(|_| {
            Error::new(ErrorKind::InvalidData, format!("{what} (MSL): text that is not UTF-8"))
        })?;
        let interface = msl::read(&text, reflection.stage, &reflection.entry)
            .map_err(at(format!("{what} (MSL)")))?;
        agree(&reflection, &interface).map_err(at(&what))?;
        let shader = |target, code| CookedShader {
            stage: reflection.stage,
            target,
            entry: reflection.entry.clone(),
            code,
            heap: reflection.heap.clone(),
            constants: reflection.constants.clone(),
            push_bytes: reflection.push_bytes,
            inputs: reflection.inputs,
            outputs: reflection.outputs,
        };
        let name = |ending: &str| format!("{stem}.{entry}.{ending}");
        if cfg!(target_os = "macos") {
            let file = format!("{}.{entry}", stem.rsplit('/').next().expect("a stem"));
            let library = metal(&scratch.path, &file, &msl).map_err(at(&what))?;
            cooked.push(Cooked {
                name: name("metallib"),
                shader: shader(ShaderTarget::MetalLibrary, library),
            });
        }
        cooked.push(Cooked { name: name("msl"), shader: shader(ShaderTarget::Msl, msl) });
        cooked.push(Cooked { name: name("spirv"), shader: shader(ShaderTarget::SpirV, code) });
    }
    Ok(cooked)
}

/// The SPIR-V target's arguments.
const SPIRV: &[&str] = &["-target", "spirv", "-fvk-use-entrypoint-name"];
/// The MSL target's.
const MSL: &[&str] = &["-target", "metal"];

impl Slang {
    /// `slangc` with what every call of the cook passes: warnings are errors, no line
    /// directive carries a path into the output, and the modules directory is the import
    /// root. It runs in the sources directory, so a diagnostic names a path below it.
    fn command(&self, sources: &Path) -> Command {
        let mut command = Command::new(&self.slangc);
        command
            .current_dir(sources)
            .args(["-warnings-as-errors", "all", "-line-directive-mode", "none"])
            .arg("-I")
            .arg(MODULES.trim_end_matches('/'));
        command
    }
}

/// The SPIR-V and the MSL agree on what the format carries once for both: each fragment
/// output has one location on both targets, and the inputs and constants are the same.
fn agree(reflection: &spirv::Reflection, interface: &msl::Interface) -> Result<()> {
    let refuse = |message: String| Err(Error::new(ErrorKind::InvalidData, message));
    if reflection.outputs != interface.outputs {
        return refuse(format!(
            "the fragment outputs are at locations {} in the SPIR-V and colours {} in the MSL: \
             give each output `[[vk::location(i)]]` beside its `SV_Target<i>`",
            bits(reflection.outputs),
            bits(interface.outputs)
        ));
    }
    if reflection.inputs != interface.inputs {
        return refuse(format!(
            "the input attachments are {} in the SPIR-V and colours {} in the MSL",
            bits(reflection.inputs),
            bits(interface.inputs)
        ));
    }
    let ids: Vec<u32> = reflection.constants.iter().map(|constant| constant.id).collect();
    if ids != interface.constants {
        return refuse(format!(
            "the specialisation constants are {ids:?} in the SPIR-V and {:?} in the MSL",
            interface.constants
        ));
    }
    Ok(())
}

/// The set bits of `bits`, as a list.
fn bits(bits: u32) -> String {
    let set: Vec<String> =
        (0..u32::BITS).filter(|i| bits & (1 << i) != 0).map(|i| i.to_string()).collect();
    if set.is_empty() { "none".to_owned() } else { set.join(", ") }
}

/// Runs a toolchain command and returns its standard output; `what` names the source in
/// the error.
fn run(command: &mut Command, what: &str) -> Result<Vec<u8>> {
    let program = Path::new(command.get_program()).file_name().unwrap_or_default();
    let program = program.to_string_lossy().into_owned();
    let output = command.stdin(Stdio::null()).output().map_err(|error| {
        Error::new(ErrorKind::NotFound, format!("{what}: cannot run {program}: {error}"))
    })?;
    if !output.status.success() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "{what}: {program} failed:\n{}",
                String::from_utf8_lossy(&output.stderr).trim_end()
            ),
        ));
    }
    Ok(output.stdout)
}

/// Compiles MSL to a metallib with Apple's `metal`, as MSL 3.1 for macOS 14, in `scratch`
/// and under the stable name `<file>.metal`, since the library records its source's name.
fn metal(scratch: &Path, file: &str, msl: &[u8]) -> Result<Vec<u8>> {
    let source = format!("{file}.metal");
    let library = format!("{file}.metallib");
    fs::write(scratch.join(&source), msl).map_err(io_error(scratch))?;
    let mut command = Command::new("xcrun");
    command
        .current_dir(scratch)
        .args(["-sdk", "macosx", "metal", "-std=metal3.1", "-mmacosx-version-min=14.0", "-Werror"])
        .arg("-o")
        .arg(&library)
        .arg(&source);
    run(&mut command, "metal")?;
    fs::read(scratch.join(&library)).map_err(io_error(scratch))
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> Error {
    move |error| Error::new(ErrorKind::Io, format!("{}: {error}", path.display()))
}

/// The error of a step on `what`, prefixed by it.
fn at(what: impl core::fmt::Display) -> impl FnOnce(Error) -> Error {
    move |error| Error::new(error.kind(), format!("{what}: {}", error.message()))
}

/// A directory of the cook's own under the temporary directory, removed when dropped.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("lactose-cook-{}-{n}", std::process::id()));
        // One left by a process of the same id that did not finish.
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(io_error(&path))?;
        Ok(Self { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Nothing reads it again, and a directory left under the temporary directory is
        // the system's to clear.
        let _ = fs::remove_dir_all(&self.path);
    }
}
