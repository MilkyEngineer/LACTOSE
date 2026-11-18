//! The integration tests of `lactose-rhi-vulkan` as one test binary (D150): each file beside
//! this one is a module of it, named after the file. What they share is declared here.
//!
//! Every test that makes a device is ignored by a plain run, which may have no driver;
//! `cargo xtask gpu` runs them on the driver it names.

#![forbid(unsafe_code)]

mod conformance;
mod device;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use lactose_rhi_api::{DeviceRequest, HEAP_SLOTS_DEFAULT, Message, MessageSink, SAMPLERS_DEFAULT};

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

/// The request of a device on the loader's first adapter, with the default heap and sampler
/// table and no surface.
fn request(messages: Arc<dyn MessageSink>) -> DeviceRequest {
    DeviceRequest {
        adapter: None,
        surface: None,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages,
    }
}
