# Unsafe in lactose-rhi-vulkan

Every `unsafe` block carries a `// SAFETY:` comment. This file states the invariants those
comments rely on. Architecture section 21 lists the reason for unsafe here: FFI to the Vulkan
driver, through `ash`, whose every driver call is an `unsafe fn`. No `unsafe` of this crate
is anything else: it has no `unsafe` data structure, and its integration tests forbid `unsafe`.
Two of its unit tests have `unsafe` blocks, each a driver call that breaks a rule on purpose:
the thread-safety canary's (below), which breaks the queue's external synchronisation, and the
injected reuse of an acquire semaphore (`src/swapchain.rs`), whose batch waits for the
semaphore under the queue's lock before the test takes it as free.

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
   validation reports an image in another layout at a copy, a clear, an attachment or a
   transition, but not an image a shader samples through the heap: no layer reports a
   slot's image sampled in another layout (reviewer-gpu's probe, under MoltenVK and
   KosmicKrisp, core and GPU-assisted validation, and Metal's), and rhi-null's
   `heap-layout` rule is the check there. Synchronisation validation reports a hazard
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

### The surface and the swapchain (`surface.rs`, `swapchain.rs`)

- A window's surface is made from the handles its `SurfaceSource` lends, read once on the
  constructor's thread; the surface keeps the source, so the handles live as long as the
  surface. An AppKit view's layer is made by `raw-window-metal` on the main thread alone
  (`pthread_main_np` is checked first, as `Layer::from_ns_view` requires), and the surface
  keeps the layer. `Raw`'s drop destroys the surface after the device and before the
  messenger and the instance; a swapchain holds the device, so none outlives it. After the
  surface is destroyed, the layer is sent `removeFromSuperlayer` on the main thread alone,
  where AppKit's view tree is changed, and the surface's retain of it is released there:
  called there at once, or moved to a closure the main queue runs
  (`raw_window_metal::Layer` is `Send`). That retain need not be the last. A drawable
  retains its layer, and MoltenVK 1.4.2 releases a presented drawable on Metal's completion
  thread after it signals the present fence, so the layer's last release, and with it the
  drop of `raw-window-metal`'s `ObserverLayer`, which removes its key-value observers of the
  view's layer, may run on that thread; that crate's drop allows it, holding the view's
  layer weakly. Nothing here takes the observers' removal to be on the main thread. A
  surface never destroyed, on the leak path, leaks its layer with it, since the driver may
  still present to it.
- An acquire signals a semaphore no operation is pending on: one never used, or one whose
  waiting submit the present queue's timeline shows complete. A present's submit waits for
  the acquire's semaphore once and signals the image's present semaphore, whose last wait,
  that image's last present, has its present fence signalled first. `vkQueuePresentKHR` is
  called under the present queue's lock, which every submit to that queue takes, after the
  submit; every signal its semaphore depends on has been submitted, as the present's walk
  decided before the submit was stamped (VUID-vkQueuePresentKHR-pWaitSemaphores-03268).
- An image given up while acquired, at `resize` or the drop, has its semaphore waited for by
  an empty batch, so that no signalled semaphore is destroyed or reused unwaited. An acquire
  semaphore whose batches fail with the device not lost (a present's submit and the empty
  batch after it, or the empty batch alone) stays signalled, with no wait pending, since a
  failed `vkQueueSubmit2` leaves its semaphores as they were: the image's next present waits
  for it, and `resize` and the drop wait for every one left with an empty batch. A `resize`
  whose batch fails is refused before it gives up the driver swapchain that signalled it,
  and a drop whose batch fails leaks the device (below), so every acquire semaphore the
  swapchain destroys has had its acquire's signal waited for by a batch that has completed,
  except after a loss. The Vulkan guide's "Swapchain Semaphore Reuse" asks it; no
  valid-usage rule found names it, since an acquire is not a submitted batch
  (VUID-vkDestroySemaphore-semaphore-05149 is of batches alone). The drop
  waits for no batch of the present queue, which a submit held by a host point ahead of it
  on a shared queue holds: it hands its acquire and present semaphores to the device
  (`Left::spent`) with the stamp of the last batch that waits for one, after waiting for its
  presents' fences, and they are destroyed once the present queue reaches that stamp, or at
  the device's last drop, once it is idle. The stamp is read before the drop takes the
  chain, whose `unsignalled` is the batch after a present the driver did not take. They hold
  a count of their driver swapchain, so that it outlives every batch that waits for one of
  its semaphores; a `resize` hands the device the same count, with no semaphore, for the
  batch after the acquire it gave up. No valid-usage rule found requires the count
  (VUID-vkDestroySwapchainKHR-swapchain-01282 is of the uses of its acquired images, not of
  a pending wait for an acquire's semaphore): it is conservative.
- A driver swapchain is destroyed by the drop of its owner (`SwapchainObj`), which every
  image object of it holds, so it outlives every view of its images and every encoder and
  submit that names one. A `resize` and the swapchain's drop forget the images' handles,
  and hand the objects of those a submit not yet known complete names to the device
  (`Shared::left`), which lets each go once its stamps are reached, or at the device's last
  drop. A driver swapchain whose images or semaphores outlive the swapchain's drop stays the
  surface's (`Left::unretired`): the next swapchain names it as `oldSwapchain`, so the
  surface has at most one swapchain that is not retired, as the spec allows a native window.
  A retired driver swapchain is destroyed by whichever thread drops its last image object or
  lets go of its last count in `Left::spent`, which may be beside the present thread's
  `vkCreateSwapchainKHR`.
  The spec allows it: `vkDestroySwapchainKHR` externally synchronises only its `swapchain`, and
  `vkCreateSwapchainKHR` its `surface` and `oldSwapchain`. MoltenVK 1.4.2 reads and writes
  the surface's `_activeSwapchain` unlocked in both calls (`MVKSwapchain.mm` 651 and
  399–400, `MVKSurface.mm` 65–66), a race inside the driver that a retired swapchain does
  not reach, since it is never the active one.
  On the leak path no owner that sees the flag destroys anything (below).
- A driver swapchain is replaced, by `resize`, and its owner let go, only once every present
  fence of it has signalled, or after a loss; the present semaphores, only once the submits
  that wait for them are complete; it is retired whether its replacement's creation
  succeeds or not. A present fence never signals after an error that enqueued nothing (out
  of memory): the fence is reused, and the image's present semaphore, left signalled, is
  waited for by an empty batch before the semaphore is signalled again or destroyed. Where
  that batch fails too, the semaphore is destroyed with its chain only after the present's
  submit that signals it has completed (`unsignalled`, which `resize` waits for and the
  drop hands over), signalled with no wait pending; the swapchain is stale, so the image is
  not presented again before it. A fence
  of a present that failed with an error the spec does not list is waited for a bound after
  its present with the present queue idle, then destroyed: the bound with the queue idle is
  taken to show that the present enqueued nothing, so that no wait for the image's present
  semaphore is pending either, and the semaphore, left signalled, is destroyed with its
  chain's (`destroy_semaphores`); the error leaves the swapchain stale, so that image is not
  presented again. The one known source is MoltenVK 1.4.2's present with no command buffer
  (`MVKQueue.mm` 739–750), which takes neither the waits nor the fence. Its drawable timeout
  (`MVKImage.mm` 1625), the same error, signals the fence, which is reaped as any other.
- A drop whose waits do not end within three seconds, that unwinds a panic with something of
  the swapchain pending, or whose empty batch for an acquire semaphore fails, leaks the
  device (`Raw::leak`), and the records keep the swapchain's mark, so no second swapchain
  is made on the surface while the leaked one may be in use. A holder that sees the flag
  destroys nothing: the device's last holder (`let_go`) lets go of what `Shared::left`
  holds and returns, with no wait for idle and no sampler or orphaned pool destroyed, and
  the device and its surface are not destroyed.
  Two reads must see it, and do, the load being `Relaxed`: the leaked driver swapchain's
  drop, ordered after the store by its `Arc`'s release sequence, since the swapchain's drop
  lets its chain's count go after the store; and what `Shared::left` holds, drained by the
  last holder's `let_go` after `Holders::let_go`'s acquire. An image object another thread
  drops before it sees the flag may destroy its views, and a plain image its image and
  memory; nothing pending names those, so that is sound.
- `vkGetPastPresentationTimingGOOGLE` is called on the swapchain from the thread that
  acquires and presents, between those calls (`Swapchain` is not `Sync`).

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
  or after the device is lost, or at once where the submit's driver call failed with no
  command buffer pending (`unsent`): its `vkEndCommandBuffer` failed, or `vkQueueSubmit2`
  failed out of memory, and "VK_ERROR_OUT_OF_*_MEMORY errors do not modify any currently
  existing Vulkan objects" (the spec's "VkResult Return Codes"), so each command buffer is
  as it was, executable or invalid, and never pending. A drop that would wait while the
  thread unwinds a panic hands its pools to the device instead (`Shared::orphan`), whose
  last holder destroys them after the device is idle, or leaks them with the device.

### Submission and waits (`submit.rs`, `device.rs`)

- `vkQueueSubmit2` is called with the queue's lock held, which every submit to that queue
  takes, so the queue is externally synchronised. Each command buffer submitted has ended
  recording and has no other pending submit (`encoding`).
- A wait, a signal or a read of a timeline is made while a count keeps its semaphore alive;
  a queue's own semaphore lives as long as the device. A host signal's value grows past every
  value the timeline was signalled to.

### Drops while unwinding

- A drop of the device's last holder, made while the thread unwinds a panic with a submit
  pending, does not wait; it marks the device leaking (`Raw::leak`), and every drop that
  sees the flag returns without destroying anything, `Raw`'s included, which frees neither
  the sink nor the loader the leaked messengers reach. An image object another thread drops
  before it sees the flag may destroy its views, and a plain image its image and memory;
  nothing pending names them (above). Nothing is destroyed under the GPU's use, at the cost
  of the device's memory. With nothing pending, the unwinding last holder tears
  the device down as any last holder does.
- The holders' count (`Holders`, `holders.rs`) is decremented with `Release` and the last
  holder takes an `Acquire` fence, as `Arc`'s drop does, so every queue call another holder
  made happens before the last holder's wait for idle. Its unit test, which has no
  `unsafe`, has each holder store to a cell of its own, `Relaxed`, and the last load the
  other's, `Relaxed`, so only the count orders the two: with the decrement weakened to
  `Relaxed`, or the fence removed, Miri's weak memory gives the load the value before the
  store and the test fails, on 16 seeds of 16 each (seeds 0 to 15, four rounds a seed).

## What Miri covers

Nightly runs the crate's unit tests under Miri with four seeds: the records' fixed
interleavings, a race of host signals and reads on threads over them, the handle table's
drops outside its entry lock and a race of its generations on threads, the holders' count
on threads, through its weak memory, and the conversions and module reads. Those are this
crate's state that has no driver in it. Miri cannot load a driver, so nothing that reaches `ash` runs under it: `Raw`,
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
behaviour for the driver, which may deadlock instead: a watchdog thread aborts the process
after 30 seconds, which the run records as the canary failed (`the test failed`), with the
watchdog's line on its standard error. It aborts, not exits: an exit runs the layer's
teardown of the live device, which takes the layer's locks, and hangs where a stuck thread
holds one (reviewer-concurrency's probe, a thread parked in the layer's callback). The
runner's deadline of each test bounds the canary as well. The racing submit is made only
while the lagging dispatch is pending, checked by a wait of no time on the point it
signals; where the dispatch ended before the pause, the lag is doubled and the race tried
again, four times at most, and then the canary fails as `the lag ended before the pause`.
