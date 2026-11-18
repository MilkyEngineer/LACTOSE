# Unsafe in lactose-rhi-vulkan

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. Architecture section 21 lists the reason for unsafe here: FFI to the Vulkan
driver, through `ash`, whose every driver call is an `unsafe fn`. No `unsafe` of this crate
is anything else: it has no `unsafe` data structure, and its tests forbid `unsafe`.

The methods of `VulkanDevice` and its encoders are safe, and their callers forbid `unsafe`, so
no call they make may reach undefined behaviour through this crate's code. Each driver call is
reached only after the checks of the contract's "Every backend" column that guard it
(`crates/rhi-api/README.md`, "The rules"), made against the records (`src/records.rs`) as
rhi-null makes them. Two residuals are accepted (D158) and are not this crate's to close:

1. **The "Null alone" column.** A breach of a clause that column lists (a resource used in
   another state than the one it is in, a cross-queue use with no transfer, an input read of
   an attachment not marked) reaches the driver unchecked. It breaks a valid-usage rule whose
   effect the specification leaves undefined; what both spikes saw of it is a wrong image.
   rhi-null refuses each in every consumer's tests, and core and synchronisation validation
   report each on a GPU (`cargo xtask gpu`).
2. **A shader's code, and what a shader reaches through data alone.** A module's header and
   its `OpCapability` and `OpExtension` instructions are checked; the rest of its code is the
   cook's and is trusted. An address, a slot or sampler index in push bytes or a buffer, an
   indirect record's contents or a dispatch's counts may fault the GPU or race a host access.
   GPU-assisted validation reports such an access in tests (`cargo xtask gpu
   --gpu-assisted`, whose canary is a read past a buffer through its address).

## The sites, by kind

### The loader, the instance and the device (`raw.rs`)

- `ash::Entry::load` opens the system's loader library, whose initialisers the loader
  documents as safe to run in any process.
- The instance, the messenger, the device, its queues and their timeline semaphores are made
  from create infos whose chained structures live through each call, and whose physical
  device is one the instance enumerated. Every property, feature, format and extension query
  is of that physical device.
- On a failure partway, `Partial` destroys what was made, once each, children first; on
  success every field is taken out of it before its drop.
- `Raw` owns the instance and the device. Every object of the device holds an `Arc<Raw>`, so
  `Raw`'s drop runs after the last of them, and the last holder of the device waited for the
  device to idle first (`Shared::let_go`). The messenger is destroyed before the instance.

### The messenger's callback (`raw.rs`, `on_message`)

- The loader passes callback data valid for the call, with strings null or NUL-terminated;
  each is copied into an owned `String` before the sink sees it.
- `user` is the address of the `Box<Arc<dyn MessageSink>>` that `Raw` keeps. The box's address
  is fixed from the instance's creation, whose `pNext` names it, to the instance's
  destruction, and `Raw` drops the box after both (a field dropped after `Drop::drop`).
- The sink's call runs inside `catch_unwind`, so a panicking sink never unwinds into the
  driver.

### Objects and their memory (`device.rs`, `heap.rs`, `pipeline.rs`)

- Buffers, images, image views, memory blocks, samplers, timelines, pipelines and shader
  modules are made from create infos the contract's checks have bounded (a size above zero,
  an image's description against the adapter's format support, a memory type the resource
  accepts, a placement inside its block) and bound once, to unbound memory large enough.
- An object is destroyed in the drop of its last count: a table's entry, and an encoder's
  `Arc` for each object it recorded. A destroy that would end the entry while a submit that
  names the object is pending is refused (`pending`), and the encoder's count lasts until
  its submit is seen complete, so the GPU is done with each object before its drop.
- A shader module lives through the one creation that uses it (`Modules`), and is destroyed
  when that creation returns, however it returns.
- The pipeline cache is created from a payload whose seal names this device's identity and
  whose Vulkan header names this device; the driver checks the rest. It is destroyed with the
  device, after every creation that used it.

### Mapped memory (`device.rs`, `Mapping`)

- Host-visible memory is mapped whole when its buffer is made and stays mapped until the
  buffer's drop. `write` and `read` copy through the mapping inside a range the caller has
  bounded by the buffer's size, while the records hold that range as a host access that no
  other access overlaps (`host-overlap`), and after the records refused a pending submit that
  writes the buffer (`pending`). No reference into the mapping outlives a copy.
- `unsafe impl Send` and `Sync for Mapping` rest on that: the pointer is to plain host memory
  with no thread affinity, and every access to it is a copy under those checks.
- A flush or an invalidate covers a range of the mapping rounded out to
  `nonCoherentAtomSize`, and no further than the allocation.

### Descriptor writes (`heap.rs`, `encoder.rs`)

- `Heap::write_slot` and `Heap::write_sampler` are `unsafe fn`: the caller holds the device's
  heap lock, which serialises every write of the heap's set, and the view or sampler written
  lives until its slot is written again and no submit that may read it is pending. The
  binding is updated after bind and while unused by pending submits, and the contract's checks
  refused a write of a slot a pending submit declares (`uuwp-used`).
- A render pass's input attachment set is the encoder's alone, written before it is bound,
  with views of images the encoder holds.

### Recording (`encoder.rs`, `submit.rs`)

- A command buffer is recorded only through `&mut` of the encoder that owns its pool, so one
  thread records it at a time and the pool is externally synchronised.
- Each command is recorded after the checks its rules name, in the scope it needs (inside a
  render pass, inside a compute pass with a pipeline bound, outside a pass), with every
  resource it names held by the encoder and every range inside its resource.
- A reset, and the drop that frees the pool, come after the encoder's submit is seen complete,
  or after the device is lost; a drop that would wait while the thread unwinds a panic leaks
  the pool instead.

### Submission and waits (`submit.rs`, `device.rs`)

- `vkQueueSubmit2` is called with the queue's lock held, which every submit to that queue
  takes, so the queue is externally synchronised. Each command buffer submitted has ended
  recording and has no other pending submit (`encoding`).
- A wait, a signal or a read of a timeline is made while a count keeps its semaphore alive;
  a queue's own semaphore lives as long as the device. A host signal's value grows past every
  value the timeline was signalled to.

### Drops while unwinding

- A drop of the device's last holder, made while the thread unwinds a panic with a submit
  pending, does not wait; it marks the device leaking (`Raw::leak`), and every object's drop
  after it returns without destroying anything. Nothing is destroyed under the GPU's use, at
  the cost of the device's memory.

## What no test here shows

The thread-safety validator of the Khronos layer has no canary: a safe call cannot race a
command buffer or a queue, so no test provokes the misuse that would show the validator live.
Its setting is recorded in each run's report with the others.
