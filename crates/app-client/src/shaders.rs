//! The cooked shaders and the pipeline cache file (`crates/app/README.md`, "What
//! `lactose-app-client` wires", step 3). The shaders are `tools/cook`'s output for
//! `shaders/`, one file `<source>.<entry>.spirv` each, in the directory the binary is given:
//! `cargo xtask gpu` writes it under `target/gpu/shaders`.

use std::path::{Path, PathBuf};

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::CookedShader;

/// The stages the client's pipelines are made from.
pub struct CookedShaders {
    /// The renderer's scatter pass.
    pub scatter: CookedShader,
    /// The renderer's triangle pass, its vertex stage.
    pub vertex: CookedShader,
    /// The renderer's triangle pass, its fragment stage.
    pub fragment: CookedShader,
    /// The present thread's pass (`shaders/client/present.slang`), its vertex stage.
    pub present_vertex: CookedShader,
    /// The present thread's pass, its fragment stage.
    pub present_fragment: CookedShader,
}

impl CookedShaders {
    /// The shaders in `directory`.
    ///
    /// # Errors
    ///
    /// A file that cannot be read, or that does not decode as a cooked shader, named.
    pub fn read(directory: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<CookedShader> {
            let path = directory.join(format!("{name}.spirv"));
            let bytes = std::fs::read(&path).map_err(|error| {
                Error::new(ErrorKind::NotFound, format!("{}: {error}", path.display()))
            })?;
            CookedShader::decode(&bytes)
        };
        Ok(Self {
            scatter: read("renderer/scatter.cs_main")?,
            vertex: read("renderer/triangle.vs_main")?,
            fragment: read("renderer/triangle.fs_main")?,
            present_vertex: read("client/present.vs_main")?,
            present_fragment: read("client/present.fs_main")?,
        })
    }
}

/// The pipeline cache file: last run's bytes for the device's constructor, and the device's
/// bytes written back after the pipelines are made. The backend checks what it is handed and
/// starts empty from bytes it rejects (`PipelineStats::cache`).
pub struct PipelineCache {
    path: PathBuf,
}

impl PipelineCache {
    /// The cache kept at `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file's bytes: empty where there is no file, or it cannot be read.
    #[must_use]
    pub fn load(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap_or_default()
    }

    /// Writes `bytes` to the file, through a file beside it renamed over it, so that a run
    /// cut short leaves the old bytes or the new and never a mixture.
    ///
    /// # Errors
    ///
    /// The file system's.
    pub fn store(&self, bytes: &[u8]) -> Result<()> {
        let io = |error: std::io::Error| {
            Error::new(ErrorKind::Io, format!("{}: {error}", self.path.display()))
        };
        if let Some(parent) = self.path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut partial = self.path.clone().into_os_string();
        partial.push(".partial");
        std::fs::write(&partial, bytes).map_err(io)?;
        std::fs::rename(&partial, &self.path).map_err(io)
    }
}
