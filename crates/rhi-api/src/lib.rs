//! The render hardware interface (architecture section 12.1): the one surface the render
//! graph, the renderer and the client's frame are written against, and that the null,
//! Vulkan and Metal backends implement.
//!
//! **Status: facade.** Every signature here is final for milestone 2's lanes; the bodies
//! that decide anything are `todo!()`, and the traits have no implementation yet.
//! `README.md` is the contract: it states what the signatures cannot, shows how the null
//! backend and native Metal honour each call, and names the conformance suite's cases.
//!
//! Where to start:
//!
//! - [`DeviceRequest`] is what a backend's constructor takes, and [`Device`] what it
//!   returns: buffers, images, the heap, pipelines, timelines, encoders and submits.
//!   [`Capabilities`] is what a device reports; what it must have is not a type.
//! - [`Encoder`] is what one thread records: transitions between [`ImageState`]s and
//!   [`BufferState`]s, passes that declare what they reach ([`PassReach`]), draws,
//!   dispatches, copies, the local read and the alias call.
//! - [`Submit`] and [`TimelinePoint`] order work on and between the queues, which are
//!   named by [`QueueRole`]; [`Swapchain`] is the present role's, held by one thread.
//! - [`CookedShader`] is the format the cooker writes and a pipeline is made from.
//! - [`Rule`] names what a backend refuses, and [`Rule::of`] reads it from an error.
//!
//! One frame's worth of calls, as a caller writes them:
//!
//! ```no_run
//! use lactose_core::Result;
//! use lactose_rhi_api::{
//!     ColourAttachment, Device, Encoder, GraphicsPipelineHandle, ImageHandle, ImageRange,
//!     ImageState, ImageTransition, Load, PassReach, QueueRole, RenderPass, StateOn, Store,
//!     Submit, Target, TimelinePoint,
//! };
//!
//! fn frame(
//!     device: &dyn Device,
//!     encoders: &mut [Box<dyn Encoder>],
//!     target: ImageHandle,
//!     pipeline: GraphicsPipelineHandle,
//!     done: TimelinePoint,
//! ) -> Result<()> {
//!     let queue = QueueRole::Graphics;
//!     let encoder = &mut encoders[0];
//!     encoder.transition_images(&[ImageTransition {
//!         image: target,
//!         range: ImageRange::FIRST,
//!         from: StateOn { state: ImageState::Undefined, queue },
//!         to: StateOn { state: ImageState::ColourAttachment, queue },
//!     }]);
//!     encoder.begin_render_pass(&RenderPass {
//!         label: "triangle",
//!         colour: &[ColourAttachment {
//!             target: Target { image: target, mip: 0, layer: 0 },
//!             load: Load::Clear([0.0, 0.0, 0.0, 1.0]),
//!             store: Store::Store,
//!             local_read: false,
//!         }],
//!         depth: None,
//!         reach: PassReach::NOTHING,
//!     });
//!     encoder.bind_graphics_pipeline(pipeline);
//!     encoder.draw(0..3, 0..1);
//!     encoder.end_pass();
//!     device.submit(queue, Submit { encoders, waits: &[], signals: &[done] })
//! }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every deciding body is `todo!()` until an implementation lane replaces it, and the parameters it will use are declared now"
)]

mod caps;
#[cfg(feature = "conformance")]
pub mod conformance;
mod device;
mod encoder;
mod message;
mod pipeline;
mod resource;
mod rule;
mod shader;
mod state;
mod surface;
mod sync;

pub use caps::{
    Capabilities, DeviceInfo, DeviceRequest, HEAP_SLOTS_DEFAULT, OFFSCREEN_TARGET_IMAGES_MIN,
    QueueLayout, QueuePath, QueueRole, SAMPLERS_DEFAULT,
};
pub use device::Device;
pub use encoder::{
    BufferCopy, BufferImageCopy, ColourAttachment, ComputePass, DepthAttachment, Encoder,
    IndexedIndirectArgs, Load, Rect, RenderPass, Store, Target, Viewport,
};
pub use message::{Message, MessageSink, MessageSource, Severity};
pub use pipeline::{
    CacheIdentity, CacheStart, ColourTargetDesc, Compare, ComputePipelineDesc,
    ComputePipelineHandle, Constant, Cull, DepthDesc, GraphicsPipelineDesc, GraphicsPipelineHandle,
    PipelineStats, StageDesc, WriteMask, open_cache, seal_cache,
};
pub use resource::{
    AddressMode, BufferDesc, BufferHandle, BufferUsage, Extent, Filter, Format, HeapSlot,
    ImageDesc, ImageHandle, ImageMemory, ImageRange, ImageUsage, MemoryBlockDesc,
    MemoryBlockHandle, MemoryClass, Placement, SamplerDesc, SamplerIndex, Sharing, kind,
};
pub use rule::{Refusal, Rule};
pub use shader::{
    COOKED_SHADER_VERSION, ConstantSlot, CookedShader, HEAP_SET, HeapArray, HeapArrayKind,
    HeapForm, HeapUse, PUSH_BYTES_MAX, ShaderStage, ShaderTarget,
};
pub use state::{
    BufferReach, BufferState, BufferTransition, ImageState, ImageTransition, PassReach, StateOn,
};
pub use surface::{Acquired, Presented, SurfaceSource, Swapchain, SwapchainDesc};
pub use sync::{Submit, TimelineHandle, TimelinePoint};

// What the contract promises at the type level, so that an implementation or an amendment
// cannot lose it silently (`README.md`, "Threads").
const _: () = {
    const fn send_sync<T: ?Sized + Send + Sync>() {}
    const fn send<T: ?Sized + Send>() {}
    const fn plain<T: Copy + Send + Sync + 'static>() {}

    // Naming each `dyn` type holds its trait object-safe: backends are handed to
    // `lactose-app` as `dyn` resources (D64).
    send_sync::<dyn Device>();
    send_sync::<dyn MessageSink>();
    send_sync::<dyn SurfaceSource>();
    send_sync::<DeviceRequest>();
    // One thread at a time, any thread: recorded on a worker, submitted elsewhere.
    send::<dyn Encoder>();
    send::<Box<dyn Encoder>>();
    send::<dyn Swapchain>();
    send::<Submit<'static>>();

    // `Send` and not `Sync`: an encoder and the swapchain are one thread's at a time. Were
    // either `Sync`, both impls below would apply and the inference would be ambiguous.
    trait NotSync<A> {
        fn holds() {}
    }
    impl<T: ?Sized> NotSync<()> for T {}
    impl<T: ?Sized + Sync> NotSync<u8> for T {}
    let _ = <dyn Encoder as NotSync<_>>::holds;
    let _ = <dyn Swapchain as NotSync<_>>::holds;

    // Handles and every value a caller keeps between calls are plain data.
    plain::<BufferHandle>();
    plain::<ImageHandle>();
    plain::<MemoryBlockHandle>();
    plain::<GraphicsPipelineHandle>();
    plain::<ComputePipelineHandle>();
    plain::<TimelineHandle>();
    plain::<TimelinePoint>();
    plain::<HeapSlot>();
    plain::<SamplerIndex>();
    plain::<Capabilities>();
    plain::<ImageTransition>();
    plain::<BufferTransition>();
    plain::<Rule>();

    // A handle is two 32-bit words, and the indirect record is the twenty bytes both
    // drivers read.
    assert!(size_of::<BufferHandle>() == 8);
    assert!(size_of::<Option<ImageHandle>>() == 8);
    assert!(size_of::<IndexedIndirectArgs>() == IndexedIndirectArgs::BYTES as usize);
    assert!(OFFSCREEN_TARGET_IMAGES_MIN >= 2);
};
