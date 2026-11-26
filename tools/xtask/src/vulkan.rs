//! The Vulkan step of `cargo xtask gpu` (milestone 2 plan, decision 5.10): the pinned Khronos
//! validation layer and the loader's environment for a named driver, and the rule a device
//! test's output passes by.
//!
//! Not a check. The binary runs each device test in a process of its own under the
//! environment [`Run::environment`] gives, and [`judge`] reads what the process wrote.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    /// LunarG's file of the SDK: on Linux the archive the tool fetches, on Windows the
    /// installer an owner runs, which the tool names and never fetches; `None` on the Mac.
    pub url: Option<String>,
    /// That file's SHA-256, the digest LunarG publishes for it, with `url`.
    pub sha256: Option<String>,
    /// The SHA-256 of the layer's library.
    pub library_sha256: String,
}

/// The hosts the pin must name: the CI runner of the `gpu (lavapipe)` job, the M1, and
/// Windows on x86-64, the laptop's and the desktop's.
pub const HOSTS: [&str; 3] = ["linux-x86_64", "macos-aarch64", "windows-x86_64"];

/// A host the pin names, by the SDK's form there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Host {
    /// The archive fetched by digest and unpacked under the target directory; the SDK's loader
    /// and the system's lavapipe.
    Linux,
    /// The installed SDK; its loader and its drivers, or the system's.
    MacOs,
    /// The installed SDK; the system's loader and the system's drivers alone.
    Windows,
}

impl Host {
    /// The host this tool runs on.
    ///
    /// # Errors
    /// On a host the pin names no layer for.
    pub fn this() -> Result<Self, String> {
        match (env::consts::OS, env::consts::ARCH) {
            ("linux", "x86_64") => Ok(Self::Linux),
            ("macos", "aarch64") => Ok(Self::MacOs),
            ("windows", "x86_64") => Ok(Self::Windows),
            (os, arch) => Err(format!("{PIN_FILE} pins no validation layer for {os} on {arch}")),
        }
    }

    /// Its name in the pin, one of [`HOSTS`].
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Linux => HOSTS[0],
            Self::MacOs => HOSTS[1],
            Self::Windows => HOSTS[2],
        }
    }

    /// The end of the url of LunarG's file the pin names for this host, of the SDK `version`:
    /// the archive on Linux, the installer on Windows; none on the Mac.
    fn named_file(self, version: &str) -> Option<String> {
        match self {
            Self::Linux => {
                Some(format!("/{version}/linux/vulkansdk-linux-x86_64-{version}.tar.xz"))
            }
            Self::MacOs => None,
            Self::Windows => {
                Some(format!("/{version}/windows/vulkansdk-windows-X64-{version}.exe"))
            }
        }
    }
}

impl Pin {
    /// Reads the pin file's text.
    ///
    /// # Errors
    /// For text that is not the pin's form, a host missing or another named, a digest that
    /// is not 64 lowercase hexadecimal digits, or a host not of its form: Linux names the
    /// archive it fetches and Windows the installer an owner runs, each with its url and
    /// digest, and the Mac names neither.
    pub fn parse(text: &str) -> Result<Self, String> {
        let pin: Self = toml::from_str(text).map_err(|e| format!("{PIN_FILE}: {e}"))?;
        let names: Vec<&str> = pin.host.keys().map(String::as_str).collect();
        if names != HOSTS {
            return Err(format!("{PIN_FILE}: the hosts are {names:?}, where {HOSTS:?} are pinned"));
        }
        for (host, (name, pinned)) in
            [Host::Linux, Host::MacOs, Host::Windows].into_iter().zip(&pin.host)
        {
            let digests = [Some(&pinned.library_sha256), pinned.sha256.as_ref()];
            if digests.into_iter().flatten().any(|digest| !hex64(digest)) {
                return Err(format!("{PIN_FILE}: {name}'s sha256 is not 64 lowercase hex digits"));
            }
            match (host.named_file(&pin.version), &pinned.url, &pinned.sha256) {
                (Some(file), Some(url), Some(_)) => {
                    if !url.starts_with("https://") || !url.ends_with(&file) {
                        return Err(format!("{PIN_FILE}: {name}'s url does not end in `{file}`"));
                    }
                }
                (None, None, None) => {}
                (named, _, _) => {
                    return Err(format!(
                        "{PIN_FILE}: {name} {} a url and sha256",
                        if named.is_some() { "needs" } else { "takes no" }
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
    /// Mesa's KosmicKrisp, from the SDK.
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

    /// Whether its runs are under Metal API and shader validation: MoltenVK's are. Not
    /// KosmicKrisp's, whose queue aborts under API validation and whose heap's textures are
    /// not resident under shader validation, defects of that driver.
    #[must_use]
    pub fn metal_validation(self) -> bool {
        self == Self::MoltenVk
    }

    /// Whether the gate's GPU-assisted form runs under it: not under MoltenVK, whose
    /// SPIRV-Cross fails on the layer's instrumented shaders.
    #[must_use]
    pub fn takes_gpu_assisted(self) -> bool {
        self != Self::MoltenVk
    }
}

/// The drivers `gpu` gates on with no `--driver`, on macOS, one after the other: each
/// catches a missing barrier the other misses (the owner's ruling of 19 November 2026).
pub const MACOS_GATE: [Driver; 2] = [Driver::MoltenVk, Driver::KosmicKrisp];

/// Metal API and shader validation, set as the Metal spike set them: API validation's
/// assertions abort, its warnings and shader validation's reports go to standard error.
pub const METAL_VALIDATION: [(&str, &str); 4] = [
    ("MTL_DEBUG_LAYER", "1"),
    ("MTL_DEBUG_LAYER_WARNING_MODE", "nslog"),
    ("MTL_SHADER_VALIDATION", "1"),
    ("MTL_SHADER_VALIDATION_REPORT_TO_STDERR", "1"),
];

/// What Metal's validators each write once on loading into a process, API validation's and
/// shader validation's, at the end of a line of the system's log.
pub const METAL_BANNERS: [&str; 2] =
    ["Metal API Validation Enabled", "Metal GPU Validation Enabled"];

/// The canary of Metal's shader validation in a run under it: a device test whose dispatch
/// reads past a buffer, and the words of the report it must be heard to write from the test's
/// own process. The banners show the validators loaded; this shows a report reaching the
/// judge, which `MTL_SHADER_VALIDATION_REPORT_TO_STDERR` unset would silence.
pub const METAL_CANARY: (&str, &str) = (
    "device::canary_metal_shader_validation_reports_a_read_past_a_buffer",
    "Invalid load in 'device' or 'constant' address space",
);

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
            // turns on for its instrumentation. Safe mode skips an access the instrumentation
            // finds invalid and still reports it: without it the access is made, and under
            // lavapipe the heap canary's slot past the heap is a host descriptor whose
            // sampling function is null, which ends the process before the layer reads its
            // report back. The sanitizer, which safe mode also turns on, stays off, so the
            // run checks what it did without safe mode.
            Self::GpuAssisted => &[
                ("VK_KHRONOS_VALIDATION_VALIDATE_CORE", "false"),
                ("VK_KHRONOS_VALIDATION_VALIDATE_SYNC", "false"),
                ("VK_KHRONOS_VALIDATION_THREAD_SAFETY", "true"),
                ("VK_KHRONOS_VALIDATION_GPUAV_ENABLE", "true"),
                ("VK_KHRONOS_VALIDATION_GPUAV_SAFE_MODE", "true"),
                ("VK_KHRONOS_VALIDATION_GPUAV_SHADER_SANITIZER", "false"),
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
                (
                    "device::canary_load_after_store_validation_reports_a_kept_target",
                    &["SYNC-HAZARD-READ-AFTER-WRITE"],
                ),
                (
                    "device::canary_synchronisation_validation_reports_a_race_across_queues",
                    &["SYNC-HAZARD-WRITE-RACING-WRITE"],
                ),
                (
                    "device::canary_synchronisation_validation_reports_a_race_across_submits",
                    &[SYNC],
                ),
                (
                    "canary::canary_thread_safety_validation_reports_a_queue_used_on_two_threads",
                    &["UNASSIGNED-Threading-MultipleThreads-Write"],
                ),
                ("conformance::each_validator_reports_its_canary", &[CORE, SYNC]),
            ],
            Self::GpuAssisted => &[
                (
                    "device::canary_gpu_assisted_validation_reports_a_read_past_a_buffer",
                    &["VUID-RuntimeSpirv-PhysicalStorageBuffer64-11819"],
                ),
                (
                    "device::canary_gpu_assisted_validation_reports_a_slot_past_the_heap",
                    &["VUID-vkCmdDraw-None-10068"],
                ),
            ],
        }
    }
}

/// Whether a device test is a canary of some run's validators, or Metal's: those of another
/// run's are not run.
#[must_use]
pub fn is_canary(test: &str) -> bool {
    test == METAL_CANARY.0
        || [Validators::Plain, Validators::GpuAssisted]
            .iter()
            .any(|validators| validators.canaries().iter().any(|(name, _)| *name == test))
}

/// The SDK's files a run loads.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sdk {
    /// The SDK's directory, which a result file writes as `<sdk>`.
    pub root: PathBuf,
    /// The directory the loader is loaded from, or `None` where a run takes the system's
    /// (Windows).
    pub loader: Option<PathBuf>,
    /// The layer's library, its digest the pin's.
    pub library: PathBuf,
    /// The layer's manifest as the SDK ships it.
    pub manifest: PathBuf,
    /// The directory of the manifests of the drivers a run names: the SDK's on the Mac, the
    /// system's (lavapipe's) on Linux; `None` where a run takes the system's drivers alone
    /// (Windows).
    pub icds: Option<PathBuf>,
}

impl Sdk {
    /// The files of the SDK at `root` on `host`, as that host's SDK lays them out.
    #[must_use]
    pub fn at(host: Host, root: PathBuf) -> Self {
        let manifest = "share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json";
        match host {
            Host::Linux => Self {
                loader: Some(root.join("lib/VulkanLoader/lib")),
                library: root.join("lib/libVkLayer_khronos_validation.so"),
                manifest: root.join(manifest),
                icds: Some(PathBuf::from("/usr/share/vulkan/icd.d")),
                root,
            },
            Host::MacOs => Self {
                loader: Some(root.join("lib")),
                library: root.join("lib").join("libVkLayer_khronos_validation.dylib"),
                manifest: root.join(manifest),
                icds: Some(root.join("share/vulkan/icd.d")),
                root,
            },
            // The manifest is beside the library, naming it by a relative path; the installer
            // registers it as an explicit layer, which a run's empty `VK_LAYER_PATH` hides.
            Host::Windows => Self {
                loader: None,
                library: root.join("Bin").join("VkLayer_khronos_validation.dll"),
                manifest: root.join("Bin").join("VkLayer_khronos_validation.json"),
                icds: None,
                root,
            },
        }
    }
}

/// Where the pinned SDK is on `host`, given the values of `VULKAN_SDK` and `HOME`: on Linux
/// under `target_dir`, where the tool unpacks it; on the Mac and on Windows at `VULKAN_SDK`,
/// which the installer sets, or where the installer puts it, `~/VulkanSDK/<version>/macOS`
/// and `C:\VulkanSDK\<version>`.
///
/// # Errors
/// On the Mac, where neither variable is set.
pub fn sdk_root(
    host: Host,
    pin: &Pin,
    target_dir: &Path,
    vulkan_sdk: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, String> {
    let version = &pin.version;
    match (host, vulkan_sdk) {
        (Host::Linux, _) => {
            Ok(target_dir.join("vulkan-sdk").join(version).join(version).join("x86_64"))
        }
        (_, Some(root)) => Ok(PathBuf::from(root)),
        (Host::MacOs, None) => home
            .map(|home| PathBuf::from(home).join("VulkanSDK").join(version).join("macOS"))
            .ok_or_else(|| "neither VULKAN_SDK nor HOME is set".to_owned()),
        (Host::Windows, None) => Ok(PathBuf::from(format!(r"C:\VulkanSDK\{version}"))),
    }
}

/// The pinned SDK on this host: on Linux fetched and unpacked under `target_dir` unless it is
/// there, on the Mac and on Windows the installed one ([`sdk_root`]), held to the pin by
/// [`check`]. Progress goes to standard error.
///
/// # Errors
/// On a host with no pin, a download or unpack that fails, an archive whose digest is not the
/// pin's, or an SDK that [`check`] refuses.
pub fn sdk(pin: &Pin, target_dir: &Path) -> Result<Sdk, String> {
    let host = Host::this()?;
    let pinned = &pin.host[host.name()];
    let root = sdk_root(host, pin, target_dir, env::var_os("VULKAN_SDK"), env::var_os("HOME"))?;
    let sdk = Sdk::at(host, root);
    if host == Host::Linux
        && digest_of(&sdk.library).ok().as_deref() != Some(pinned.library_sha256.as_str())
    {
        let unpacked = target_dir.join("vulkan-sdk").join(&pin.version);
        fetch_linux(pin, pinned, target_dir, &unpacked)?;
    }
    check(pin, host, &sdk)?;
    Ok(sdk)
}

/// Holds the SDK at `sdk` to the pin for `host`: first its layer's manifest, which must name
/// the pinned layer at the pinned version, so that another SDK, which the installer's
/// `VULKAN_SDK` names once a later one is installed, is refused as that version; then the
/// layer's library, whose SHA-256 must be the pin's.
///
/// # Errors
/// An SDK that is not there, which names the installer on Windows; a manifest of another
/// layer or version; a library of another digest.
pub fn check(pin: &Pin, host: Host, sdk: &Sdk) -> Result<(), String> {
    let pinned = &pin.host[host.name()];
    let manifest = fs::read_to_string(&sdk.manifest).map_err(|e| {
        let install = match (host, &pinned.url, &pinned.sha256) {
            (Host::Windows, Some(url), Some(sha256)) => format!(
                ": install it with LunarG's installer {url} (SHA-256 {sha256}) as administrator, \
                 which this tool does not fetch"
            ),
            _ => String::new(),
        };
        format!(
            "{}: {e}; the Vulkan SDK {} is not installed at {}{install}",
            sdk.manifest.display(),
            pin.version,
            sdk.root.display()
        )
    })?;
    let (name, api) = layer_of(&manifest)?;
    if name != LAYER || api != pin.api_version {
        return Err(format!(
            "{} is {name} {api}: another SDK than the Vulkan SDK {}, whose {LAYER} is {}, which \
             {PIN_FILE} pins",
            sdk.manifest.display(),
            pin.version,
            pin.api_version
        ));
    }
    let digest = digest_of(&sdk.library).map_err(|e| format!("{}: {e}", sdk.library.display()))?;
    if digest != pinned.library_sha256 {
        return Err(format!(
            "{} has SHA-256 {digest}, where {PIN_FILE} pins {}",
            sdk.library.display(),
            pinned.library_sha256
        ));
    }
    Ok(())
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
/// `disable_environment` an implicit layer must name. Loaded from a directory of its own,
/// the run's whole implicit layer path, with the explicit layer path emptied, the layer is on
/// in every process of the run and is the only layer loaded, and the loader warns of no
/// duplicate and no layer forced by the environment: warnings it sends to the debug
/// messenger, which counts them.
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
    /// Every inherited variable the run removes (`cleared`), by name, for the result file.
    pub cleared: Vec<String>,
    /// Whether the run sets Metal's validation, whose banners each test must then print.
    pub metal: bool,
    /// The canaries the run must each hear, by test and the layer's identifiers: its
    /// validators', and with the plain ones under Metal's validation [`METAL_CANARY`], of
    /// which the layer hears nothing.
    pub canaries: Vec<(&'static str, &'static [&'static str])>,
}

/// The variables that load `sdk`'s loader, where the host takes the SDK's, and, but under
/// `system`, pin `driver`'s manifest as the SDK, or on Linux the system, ships it.
fn loader_and_driver(driver: Driver, sdk: &Sdk) -> Result<Vec<(String, String)>, String> {
    let path = |path: &Path| path.to_string_lossy().into_owned();
    // On macOS the fallback path: `DYLD_LIBRARY_PATH` would also redirect a driver the
    // loader opens by its absolute path to the SDK's copy of the same name, which the
    // loader reports as a message under `system`, where the drivers are the system's.
    let library_path =
        if cfg!(target_os = "macos") { "DYLD_FALLBACK_LIBRARY_PATH" } else { "LD_LIBRARY_PATH" };
    let mut set: Vec<(String, String)> =
        sdk.loader.iter().map(|loader| (library_path.to_owned(), path(loader))).collect();
    let icd = match (driver, &sdk.icds) {
        (Driver::System, _) => None,
        (_, None) => {
            return Err(format!(
                "the driver {} is not this host's: a run here takes the system's drivers, \
                 `--driver system`",
                driver.name()
            ));
        }
        (Driver::Lavapipe, Some(icds)) => Some(lavapipe_icd(icds)?),
        (Driver::MoltenVk, Some(icds)) => Some(icds.join("MoltenVK_icd.json")),
        (Driver::KosmicKrisp, Some(icds)) => Some(icds.join("libkosmickrisp_icd.json")),
    };
    if let Some(icd) = icd {
        if !icd.is_file() {
            return Err(format!("{}: no such driver manifest", icd.display()));
        }
        set.push(("VK_DRIVER_FILES".to_owned(), path(&icd)));
    }
    Ok(set)
}

/// The variables that give the loader its layers: the implicit ones from `implicit` alone,
/// which the loader then reads in place of every other place it looks for them, the Windows
/// registry's keys among them, and the explicit ones from the empty `no_layers`, which hides
/// the SDK's, the installer's registry entries among them. A layer the loader is made to skip
/// by `VK_LOADER_LAYERS_DISABLE` it warns of instead, and the debug messenger counts the
/// warning; one added beside the registry's by `VK_ADD_IMPLICIT_LAYER_PATH` joins every
/// layer the machine registers.
fn layer_paths(implicit: &Path, no_layers: &Path) -> [(String, String); 2] {
    let path = |path: &Path| path.to_string_lossy().into_owned();
    [
        ("VK_IMPLICIT_LAYER_PATH".to_owned(), path(implicit)),
        ("VK_LAYER_PATH".to_owned(), path(no_layers)),
    ]
}

/// Variables a run removes from the environment it inherits, before it sets its own: each
/// would change which layers or drivers load, Metal's validation, or the tests' device.
/// `METAL_DEVICE_WRAPPER_TYPE`, which Xcode sets, turns Metal's API validation on as
/// `MTL_DEBUG_LAYER` does.
const CLEARED: [&str; 12] = [
    "DYLD_LIBRARY_PATH",
    "VK_INSTANCE_LAYERS",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "VK_ADD_DRIVER_FILES",
    "VK_ADD_LAYER_PATH",
    "VK_IMPLICIT_LAYER_PATH",
    "VK_ADD_IMPLICIT_LAYER_PATH",
    "METAL_DEVICE_WRAPPER_TYPE",
    "LACTOSE_VALIDATION_OFF",
    "LACTOSE_GPU_ADAPTER",
    "LACTOSE_GPU_SHADERS",
];

/// Every family of variables a run removes: the layer's settings under both their names
/// (`VK_KHRONOS_VALIDATION_*`, and the `VK_LAYER_*` its manifest names, with the layer
/// paths), the loader's own, MoltenVK's (`MVK_*`), Mesa's (`MESA_*`, KosmicKrisp's and
/// lavapipe's), and Metal's, which a run under MoltenVK sets and any other runs without. A
/// driver's variables outside these are not cleared.
const CLEARED_PREFIXES: [&str; 6] =
    ["VK_KHRONOS_VALIDATION_", "VK_LAYER_", "VK_LOADER_", "MVK_", "MESA_", "MTL_"];

/// The names among `inherited` a run removes, sorted.
#[must_use]
pub fn cleared(inherited: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = inherited
        .into_iter()
        .filter(|name| {
            CLEARED.contains(&name.as_str())
                || CLEARED_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

impl Run {
    /// The environment of a run of `driver` on `sdk`, with the layer's implicit manifest in
    /// `layers`, the loader's only implicit layer path, an empty directory at `no_layers`, its
    /// explicit layer path, `validators` on, Metal's validation
    /// where the driver runs under it, the suite's shaders in `shaders` and, under `system`,
    /// the device `adapter` names.
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
        set.extend(loader_and_driver(driver, sdk)?);
        set.extend(layer_paths(layers, no_layers));
        for (name, value) in validators.settings() {
            set.push(((*name).to_owned(), (*value).to_owned()));
        }
        let metal = driver.metal_validation();
        if metal {
            set.extend(METAL_VALIDATION.map(|(name, value)| (name.to_owned(), value.to_owned())));
        }
        // MoltenVK's warnings and errors on standard error, which count. Mesa's drivers print
        // a banner for a driver that is not conformant, which is no message about a call.
        set.push(("MVK_CONFIG_LOG_LEVEL".to_owned(), "2".to_owned()));
        set.push(("MESA_VK_IGNORE_CONFORMANCE_WARNING".to_owned(), "1".to_owned()));
        set.push(("LACTOSE_GPU_SHADERS".to_owned(), path(shaders)));
        if let Some(adapter) = adapter {
            set.push(("LACTOSE_GPU_ADAPTER".to_owned(), adapter.to_owned()));
        }
        let names = std::env::vars_os().filter_map(|(name, _)| name.into_string().ok());
        let mut canaries = validators.canaries().to_vec();
        if metal && validators == Validators::Plain {
            canaries.push((METAL_CANARY.0, &[]));
        }
        Ok(Self { set, cleared: cleared(names), metal, canaries })
    }

    /// The environment of a measurement on `driver` (`cargo xtask measure`): no layer at
    /// all, the loader's implicit and explicit layer paths both the empty `no_layers`, and no
    /// Metal validation, so the figures are the driver's alone. The loader is `sdk`'s where one
    /// is given, and the driver's manifest is the SDK's, as
    /// [`environment`](Self::environment) pins it; under `system` with no SDK the system's
    /// loader finds the system's drivers (Windows, where a measurement needs no layer).
    /// MoltenVK's warnings and errors still reach standard error, where the measurement's own
    /// sink fails on any message the device sends.
    ///
    /// # Errors
    /// When the driver's manifest is not where this host keeps it, or a driver other than
    /// `system` is given no SDK.
    pub fn measured(
        driver: Driver,
        sdk: Option<&Sdk>,
        no_layers: &Path,
        adapter: Option<&str>,
    ) -> Result<Self, String> {
        let mut set = match sdk {
            Some(sdk) => loader_and_driver(driver, sdk)?,
            None if driver == Driver::System => Vec::new(),
            None => return Err(format!("the driver {} is the pinned SDK's", driver.name())),
        };
        set.extend(layer_paths(no_layers, no_layers));
        set.push(("MVK_CONFIG_LOG_LEVEL".to_owned(), "2".to_owned()));
        set.push(("MESA_VK_IGNORE_CONFORMANCE_WARNING".to_owned(), "1".to_owned()));
        if let Some(adapter) = adapter {
            set.push(("LACTOSE_GPU_ADAPTER".to_owned(), adapter.to_owned()));
        }
        let names = std::env::vars_os().filter_map(|(name, _)| name.into_string().ok());
        Ok(Self { set, cleared: cleared(names), metal: false, canaries: Vec::new() })
    }

    /// Applies the run's environment to `command`: the inherited variables it clears
    /// removed, then its own set.
    pub fn apply<'c>(&self, command: &'c mut Command) -> &'c mut Command {
        for name in &self.cleared {
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

/// The Swift script of the Metal step.
pub const METAL_SCRIPT: &str = "tools/xtask/metal/pipelines.swift";
/// Builds the test target `target` of `package`, or its library's unit tests for `None`, and
/// returns its binary.
///
/// # Errors
/// If cargo cannot run, the build fails, or it builds no such binary.
pub fn test_binary(
    cargo: &str,
    root: &Path,
    package: &str,
    target: Option<&str>,
) -> Result<PathBuf, String> {
    let selected = target.map_or(vec!["--lib"], |target| vec!["--test", target]);
    let target = target.map_or_else(|| package.replace('-', "_"), str::to_owned);
    let output = Command::new(cargo)
        .current_dir(root)
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
///
/// # Errors
/// If the binary cannot run or fails to list them.
pub fn ignored_tests(binary: &Path) -> Result<Vec<String>, String> {
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

/// Each validator's canary in the Metal script, every phrasing seen so far of what the
/// validator says of it, and whether that aborts the process. Each phrasing names the
/// canary's own misuse: no buffer bound at index 0, and the `canary` kernel's store at
/// offset 4194304, the script's index `1 << 20` into a buffer of one `uint`.
pub const METAL_CANARIES: [(&str, &[&str], bool); 2] = [
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
#[must_use]
pub fn said<'a>(phrasings: &[&str], stderr: &'a str) -> Option<&'a str> {
    // ASCII lowercase keeps every byte where it was, so a match's offsets hold in `stderr`.
    let lower = stderr.to_ascii_lowercase();
    phrasings.iter().find_map(|phrasing| {
        let at = lower.find(&phrasing.to_ascii_lowercase())?;
        Some(&stderr[at..at + phrasing.len()])
    })
}

/// What one run of the Metal script printed.
pub struct StepOutput {
    /// How it ended.
    pub status: std::process::ExitStatus,
    /// Its standard output.
    pub stdout: String,
    /// Its standard error.
    pub stderr: String,
}

/// Runs the Metal script, from the repository root `root`, under Metal API and shader
/// validation ([`METAL_VALIDATION`]).
///
/// # Errors
/// If `swift` cannot run.
pub fn metal_step(root: &Path, args: &[&str]) -> Result<StepOutput, String> {
    let output = Command::new("swift")
        .current_dir(root)
        .arg(METAL_SCRIPT)
        .args(args)
        .envs(METAL_VALIDATION)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run swift: {e}"))?;
    Ok(StepOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Under `out`, the directory `layers`, holding the pinned layer's implicit manifest for
/// `sdk` and nothing else, and the empty directory `no-layers` ([`no_layer_dir`]), as
/// [`Run::environment`] takes them. Each is emptied first: the loader loads every manifest
/// in the implicit layer path.
///
/// # Errors
/// If a directory or the manifest cannot be written, or the SDK's manifest read.
pub fn layer_dirs(sdk: &Sdk, out: &Path) -> Result<[PathBuf; 2], String> {
    let layers = out.join("layers");
    remove(&layers)?;
    fs::create_dir_all(&layers).map_err(|e| format!("{}: {e}", layers.display()))?;
    let no_layers = no_layer_dir(out)?;
    let manifest = fs::read_to_string(&sdk.manifest)
        .map_err(|e| format!("{}: {e}", sdk.manifest.display()))?;
    let implicit = layers.join("VkLayer_khronos_validation.json");
    fs::write(&implicit, implicit_manifest(&manifest, &sdk.library)?)
        .map_err(|e| format!("{}: {e}", implicit.display()))?;
    Ok([layers, no_layers])
}

/// The empty directory `no-layers` under `out`, as [`Run::measured`] takes it, emptied of
/// anything an earlier hand put there: a manifest in it would load as an implicit layer.
///
/// # Errors
/// If the directory cannot be emptied or made.
pub fn no_layer_dir(out: &Path) -> Result<PathBuf, String> {
    let dir = out.join("no-layers");
    remove(&dir)?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// What one device test's process came to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    /// The suite's own word for the case, `passed` or `skipped, <why>`, where it printed one.
    pub said: Option<String>,
    /// Every message the test's sinks printed, each a line.
    pub messages: Vec<String>,
    /// Every line in which the driver, or Metal's validation under it, reported an error or
    /// a warning.
    pub driver: Vec<String>,
    /// Every line of the report [`METAL_CANARY`] must be heard to write, in its run.
    pub reports: Vec<String>,
    /// Why the test fails the rule, or `None` where it passes.
    pub failure: Option<String>,
}

/// The lines a driver writes for its errors and warnings: MoltenVK's, and Mesa's log.
const DRIVER_LINES: [&str; 5] =
    ["[mvk-error]", "[mvk-warn]", "MESA: error", "MESA: warning", "WARNING: "];

/// The lines Metal's validation writes for what it finds: shader validation's reports, API
/// validation's assertions; its warnings are the system log's lines (`logged`).
const METAL_LINES: [&str; 2] = ["Invalid Metal usage", "failed assertion"];

/// Whether `line` is one of the system log's, as `NSLog` writes it to standard error: the
/// date, the time to the millisecond, then the process's name and `[pid:thread]`.
fn logged(line: &str) -> bool {
    let shape = b"dddd-dd-dd dd:dd:dd.ddd ";
    line.len() > shape.len()
        && line.bytes().zip(shape).all(|(byte, &want)| match want {
            b'd' => byte.is_ascii_digit(),
            want => byte == want,
        })
        && line[shape.len()..].split(' ').next().is_some_and(|name| name.ends_with(']'))
}

/// Reads the output of the device test `test`, run alone, and judges it by decision 5.10's
/// rule: the process succeeded; the driver wrote no error or warning line; and its sinks
/// printed no message, or, for a canary, at least one of each of `expected` and nothing
/// else. Under Metal's validation (`metal`), it wrote no report, assertion or other line of
/// the log, and each of its validators printed its banner, unless the case says it skipped:
/// a case of the null backend's alone skips before it makes a device, and Metal never loads.
/// There [`METAL_CANARY`]'s test must write its report, and its report alone.
#[must_use]
pub fn judge(
    test: &str,
    succeeded: bool,
    output: &str,
    expected: Option<&[&str]>,
    metal: bool,
) -> Verdict {
    let messages: Vec<String> = output
        .lines()
        .filter_map(|line| line.find("message: Message {").map(|at| line[at..].to_owned()))
        .collect();
    let report = (metal && test == METAL_CANARY.0).then_some(METAL_CANARY.1);
    let reported = |line: &str| report.is_some_and(|words| line.contains(words));
    let metal_line = |line: &str| {
        metal
            && !reported(line)
            && (METAL_LINES.iter().any(|mark| line.contains(mark))
                || logged(line) && !METAL_BANNERS.iter().any(|banner| line.ends_with(banner)))
    };
    let reports: Vec<String> =
        output.lines().filter(|line| reported(line)).map(str::to_owned).collect();
    let driver: Vec<String> = output
        .lines()
        .filter(|line| DRIVER_LINES.iter().any(|mark| line.contains(mark)) || metal_line(line))
        .map(str::to_owned)
        .collect();
    let name = test.rsplit("::").next().unwrap_or(test);
    let said = output.lines().find_map(|line| {
        let at = line.find(&format!("{name}: "))?;
        let word = &line[at + name.len() + 2..];
        (word == "passed" || word.starts_with("skipped, ")).then(|| word.to_owned())
    });
    let skipped = said.as_deref().is_some_and(|word| word.starts_with("skipped, "));
    let unloaded = METAL_BANNERS.iter().find(|banner| {
        metal && !skipped && !output.lines().any(|line| logged(line) && line.ends_with(*banner))
    });
    let failure = if !succeeded {
        Some("the test failed".to_owned())
    } else if !driver.is_empty() {
        Some(format!("{} line(s) of the driver's or Metal's errors or warnings", driver.len()))
    } else if let Some(banner) = unloaded {
        Some(format!("Metal's validation did not print `{banner}`: is it loaded?"))
    } else if let Some(words) = report.filter(|_| reports.is_empty()) {
        Some(format!("Metal's report `{words}` was not heard: does it reach standard error?"))
    } else {
        match expected {
            None | Some(_) if skipped && !messages.is_empty() => {
                Some(format!("{} message(s) from a skipped case", messages.len()))
            }
            None if !messages.is_empty() => Some(format!("{} message(s)", messages.len())),
            None => None,
            // A canary skipped for what its device lacks, which it names.
            Some(_) if skipped => None,
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
    Verdict { said, messages, driver, reports, failure }
}

/// How long one device test's process may run before the run kills it and records it
/// failed. The longest device test on the M1 takes under three seconds under either
/// driver, so this is a margin of forty times, for a loaded machine and for a CPU driver
/// such as lavapipe, which runs the suite's dispatches several times slower; and the
/// thread-safety canary's watchdog aborts at 30 seconds, inside it, so that a hang there
/// records the watchdog's own line.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// How one device test's process ended, and what it wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ended {
    /// Whether it exited with success.
    pub succeeded: bool,
    /// Its exit status as std's `ExitStatus` words it: on unix `exit status: 101`, or
    /// `signal: 11 (SIGSEGV)` for a process a signal killed, with ` (core dumped)` where a
    /// core was written; on Windows `exit code: 101`, or `exit code: 0xc0000005`.
    pub status: String,
    /// Its standard output.
    pub stdout: String,
    /// Its standard error.
    pub stderr: String,
    /// The deadline it ran past, and was killed at, if it did.
    pub killed_at: Option<Duration>,
}

impl Ended {
    /// Its standard output, then its standard error: what [`judge`] reads.
    #[must_use]
    pub fn text(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Runs `command` with its standard output and error captured, and kills it where it runs
/// past `deadline`.
///
/// # Errors
///
/// The process cannot be started, waited on or killed.
pub fn run_bounded(command: &mut Command, deadline: Duration) -> io::Result<Ended> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    // Both streams are drained while it runs, so that a full pipe never stalls it.
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|s| Box::new(s) as _));
    let stderr = drain(child.stderr.take().map(|s| Box::new(s) as _));
    let started = Instant::now();
    let (status, killed_at) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, None);
        }
        if started.elapsed() >= deadline {
            child.kill()?;
            break (child.wait()?, Some(deadline));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let text = |bytes: thread::JoinHandle<Vec<u8>>| {
        String::from_utf8_lossy(&bytes.join().unwrap_or_default()).into_owned()
    };
    Ok(Ended {
        succeeded: status.success() && killed_at.is_none(),
        status: status.to_string(),
        stdout: text(stdout),
        stderr: text(stderr),
        killed_at,
    })
}

/// [`judge`] of an ended test: one killed at its deadline fails for that, whatever it wrote.
#[must_use]
pub fn judge_ended(test: &str, ended: &Ended, expected: Option<&[&str]>, metal: bool) -> Verdict {
    let mut verdict = judge(test, ended.succeeded, &ended.text(), expected, metal);
    if let Some(deadline) = ended.killed_at {
        verdict.failure = Some(format!("past its deadline of {} s", deadline.as_secs_f64()));
    }
    verdict
}

/// The most lines of a failed test's output its entry in the report holds, of its standard
/// output and its standard error together.
pub const TAIL_LINES: usize = 40;

/// The entry of the device test `test` of `package` in its run's report: its name and word,
/// then each message, report and driver line it printed, then each line of its output that
/// ends in `retrying`, which a device test prints where it ran a step again and still
/// passed (`crates/renderer/tests/device.rs`, `held`); for a failed test, how its process
/// ended and the last lines of its standard output and of its standard error, [`TAIL_LINES`]
/// at most between them, each half of them and a share one leaves to the other. The run
/// prints a failed test's entry to standard error too, then its [`output`] in full.
#[must_use]
pub fn entry(package: &str, test: &str, verdict: &Verdict, ended: &Ended) -> String {
    let mut entry = format!("{package} {test}: {}\n", verdict.word());
    for line in verdict.messages.iter().chain(&verdict.reports).chain(&verdict.driver) {
        entry.push_str(&format!("  {line}\n"));
    }
    let output = ended.stdout.lines().chain(ended.stderr.lines()).map(str::trim_end);
    for line in output.filter(|line| line.ends_with("retrying")) {
        entry.push_str(&format!("  {line}\n"));
    }
    if verdict.failure.is_none() {
        return entry;
    }
    entry.push_str(&format!("  ended with {}\n", ended.status));
    let out: Vec<&str> = ended.stdout.lines().collect();
    let err: Vec<&str> = ended.stderr.lines().collect();
    let from_out = out.len().min((TAIL_LINES / 2).max(TAIL_LINES.saturating_sub(err.len())));
    let from_err = err.len().min(TAIL_LINES - from_out);
    for (stream, lines, kept) in
        [("standard output", out, from_out), ("standard error", err, from_err)]
    {
        if lines.is_empty() {
            entry.push_str(&format!("  {stream}: nothing\n"));
            continue;
        }
        entry.push_str(&format!("  {stream}, its last {kept} of {} line(s):\n", lines.len()));
        for line in &lines[lines.len() - kept..] {
            entry.push_str(&format!("    {line}\n"));
        }
    }
    entry
}

/// The whole of what the failed device test `test` wrote, for the run's log: its standard
/// output, then its standard error, each in full under a heading of its own. The report keeps
/// the [`entry`] alone, whose tail is cut at [`TAIL_LINES`].
#[must_use]
pub fn output(test: &str, ended: &Ended) -> String {
    let mut output = String::new();
    for (stream, text) in [("standard output", &ended.stdout), ("standard error", &ended.stderr)] {
        if text.is_empty() {
            output.push_str(&format!("gpu: {test}, its {stream}: nothing\n"));
            continue;
        }
        output.push_str(&format!("gpu: {test}, its {stream} in full:\n{text}"));
        if !text.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

impl Verdict {
    /// The run file's word for the test: `FAILED, <why>`, the suite's own word, or `passed`.
    #[must_use]
    pub fn word(&self) -> String {
        match (&self.failure, &self.said) {
            (Some(why), _) => format!("FAILED, {why}"),
            (None, Some(said)) => said.clone(),
            (None, None) => "passed".to_owned(),
        }
    }
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

    fn value(run: &Run, name: &str) -> Option<String> {
        run.set.iter().find(|(set, _)| set == name).map(|(_, value)| value.clone())
    }

    /// A fresh directory under the system's temporary one, named for the test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-vulkan-{name}-{}", std::process::id()));
        remove(&dir).unwrap();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A measurement's environment loads no layer and sets no validation, and pins the
    /// driver's manifest as `gpu` does.
    #[test]
    fn a_measurement_runs_with_no_layer_on_the_pinned_driver() {
        let root = scratch("measure-sdk");
        let no_layers = no_layer_dir(&root).unwrap();
        let system = Run::measured(Driver::System, None, &no_layers, Some("Apple M1")).unwrap();
        let empty = Some(no_layers.to_string_lossy().into_owned());
        assert_eq!(value(&system, "VK_IMPLICIT_LAYER_PATH"), empty);
        assert_eq!(value(&system, "VK_LAYER_PATH"), empty);
        assert_eq!(value(&system, "LACTOSE_GPU_ADAPTER").as_deref(), Some("Apple M1"));
        assert_eq!(value(&system, "VK_DRIVER_FILES"), None);
        assert!(!system.metal && system.canaries.is_empty());
        for (name, _) in &system.set {
            assert!(!name.starts_with("VK_KHRONOS_VALIDATION_") && !name.starts_with("MTL_"));
        }
        let error = Run::measured(Driver::MoltenVk, None, &no_layers, None).err().expect("no SDK");
        assert!(error.contains("SDK"), "{error}");
        let icds = root.join("icd.d");
        fs::create_dir_all(&icds).unwrap();
        let sdk = Sdk {
            root: root.clone(),
            loader: Some(root.join("lib")),
            library: root.join("lib").join("layer"),
            manifest: root.join("manifest.json"),
            icds: Some(icds.clone()),
        };
        let missing = Run::measured(Driver::KosmicKrisp, Some(&sdk), &no_layers, None);
        let error = missing.err().expect("no manifest");
        assert!(error.contains("no such driver manifest"), "{error}");
        fs::write(icds.join("libkosmickrisp_icd.json"), "{}").unwrap();
        let krisp = Run::measured(Driver::KosmicKrisp, Some(&sdk), &no_layers, None).unwrap();
        let manifest = icds.join("libkosmickrisp_icd.json");
        assert_eq!(value(&krisp, "VK_DRIVER_FILES"), Some(manifest.to_string_lossy().into()));
        let loader = root.join("lib").to_string_lossy().into_owned();
        assert!(krisp.set.iter().any(|(_, value)| *value == loader));
        assert_eq!(value(&krisp, "LACTOSE_GPU_ADAPTER"), None);
        // What an earlier hand left in the empty directory is gone: it would load as a layer.
        fs::write(no_layers.join("a-layer.json"), "{}").unwrap();
        assert_eq!(fs::read_dir(no_layer_dir(&root).unwrap()).unwrap().count(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    /// D165: the loader reads a layer the registry or a system directory holds beside one
    /// `VK_ADD_IMPLICIT_LAYER_PATH` adds, and warns of each layer `VK_LOADER_LAYERS_DISABLE`
    /// skips. No run, gated or measured, on any driver or host, sets either; each sets the
    /// exclusive implicit path, and both are cleared from what it inherits.
    #[test]
    fn no_run_adds_to_the_machines_layers_or_disables_them() {
        let root = scratch("exclusive");
        let [layers, no_layers] = [root.join("layers"), no_layer_dir(&root).unwrap()];
        let icds = root.join("icd.d");
        fs::create_dir_all(&icds).unwrap();
        for icd in ["MoltenVK_icd.json", "libkosmickrisp_icd.json", "lvp_icd.x86_64.json"] {
            fs::write(icds.join(icd), "{}").unwrap();
        }
        let sdk = |host| Sdk { icds: Some(icds.clone()), ..Sdk::at(host, root.clone()) };
        let windows = Sdk::at(Host::Windows, root.clone());
        let gated = |driver, sdk: &Sdk, validators| {
            Run::environment(driver, sdk, [&layers, &no_layers, &root], validators, None).unwrap()
        };
        let mut runs = Vec::new();
        for driver in [Driver::Lavapipe, Driver::MoltenVk, Driver::KosmicKrisp, Driver::System] {
            for validators in [Validators::Plain, Validators::GpuAssisted] {
                runs.push((gated(driver, &sdk(Host::MacOs), validators), &layers));
                runs.push((gated(driver, &sdk(Host::Linux), validators), &layers));
            }
            let measured = Run::measured(driver, Some(&sdk(Host::Linux)), &no_layers, None);
            runs.push((measured.unwrap(), &no_layers));
        }
        runs.push((gated(Driver::System, &windows, Validators::Plain), &layers));
        let measured = Run::measured(Driver::System, Some(&windows), &no_layers, None);
        runs.push((measured.unwrap(), &no_layers));
        runs.push((Run::measured(Driver::System, None, &no_layers, None).unwrap(), &no_layers));
        for (run, implicit) in &runs {
            for name in ["VK_ADD_IMPLICIT_LAYER_PATH", "VK_LOADER_LAYERS_DISABLE"] {
                assert_eq!(value(run, name), None, "{name} in {:?}", run.set);
            }
            let implicit = Some(implicit.to_string_lossy().into_owned());
            assert_eq!(value(run, "VK_IMPLICIT_LAYER_PATH"), implicit, "{:?}", run.set);
            assert_eq!(value(run, "VK_LAYER_PATH"), Some(no_layers.to_string_lossy().into()));
        }
        let inherited = ["VK_ADD_IMPLICIT_LAYER_PATH", "VK_LOADER_LAYERS_DISABLE"];
        assert_eq!(cleared(inherited.map(str::to_owned)), inherited);
        fs::remove_dir_all(&root).unwrap();
    }

    /// On Windows a run loads the system's loader, which the GPU's driver installs, and the
    /// system's drivers: it sets no library path, which would make Windows look for DLLs where
    /// the SDK is, and names no driver manifest. A driver the SDK ships elsewhere is refused.
    #[test]
    fn on_windows_a_run_takes_the_systems_loader_and_drivers() {
        let root = PathBuf::from(r"C:\VulkanSDK\1.4.363.1");
        let sdk = Sdk::at(Host::Windows, root.clone());
        assert_eq!((sdk.loader.as_ref(), sdk.icds.as_ref()), (None, None));
        assert_eq!(sdk.library, root.join("Bin").join("VkLayer_khronos_validation.dll"));
        assert_eq!(sdk.manifest, root.join("Bin").join("VkLayer_khronos_validation.json"));
        let dirs = [Path::new("layers"), Path::new("no-layers"), Path::new("shaders")];
        let run =
            Run::environment(Driver::System, &sdk, dirs, Validators::GpuAssisted, Some("AMD"));
        let run = run.unwrap();
        let names: Vec<&str> = run.set.iter().map(|(name, _)| name.as_str()).collect();
        for name in ["LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "PATH", "VK_DRIVER_FILES"] {
            assert!(!names.contains(&name), "{name} in {names:?}");
        }
        assert_eq!(value(&run, "VK_IMPLICIT_LAYER_PATH").as_deref(), Some("layers"));
        assert_eq!(value(&run, "LACTOSE_GPU_ADAPTER").as_deref(), Some("AMD"));
        for driver in [Driver::Lavapipe, Driver::MoltenVk, Driver::KosmicKrisp] {
            let error =
                Run::environment(driver, &sdk, dirs, Validators::Plain, None).err().unwrap();
            assert!(error.contains("`--driver system`"), "{error}");
            let measured = Run::measured(driver, Some(&sdk), dirs[1], None).err().unwrap();
            assert!(measured.contains("`--driver system`"), "{measured}");
        }
    }

    /// Where each host's SDK is: `VULKAN_SDK` first on the hosts an installer serves, the
    /// installer's own place without it; on Linux the unpacked archive whatever is set.
    #[test]
    fn the_sdk_is_where_its_host_keeps_it() {
        let pin = Pin::parse(PIN).unwrap();
        let target = Path::new("target");
        let set = || Some(OsString::from("elsewhere"));
        let home = || Some(OsString::from("home"));
        let root = |host, vulkan_sdk, home| sdk_root(host, &pin, target, vulkan_sdk, home);
        let unpacked = target.join("vulkan-sdk/1.4.363.1/1.4.363.1/x86_64");
        assert_eq!(root(Host::Linux, set(), None).unwrap(), unpacked);
        assert_eq!(root(Host::MacOs, set(), home()).unwrap(), PathBuf::from("elsewhere"));
        let installed = Path::new("home/VulkanSDK/1.4.363.1/macOS");
        assert_eq!(root(Host::MacOs, None, home()).unwrap(), installed);
        assert!(root(Host::MacOs, None, None).unwrap_err().contains("HOME"));
        assert_eq!(root(Host::Windows, set(), None).unwrap(), PathBuf::from("elsewhere"));
        let installed = PathBuf::from(r"C:\VulkanSDK\1.4.363.1");
        assert_eq!(root(Host::Windows, None, home()).unwrap(), installed);
        let names = [Host::Linux, Host::MacOs, Host::Windows].map(Host::name);
        assert_eq!(names, HOSTS);
    }

    /// An SDK is held to the pin by its manifest before its library: the installer's
    /// `VULKAN_SDK` names the latest SDK installed, whose library would only fail a digest,
    /// where its manifest says which SDK it is. An SDK that is not there names the installer
    /// on Windows, and the tool fetches nothing.
    #[test]
    fn an_sdk_is_held_to_the_pin_by_its_manifest_then_its_library() {
        let pin = Pin::parse(PIN).unwrap();
        let root = scratch("check");
        let sdk = Sdk::at(Host::Windows, root.clone());
        let absent = check(&pin, Host::Windows, &sdk).unwrap_err();
        let installer = &pin.host["windows-x86_64"];
        for expected in [installer.url.as_deref().unwrap(), installer.sha256.as_deref().unwrap()] {
            assert!(absent.contains(expected), "{absent}");
        }
        assert!(absent.contains("not installed at") && absent.contains("does not fetch"));
        let mac = check(&pin, Host::MacOs, &Sdk::at(Host::MacOs, root.clone())).unwrap_err();
        assert!(mac.contains("not installed at") && !mac.contains("https://"), "{mac}");
        fs::create_dir_all(root.join("Bin")).unwrap();
        let manifest = |api: &str| {
            format!(
                r#"{{"file_format_version": "1.2.0", "layer": {{"name": "{LAYER}", "api_version": "{api}", "library_path": ".\\VkLayer_khronos_validation.dll"}}}}"#
            )
        };
        // Another SDK's manifest, no library beside it: refused by the version it names.
        fs::write(&sdk.manifest, manifest("1.4.370")).unwrap();
        let other = check(&pin, Host::Windows, &sdk).unwrap_err();
        assert!(other.contains("1.4.370") && other.contains("another SDK"), "{other}");
        fs::write(&sdk.manifest, manifest(&pin.api_version)).unwrap();
        let no_library = check(&pin, Host::Windows, &sdk).unwrap_err();
        assert!(no_library.starts_with(&sdk.library.display().to_string()), "{no_library}");
        fs::write(&sdk.library, "not the layer").unwrap();
        let wrong = check(&pin, Host::Windows, &sdk).unwrap_err();
        assert!(wrong.contains(&format!("pins {}", installer.library_sha256)), "{wrong}");
        // The pin's digest held by a stand-in: the check passes on the digest alone.
        let mut stand_in = pin.clone();
        let digest = digest_of(&sdk.library).unwrap();
        stand_in.host.get_mut("windows-x86_64").unwrap().library_sha256 = digest;
        assert_eq!(check(&stand_in, Host::Windows, &sdk), Ok(()));
        fs::remove_dir_all(&root).unwrap();
    }

    /// The Windows manifest names its library relative to itself; the implicit copy names
    /// it absolutely, as the loader reads it from another directory.
    #[test]
    fn the_windows_manifest_is_made_implicit_with_its_library_absolute() {
        let manifest = r#"{"file_format_version": "1.2.0", "layer": {"name": "VK_LAYER_KHRONOS_validation", "api_version": "1.4.363", "library_path": ".\\VkLayer_khronos_validation.dll"}}"#;
        let library = r"C:\VulkanSDK\1.4.363.1\Bin\VkLayer_khronos_validation.dll";
        let made = implicit_manifest(manifest, Path::new(library)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&made).unwrap();
        assert_eq!(value["layer"]["library_path"], library);
    }

    #[test]
    fn the_tracked_pin_reads() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(pin.host.len(), HOSTS.len());
        let windows = &pin.host["windows-x86_64"];
        assert!(windows.url.as_deref().unwrap().ends_with("vulkansdk-windows-X64-1.4.363.1.exe"));
    }

    #[test]
    fn a_pin_missing_a_host_or_not_of_its_hosts_form_is_refused() {
        let missing = PIN.replace("[host.macos-aarch64]", "[host.macos-x86_64]");
        assert!(Pin::parse(&missing).unwrap_err().contains("the hosts are"));
        let fetched = PIN.replace(
            "[host.macos-aarch64]\n",
            "[host.macos-aarch64]\nurl = \"https://example.invalid/a\"\n",
        );
        assert!(Pin::parse(&fetched).unwrap_err().contains("takes no"), "{fetched}");
        // Windows names its installer, the url with the digest.
        let pin = Pin::parse(PIN).unwrap();
        let installer = pin.host["windows-x86_64"].sha256.clone().unwrap();
        let unnamed = PIN.replace(&format!("sha256 = \"{installer}\"\n"), "");
        assert!(Pin::parse(&unnamed).unwrap_err().contains("windows-x86_64 needs"), "{unnamed}");
        let archive = PIN.replace("/windows/vulkansdk-windows-X64-", "/windows/vulkansdk-linux-");
        assert!(Pin::parse(&archive).unwrap_err().contains("url does not end in"));
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
        let verdict = judge("conformance::a_case", true, CLEAN, None, false);
        assert_eq!(verdict.failure, None);
        assert_eq!(verdict.said.as_deref(), Some("passed"));
        let skipped = "test conformance::b ... b: skipped, no window\nok\n";
        assert_eq!(
            judge("conformance::b", true, skipped, None, false).said.as_deref(),
            Some("skipped, no window")
        );
    }

    #[test]
    fn a_message_or_a_driver_line_fails_a_case_that_passed() {
        let sent = format!("{CLEAN}{}", message("VUID-x"));
        assert!(
            judge("conformance::a_case", true, &sent, None, false)
                .failure
                .unwrap()
                .contains("1 message")
        );
        let warned = format!("{CLEAN}[mvk-warn] something\n");
        assert!(
            judge("conformance::a_case", true, &warned, None, false)
                .failure
                .unwrap()
                .contains("driver")
        );
        assert!(judge("conformance::a_case", false, CLEAN, None, false).failure.is_some());
    }

    #[test]
    fn a_canary_passes_on_its_messages_alone_and_fails_silent_or_with_another() {
        let ids: &[&str] = &["A", "B"];
        let both = format!("{}{}{}", message("A"), message("B"), message("A"));
        assert_eq!(judge("device::canary", true, &both, Some(ids), false).failure, None);
        let silent =
            judge("device::canary", true, &message("A"), Some(ids), false).failure.unwrap();
        assert!(silent.contains("B was not heard"), "{silent}");
        let other = format!("{both}{}", message("C"));
        assert!(
            judge("device::canary", true, &other, Some(ids), false)
                .failure
                .unwrap()
                .contains("not the canary's")
        );
        // An identifier that merely begins with the canary's is another message.
        let longer = format!("{both}{}", message("A-and-more"));
        assert!(judge("device::canary", true, &longer, Some(ids), false).failure.is_some());
    }

    #[test]
    fn a_canary_may_skip_for_a_reason_and_send_nothing() {
        let ids: &[&str] = &["A"];
        let skipped = "test device::canary ... canary: skipped, one queue\nok\n";
        let verdict = judge("device::canary", true, skipped, Some(ids), false);
        assert_eq!(verdict.failure, None);
        assert_eq!(verdict.said.as_deref(), Some("skipped, one queue"));
        let loud = format!("{skipped}{}", message("A"));
        assert!(judge("device::canary", true, &loud, Some(ids), false).failure.is_some());
        assert!(judge("conformance::b", true, &loud, None, false).failure.is_some());
    }

    /// What a process under Metal's validation writes on loading it, through the system log.
    const BANNERS: &str = "2026-11-19 04:55:04.377 integration-0392d6b825cbf81d[22328:15687764] \
                           Metal API Validation Enabled\n2026-11-19 04:55:04.377 \
                           integration-0392d6b825cbf81d[22328:15687764] Metal GPU Validation \
                           Enabled\n";

    #[test]
    fn under_metal_validation_a_case_passes_with_both_banners_and_nothing_else() {
        let clean = format!("{BANNERS}{CLEAN}");
        assert_eq!(judge("conformance::a_case", true, &clean, None, true).failure, None);
        // Not loaded: a run that set the variables and heard no banner proves nothing.
        let silent = judge("conformance::a_case", true, CLEAN, None, true).failure.unwrap();
        assert!(silent.contains("Metal API Validation Enabled"), "{silent}");
        let half = format!("{}\n{CLEAN}", BANNERS.lines().next().unwrap());
        let half = judge("conformance::a_case", true, &half, None, true).failure.unwrap();
        assert!(half.contains("Metal GPU Validation Enabled"), "{half}");
        // A report of shader validation's, as it wrote one for a read past a buffer.
        let report = format!(
            "{clean}Invalid Metal usage: Invalid load in 'device' or 'constant' address space at \
             offset 256, executing compute function: \"cs_main\" \n"
        );
        let verdict = judge("conformance::a_case", true, &report, None, true);
        assert_eq!(verdict.driver.len(), 1);
        assert!(verdict.failure.unwrap().contains("Metal's errors or warnings"));
        // Any other line of the log is a warning of API validation's.
        let warned = format!(
            "{clean}2026-11-19 04:55:04.390 integration-0392d6b825cbf81d[22328:15687764] a \
             warning\n"
        );
        assert!(judge("conformance::a_case", true, &warned, None, true).failure.is_some());
        // A case that skips may make no device, and Metal never loads; it still writes
        // nothing.
        let skipped = "test conformance::b ... b: skipped, null alone\nok\n";
        assert_eq!(judge("conformance::b", true, skipped, None, true).failure, None);
        let skipped_warned = format!("{skipped}{}", warned.lines().last().unwrap());
        assert!(judge("conformance::b", true, &skipped_warned, None, true).failure.is_some());
        // Without Metal's validation neither is looked for.
        assert_eq!(judge("conformance::a_case", true, &warned, None, false).failure, None);
    }

    /// A stand-in for a hung test, `sleep`, is killed at its deadline and recorded failed for
    /// it; one that ends inside its deadline is judged as before.
    #[cfg(unix)]
    #[test]
    fn a_test_past_its_deadline_is_killed_and_recorded_failed() {
        let started = Instant::now();
        let ended = run_bounded(Command::new("sleep").arg("600"), Duration::from_millis(200))
            .expect("sleep runs");
        assert!(started.elapsed() < Duration::from_secs(60), "not killed at its deadline");
        assert_eq!(ended.killed_at, Some(Duration::from_millis(200)));
        assert!(!ended.succeeded);
        let verdict = judge_ended("device::a_case", &ended, None, false);
        assert_eq!(verdict.word(), "FAILED, past its deadline of 0.2 s");
        let canary = judge_ended("device::a_canary", &ended, Some(&["AN-ID"]), false);
        assert_eq!(canary.word(), "FAILED, past its deadline of 0.2 s");
        let mut command = Command::new("sh");
        command.args(["-c", "echo 'test device::a_case ... a_case: passed'"]);
        let ended = run_bounded(&mut command, Duration::from_secs(60)).expect("sh runs");
        assert_eq!((ended.succeeded, ended.killed_at), (true, None));
        assert_eq!(judge_ended("device::a_case", &ended, None, false).word(), "passed");
    }

    fn ended(status: &str, stdout: &str, stderr: &str) -> Ended {
        Ended {
            succeeded: status == "exit status: 0",
            status: status.to_owned(),
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
            killed_at: None,
        }
    }

    /// A process that died silently, as the nightly's heap canary did under lavapipe: its
    /// entry says how it ended and what little it wrote, where the word alone said neither.
    #[test]
    fn a_failed_tests_entry_says_how_it_ended_and_what_it_wrote() {
        let test = "device::a_canary";
        let died =
            ended("signal: 11 (SIGSEGV)", "\nrunning 1 test\ntest device::a_canary ... ", "");
        let verdict = judge_ended(test, &died, Some(&["AN-ID"]), false);
        assert_eq!(
            entry("a-package", test, &verdict, &died),
            "a-package device::a_canary: FAILED, the test failed\n  ended with signal: 11 \
             (SIGSEGV)\n  standard output, its last 3 of 3 line(s):\n    \n    running 1 \
             test\n    test device::a_canary ... \n  standard error: nothing\n"
        );
        // Where the system wrote a core, std's wording says so, and the entry keeps it.
        let dumped = ended("signal: 11 (SIGSEGV) (core dumped)", "", "");
        let verdict = judge_ended(test, &dumped, Some(&["AN-ID"]), false);
        let dumped = entry("a-package", test, &verdict, &dumped);
        assert!(dumped.contains("  ended with signal: 11 (SIGSEGV) (core dumped)\n"), "{dumped}");
        // A passing test's entry is its word and messages alone.
        let passed = ended("exit status: 0", &message("AN-ID"), "");
        let verdict = judge_ended(test, &passed, Some(&["AN-ID"]), false);
        assert_eq!(
            entry("a-package", test, &verdict, &passed),
            format!("a-package device::a_canary: passed\n  {}", message("AN-ID"))
        );
        // A passing test's retries are kept beside its word, from either stream.
        let retried = ended(
            "exit status: 0",
            "test device::a_canary ... ok\n",
            "device::a_canary: Lag at 2 rounds was reached before the release: retrying\n",
        );
        let verdict = judge_ended(test, &retried, None, false);
        assert_eq!(
            entry("a-package", test, &verdict, &retried),
            "a-package device::a_canary: passed\n  device::a_canary: Lag at 2 rounds was \
             reached before the release: retrying\n"
        );
        // A message fails a case whose process succeeded: its entry has the tail too.
        let sent = ended("exit status: 0", &format!("{CLEAN}{}", message("VUID-x")), "");
        let verdict = judge_ended("conformance::a_case", &sent, None, false);
        let sent = entry("a-package", "conformance::a_case", &verdict, &sent);
        assert!(sent.contains("FAILED, 1 message(s)\n  message: Message {"), "{sent}");
        assert!(sent.contains("ended with exit status: 0\n"), "{sent}");
    }

    /// The log has a failed test's whole output, past the entry's tail: each stream as it was
    /// written, under its heading, a line it left open ended.
    #[test]
    fn a_failed_tests_whole_output_is_for_the_log() {
        let lines: String = (1..=100).map(|at| format!("err {at}\n")).collect();
        let died = ended("signal: 6 (SIGABRT)", "running 1 test\ntest device::a ... ", &lines);
        assert_eq!(
            output("device::a", &died),
            format!(
                "gpu: device::a, its standard output in full:\nrunning 1 test\ntest device::a \
                 ... \ngpu: device::a, its standard error in full:\n{lines}"
            )
        );
        let silent = ended("signal: 11 (SIGSEGV)", "", "");
        assert_eq!(
            output("device::a", &silent),
            "gpu: device::a, its standard output: nothing\ngpu: device::a, its standard error: \
             nothing\n"
        );
    }

    /// The tail holds [`TAIL_LINES`] at most: each stream half, and what one leaves the other.
    #[test]
    fn a_failed_tests_tail_is_the_last_lines_of_each_stream() {
        let numbered = |name: &str, count: usize| -> String {
            (1..=count).map(|at| format!("{name} {at}\n")).collect()
        };
        let tail = |out: usize, err: usize| {
            let failed = ended("exit status: 101", &numbered("out", out), &numbered("err", err));
            let verdict = judge_ended("device::a_case", &failed, None, false);
            entry("a-package", "device::a_case", &verdict, &failed)
        };
        let short_out = tail(2, 100);
        assert!(short_out.contains("standard output, its last 2 of 2 line(s):\n    out 1\n"));
        assert!(short_out.contains("standard error, its last 38 of 100 line(s):\n    err 63\n"));
        assert!(short_out.ends_with("    err 100\n"), "{short_out}");
        let both_long = tail(100, 100);
        assert!(both_long.contains("its last 20 of 100 line(s):\n    out 81\n"), "{both_long}");
        assert!(both_long.contains("its last 20 of 100 line(s):\n    err 81\n"), "{both_long}");
        let short_err = tail(100, 3);
        assert!(short_err.contains("its last 37 of 100 line(s):\n    out 64\n"), "{short_err}");
        assert!(short_err.contains("its last 3 of 3 line(s):\n    err 1\n"), "{short_err}");
        for entry in [short_out, both_long, short_err] {
            assert_eq!(entry.lines().filter(|line| line.starts_with("    ")).count(), TAIL_LINES);
        }
    }

    /// A real process a signal kills is worded so by its status, its two streams kept apart.
    #[cfg(unix)]
    #[test]
    fn a_process_a_signal_killed_is_recorded_with_its_signal() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo out; echo err >&2; kill -SEGV $$"]);
        let died = run_bounded(&mut command, Duration::from_secs(60)).expect("sh runs");
        assert!(!died.succeeded);
        assert_eq!((died.stdout.as_str(), died.stderr.as_str()), ("out\n", "err\n"));
        // ` (core dumped)` follows where the system writes a core.
        assert!(died.status.starts_with("signal: 11 (SIGSEGV)"), "{}", died.status);
        let verdict = judge_ended("device::a_case", &died, None, false);
        let entry = entry("a-package", "device::a_case", &verdict, &died);
        assert!(entry.contains(&format!("  ended with {}\n", died.status)), "{entry}");
    }

    #[test]
    fn metals_canary_passes_on_its_report_alone_and_fails_without_it() {
        let (test, words) = METAL_CANARY;
        let clean = format!("{BANNERS}{CLEAN}");
        let report = format!(
            "{clean}Invalid Metal usage: {words} at offset 256, executing compute function:              \"cs_main\" \n"
        );
        let heard = judge(test, true, &report, Some(&[]), true);
        assert_eq!(heard.failure, None);
        assert_eq!((heard.reports.len(), heard.driver.len()), (1, 0));
        let silent = judge(test, true, &clean, Some(&[]), true).failure.unwrap();
        assert!(silent.contains("was not heard"), "{silent}");
        // Another case's report fails it, and another report fails the canary.
        assert!(judge("device::a_case", true, &report, None, true).failure.is_some());
        let other = format!("{report}Invalid Metal usage: Invalid device store at offset 8\n");
        assert!(judge(test, true, &other, Some(&[]), true).failure.is_some());
        // The layer hears nothing of it.
        let sent = format!("{report}{}", message("VUID-x"));
        assert!(judge(test, true, &sent, Some(&[]), true).failure.is_some());
    }

    #[test]
    fn a_log_line_is_known_by_its_shape() {
        assert!(logged(BANNERS.lines().next().unwrap()));
        assert!(!logged("2026-11-19 04:55:04.377 no process"));
        assert!(!logged("test conformance::a_case ... a_case: passed"));
        assert!(!logged("2026-11-19"));
    }

    #[test]
    fn metal_validation_is_moltenvks_alone() {
        let under: Vec<Driver> =
            [Driver::Lavapipe, Driver::MoltenVk, Driver::KosmicKrisp, Driver::System]
                .into_iter()
                .filter(|driver| driver.metal_validation())
                .collect();
        assert_eq!(under, [Driver::MoltenVk]);
        assert_eq!(MACOS_GATE, [Driver::MoltenVk, Driver::KosmicKrisp]);
    }

    #[test]
    fn a_run_clears_every_inherited_setting_of_the_loader_and_the_layer() {
        let inherited = [
            "PATH",
            "VK_KHRONOS_VALIDATION_MESSAGE_ID_FILTER",
            "VK_LOADER_DEBUG",
            "LACTOSE_VALIDATION_OFF",
            "VK_ADD_LAYER_PATH",
            "VK_DRIVER_FILES",
            "MVK_CONFIG_LOG_LEVEL",
            "VK_KHRONOS_VALIDATION_ENABLES",
            "MTL_DEBUG_LAYER",
            "VK_LAYER_MESSAGE_ID_FILTER",
            "VK_LAYER_SETTINGS_PATH",
            "MVK_CONFIG_USE_METAL_ARGUMENT_BUFFERS",
            "MESA_DEBUG",
            "METAL_DEVICE_WRAPPER_TYPE",
            "HOME",
        ];
        assert_eq!(
            cleared(inherited.map(str::to_owned)),
            [
                "LACTOSE_VALIDATION_OFF",
                "MESA_DEBUG",
                "METAL_DEVICE_WRAPPER_TYPE",
                "MTL_DEBUG_LAYER",
                "MVK_CONFIG_LOG_LEVEL",
                "MVK_CONFIG_USE_METAL_ARGUMENT_BUFFERS",
                "VK_ADD_LAYER_PATH",
                "VK_DRIVER_FILES",
                "VK_KHRONOS_VALIDATION_ENABLES",
                "VK_KHRONOS_VALIDATION_MESSAGE_ID_FILTER",
                "VK_LAYER_MESSAGE_ID_FILTER",
                "VK_LAYER_SETTINGS_PATH",
                "VK_LOADER_DEBUG",
            ]
        );
    }

    #[test]
    fn every_canary_is_known_as_one() {
        assert!(is_canary("device::canary_core_validation_reports_a_wrong_layout"));
        assert!(is_canary("device::canary_gpu_assisted_validation_reports_a_read_past_a_buffer"));
        assert!(!is_canary("device::device_buffer_is_filled_through_a_staging_copy"));
    }

    /// A device test named as a canary that no run lists would be judged as a test that must
    /// send nothing, and fail every run.
    #[test]
    fn every_canary_the_device_tests_name_is_listed() {
        // Each file of device tests, and the module path its tests are named under.
        for (file, module) in [("tests/device.rs", "device::"), ("src/canary.rs", "canary::")] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/rhi-vulkan");
            let source = fs::read_to_string(path.join(file)).expect("rhi-vulkan's device tests");
            let named: Vec<&str> = source
                .lines()
                .filter_map(|line| line.trim().strip_prefix("fn canary_"))
                .filter_map(|rest| rest.split('(').next())
                .collect();
            for name in &named {
                let test = format!("{module}canary_{name}");
                assert!(is_canary(&test), "{test} is in no list");
            }
            let listed = [Validators::Plain, Validators::GpuAssisted]
                .iter()
                .flat_map(|validators| validators.canaries())
                .map(|(name, _)| *name)
                .chain([METAL_CANARY.0])
                .filter(|name| name.starts_with(module))
                .count();
            assert_eq!(named.len(), listed, "a listed canary is not in {file}: {named:?}");
        }
    }
}
