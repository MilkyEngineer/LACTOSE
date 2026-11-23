# Unsafe in lactose-rhi-vulkan

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. Architecture section 21 lists the reason for unsafe here: FFI to the Vulkan
driver, through `ash`, whose every driver call is an `unsafe fn`. No `unsafe` of this crate
is anything else: it has no `unsafe` data structure, and its integration tests forbid `unsafe`.
The one `unsafe` block of its unit tests is the thread-safety canary's (below), a driver call
that breaks the queue's external synchronisation on purpose.

The methods of `VulkanDevice` and its encoders are safe, and their callers forbid `unsafe`, so
no call they make may reach undefined behaviour through this crate's code. Each driver call is
reached only after the checks of the contract's "Every backend" column that guard it
(`crates/rhi-api/README.md`, "The rules"), made against the records (`src/records.rs`) as
rhi-null makes them. Two residuals are accepted (D158) and are not this crate's to close:

1. **The "Null alone" column.** A breach of a clause that column lists (a resource used in
   another state than the one it is in, a cross-queue use with no transfer, an input read of
   an attachment not marked) reaches the driver unchecked. It breaks a valid-usage rule whose
   effect the specification leaves undefined; what both spikes saw of it is a wrong image.
   rhi-null refuses each in every consumer's tests. On a GPU (`cargo xtask gpu`) core
   validation reports an image in another layout, and synchronisation validation a hazard
   of a copy, a clear, an attachment or a layout transition within a submit and across
   submits and queues: the device learns that a submit has completed only through a wait
   that succeeds (`Shared::reached`), as the layer (1.4.363) takes any read of a timeline's
   counter as the host seeing every batch it was given complete, and then hears nothing
   across submits. The canaries show it live within one command buffer, across two submits
   of one queue, and across two queues with no semaphore between them. Synchronisation
   validation does not track a shader's access through an address or the heap, or an
   indirect or index read, and core validation keeps no state for a buffer: a buffer
   written through its device address and then transitioned from a state it is not in is
   reported by neither, under MoltenVK or KosmicKrisp, so a consumer must not rely on a
   clean GPU run there. For those uses rhi-null is the check, and a GPU test of them needs a
   lagging writer and a mutation that drops the barrier, as
   `dispatches_of_one_pass_see_each_others_writes` and `dispatch_indirect_reads_its_counts`
   have. A cross-queue use with the semaphore and without the family transfer is a hazard
   of neither validator, and no canary shows it reported.
2. **A shader's code, and what a shader reaches through data alone.** A module's header and
   its `OpCapability` and `OpExtension` instructions are checked; the rest of its code is the
   cook's and is trusted. An address, a slot or sampler index in push bytes or a buffer, an
   indirect record's contents or a dispatch's counts may fault the GPU or race a host access.
   GPU-assisted validation reports such an access in tests (`cargo xtask gpu
   --gpu-assisted`, whose canaries are a read past a buffer through its address and a sample
   of slot `heap_slots`, one past the heap, by an index in push bytes), and on the M1 Metal
   shader validation reports one past a buffer in every run under MoltenVK (`cargo xtask
   gpu`, whose Metal step's canary is a store past a buffer, and whose Vulkan step under
   MoltenVK hears Metal's report of a read past a buffer from the test's own process,
   `canary_metal_shader_validation_reports_a_read_past_a_buffer`).

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
- `Raw` owns the instance, the device, the messengers' sink and the loader. Every object of
  the device holds an `Arc<Raw>`, so `Raw`'s drop runs after the last of them, and the last
  holder of the device waited for the device to idle first (`Shared::let_go`). The messenger
  is destroyed before the instance, and the sink is freed and the loader released after the
  instance, never before: the loader is a `ManuallyDrop` and the sink a pointer with no drop
  of its own, so neither goes with the fields. On the leak path neither is freed.

### The messenger's callback (`raw.rs`, `on_message`)

- The loader passes callback data valid for the call, with strings null or NUL-terminated;
  each is copied into an owned `String` before the sink sees it.
- `user` is the address of a leaked `Box<Arc<dyn MessageSink>>` (`Sink`), made before the
  instance, whose `pNext` names it. It is freed once, by `Raw`'s drop or `Partial`'s, after
  the instance is destroyed, and never while an instance or a messenger that names it
  lives: a device leaked while unwinding keeps it for the life of the process.
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
- No destroying call is made under the records' lock, an entry's or a slot's: a table's
  replaced or removed count, a slot's replaced image and a reaped sampler are dropped or
  destroyed once those locks are given up, under a queue's or the heap's lock alone. A
  message the layer sends from such a call reaches the sink with no lock held that the
  sink's own calls of the device would take.
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
  or after the device is lost. A drop that would wait while the thread unwinds a panic hands
  its pools to the device instead (`Shared::orphan`), whose last holder destroys them after
  the device is idle, or leaks them with the device.

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
  after it returns without destroying anything, `Raw`'s included, which frees neither the
  sink nor the loader the leaked messengers reach. Nothing is destroyed under the GPU's use,
  at the cost of the device's memory. With nothing pending, the unwinding last holder tears
  the device down as any last holder does.
- The holders' count (`Holders`, `holders.rs`) is decremented with `Release` and the last
  holder takes an `Acquire` fence, as `Arc`'s drop does, so every queue call another holder
  made happens before the last holder's wait for idle. Its unit test has each holder write
  plain memory that the last reads, so Miri reports a data race where the ordering is
  weakened: each holder's cell is an `UnsafeCell` shared through a test-only
  `unsafe impl Sync`, written by its holder and read only by the last, after `let_go`.

## What Miri covers

Nightly runs the crate's unit tests under Miri with four seeds: the records' fixed
interleavings, a race of host signals and reads on threads over them, the handle table's
drops outside its entry lock and a race of its generations on threads, the holders' count
on threads, and the conversions and module reads. Those are this crate's state that has no
driver in it. Miri cannot load a driver, so nothing that reaches `ash` runs under it: `Raw`,
`Mapping`, `on_message`, `leaking` and `lost`, and the encoders. Those run on devices in `cargo xtask
gpu`, the unwinding drops included, where a data race between two of this crate's threads is
seen only if a validator or a test's assertion meets it.

## The thread-safety canary

A safe call cannot race a command buffer or a queue, so the thread-safety validator's canary
makes its race below the contract: `src/canary.rs`, a unit test that only `cargo xtask gpu`
runs, waits the graphics queue idle through `ash` with no queue lock on one thread, under a
dispatch that runs for about two seconds, and submits to the queue through the safe API from
another 300 ms later. The wait lasts until the dispatch ends, so both calls are inside the
layer at once, and the layer reports `UNASSIGNED-Threading-MultipleThreads-Write` at the
submit, under MoltenVK and KosmicKrisp. It is compiled into the unit tests alone, and ignored
there: only `cargo xtask gpu` runs it, in a process of its own. The race is undefined
behaviour for the driver, which may deadlock instead: a watchdog thread ends the process
with a failure after 30 seconds, which the run records as the canary failed (`the test
failed`), with the watchdog's line on its standard error.
