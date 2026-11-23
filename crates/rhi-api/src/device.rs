//! The device: the one object a backend hands out, as a `dyn` resource (D64).
//!
//! Every method takes `&self` and may be called from any thread. A call returns an error
//! of kind `DeviceLost` when the device is lost, a refusal when it breaks a rule the
//! backend checks ([`Rule::of`](crate::Rule::of)), and otherwise what its own
//! documentation names; `README.md`, "Device loss" and "The rules", state what holds for
//! every call, and each method below states only what is its own.

use core::time::Duration;

use lactose_core::Result;

use crate::caps::{Capabilities, DeviceInfo, QueueRole};
use crate::encoder::Encoder;
use crate::pipeline::{
    ComputePipelineDesc, ComputePipelineHandle, GraphicsPipelineDesc, GraphicsPipelineHandle,
    PipelineStats,
};
use crate::resource::{
    BufferDesc, BufferHandle, HeapSlot, ImageDesc, ImageHandle, MemoryBlockDesc, MemoryBlockHandle,
    Placement, SamplerDesc, SamplerIndex,
};
use crate::surface::{Swapchain, SwapchainDesc};
use crate::sync::{Submit, TimelineHandle, TimelinePoint};

/// A graphics device and its queues.
///
/// The device lives until this object, every encoder and the swapchain it made are
/// dropped: each of them holds it. The last drop waits for every submit the device has
/// taken and then destroys what is left (`README.md`, "Handles and lifetimes").
pub trait Device: Send + Sync {
    /// What the device is.
    fn info(&self) -> &DeviceInfo;

    /// What the device reports. The same value for the device's life.
    fn capabilities(&self) -> &Capabilities;

    /// Whether the device is lost: some call has returned `ErrorKind::DeviceLost`, and
    /// every call that reaches the device from now on returns it.
    fn is_lost(&self) -> bool;

    /// Makes a buffer, in [`BufferState::Undefined`](crate::BufferState) on no queue.
    ///
    /// # Errors
    ///
    /// `ErrorKind::ResourceExhausted` when the device has no memory for it.
    fn create_buffer(&self, desc: &BufferDesc<'_>) -> Result<BufferHandle>;

    /// Destroys a buffer. Refused on every backend while a pending submit uses it
    /// ([`Rule::Pending`](crate::Rule::Pending)), the caller waiting for the timeline value
    /// that frees it, and while a host access to it is in progress
    /// ([`Rule::HostOverlap`](crate::Rule::HostOverlap)).
    fn destroy_buffer(&self, buffer: BufferHandle) -> Result<()>;

    /// The buffer's device address: the same value for as long as the buffer lives, and
    /// distinct from every byte of every other live buffer. The buffer's usage holds
    /// `DEVICE_ADDRESS`, or the call is refused ([`Rule::Usage`](crate::Rule::Usage)).
    fn buffer_address(&self, buffer: BufferHandle) -> Result<u64>;

    /// Lends the host `len` bytes of an `Upload` buffer from `offset` to write, for the
    /// length of the call: `fill` is called once, with exactly that range. What `fill`
    /// leaves there is what the device reads in submits made after
    /// [`flush_buffer`](Self::flush_buffer) covers it. What the lent bytes hold when
    /// `fill` is called is unspecified, and reading them may be slow: `fill` writes every
    /// byte of the range and reads none.
    ///
    /// Calls on ranges of one buffer that do not overlap may run at once on several
    /// threads. A call whose range overlaps one in progress is refused, and so is `fill`
    /// calling the device for the same buffer, a destroy of it included
    /// ([`Rule::HostOverlap`](crate::Rule::HostOverlap)), and while the call is in progress
    /// a submit that writes the buffer is refused by the same rule. A range outside the
    /// buffer is refused ([`Rule::Bounds`](crate::Rule::Bounds)), and so is a call while a
    /// pending submit writes the buffer, by a copy into it or a pass that declares it
    /// `ShaderWrite` ([`Rule::Pending`](crate::Rule::Pending)); a submit that only
    /// transitions the buffer touches none of its bytes and holds no call back. All on
    /// every backend: the bytes lent are never ones the device may be writing.
    fn write_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        fill: &mut dyn FnMut(&mut [u8]),
    ) -> Result<()>;

    /// Lends the host `len` bytes of a `Readback` buffer from `offset` to read, for the
    /// length of the call: `read` is called once, with exactly that range, which holds
    /// what completed submits wrote once
    /// [`invalidate_buffer`](Self::invalidate_buffer) has covered it. The buffer is in
    /// `HostRead`. A call while a pending submit writes the buffer, by a copy into it or
    /// a pass that declares it `ShaderWrite`, or changes its state, is refused on every
    /// backend ([`Rule::Pending`](crate::Rule::Pending)): the bytes lent are never ones
    /// the device may be writing. Overlap, bounds and a submit made during the call
    /// are as [`write_buffer`](Self::write_buffer)'s.
    fn read_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        read: &mut dyn FnMut(&[u8]),
    ) -> Result<()>;

    /// Makes the host's writes to a range of an `Upload` buffer visible to the device.
    /// Called after every write and before the submit that reads it; it does nothing where
    /// [`coherent_host_memory`](Capabilities::coherent_host_memory) is reported. A buffer
    /// of another class is refused ([`Rule::Usage`](crate::Rule::Usage)), and so is a range
    /// that overlaps a host access in progress
    /// ([`Rule::HostOverlap`](crate::Rule::HostOverlap)), on every backend: a device
    /// backend's flush is rounded out to the driver's atom.
    fn flush_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()>;

    /// Makes the device's writes to a range of a `Readback` buffer visible to the host.
    /// Called after the submit that wrote it completes and before the read; it does nothing
    /// where [`coherent_host_memory`](Capabilities::coherent_host_memory) is reported. A
    /// call while a pending submit writes the buffer or changes its state is refused on
    /// every backend ([`Rule::Pending`](crate::Rule::Pending)): it would be taken for the
    /// invalidate after the write. Class and overlap are as
    /// [`flush_buffer`](Self::flush_buffer)'s: an invalidate of memory the host writes
    /// would discard what it has not flushed.
    fn invalidate_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()>;

    /// Makes an image, every subresource
    /// [`ImageState::Undefined`](crate::ImageState::Undefined) on no queue.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Unsupported` for a format or an extent the device cannot make;
    /// `ErrorKind::ResourceExhausted` when it has no memory for it; refused for a
    /// description no device honours, a memoryless request without `TRANSIENT` among
    /// them ([`Rule::Usage`](crate::Rule::Usage)).
    fn create_image(&self, desc: &ImageDesc<'_>) -> Result<ImageHandle>;

    /// Destroys an image. Refused on every backend while a pending submit uses it or a
    /// heap slot holds it ([`Rule::Pending`](crate::Rule::Pending)).
    fn destroy_image(&self, image: ImageHandle) -> Result<()>;

    /// Whether the image has no memory outside its pass: `true` only for an image made
    /// with a memoryless request the device honoured.
    fn image_memoryless(&self, image: ImageHandle) -> Result<bool>;

    /// What a block must offer an image made from `desc`, whatever `desc.memory` says.
    /// Every block takes any image this accepts.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Unsupported` for an image no block can hold, such as one the driver
    /// gives memory of its own alone.
    fn image_placement(&self, desc: &ImageDesc<'_>) -> Result<Placement>;

    /// Makes a block of device memory for images to be placed in.
    ///
    /// # Errors
    ///
    /// `ErrorKind::ResourceExhausted` when the device has no memory for it.
    fn create_memory_block(&self, desc: &MemoryBlockDesc<'_>) -> Result<MemoryBlockHandle>;

    /// Destroys a block. Refused on every backend while an image placed in it lives
    /// ([`Rule::Pending`](crate::Rule::Pending)).
    fn destroy_memory_block(&self, block: MemoryBlockHandle) -> Result<()>;

    /// Puts `image`, whole, in a heap slot. The image's usage holds `SAMPLED`. The write
    /// is seen by every pass that begins after the call returns; an encoder with a pass
    /// that declared the slot before it is refused at its submit
    /// ([`Rule::HeapLayout`](crate::Rule::HeapLayout)). Refused on every backend while a
    /// pass of a pending submit declares the slot
    /// ([`Rule::HeapWrite`](crate::Rule::HeapWrite)): a slot is written again only after
    /// the timeline value that frees it.
    fn write_heap_slot(&self, slot: HeapSlot, image: ImageHandle) -> Result<()>;

    /// Empties a heap slot, under the rule of [`write_heap_slot`](Self::write_heap_slot). An
    /// empty slot holds the backend's placeholder image, so a shader that reaches it through
    /// data reads an unspecified value and never an unwritten descriptor.
    fn clear_heap_slot(&self, slot: HeapSlot) -> Result<()>;

    /// Makes a sampler and returns its index in the sampler table: an index no live
    /// sampler has.
    ///
    /// # Errors
    ///
    /// `ErrorKind::ResourceExhausted` when the table is full.
    fn create_sampler(&self, desc: &SamplerDesc) -> Result<SamplerIndex>;

    /// Frees a sampler's index for reuse; until it is reused the entry holds the backend's
    /// placeholder sampler. A sampler has no declaration, so any pending
    /// submit may use it: refused on every backend while a submit made before the call is
    /// pending ([`Rule::Pending`](crate::Rule::Pending)).
    fn destroy_sampler(&self, sampler: SamplerIndex) -> Result<()>;

    /// Makes a graphics pipeline.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Unsupported` for a shader target the backend does not run; refused for a
    /// SPIR-V module whose `OpCapability` or `OpExtension` instructions name one the device
    /// did not enable ([`Rule::Capability`](crate::Rule::Capability), every backend that
    /// takes SPIR-V; a Metal target declares none); `ErrorKind::InvalidData` for a module
    /// whose magic or version is not its target's, a shader the driver refuses (Metal's
    /// compiler refuses what the GPU cannot run), a stage that breaks the cooked format
    /// ([`CookedShader::check`](crate::CookedShader::check)), a stage in the wrong place, a
    /// constant the shader does not declare, given twice, or whose
    /// [`ConstantValue`](crate::ConstantValue) is not of its slot's type, a heap array whose
    /// capacity is not the device's, a varying the fragment stage reads that the vertex
    /// stage does not write at its location with its component type and count
    /// ([`check_varyings`](crate::check_varyings)), a compute stage whose workgroup is above
    /// the pipeline's `maxTotalThreadsPerThreadgroup` on Metal or, where a backend checks it,
    /// whose module's `LocalSize` is another, a colour target with a write
    /// mask that is not empty which the fragment stage does not write or reads as an input,
    /// or a depth format that is not one (`README.md`, "Pipelines and the cooked shader
    /// format").
    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<'_>,
    ) -> Result<GraphicsPipelineHandle>;

    /// Makes a compute pipeline. Errors as
    /// [`create_graphics_pipeline`](Self::create_graphics_pipeline)'s.
    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<'_>,
    ) -> Result<ComputePipelineHandle>;

    /// Destroys a graphics pipeline. Refused on every backend while a pending submit uses
    /// it ([`Rule::Pending`](crate::Rule::Pending)).
    fn destroy_graphics_pipeline(&self, pipeline: GraphicsPipelineHandle) -> Result<()>;

    /// Destroys a compute pipeline. Refused on every backend while a pending submit uses
    /// it ([`Rule::Pending`](crate::Rule::Pending)).
    fn destroy_compute_pipeline(&self, pipeline: ComputePipelineHandle) -> Result<()>;

    /// The driver's pipeline cache as it is now, sealed
    /// ([`seal_cache`](crate::seal_cache)), for the caller to write to disk and hand to
    /// the next run's [`DeviceRequest`](crate::DeviceRequest). Empty where the backend has
    /// nothing to save or may not save now (`README.md`, "The pipeline cache").
    fn pipeline_cache(&self) -> Result<Vec<u8>>;

    /// What the device has done with pipelines so far. Never fails, on a lost device
    /// either.
    fn pipeline_stats(&self) -> PipelineStats;

    /// Makes a timeline whose value is `initial`.
    fn create_timeline(&self, initial: u64) -> Result<TimelineHandle>;

    /// Destroys a timeline. Refused on every backend while a pending submit waits for or
    /// signals it ([`Rule::Pending`](crate::Rule::Pending)).
    fn destroy_timeline(&self, timeline: TimelineHandle) -> Result<()>;

    /// The timeline's value now. It never decreases. A device backend records the value
    /// it saw before it returns, so the checks of later calls know the timeline has reached
    /// it (`README.md`, "Timelines").
    fn timeline_value(&self, timeline: TimelineHandle) -> Result<u64>;

    /// Signals a point from the host: the timeline's value becomes `point.value`. The value
    /// is greater than the timeline's and less than every value a pending submit will
    /// signal to it, and within 2^31 of the timeline's and of every value a pending submit
    /// waits for or signals on it; every pending submit that signals the timeline waits, on
    /// it, for a value greater than the timeline's and no greater than `point.value`; and
    /// no other host signal of it is in progress. The timeline's value here is the greatest
    /// the device knows it has reached (`README.md`, "Timelines"). Otherwise the call is
    /// refused on every backend ([`Rule::Timeline`](crate::Rule::Timeline)), since a
    /// promise or a host signal could pass this one.
    fn signal_timeline(&self, point: TimelinePoint) -> Result<()>;

    /// Blocks until the point is reached or `timeout` has passed, and returns whether it
    /// was reached. A zero timeout polls. A device backend that returns `true` records the
    /// point as reached before it returns, so no later call is refused as `pending` for a
    /// submit that signals it (`README.md`, "Queues by role").
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`, at once and without waiting, when the device is lost
    /// before the point is reached.
    fn wait_timeline(&self, point: TimelinePoint, timeout: Duration) -> Result<bool>;

    /// Makes an encoder for `queue`'s role, recording. Its memory is its own: two
    /// encoders record on two threads at once with no lock between them.
    fn create_encoder(&self, queue: QueueRole, label: &str) -> Result<Box<dyn Encoder>>;

    /// Submits to the queue that serves `queue`: `Graphics` or `Compute`. The present
    /// role is submitted to through [`Swapchain::present`] alone
    /// ([`Rule::PresentQueue`](crate::Rule::PresentQueue)). Submits to one queue run in
    /// the order the calls return; it is the caller that orders two threads' submits. A
    /// submit with no encoders is valid: it waits and signals. A submit that writes a
    /// buffer, by a copy into it or a pass that declares it `ShaderWrite`, is refused while
    /// a host access to the buffer is in progress
    /// ([`Rule::HostOverlap`](crate::Rule::HostOverlap)); its signals are refused by the
    /// rules of `README.md`, "Timelines" ([`Rule::Timeline`](crate::Rule::Timeline)),
    /// among them a signal not greater than a value the same submit waits for on the
    /// timeline, and one of a timeline another queue's submit has a pending signal of,
    /// where no wait orders that signal first: a wait for one signal of a submit orders
    /// the earlier submits of its queue and not the submit's other signals. Both on every
    /// backend.
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`; a refusal, by any rule a command of the submit breaks in
    /// submission order, which leaves everything as it was ([`Submit`]).
    fn submit(&self, queue: QueueRole, submit: Submit<'_>) -> Result<()>;

    /// Makes the swapchain of the surface the device was created with. A device has one
    /// swapchain at a time.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Unsupported` for a device created with no surface, and for a surface
    /// the device cannot present to; refused while the device's swapchain lives
    /// ([`Rule::Swapchain`](crate::Rule::Swapchain)); refused for an extent with a zero
    /// dimension ([`Rule::Bounds`](crate::Rule::Bounds)). Both on every backend. At a
    /// surface whose current extent is empty, such as a minimised window on Windows, it
    /// succeeds without making images: the swapchain is stale until a
    /// [`Swapchain::resize`] finds the surface not empty.
    fn create_swapchain(&self, desc: &SwapchainDesc) -> Result<Box<dyn Swapchain>>;
}
