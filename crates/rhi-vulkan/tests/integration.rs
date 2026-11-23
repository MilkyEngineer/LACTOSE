//! The integration tests of `lactose-rhi-vulkan` as one test binary (D150): each file beside
//! this one is a module of it, named after the file. What they share is declared here.
//!
//! Every test that makes a device is ignored by a plain run, which may have no driver;
//! `cargo xtask gpu` runs them on the driver it names.

#![forbid(unsafe_code)]

mod conformance;
mod device;
mod swapchain;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    CookedShader, DeviceRequest, HEAP_SLOTS_DEFAULT, Message, MessageSink, SAMPLERS_DEFAULT,
};

/// A sink that counts what it is sent and keeps its text, for a test to assert on.
#[derive(Default)]
struct Counting {
    count: AtomicUsize,
    texts: Mutex<Vec<String>>,
}

impl Counting {
    fn count(&self) -> usize {
        // Relaxed: read after the calls that send have returned.
        self.count.load(Ordering::Relaxed)
    }

    fn texts(&self) -> Vec<String> {
        self.texts.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl MessageSink for Counting {
    fn message(&self, message: &Message<'_>) {
        self.count.fetch_add(1, Ordering::Relaxed);
        eprintln!("message: {message:?}");
        self.texts.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
    }
}

/// The request of a device on the adapter `LACTOSE_GPU_ADAPTER` names, or the loader's first,
/// with the default heap and sampler table and no surface.
fn request(messages: Arc<dyn MessageSink>) -> DeviceRequest {
    DeviceRequest {
        adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
        surface: None,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages,
    }
}

/// The suite's cooked shader `name`, `<source>.<entry>`: what `ricotta --shaders` wrote into
/// the directory `LACTOSE_GPU_SHADERS` names for `shaders/conformance/<source>.slang`'s entry
/// point and the SPIR-V target.
fn suite_shader(name: &str) -> Result<CookedShader> {
    cooked(&format!("conformance/{name}"))
}

/// The engine's cooked shader `name`, `<source>.<entry>`, of `shaders/<source>.slang`, as
/// `suite_shader` reads the suite's.
fn engine_shader(name: &str) -> Result<CookedShader> {
    cooked(name)
}

fn cooked(name: &str) -> Result<CookedShader> {
    let Some(dir) = std::env::var_os("LACTOSE_GPU_SHADERS").map(PathBuf::from) else {
        return Err(Error::new(
            ErrorKind::NotFound,
            "LACTOSE_GPU_SHADERS is not set: the shaders were not cooked",
        ));
    };
    let path = dir.join(format!("{name}.spirv"));
    let bytes = std::fs::read(&path)
        .map_err(|e| Error::new(ErrorKind::NotFound, format!("{}: {e}", path.display())))?;
    CookedShader::decode(&bytes)
}
