//! `cargo xtask slang`: the one command that installs the pinned Slang release on a host
//! (milestone 2 plan, decision 5.8). It reads the pin, fetches this host's archive, refuses it
//! unless its SHA-256 is the pin's, unpacks it under the target directory and prints the path
//! of its `slangc`.
//!
//! Not a check. It runs the host's own `curl` and `tar`, which every CI host and every owner
//! machine has (on Windows the copies in `System32`), so the download needs no dependency.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The repository-relative path of the pin, which the cook reads too.
pub const PIN_FILE: &str = "tools/cook/slang.toml";

/// The pin: one Slang version and each host's archive of it.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The version `slangc -version` reports.
    pub version: String,
    /// Each host's archive, by the host's name in the release's file names.
    #[serde(rename = "archive")]
    pub archives: BTreeMap<String, Archive>,
}

/// One host's archive.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// Where it is fetched from.
    pub url: String,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    pub sha256: String,
}

/// The hosts the pin must name: the three CI and owner platforms.
pub const HOSTS: [&str; 3] = ["linux-x86_64", "macos-aarch64", "windows-x86_64"];

impl Pin {
    /// Reads the pin file's text.
    ///
    /// # Errors
    /// For text that is not the pin's form, an archive missing for one of [`HOSTS`] or named
    /// for another host, a digest that is not 64 lowercase hexadecimal digits, or a URL that
    /// does not end in the release's own file name for that version and host.
    pub fn parse(text: &str) -> Result<Self, String> {
        let pin: Self = toml::from_str(text).map_err(|e| format!("{PIN_FILE}: {e}"))?;
        for host in HOSTS {
            if !pin.archives.contains_key(host) {
                return Err(format!("{PIN_FILE}: no archive for {host}"));
            }
        }
        for (host, archive) in &pin.archives {
            if !HOSTS.contains(&host.as_str()) {
                return Err(format!("{PIN_FILE}: an archive for {host}, which is not a host"));
            }
            let digest = &archive.sha256;
            if digest.len() != 64 || !digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(format!("{PIN_FILE}: {host}'s sha256 is not 64 lowercase hex digits"));
            }
            let file = format!("/v{0}/slang-{0}-{host}.tar.gz", pin.version);
            if !archive.url.starts_with("https://") || !archive.url.ends_with(&file) {
                return Err(format!("{PIN_FILE}: {host}'s url does not end in `{file}`"));
            }
        }
        Ok(pin)
    }
}

/// This host's name in the pin, or why it has none.
///
/// # Errors
/// On a platform the pin names no archive for.
pub fn host() -> Result<&'static str, String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("macos", "aarch64") => Ok("macos-aarch64"),
        ("windows", "x86_64") => Ok("windows-x86_64"),
        (os, arch) => Err(format!("the pin has no Slang archive for {os} on {arch}")),
    }
}

/// The path of `slangc` in an installed release's directory.
#[must_use]
pub fn slangc_in(dir: &Path) -> PathBuf {
    dir.join("bin").join(if cfg!(windows) { "slangc.exe" } else { "slangc" })
}

/// The directory the pinned release is installed in, under `target_dir`.
#[must_use]
pub fn install_dir(target_dir: &Path, pin: &Pin) -> PathBuf {
    target_dir.join("slang").join(&pin.version)
}

/// What `slangc -version` prints, trimmed: the version on standard error, or on standard
/// output if standard error is empty.
///
/// # Errors
/// If `slangc` cannot be run or exits with a failure.
pub fn version_of(slangc: &Path) -> Result<String, String> {
    let output = Command::new(slangc)
        .arg("-version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", slangc.display()))?;
    if !output.status.success() {
        return Err(format!("{} -version ended with {}", slangc.display(), output.status));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Ok(if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        stderr
    })
}

/// Installs the pinned release for this host under `target_dir`, unless it is already there,
/// and returns its `slangc`. Progress goes to standard error.
///
/// The archive is kept under `slang/download/` and fetched again only if its digest is not
/// the pin's. It is unpacked into a fresh `<version>.partial/` directory, whose `slangc` must
/// report the pinned version, and renamed into place: an interrupted install leaves no
/// directory that passes for an installed one.
///
/// # Errors
/// If the host has no archive, `curl` or `tar` fails, the digest differs from the pin's (the
/// file is then deleted), or the unpacked `slangc` reports another version.
pub fn install(pin: &Pin, target_dir: &Path) -> Result<PathBuf, String> {
    let host = host()?;
    let archive = &pin.archives[host];
    let dir = install_dir(target_dir, pin);
    let slangc = slangc_in(&dir);
    if slangc.is_file() && version_of(&slangc).as_deref() == Ok(pin.version.as_str()) {
        eprintln!("slang: {} is installed", pin.version);
        return Ok(slangc);
    }
    let downloads = target_dir.join("slang").join("download");
    fs::create_dir_all(&downloads).map_err(|e| format!("{}: {e}", downloads.display()))?;
    let file = downloads.join(format!("slang-{}-{host}.tar.gz", pin.version));
    if digest_of(&file).ok().as_deref() != Some(archive.sha256.as_str()) {
        eprintln!("slang: fetching {}", archive.url);
        run(Command::new(system_tool("curl"))
            .args(["--fail", "--silent", "--show-error", "--location", "--retry", "3", "--output"])
            .arg(&file)
            .arg(&archive.url))?;
        let digest = digest_of(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        if digest != archive.sha256 {
            // Left in place, a bad file would pass for a cached download on no later run, but
            // would still sit where a reader might take it for the release.
            let _ = fs::remove_file(&file);
            return Err(format!(
                "{} has SHA-256 {digest}, where {PIN_FILE} pins {}",
                archive.url, archive.sha256
            ));
        }
    }
    let partial = target_dir.join("slang").join(format!("{}.partial", pin.version));
    remove(&partial)?;
    fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    eprintln!("slang: unpacking {}", file.display());
    run(Command::new(system_tool("tar")).arg("-xzf").arg(&file).arg("-C").arg(&partial))?;
    let found = version_of(&slangc_in(&partial))?;
    if found != pin.version {
        return Err(format!(
            "the archive's slangc reports {found}, where {PIN_FILE} pins {}",
            pin.version
        ));
    }
    remove(&dir)?;
    fs::rename(&partial, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    eprintln!("slang: {} installed", pin.version);
    Ok(slangc)
}

/// Cooks the shader sources under `sources` with `slangc` through `ricotta`, run by `cargo`
/// from `root`: each entry's code into `out/code`, as `--code` writes it, the cooked sources
/// into `out/shaders`, and the pack into `out/shaders.pack`. Removes what an earlier cook
/// left in the two directories first, so that a shader since removed is not loaded again.
/// Ricotta's own output goes to standard error. Returns `out/code`.
///
/// # Errors
/// If a directory cannot be removed, or ricotta cannot run or fails.
pub fn cook(
    cargo: &str,
    root: &Path,
    slangc: &Path,
    sources: &Path,
    out: &Path,
) -> Result<PathBuf, String> {
    let code = out.join("code");
    let shaders = out.join("shaders");
    for dir in [&code, &shaders] {
        remove(dir)?;
    }
    let status = Command::new(cargo)
        .args(["run", "--locked", "--quiet", "--package", "lactose-cook", "--bin", "ricotta", "--"])
        .arg("--slangc")
        .arg(slangc)
        .arg("--code")
        .arg(&code)
        .arg("--shaders")
        .arg(&shaders)
        .arg(sources)
        .arg(out.join("shaders.pack"))
        .current_dir(root)
        .stdout(Stdio::from(io::stderr()))
        .status()
        .map_err(|e| format!("cannot run ricotta: {e}"))?;
    if !status.success() {
        return Err(format!("ricotta ended with {status}"));
    }
    Ok(code)
}

/// The SHA-256 of a file's bytes, in lowercase hexadecimal.
fn digest_of(file: &Path) -> io::Result<String> {
    let bytes = fs::read(file)?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// `curl` or `tar`: on Windows the copy the system ships in `System32`, since a `tar` found
/// first on the path (Git's, on a runner) reads `C:` in a path as a remote host.
fn system_tool(name: &str) -> PathBuf {
    match env::var_os("SystemRoot") {
        Some(root) if cfg!(windows) => {
            Path::new(&root).join("System32").join(format!("{name}.exe"))
        }
        _ => PathBuf::from(name),
    }
}

fn run(command: &mut Command) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(io::stderr()))
        .status()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{program} ended with {status}")) }
}

fn remove(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(format!("{}: {error}", dir.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = include_str!("../../cook/slang.toml");

    #[test]
    fn the_tracked_pin_reads() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(pin.archives.len(), HOSTS.len());
    }

    #[test]
    fn a_pin_missing_a_host_or_naming_another_is_refused() {
        let missing = PIN.replace("[archive.windows-x86_64]", "[archive.windows-aarch64]");
        let error = Pin::parse(&missing).unwrap_err();
        assert!(error.contains("no archive for windows-x86_64"), "{error}");
    }

    #[test]
    fn a_url_of_another_version_is_refused() {
        let pin = Pin::parse(PIN).unwrap();
        let other = PIN.replacen(&format!("/v{}/", pin.version), "/v2026.22/", 1);
        let error = Pin::parse(&other).unwrap_err();
        assert!(error.contains("url does not end in"), "{error}");
    }

    #[test]
    fn a_digest_that_is_not_hex_is_refused() {
        let pin = Pin::parse(PIN).unwrap();
        let digest = &pin.archives["macos-aarch64"].sha256;
        let error = Pin::parse(&PIN.replace(digest.as_str(), &digest.to_uppercase())).unwrap_err();
        assert!(error.contains("64 lowercase hex digits"), "{error}");
    }
}
