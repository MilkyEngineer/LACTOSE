//! What a device is asked for and what it reports: the request a backend's constructor
//! takes, the reported capabilities, and the queues by role.
//!
//! A capability is required or reported. The required set is not a type: a device that
//! lacks any of it is not created (`README.md`, "Capabilities"). The reported set is
//! [`Capabilities`], fixed for the device's life.

use std::sync::Arc;

use crate::message::MessageSink;
use crate::surface::SurfaceSource;

/// What a queue is for. A device has one queue for each role or fewer: where it has
/// fewer, [`QueueLayout`] says which roles the graphics queue serves.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QueueRole {
    /// The frame's draws and copies.
    Graphics,
    /// The frame's dispatches that run beside its draws.
    Compute,
    /// Every command that reads or writes a swapchain image. Submitted to only through
    /// [`Swapchain::present`](crate::Swapchain::present), by the present thread.
    Present,
}

/// Whether a role has a queue of its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QueuePath {
    /// The role has its own queue, which no submit of another role reaches.
    Own,
    /// The graphics queue serves the role, under the backend's lock.
    Shared,
}

/// Which path a device takes for each role (milestone 2 plan, decision 5.6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QueueLayout {
    /// The compute role's path.
    pub compute: QueuePath,
    /// The present role's path. `Own` only where the device offers a queue that can draw
    /// and present to the surface and that no other role uses.
    pub present: QueuePath,
}

impl QueueLayout {
    /// One queue: every role shares the graphics queue.
    pub const ONE: Self = Self { compute: QueuePath::Shared, present: QueuePath::Shared };
    /// A queue for each role.
    pub const SEVERAL: Self = Self { compute: QueuePath::Own, present: QueuePath::Own };

    /// The role whose queue serves `role`: `role` itself where its path is `Own`, and
    /// `Graphics` otherwise.
    #[must_use]
    pub const fn serving(self, role: QueueRole) -> QueueRole {
        match (role, self.compute, self.present) {
            (QueueRole::Compute, QueuePath::Own, _) => QueueRole::Compute,
            (QueueRole::Present, _, QueuePath::Own) => QueueRole::Present,
            _ => QueueRole::Graphics,
        }
    }

    /// Whether `a` and `b` are served by one queue, so that a resource crosses between
    /// them with no release and acquire.
    #[must_use]
    pub const fn same_queue(self, a: QueueRole, b: QueueRole) -> bool {
        self.serving(a) as u8 == self.serving(b) as u8
    }
}

/// What a device reports. Fixed from creation: no call changes it, and the null backend
/// is created with the set a test wants to run under.
///
/// A call that needs a capability reported `false` is refused by
/// [`Rule::Capability`](crate::Rule::Capability) on every backend. There is nothing to
/// enable: a backend turns on, at creation, every reported capability its driver offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    /// Whether [`draw_indexed_indirect`](crate::Encoder::draw_indexed_indirect) takes a
    /// count above one.
    pub multi_draw_indirect: bool,
    /// Whether
    /// [`draw_indexed_indirect_count`](crate::Encoder::draw_indexed_indirect_count) may be
    /// called.
    pub indirect_count: bool,
    /// Whether a shader may use 64-bit atomic operations on buffer memory. The facade has
    /// no call that depends on it: the renderer chooses its shader by it.
    pub buffer_atomics_64: bool,
    /// Whether a memoryless request for a colour attachment whose usage holds `INPUT` is
    /// honoured: the G-buffer's. A colour image without `INPUT` is no case of it.
    pub memoryless_colour: bool,
    /// Whether a memoryless request for a depth attachment is honoured.
    pub memoryless_depth: bool,
    /// Whether a memoryless attachment keeps its contents across a
    /// [`local_read`](crate::Encoder::local_read) in its pass. Where it does not, the
    /// render graph does not ask for memoryless in a pass that reads locally, and the null
    /// backend holds it to that ([`Rule::Split`](crate::Rule::Split)).
    pub memoryless_survives_local_read: bool,
    /// Whether host-visible memory is coherent, so that
    /// [`flush_buffer`](crate::Device::flush_buffer) and
    /// [`invalidate_buffer`](crate::Device::invalidate_buffer) do nothing. They are called
    /// either way.
    pub coherent_host_memory: bool,
    /// Which roles have a queue of their own.
    pub queues: QueueLayout,
    /// The heap's capacity in slots: the request's, which the device met.
    pub heap_slots: u32,
    /// The sampler table's capacity: the request's, which the device met.
    pub samplers: u32,
    /// How many colour attachments a render pass may have. At least four.
    pub colour_attachments: u32,
}

impl Capabilities {
    /// The least a device may report: no optional capability, one queue, coherent memory,
    /// a heap of [`HEAP_SLOTS_DEFAULT`] slots, a sampler table of [`SAMPLERS_DEFAULT`] and
    /// four colour attachments. A test starts from it and sets the fields its case is about.
    pub const LEAST: Self = Self {
        multi_draw_indirect: false,
        indirect_count: false,
        buffer_atomics_64: false,
        memoryless_colour: false,
        memoryless_depth: false,
        memoryless_survives_local_read: false,
        coherent_host_memory: true,
        queues: QueueLayout::ONE,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        colour_attachments: 4,
    };
}

/// The heap capacity the cooked shaders of milestone 2 are compiled for: the fixed array's
/// length in the form the native Metal spike chose.
pub const HEAP_SLOTS_DEFAULT: u32 = 1024;

/// The sampler table's capacity the cooked shaders of milestone 2 are compiled for: the
/// sampler array's length, which a pipeline's creation compares with the device's
/// [`samplers`](Capabilities::samplers).
pub const SAMPLERS_DEFAULT: u32 = 64;

/// The least number of images an offscreen target the present thread reads is made of
/// (milestone 2 plan, decision 5.6): the present thread holds one, and the frame renders
/// into another.
pub const OFFSCREEN_TARGET_IMAGES_MIN: u32 = 2;

/// What every backend's constructor takes. The facade has no constructor of its own:
/// `lactose-app-client` calls the backend crate's, and hands `lactose-app` the
/// [`Device`](crate::Device) it returns (D64).
///
/// A device that lacks any of the required set is not created: the constructor returns
/// `ErrorKind::Unsupported` and names what is missing.
#[derive(Clone)]
pub struct DeviceRequest {
    /// The adapter to use, by a substring of its name, where a machine has several; `None`
    /// takes the backend's first choice. [`DeviceInfo::adapter`] records which was used.
    pub adapter: Option<String>,
    /// The window the device presents to, or `None` for a device that never presents. The
    /// backend reads its handles once, inside the constructor, on the calling thread, and
    /// keeps the `Arc` for as long as its surface lives (`README.md`, "The surface").
    pub surface: Option<Arc<dyn SurfaceSource>>,
    /// The heap's capacity in slots. A device whose limit is below it is not created.
    pub heap_slots: u32,
    /// The sampler table's capacity. A device whose limit is below it is not created.
    pub samplers: u32,
    /// The bytes [`Device::pipeline_cache`](crate::Device::pipeline_cache) returned in an
    /// earlier run, or empty. Bytes that fail a check start the cache empty and never fail
    /// the creation ([`CacheStart`](crate::CacheStart)).
    pub pipeline_cache: Vec<u8>,
    /// Where every validation and driver message goes.
    pub messages: Arc<dyn MessageSink>,
}

/// What a device is, for a log line and a result file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceInfo {
    /// The backend's name: `null`, `vulkan`, `metal`.
    pub backend: &'static str,
    /// The adapter's name as the driver gives it.
    pub adapter: String,
    /// The driver and its version as the driver gives them, or empty.
    pub driver: String,
}
