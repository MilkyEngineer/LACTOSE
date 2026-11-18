//! The Vulkan step of `cargo xtask gpu` (milestone 2 plan, decision 5.10): the pinned Khronos
//! validation layer and the loader's environment for a named driver, and the rule a device
//! test's output passes by.
//!
//! Not a check. The binary runs each device test in a process of its own under the
//! environment [`Run::environment`] gives, and [`judge`] reads what the process wrote.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The repository-relative path of the layer's pin.
pub const PIN_FILE: &str = "tools/xtask/vulkan-layer.toml";

/// The layer's name in its manifest.
const LAYER: &str = "VK_LAYER_KHRONOS_validation";

/// The pin: one SDK's version, the version its layer's manifest reports, and each host's
/// copy of the layer.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The SDK's version, as its archive and its directory name it.
    pub version: String,
    /// The `api_version` of the layer's manifest.
    pub api_version: String,
    /// Each host's copy, by the host's name in `slang`'s pin.
    pub host: BTreeMap<String, HostPin>,
}

/// One host's copy of the layer.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct HostPin {
    /// The SDK archive fetched on this host, or `None` where an installer put the SDK.
    pub url: Option<String>,
    /// The archive's SHA-256, with `url`.
    pub sha256: Option<String>,
    /// The SHA-256 of the layer's library.
    pub library_sha256: String,
}

/// The hosts the pin must name: the CI runner of the `gpu (lavapipe)` job and the M1.
pub const HOSTS: [&str; 2] = ["linux-x86_64", "macos-aarch64"];

impl Pin {
    /// Reads the pin file's text.
    ///
    /// # Errors
    /// For text that is not the pin's form, a host missing or another named, a digest that
    /// is not 64 lowercase hexadecimal digits, or an archive on one host and not the other's
    /// form: Linux's is fetched, with its digest, and the Mac's installed.
    pub fn parse(text: &str) -> Result<Self, String> {
        let pin: Self = toml::from_str(text).map_err(|e| format!("{PIN_FILE}: {e}"))?;
        let names: Vec<&str> = pin.host.keys().map(String::as_str).collect();
        if names != HOSTS {
            return Err(format!("{PIN_FILE}: the hosts are {names:?}, where {HOSTS:?} are pinned"));
        }
        for (name, host) in &pin.host {
            let digests = [Some(&host.library_sha256), host.sha256.as_ref()];
            if digests.into_iter().flatten().any(|digest| !hex64(digest)) {
                return Err(format!("{PIN_FILE}: {name}'s sha256 is not 64 lowercase hex digits"));
            }
            let fetched = name == "linux-x86_64";
            match (&host.url, &host.sha256) {
                (Some(url), Some(_)) if fetched => {
                    let file = format!("/{0}/linux/vulkansdk-linux-x86_64-{0}.tar.xz", pin.version);
                    if !url.starts_with("https://") || !url.ends_with(&file) {
                        return Err(format!("{PIN_FILE}: {name}'s url does not end in `{file}`"));
                    }
                }
                (None, None) if !fetched => {}
                _ => {
                    return Err(format!(
                        "{PIN_FILE}: {name} {} an archive's url and sha256",
                        if fetched { "needs" } else { "takes no" }
                    ));
                }
            }
        }
        Ok(pin)
    }
}

fn hex64(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The drivers `gpu` runs the device tests under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Driver {
    /// Mesa's software driver, on Linux, from the system's package.
    Lavapipe,
    /// MoltenVK, from the SDK.
    MoltenVk,
    /// Mesa's KosmicKrisp, from the SDK: a record, never a gate (decision 5.7).
    KosmicKrisp,
    /// Whatever drivers the system's loader finds, the device selected by name.
    System,
}

impl Driver {
    /// The driver of a name `--driver` takes.
    #[must_use]
    pub fn of(name: &str) -> Option<Self> {
        match name {
            "lavapipe" => Some(Self::Lavapipe),
            "moltenvk" => Some(Self::MoltenVk),
            "kosmickrisp" => Some(Self::KosmicKrisp),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    /// Its name on the command line and in the result file's.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Lavapipe => "lavapipe",
            Self::MoltenVk => "moltenvk",
            Self::KosmicKrisp => "kosmickrisp",
            Self::System => "system",
        }
    }

    /// Whether a failed case fails the run: under every driver but KosmicKrisp, whose run
    /// is that driver's record (decision 5.7).
    #[must_use]
    pub fn gates(self) -> bool {
        self != Self::KosmicKrisp
    }
}

/// Which validators a run turns on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Validators {
    /// Core, synchronisation and thread-safety validation: the pull request's run.
    Plain,
    /// GPU-assisted validation, with thread-safety validation: the nightly form. Core and
    /// synchronisation validation are off, as the layer advises beside it.
    GpuAssisted,
}

impl Validators {
    /// The layer's settings, as the environment variables that set them.
    #[must_use]
    pub fn settings(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Plain => &[
                ("VK_KHRONOS_VALIDATION_VALIDATE_CORE", "true"),
                ("VK_KHRONOS_VALIDATION_VALIDATE_SYNC", "true"),
                ("VK_KHRONOS_VALIDATION_SYNCVAL_LOAD_OP_AFTER_STORE_OP_VALIDATION", "true"),
                ("VK_KHRONOS_VALIDATION_THREAD_SAFETY", "true"),
                ("VK_KHRONOS_VALIDATION_GPUAV_ENABLE", "false"),
            ],
            // The filter drops the layer's own note, on every device, of the features it
            // turns on for its instrumentation.
            Self::GpuAssisted => &[
                ("VK_KHRONOS_VALIDATION_VALIDATE_CORE", "false"),
                ("VK_KHRONOS_VALIDATION_VALIDATE_SYNC", "false"),
                ("VK_KHRONOS_VALIDATION_THREAD_SAFETY", "true"),
                ("VK_KHRONOS_VALIDATION_GPUAV_ENABLE", "true"),
                ("VK_KHRONOS_VALIDATION_MESSAGE_ID_FILTER", "WARNING-Setting-Limit-Adjusted"),
            ],
        }
    }

    /// The canaries the run's validators must each answer: a device test's name, and every
    /// message identifier it must be heard to send, each at least once and nothing else.
    #[must_use]
    pub fn canaries(self) -> &'static [(&'static str, &'static [&'static str])] {
        const CORE: &str = "VUID-vkCmdDraw-None-09600";
        const SYNC: &str = "SYNC-HAZARD-WRITE-AFTER-WRITE";
        match self {
            Self::Plain => &[
                ("device::canary_core_validation_reports_a_wrong_layout", &[CORE]),
                ("device::canary_synchronisation_validation_reports_a_race", &[SYNC]),
                ("conformance::each_validator_reports_its_canary", &[CORE, SYNC]),
            ],
            Self::GpuAssisted => &[(
                "device::canary_gpu_assisted_validation_reports_a_read_past_a_buffer",
                &["VUID-RuntimeSpirv-PhysicalStorageBuffer64-11819"],
            )],
        }
    }
}

/// Whether a device test is a canary of some run's validators: those of another run's are
/// not run.
#[must_use]
pub fn is_canary(test: &str) -> bool {
    [Validators::Plain, Validators::GpuAssisted]
        .iter()
        .any(|validators| validators.canaries().iter().any(|(name, _)| *name == test))
}

/// The SDK's files a run loads.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sdk {
    /// The directory the loader is loaded from.
    pub loader: PathBuf,
    /// The layer's library, its digest the pin's.
    pub library: PathBuf,
    /// The layer's manifest as the SDK ships it.
    pub manifest: PathBuf,
    /// The SDK's drivers' manifests, on the Mac.
    pub icds: PathBuf,
}

/// This host's name in the pin, or why it has none.
///
/// # Errors
/// On a host the pin names no layer for.
pub fn host() -> Result<&'static str, String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("macos", "aarch64") => Ok("macos-aarch64"),
        (os, arch) => Err(format!("{PIN_FILE} pins no validation layer for {os} on {arch}")),
    }
}

/// The pinned SDK on this host: on Linux fetched and unpacked under `target_dir` unless it is
/// there, on the Mac the installed one. Progress goes to standard error.
///
/// # Errors
/// On a host with no pin, an SDK that is not there, a download or unpack that fails, an
/// archive or a layer library whose digest is not the pin's, or a manifest that is not the
/// pinned layer's.
pub fn sdk(pin: &Pin, target_dir: &Path) -> Result<Sdk, String> {
    let host = host()?;
    let pinned = &pin.host[host];
    let sdk = if host == "linux-x86_64" {
        let root = target_dir.join("vulkan-sdk").join(&pin.version);
        let sdk = linux_sdk(&root.join(&pin.version).join("x86_64"));
        if digest_of(&sdk.library).ok().as_deref() != Some(pinned.library_sha256.as_str()) {
            fetch_linux(pin, pinned, target_dir, &root)?;
        }
        sdk
    } else {
        let root = match env::var_os("VULKAN_SDK") {
            Some(root) => PathBuf::from(root),
            None => home()?.join("VulkanSDK").join(&pin.version).join("macOS"),
        };
        Sdk {
            loader: root.join("lib"),
            library: root.join("lib").join("libVkLayer_khronos_validation.dylib"),
            manifest: root.join("share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json"),
            icds: root.join("share/vulkan/icd.d"),
        }
    };
    let digest = digest_of(&sdk.library).map_err(|e| {
        format!(
            "{}: {e}; the Vulkan SDK {} is not installed there",
            sdk.library.display(),
            pin.version
        )
    })?;
    if digest != pinned.library_sha256 {
        return Err(format!(
            "{} has SHA-256 {digest}, where {PIN_FILE} pins {}",
            sdk.library.display(),
            pinned.library_sha256
        ));
    }
    let manifest = fs::read_to_string(&sdk.manifest)
        .map_err(|e| format!("{}: {e}", sdk.manifest.display()))?;
    let (name, api) = layer_of(&manifest)?;
    if name != LAYER || api != pin.api_version {
        return Err(format!(
            "{} is {name} {api}, where {PIN_FILE} pins {LAYER} {}",
            sdk.manifest.display(),
            pin.api_version
        ));
    }
    Ok(sdk)
}

fn linux_sdk(root: &Path) -> Sdk {
    Sdk {
        loader: root.join("lib/VulkanLoader/lib"),
        library: root.join("lib/libVkLayer_khronos_validation.so"),
        manifest: root.join("share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json"),
        icds: PathBuf::from("/usr/share/vulkan/icd.d"),
    }
}

fn home() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "neither VULKAN_SDK nor HOME is set".to_owned())
}

/// Fetches the Linux archive unless a download of the pinned digest is there, and unpacks
/// the loader, the layer and its manifest into a fresh `<root>.partial`, renamed into place.
fn fetch_linux(pin: &Pin, pinned: &HostPin, target_dir: &Path, root: &Path) -> Result<(), String> {
    let (Some(url), Some(sha256)) = (&pinned.url, &pinned.sha256) else {
        return Err(format!("{PIN_FILE}: linux-x86_64 has no archive"));
    };
    let downloads = target_dir.join("vulkan-sdk").join("download");
    fs::create_dir_all(&downloads).map_err(|e| format!("{}: {e}", downloads.display()))?;
    let file = downloads.join(format!("vulkansdk-linux-x86_64-{}.tar.xz", pin.version));
    if digest_of(&file).ok().as_deref() != Some(sha256.as_str()) {
        eprintln!("gpu: fetching {url}");
        run(Command::new("curl")
            .args(["--fail", "--silent", "--show-error", "--location", "--retry", "3", "--output"])
            .arg(&file)
            .arg(url))?;
        let digest = digest_of(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        if digest != *sha256 {
            let _ = fs::remove_file(&file);
            return Err(format!("{url} has SHA-256 {digest}, where {PIN_FILE} pins {sha256}"));
        }
    }
    let partial = root.with_extension("partial");
    remove(&partial)?;
    fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    eprintln!("gpu: unpacking the loader and the layer from {}", file.display());
    let inside = format!("{}/x86_64", pin.version);
    run(Command::new("tar")
        .arg("-xJf")
        .arg(&file)
        .arg("-C")
        .arg(&partial)
        .arg(format!("{inside}/lib/VulkanLoader/lib"))
        .arg(format!("{inside}/lib/libVkLayer_khronos_validation.so"))
        .arg(format!("{inside}/share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json")))?;
    remove(root)?;
    fs::rename(&partial, root).map_err(|e| format!("{}: {e}", root.display()))
}

/// The layer's name and `api_version` in a manifest's text.
fn layer_of(manifest: &str) -> Result<(String, String), String> {
    let value: serde_json::Value =
        serde_json::from_str(manifest).map_err(|e| format!("the layer's manifest: {e}"))?;
    let field = |key: &str| value["layer"][key].as_str().map(str::to_owned);
    match (field("name"), field("api_version")) {
        (Some(name), Some(api)) => Ok((name, api)),
        _ => Err("the layer's manifest names no layer and api_version".to_owned()),
    }
}

/// The SDK's manifest made an implicit layer's: its library by its absolute path, and the
/// `disable_environment` an implicit layer must name. Loaded from a directory of its own
/// with the explicit layer path emptied, the layer is on in every process of the run, and the
/// loader warns of no duplicate and no layer forced by the environment: warnings it sends to
/// the debug messenger, which counts them.
///
/// # Errors
/// On a manifest that is not JSON with a `layer` object.
pub fn implicit_manifest(manifest: &str, library: &Path) -> Result<String, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(manifest).map_err(|e| format!("the layer's manifest: {e}"))?;
    let Some(layer) = value.get_mut("layer").and_then(serde_json::Value::as_object_mut) else {
        return Err("the layer's manifest has no layer".to_owned());
    };
    layer.insert("library_path".to_owned(), library.to_string_lossy().into_owned().into());
    layer.insert(
        "disable_environment".to_owned(),
        serde_json::json!({ "LACTOSE_VALIDATION_OFF": "1" }),
    );
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

/// What the loader, the layer and the tests read from the environment of one run.
pub struct Run {
    /// Every variable set, in the order the result file records them.
    pub set: Vec<(String, String)>,
}

/// Variables a run removes from the environment it inherits: each would change which layers
/// or drivers load, or the layer's settings.
pub const CLEARED: [&str; 7] = [
    "VK_INSTANCE_LAYERS",
    "VK_LOADER_LAYERS_ENABLE",
    "VK_LOADER_LAYERS_DISABLE",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
    "VK_LAYER_SETTINGS_PATH",
    "VK_KHRONOS_VALIDATION_DEBUG_ACTION",
];

impl Run {
    /// The environment of a run of `driver` on `sdk`, with the layer's implicit manifest in
    /// `layers`, an empty directory at `no_layers`, `validators` on, the suite's shaders in
    /// `shaders` and, under `system`, the device `adapter` names.
    ///
    /// # Errors
    /// When the driver's manifest is not where this host keeps it.
    pub fn environment(
        driver: Driver,
        sdk: &Sdk,
        dirs: [&Path; 3],
        validators: Validators,
        adapter: Option<&str>,
    ) -> Result<Self, String> {
        let [layers, no_layers, shaders] = dirs;
        let path = |path: &Path| path.to_string_lossy().into_owned();
        let mut set: Vec<(String, String)> = Vec::new();
        let library_path =
            if cfg!(target_os = "macos") { "DYLD_LIBRARY_PATH" } else { "LD_LIBRARY_PATH" };
        set.push((library_path.to_owned(), path(&sdk.loader)));
        let icd = match driver {
            Driver::Lavapipe => Some(lavapipe_icd(&sdk.icds)?),
            Driver::MoltenVk => Some(sdk.icds.join("MoltenVK_icd.json")),
            Driver::KosmicKrisp => Some(sdk.icds.join("libkosmickrisp_icd.json")),
            Driver::System => None,
        };
        if let Some(icd) = icd {
            if !icd.is_file() {
                return Err(format!("{}: no such driver manifest", icd.display()));
            }
            set.push(("VK_DRIVER_FILES".to_owned(), path(&icd)));
        }
        set.push(("VK_ADD_IMPLICIT_LAYER_PATH".to_owned(), path(layers)));
        set.push(("VK_LAYER_PATH".to_owned(), path(no_layers)));
        for (name, value) in validators.settings() {
            set.push(((*name).to_owned(), (*value).to_owned()));
        }
        // MoltenVK's warnings and errors on standard error, which count. Mesa's drivers print
        // a banner for a driver that is not conformant, which is no message about a call.
        set.push(("MVK_CONFIG_LOG_LEVEL".to_owned(), "2".to_owned()));
        set.push(("MESA_VK_IGNORE_CONFORMANCE_WARNING".to_owned(), "1".to_owned()));
        set.push(("LACTOSE_GPU_SHADERS".to_owned(), path(shaders)));
        if let Some(adapter) = adapter {
            set.push(("LACTOSE_GPU_ADAPTER".to_owned(), adapter.to_owned()));
        }
        Ok(Self { set })
    }

    /// Applies the run's environment to `command`.
    pub fn apply<'c>(&self, command: &'c mut Command) -> &'c mut Command {
        for name in CLEARED {
            command.env_remove(name);
        }
        command.envs(self.set.iter().map(|(name, value)| (name, value)))
    }
}

/// The first of lavapipe's manifests in `dir`, by name: Mesa's package installs one for each
/// architecture it was built for.
fn lavapipe_icd(dir: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("lvp_icd") && name.ends_with(".json"))
        })
        .collect();
    found.sort();
    let Some(first) = found.into_iter().next() else {
        return Err(format!(
            "no lavapipe manifest in {}: is mesa-vulkan-drivers installed?",
            dir.display()
        ));
    };
    Ok(first)
}

/// What one device test's process came to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    /// The suite's own word for the case, `passed` or `skipped, <why>`, where it printed one.
    pub said: Option<String>,
    /// Every message the test's sinks printed, each a line.
    pub messages: Vec<String>,
    /// Every line in which the driver reported an error or a warning.
    pub driver: Vec<String>,
    /// Why the test fails the rule, or `None` where it passes.
    pub failure: Option<String>,
}

/// The lines a driver writes for its errors and warnings: MoltenVK's, and Mesa's log.
const DRIVER_LINES: [&str; 5] =
    ["[mvk-error]", "[mvk-warn]", "MESA: error", "MESA: warning", "WARNING: "];

/// Reads the output of the device test `test`, run alone, and judges it by decision 5.10's
/// rule: the process succeeded; the driver wrote no error or warning line; and its sinks
/// printed no message, or, for a canary, at least one of each of `expected` and nothing
/// else.
#[must_use]
pub fn judge(test: &str, succeeded: bool, output: &str, expected: Option<&[&str]>) -> Verdict {
    let messages: Vec<String> = output
        .lines()
        .filter_map(|line| line.find("message: Message {").map(|at| line[at..].to_owned()))
        .collect();
    let driver: Vec<String> = output
        .lines()
        .filter(|line| DRIVER_LINES.iter().any(|mark| line.contains(mark)))
        .map(str::to_owned)
        .collect();
    let name = test.rsplit("::").next().unwrap_or(test);
    let said = output.lines().find_map(|line| {
        let at = line.find(&format!("{name}: "))?;
        let word = &line[at + name.len() + 2..];
        (word == "passed" || word.starts_with("skipped, ")).then(|| word.to_owned())
    });
    let failure = if !succeeded {
        Some("the test failed".to_owned())
    } else if !driver.is_empty() {
        Some(format!("{} line(s) of the driver's errors or warnings", driver.len()))
    } else {
        match expected {
            None if !messages.is_empty() => Some(format!("{} message(s)", messages.len())),
            None => None,
            Some(ids) => {
                let says = |message: &String, id: &str| message.contains(&format!("id: \"{id}\""));
                if let Some(other) =
                    messages.iter().find(|message| !ids.iter().any(|id| says(message, id)))
                {
                    Some(format!("a message that is not the canary's: {other}"))
                } else {
                    ids.iter().find(|id| !messages.iter().any(|message| says(message, id))).map(
                        |silent| format!("the canary {silent} was not heard: is the layer loaded?"),
                    )
                }
            }
        }
    };
    Verdict { said, messages, driver, failure }
}

/// The SHA-256 of a file's bytes, in lowercase hexadecimal.
fn digest_of(file: &Path) -> io::Result<String> {
    let bytes = fs::read(file)?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
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

    const PIN: &str = include_str!("../vulkan-layer.toml");

    #[test]
    fn the_tracked_pin_reads() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(pin.host.len(), HOSTS.len());
    }

    #[test]
    fn a_pin_missing_a_host_or_with_an_archive_on_the_mac_is_refused() {
        let missing = PIN.replace("[host.macos-aarch64]", "[host.windows-x86_64]");
        assert!(Pin::parse(&missing).unwrap_err().contains("the hosts are"));
        let fetched = PIN.replace(
            "[host.macos-aarch64]\n",
            "[host.macos-aarch64]\nurl = \"https://example.invalid/a\"\n",
        );
        assert!(Pin::parse(&fetched).unwrap_err().contains("takes no"), "{fetched}");
    }

    #[test]
    fn a_url_of_another_version_or_a_digest_not_hex_is_refused() {
        let pin = Pin::parse(PIN).unwrap();
        let other = PIN.replacen(&format!("/{}/linux/", pin.version), "/1.4.341.0/linux/", 1);
        assert!(Pin::parse(&other).unwrap_err().contains("url does not end in"));
        let digest = &pin.host["macos-aarch64"].library_sha256;
        let upper = PIN.replace(digest.as_str(), &digest.to_uppercase());
        assert!(Pin::parse(&upper).unwrap_err().contains("64 lowercase hex digits"));
    }

    #[test]
    fn the_implicit_manifest_names_the_library_and_a_disable_variable() {
        let manifest = r#"{"file_format_version": "1.2.0", "layer": {"name": "VK_LAYER_KHRONOS_validation", "api_version": "1.4.363", "library_path": "../../../lib/libVkLayer_khronos_validation.dylib"}}"#;
        let out =
            implicit_manifest(manifest, Path::new("/sdk/lib/libVkLayer_khronos_validation.dylib"))
                .unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["layer"]["library_path"], "/sdk/lib/libVkLayer_khronos_validation.dylib");
        assert_eq!(value["layer"]["disable_environment"]["LACTOSE_VALIDATION_OFF"], "1");
        assert_eq!(layer_of(&out).unwrap(), (LAYER.to_owned(), "1.4.363".to_owned()));
    }

    const CLEAN: &str = "running 1 test\ntest conformance::a_case ... a_case: passed\nok\n";

    fn message(id: &str) -> String {
        format!(
            "message: Message {{ source: Validation, severity: Error, id: \"{id}\", text: \"t\" }}\n"
        )
    }

    #[test]
    fn a_clean_case_passes_and_its_word_is_kept() {
        let verdict = judge("conformance::a_case", true, CLEAN, None);
        assert_eq!(verdict.failure, None);
        assert_eq!(verdict.said.as_deref(), Some("passed"));
        let skipped = "test conformance::b ... b: skipped, no window\nok\n";
        assert_eq!(
            judge("conformance::b", true, skipped, None).said.as_deref(),
            Some("skipped, no window")
        );
    }

    #[test]
    fn a_message_or_a_driver_line_fails_a_case_that_passed() {
        let sent = format!("{CLEAN}{}", message("VUID-x"));
        assert!(
            judge("conformance::a_case", true, &sent, None).failure.unwrap().contains("1 message")
        );
        let warned = format!("{CLEAN}[mvk-warn] something\n");
        assert!(
            judge("conformance::a_case", true, &warned, None).failure.unwrap().contains("driver")
        );
        assert!(judge("conformance::a_case", false, CLEAN, None).failure.is_some());
    }

    #[test]
    fn a_canary_passes_on_its_messages_alone_and_fails_silent_or_with_another() {
        let ids: &[&str] = &["A", "B"];
        let both = format!("{}{}{}", message("A"), message("B"), message("A"));
        assert_eq!(judge("device::canary", true, &both, Some(ids)).failure, None);
        let silent = judge("device::canary", true, &message("A"), Some(ids)).failure.unwrap();
        assert!(silent.contains("B was not heard"), "{silent}");
        let other = format!("{both}{}", message("C"));
        assert!(
            judge("device::canary", true, &other, Some(ids))
                .failure
                .unwrap()
                .contains("not the canary's")
        );
        // An identifier that merely begins with the canary's is another message.
        let longer = format!("{both}{}", message("A-and-more"));
        assert!(judge("device::canary", true, &longer, Some(ids)).failure.is_some());
    }

    #[test]
    fn every_canary_is_known_as_one() {
        assert!(is_canary("device::canary_core_validation_reports_a_wrong_layout"));
        assert!(is_canary("device::canary_gpu_assisted_validation_reports_a_read_past_a_buffer"));
        assert!(!is_canary("device::device_buffer_is_filled_through_a_staging_copy"));
    }
}
