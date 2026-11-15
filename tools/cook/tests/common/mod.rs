//! What the cooker's tests share: an executor and a pool, a directory of the test's own
//! under `CARGO_TARGET_TMPDIR`, the committed sources, and the binary run as a process.

#![allow(dead_code, reason = "each test binary uses part of this")]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use lactose_io::{Io, IoConfig};
use lactose_task::Executor;

/// An executor, a pool and an empty directory of the test's own.
pub struct Env {
    pub executor: Executor,
    pub io: Io,
    pub dir: PathBuf,
}

impl Env {
    pub fn new(name: &str) -> Self {
        let executor = Executor::builder().build().expect("an executor");
        let platform = executor.platform().as_ref();
        let io = Io::start(platform, IoConfig::for_platform(platform)).expect("the pool");
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cook").join(name);
        // A previous run's files would be read as this run's.
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("{} not removed: {error}", dir.display()),
        }
        executor.block_on(io.create_dir_all(dir.clone())).expect("the test's directory");
        Self { executor, io, dir }
    }

    pub fn wait<F: Future>(&self, future: F) -> F::Output {
        self.executor.block_on(future)
    }

    /// Writes `files`, each a path below `root` and its content, in the order given.
    pub fn write_tree(&self, root: &Path, files: &[(&str, &[u8])]) {
        for (path, bytes) in files {
            let path = root.join(path);
            self.wait(self.io.create_dir_all(path.parent().unwrap())).unwrap();
            self.wait(self.io.replace(path, bytes.to_vec())).unwrap();
        }
    }

    pub fn read(&self, path: &Path) -> Vec<u8> {
        self.wait(self.io.read_file(path)).unwrap()
    }
}

/// The committed sources: `characters/knight.gltf`.
pub fn sources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("sources")
}

/// The committed knight source's bytes.
pub fn knight() -> Vec<u8> {
    std::fs::read(sources().join("characters").join("knight.gltf")).expect("the fixture")
}

/// Runs the `ricotta` binary on `sources` and `pack`, asserts it succeeded, and returns
/// what it printed.
pub fn run_binary(sources: &Path, pack: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ricotta"))
        .arg(sources)
        .arg(pack)
        .output()
        .expect("ricotta runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ricotta failed: {}\n{stderr}", output.status);
    assert!(stderr.is_empty(), "ricotta wrote to stderr: {stderr}");
    String::from_utf8(output.stdout).expect("UTF-8")
}
