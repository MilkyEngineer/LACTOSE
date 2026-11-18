//! The integration tests of `lactose-rhi-null` as one test binary (D150): each file beside
//! this one is a module of it, named after the file. What they share is declared here.

#![forbid(unsafe_code)]

mod conformance;
mod null;

use std::sync::Arc;

use lactose_rhi_api::{Capabilities, DeviceRequest, Message, MessageSink};
use lactose_rhi_null::NullSurface;

/// A sink that drops what it is sent: null sends nothing.
struct Silent;

impl MessageSink for Silent {
    fn message(&self, _: &Message<'_>) {}
}

/// The request of a device of `set`, with its own surface where `presents`.
fn request(set: &Capabilities, presents: bool, messages: Arc<dyn MessageSink>) -> DeviceRequest {
    DeviceRequest {
        adapter: None,
        surface: presents.then(|| Arc::new(NullSurface) as _),
        heap_slots: set.heap_slots,
        samplers: set.samplers,
        pipeline_cache: Vec::new(),
        messages,
    }
}
