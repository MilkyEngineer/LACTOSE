# lactose-rhi-vulkan

The Vulkan backend of `lactose-rhi-api`: a device made against the contract's
required set on a Vulkan 1.3 driver with `VK_KHR_dynamic_rendering_local_read`, reporting what
it offers of the optional set, that checks every rule of the contract's "Every backend" column
before the driver call the rule guards, with a swapchain where the request names a window or a
test asks for a headless surface. It runs on Linux and Windows drivers, lavapipe in CI,
and on macOS through MoltenVK or KosmicKrisp. Test support: the render crates may run their
tests on a device through it (decision 5.12 of the milestone 2 plan).

| | |
|---|---|
| Tier | backend, `[test_support]` and `[server_denylist]` in `crate-tiers.toml` |
| Determinism boundary | outside; a boundary crate's test may not name it (`cargo xtask boundary`) |
| Unsafe | yes, FFI to the Vulkan driver (architecture section 21); see `SAFETY.md` |

**Status: implemented.** The swapchain runs on a headless surface here and on a window in
`lactose-platform`'s window cases, which make the device in a main-lane call and the
swapchain, its resize and its drop on the present thread ("The swapchain").
`crates/rhi-api/README.md` is the contract, and this file states only what the contract
leaves to the backend.

## The public items

| Item | What it is |
|---|---|
| `VulkanDevice::new(DeviceRequest)` | A device on the adapter whose name contains the request's `adapter`, or on the loader's first; its sink takes every warning and error of the validation layer, the loader and the driver through the debug messenger; its pipeline cache starts from the request's bytes, empty, loaded or rejected; with the request's `surface`, a surface of that window and the present queue |
| `VulkanDevice::headless(DeviceRequest)` | As `new`, presenting to a surface no window shows (`VK_EXT_headless_surface`), for tests that need a swapchain; `InvalidData` for a request that names a window, the caller's error, and `Unsupported` on a driver with no headless surface |

`VulkanDevice` is a `Device`; its encoders are `Box<dyn Encoder>`.

One feature, `swapchain-copies`, is for dev-dependency edges alone: with it a swapchain's
images are made for copies from and to as well, where the surface allows both, so that a
window test reads a swapchain image back. A hidden switch it adds,
`VulkanDevice::set_swapchain_copies`, turns that off for the swapchains made after it, so
that a window test presents with the usage a shipped build has. The contract's swapchain
images are colour attachments and nothing else, and nothing that ships turns it on.

## What the device makes

- **The loader** is opened at run time (`ash::Entry::load`), so a process with no Vulkan
  loader gets `Unsupported` from `new`, never a link error. Every driver call goes through
  `ash` 0.38.
- **The adapter** must offer Vulkan 1.3 with `VK_KHR_dynamic_rendering_local_read` and the
  features `missing` in `src/raw.rs` lists: dynamic rendering and its local read,
  synchronization2, timeline semaphores, buffer device addresses, descriptors partially bound,
  updated after bind and while unused by pending submits, non-uniform indexing of sampled
  images, independent blend, `shaderInt64`, storage image reads without a format and indirect
  draws' first instance. Its limits must be at or above `Capabilities::LEAST`'s, and at or
  above the heap and sampler table the request asks for: the update-after-bind sampled
  images and samplers, per stage and per set, `maxPerStageUpdateAfterBindResources` against
  the heap, the table and a pass's attachments together, and `maxSamplerAllocationCount`
  against the table and its placeholder. It must offer host-visible memory. A lack is
  `Unsupported`, naming it. Each optional capability the adapter offers is
  enabled and reported.
- **Queues by role.** The graphics queue is the first of a family that draws and computes.
  Compute has a queue of its own in a family that computes and does not draw, else in another
  family that computes, else as a second queue of the graphics family; with none it shares the
  graphics queue (`QueueLayout`). On a device with a surface, present has a queue of its own
  in a family other than the graphics family that draws and can present to the surface, else
  a further queue of the graphics family, else it shares the graphics queue where that family
  presents, and the device is `Unsupported` where no family presents; on a device with no
  surface it shares the graphics queue. The compute family may not draw, so a `Compute` encoder's render pass and depth
  image copy are refused (`encoding`) on every layout, as the contract says; its graphics
  bind and draw, which need a render pass, are refused by the scope rule. Every queue has a timeline semaphore that each submit to it signals to the submit's
  stamp after its last command. Before each check the device learns how far a queue or a
  timeline has run with a wait of no time on each value it awaits, never a read of the
  counter, so what it counts as pending is completion: synchronization validation takes a
  counter read as the host seeing every batch complete, and then reports no hazard across
  submits.
- **Memory.** One `VkDeviceMemory` per buffer and per image not placed in a block, of the type
  its class prefers: `Device` a device-local type that is not host-visible where there is one,
  `Upload` a host-visible coherent one, `Readback` host-visible, cached and coherent. Host
  memory is mapped whole for the resource's life; a flush or an invalidate is rounded out to
  `nonCoherentAtomSize`. A memoryless request takes a lazily allocated type where the device
  reports the attachment's kind memoryless, and ordinary memory otherwise. An allocation
  larger than its type's heap or `maxMemoryAllocationSize`, and a buffer larger than
  `maxBufferSize`, is refused `ResourceExhausted` before any driver call, as null refuses a
  buffer past its bound. An image placed in a block is bound at its offset; its alias call
  records a global memory barrier from every command's writes to every later access, which
  the aliasing rule needs between the ending image's writes and the new one's, and which the
  image's own barrier from `Undefined`, scoped to its range, does not give.
- **The heap and the sampler table** are one descriptor set (set 1), partially bound and
  updated after bind, with a placeholder image in each empty slot and a placeholder sampler in
  each free entry, so a slot is never unbound. A slot write and a sampler's free are the
  contract's two holds, under the heap's lock. The device asks the driver whether it
  supports the set's layout (`vkGetDescriptorSetLayoutSupport`), and the input attachments',
  before it makes them, and refuses a request it does not (`Unsupported`): a driver may
  support fewer descriptors in one set than its per-type limits name. MoltenVK 1.4.2
  supports at most 1211 slots and samplers in the set, one below its `maxPerSetDescriptors`
  of 1212, so beside the default 64 samplers its heap holds at most 1147 slots (the device
  test `heap_past_the_drivers_support_is_refused` holds it to that boundary); KosmicKrisp
  supports a million. The renderer's heap must fit the smaller. A
  sampler that would make the device's live samplers, the placeholder and those retired
  under a pending submit included, reach `maxSamplerAllocationCount` is refused
  `ResourceExhausted`.
- **Pipelines** are made from cooked SPIR-V (`CookedShader` version 2), one stage module per
  creation, through one pipeline cache. Before the driver sees a stage, `CookedShader::check`
  holds it to the format's rules and, for a graphics pipeline, `check_varyings` holds the
  fragment stage's reads to the vertex stage's writes: the layer reports nothing for a `uint`
  varying read as an `int` on MoltenVK or KosmicKrisp (the GPU review of `shader-format`), so
  the format's refusal is the only guard. The module's header is checked and its `OpCapability` and `OpExtension` instructions
  are compared with what the device enabled (`capability`). The module must declare an
  `OpEntryPoint` of the stage's `entry` name and its execution model (`Vertex`, `Fragment`,
  `GLCompute`), or the stage is refused `InvalidData`: the driver takes an entry by its name
  alone. Each given constant is one
  four-byte map entry of its specialisation data, its `ConstantValue` of its slot's type, a
  `Bool` widened to a `VkBool32` of 0 or 1 (VUID-VkSpecializationMapEntry-constantID-00776);
  an unwidened one-byte `Bool` reads back right on both Apple drivers, so the layer's message
  is what shows it. Vulkan runs a compute stage's workgroups at its module's size, so a stage
  whose `workgroup` is not the one `OpExecutionMode LocalSize` its module states for its entry
  point is refused `InvalidData`, as is one whose module states none, states two, or decorates
  an id `BuiltIn WorkgroupSize`. Each creation asks the driver for its creation feedback; a
  stage the driver marks as an application cache hit counts as a hit in `pipeline_stats`,
  and one it does not mark valid counts neither way: lavapipe (Mesa 26.0.8) marks the
  pipeline's feedback valid and zeroes every stage's, so it counts no stage.
- **The pipeline cache** is sealed with `lactose-rhi-api`'s seal under the vendor, the device
  and the driver's cache UUID. Where the sealed bytes are kept is the caller's. A payload that fails the seal, whose Vulkan header names
  another device, or that the driver refuses, starts the cache empty and `Rejected`.
- **The encoder** owns a command pool and one primary command buffer on its queue's family.
  It records as the caller calls, after every check of the "Every backend" column a command
  can be refused by while recording; the first breach stops the recording, and its submit is
  refused with the breach's rule. A transition is recorded as one `vkCmdPipelineBarrier2` on
  the recording queue, its scopes from the two states (`lactose-rhi-api`'s README, "States");
  its elements are unordered, so a call that names one subresource or one buffer twice is
  refused `state` at the call, before any of it is recorded, images by
  `ImageRange::overlaps` and buffers by handle, as null compares; a family without
  `GRAPHICS` gets no graphics stage. A re-take from `Undefined` carries `ALL_COMMANDS` and
  `MEMORY_WRITE` as its source scope. Two roles on two queues of one family are one layout
  change on the releasing queue and a semaphore (D160). A local read is a by-region barrier
  from colour attachment output to fragment shader input reads inside the render pass, and a
  render pass that marks attachments for it binds an input attachment set from the encoder's
  own pools.
- **The submit** takes its queue's lock and the records', makes every check a submit can be
  refused by, stamps the submit, gives the records' lock up and calls `vkQueueSubmit2` with
  the caller's waits and signals and the queue's own timeline signal last.

## The swapchain

- **The surface** is made from the window's handles as the constructor reads them: an
  AppKit view's through a `CAMetalLayer` that `raw-window-metal` makes as the view's
  sublayer and keeps at the view's bounds and `contentsScale` on the main thread
  (`VK_EXT_metal_surface`; the constructor runs on the main thread there, as AppKit asks, or
  is `Unsupported`), a Win32, Xlib, XCB or Wayland window's through its `VK_KHR_*_surface`.
  The instance takes `VK_KHR_surface`, `VK_KHR_get_surface_capabilities2` and every surface
  maintenance extension offered; the device `VK_KHR_swapchain` and
  `VK_EXT_swapchain_maintenance1` where the instance took `VK_EXT_surface_maintenance1`, else
  the `KHR` form of each by name, with the `swapchainMaintenance1` feature. At the device's
  last drop, once the surface is destroyed, the layer made for an AppKit view is taken out of
  the view (`removeFromSuperlayer`) and the surface's retain of it released on the main
  thread: at once on it, and otherwise from the main queue (`dispatch`), which runs once
  that thread returns to its run loop. A presented drawable's retain may be the layer's
  last, which MoltenVK releases on Metal's completion thread, so the layer's observers of
  the view's layer may be removed there. A view whose own layer is a `CAMetalLayer` keeps
  it. A device leaked on the leak path leaves the layer in the view, alive. `ash` 0.38
  names neither `KHR` form; their structures and values are the `EXT` form's, which the
  backend uses for both. The backend calls no
  `vkReleaseSwapchainImagesEXT`, which the `KHR` form names otherwise.
- **The images** are `FIFO`, of the first of `B8G8R8A8_UNORM`, `R8G8B8A8_UNORM` and their
  `SRGB` forms the surface offers in `SRGB_NONLINEAR`, three where the surface allows,
  exclusive to the present queue's family, colour attachments alone (and copies, under the
  `swapchain-copies` feature). Their handles are the
  device's images, with a colour attachment view each; a destroy of one is refused
  (`swapchain`). Each image's object holds the driver swapchain, which is destroyed at the
  drop of its last holder: a `resize` or the drop forgets the handles, but an encoder that
  recorded an image, or a submit not yet known complete that names one, keeps the image and
  its driver swapchain until it lets go or its stamp is reached. Where one outlives the
  swapchain's drop, the driver swapchain stays the surface's: the next swapchain is made from
  it (`oldSwapchain`), or the device's last drop destroys it. A swapchain image's transition to `Present` is to `PRESENT_SRC_KHR`; any
  other image moved to `Present` is in `GENERAL`.
- **An acquire** signals a semaphore of a pool, taken again only once the submit that waited
  for it is known complete by the present queue's own timeline. Each image has its own
  present semaphore, which the present's submit signals and its present waits for; before it
  is signalled again, the image's last present fence (`VkSwapchainPresentFenceInfoEXT`,
  chained to every present) has signalled. An acquire's timeout, `VK_TIMEOUT` or
  `VK_NOT_READY`, is `None`; `VK_ERROR_OUT_OF_DATE_KHR` and `VK_ERROR_OUT_OF_POOL_MEMORY` are
  `Stale`, and the swapchain stays stale until a `resize`.
- **A present** walks the records under the present queue's lock and stamps its submit once
  the walk finds it released, then makes the submit, with the acquire's semaphore among its
  waits and the image's present semaphore among its signals, and `vkQueuePresentKHR` under
  the same lock. Where the submit's driver call fails and the device is not lost, the stamp
  is reached by an empty batch that waits for the acquire's semaphore, and the image stays
  acquired for a retry that waits for nothing; where that batch fails too, the semaphore
  stays signalled and the retry waits for it. The present's fence is taken before the
  submit. A present's failure after its submit is taken is `DeviceLost` for
  `VK_ERROR_DEVICE_LOST` and `Presented::Stale`, with an id, for every other (6(i) of the
  frame's design), the swapchain stale until a `resize`. What the present fence does after
  each error differs, by the spec and under MoltenVK 1.4.2, and the backend follows it:

  | `vkQueuePresentKHR`'s error | Enqueued | The fence | The backend |
  |---|---|---|---|
  | `VK_ERROR_OUT_OF_HOST_MEMORY`, `VK_ERROR_OUT_OF_DEVICE_MEMORY` | No: the spec names only the three below as enqueued | Never signals | Takes the fence back, and waits for the image's present semaphore, left signalled, with an empty batch, which `resize` waits for and the drop's semaphores are destroyed after; where that batch fails, the present's submit, which signals the semaphore, takes its place |
  | `VK_ERROR_OUT_OF_DATE_KHR`, `VK_ERROR_SURFACE_LOST_KHR`, `VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT` | Yes, by the spec | Signals | Waits for it as for a shown present's |
  | `VK_ERROR_DEVICE_LOST` | Either | Need not signal | Nothing waits after a loss |
  | Any other, MoltenVK's `VK_ERROR_OUT_OF_POOL_MEMORY` among them | Not stated | MoltenVK 1.4.2 signals it where the drawable timed out, and never where the present had no command buffer | Waits for it as for a shown present's; one not signalled a bound after its present, with the present queue idle, is taken to have enqueued nothing and is destroyed, and the image's present semaphore with its chain: MoltenVK's present with no command buffer is the one known source |

  No safe call makes a present fail on a headless surface: a unit test injects the refusal
  with nothing enqueued, out of memory and an unlisted error. The enqueued errors and the
  late signal of an unlisted one are by inspection.
- **`resize`** waits for every pending present fence within the bound and is refused `pending` past
  it; it spends every acquire semaphore no batch has waited for, an acquired image's or one whose
  batches failed, with an empty batch, which holds a count of the old driver swapchain until it
  completes, as the drop's batches do, and is refused with the driver's error, before it gives up
  the old swapchain, where one fails: the acquired image is then given up, the swapchain is not
  stale, its extent and images are as they were, and the next `acquire` takes an image of them, as
  `lactose-rhi-api`'s `resize` says. Where a wait for the presents fails with the driver's error,
  the acquired image is given up all the same, by those batches, and the swapchain is as it was. It
  makes the new swapchain from the old (`oldSwapchain`), drains the old one's display times, lets go
  of it and forgets its images' handles, which are then `destroyed`. An extent is clamped to the
  surface's minimum and maximum as queried just before. Where `vkCreateSwapchainKHR` or the new
  images' handles fail, the old swapchain is retired all the same (the spec retires it either way):
  the swapchain has no images, reports a zero extent and is stale, its acquire answering `Stale` and
  its present refused, until a later `resize` makes one from nothing.
- **A zero extent.** While the surface's current or largest extent has a zero dimension, as a
  minimised window's may, no driver swapchain is made: a `create_swapchain` there has no
  images, and a `resize` there keeps the images it has, as `resize`'s contract says. Either
  way the swapchain is stale, `acquire` answers `Stale`, and a `resize` once the extent is
  not zero makes one. A unit test holds the decision, which reads only the
  surface's capabilities.
- **The drop** waits, for at most three seconds, for the pending present fences, which the
  presentation engine signals, and while its thread unwinds a panic waits for nothing. It spends
  every acquire semaphore no batch has waited for with an empty batch, as `resize` does, and waits
  for no batch of the present queue: it hands the acquire and present semaphores to the device with
  the stamp of the last batch that waits for one, the batch after a present the driver did not take
  included, or where that batch failed the present's submit, and a count of the driver swapchain,
  and they are destroyed once the present queue reaches that stamp, or at the device's last drop. So
  a submit held by a host point on a queue the present role shares, as on a driver of one queue such
  as lavapipe, neither holds the drop nor leaks the device, and a `create_swapchain` after it is
  made from the old driver swapchain. Where a present may still be in the presentation engine's use,
  its fence not signalled within the wait, or an acquire's semaphore is left with no batch to wait
  for it, while unwinding or where its empty batch failed, so that the acquire's signal may be
  pending, the drop leaks the device (`Raw::leak`), as the device's last holder does with a submit
  pending: nothing of the swapchain or the device is destroyed under that use, at the cost of the
  device's memory, and the records keep the swapchain's mark, so a second `create_swapchain` is
  refused (`swapchain`) and never reaches the driver with the surface in use. After a loss it
  destroys them at once.
- **The bound** of the contract is one second: an acquire's timeout, a present's wait for its
  submit's release, and `resize`'s wait for the presents. The driver's own calls, the
  present's included, are bounded by the driver alone: under MoltenVK as
  `crates/rhi-api/README.md` states, and on other drivers unmeasured here.
- **Headless.** MoltenVK 1.4.2 and lavapipe offer `VK_EXT_headless_surface`, so the headless
  tests run under lavapipe in CI; KosmicKrisp offers none, and they skip there.
- **On a window under MoltenVK 1.4.2** (`lactose-platform`'s window cases, on the M1): an
  acquire returns at once, a window's resize makes a present `SUBOPTIMAL` and never
  `OUT_OF_DATE`, which the contract's `Shown` hides, so a caller rebuilds at the window's
  reported size; and a minimised window's drawable did not block, in 8 s of presents, each
  shown. A drawable MoltenVK cannot get, after about a second per image, is
  `VK_ERROR_OUT_OF_POOL_MEMORY` and so `Stale`; no run here made one.
- **On a window under KosmicKrisp** (SDK 1.4.363): present shares the graphics queue, an
  acquire waits for its image (a median 14 ms of a 16.7 ms refresh), which reads as the drawable
  taken there rather than inside a submit, a present returns within a millisecond, a minimised
  window's drawable did not block either, and there is no display-time source.

## The display-time report

- **Source:** `VK_GOOGLE_display_timing`'s `vkGetPastPresentationTimingGOOGLE`, on a device
  with a window on macOS whose driver offers the extension, which is MoltenVK; every present
  chains `VkPresentTimesInfoGOOGLE` with its id's low 32 bits and `desiredPresentTime` zero,
  and a report's id is widened back from the last present's. Every other device answers
  `false`: a headless surface, KosmicKrisp and lavapipe, which offer no source, and a device on
  another system, where no clock pairing is built. `VK_EXT_present_timing` is not in `ash`
  0.38, and no device here offers it.
- **The instant:** `actualPresentTime`, which MoltenVK 1.4.2 takes from the drawable's
  `presentedTime`, in nanoseconds of the host clock `CLOCK_UPTIME_RAW` that `Instant` reads on
  macOS. The swapchain reads both clocks at its creation and moves each time onto `Instant` by
  that pairing: the host clock is read between two `Instant`s, again until the two are
  within 10 µs or for 100 reads, and the narrowest read is paired with its bracket's
  midpoint, so the pairing's error is at most half that bracket, 5 µs where the bound was
  met.
- **A present the display never took** cannot be told apart: MoltenVK 1.4.2 reports one with
  the time its handler ran, not zero. A caller takes a present as displayed once its id or a
  later one is reported.
- **The call** reads the driver's history into a buffer of 64 entries, more than the 60
  MoltenVK keeps, and again while the driver says more are held, at most 256 entries a call;
  MoltenVK takes off every entry it writes. The entries are sorted by id, those at or below
  the last reported left out and repeats dropped, and appended to the caller's buffer, which
  the swapchain's own buffers, kept for their capacity, reach without allocating once they
  have room. Unit tests hold the widening across the 32-bit wrap, the watermark, unsorted and
  repeated entries, a backlog followed by a new read, and the read's guard. It never waits:
  the bound, 5 ms, held at every call of `lactose-platform`'s window run, whose presents each
  waited for an image, and the longest call measured in two runs was 0.04 ms.
- **The delay:** in those runs under MoltenVK 1.4.2 at 60 Hz, a present was shown a median
  47 to 48 ms after its call returned (three refreshes, the FIFO queue full), and reported a
  median 2 to 3 ms after it was shown, at most 35 ms. A replaced swapchain's history is
  drained before it is destroyed, and none of the runs' replaced swapchains, two in each,
  lost a report; a report that arrives after the drain would be lost.

## Locks

`Shared::records` holds every record (`src/records.rs`). The order is: a queue's lock or the
heap's, never both and never two queues'; then the records'; then one entry's, a handle's in a
table or a heap slot's. Recording takes one entry's lock alone, once for each handle between
resets, and never the records'. No lock but a queue's or the heap's is held across a driver
call, and none across a wait or the caller's closure: a write and a read lend their range
under the records' lock, copy with none held, and clear the range in a second hold; a flush
and an invalidate are checked under the records' lock and call the driver with none held,
holding no range, as null's hold none. A destroying call is a driver call too: a table's replaced or
removed count, a slot's replaced image and a reaped sampler are dropped or destroyed once the
records', entry and slot locks are given up, so a message the layer sends from one reaches
the sink under a queue's or the heap's lock at most.

The drops that wait: an encoder's, for its pending submit, and the last holder of the device's (the
device or an encoder), for the device to idle. An encoder whose submit's driver call failed out of
memory, or whose end failed, has no command buffer pending, by the spec, and its drop and reset wait
for nothing, though the stamp its submit took is reached only by a later batch of the queue where
the empty batch after the failure failed too. A drop while the thread unwinds a panic does not wait.
An encoder's, with its submit pending, hands its pools to the device, whose last holder destroys
them once the device is idle. The last holder's, with a submit pending, leaks the device whole, the
messengers' sink and the loader included, rather than destroy anything under the GPU's use; with
nothing pending it tears down as any last holder does.

## Tests

Every test that makes a device is ignored by a plain run, which may have no driver;
`cargo xtask gpu` runs each ignored test of the library's `canary` and `swapchain` modules and of the
`integration` binary in a process of its own, under the pinned Khronos validation layer, by
decision 5.10's rule (`tools/xtask/README.md`, "GPU runs"). On the M1 the gate is both
drivers: MoltenVK under Metal API and shader validation as well, and KosmicKrisp without.

- `tests/conformance.rs`: every case of `lactose-rhi-api`'s suite, one test each, on the
  driver the loader finds. A case of a check null alone makes, a case that needs a device
  loss, a window or a capability the device does not report, is skipped and says why. The
  output cases read their shaders from `$LACTOSE_GPU_SHADERS`, which `cargo xtask gpu` cooks.
- `tests/device.rs`: the device's facts, sent nothing when clean; a device-only buffer filled
  through a staging copy and read back through a copy; the engine's shader,
  `shaders/surface.slang` as cooked, making its two pipelines, not run; a compute stage whose
  workgroup is not its module's, refused; an entry point the module lacks, or declares for the
  other stage, refused; a graphics signal, and a host signal, beside a lagging compute
  submit's promise, refused while it is in flight, skipped where compute shares the graphics
  queue; a heap and sampler table past the driver's support refused
  `Unsupported`, and one within it made; a buffer and a block past the device's size refused
  `ResourceExhausted`; a cache that fails a check (a corrupt payload, a truncated file, a
  wrong checksum, a stale header) starting empty; a cache whose directory cannot be written,
  which the run reports and goes past; and the drops while a thread unwinds: a last holder
  with a submit pending leaks the device and keeps its messengers' sink, one with nothing
  pending destroys everything, and an encoder dropped with its submit pending hands its pools
  to the last holder.
- Each validator's canary, a misuse whose message alone the run must hear. In
  `tests/device.rs`: core, an image declared in a state it is not in; synchronisation, two
  copies into one range; its load-after-store check, a target stored by one render pass and
  kept by the next with no transition; synchronisation across queues, a target a lagging
  graphics submit stores and a compute submit takes from `Undefined` with no semaphore, skipped
  where compute shares the graphics queue; synchronisation across two submits of one queue,
  which every device runs, a buffer a lagging submit copies into and a second submit takes
  from a state it is not in; GPU-assisted, a shader's read past a buffer, and a draw that
  samples slot `heap_slots` through push bytes, both run by `--gpu-assisted` alone; and
  Metal's shader validation under MoltenVK, the same read past a buffer, whose report on
  standard error `cargo xtask gpu` must hear. In `src/canary.rs`, a unit test since its
  misuse is a driver call no safe call makes: thread safety, the graphics queue waited idle
  with no lock on one thread under a lagging dispatch while another submits to it, made only
  while the dispatch is pending (its lag doubled up to four times where it ended first), with
  a watchdog that aborts the process after 30 seconds should the driver hang.
- `tests/swapchain.rs`, on a headless surface: 48 frames, two in flight, presented across a resize,
  each clearing a target on the graphics queue that crosses to the present queue, exclusively by a
  release and an acquire where present has its own queue and concurrently, and is read back there
  against the frame's colour, each present shown with the next id and none stale; a shutdown with
  three frames in flight, again with an image acquired and never presented; an encoder that recorded
  the acquired image recording it again after a `resize`, and after the drop with a second swapchain
  made, silent under the layer; and a request that names a window refused by `headless` with
  `InvalidData`, which needs no driver. In `src/swapchain.rs`, unit tests, since no safe call makes
  their failures or waits for an acquire: an acquire semaphore taken again while a batch's wait for
  it is held, which the layer must report (`VUID-vkAcquireNextImageKHR-semaphore-01779`), heard by a
  sink that prints nothing, a core validation check, so the GPU-assisted run, which turns core
  validation off, skips it; a drop whose present never ends, which leaks the device, the layer
  silent, and refuses a second swapchain; on a device whose present role is served by the graphics
  queue, a drop behind a graphics submit held there by a host point on the acquired image, which
  neither waits for it nor leaks the device, a second swapchain made at once that presents once the
  point is signalled; a `resize` whose `vkCreateSwapchainKHR` or image fails, stale until a later
  `resize`, the injected `vkCreateSwapchainKHR` failure retiring the old driver swapchain as the
  call does; a `resize` whose wait for a shown present's fence fails out of host memory, injected,
  which gives the acquired image up and leaves the swapchain as it was, not stale, the image's
  acquire semaphore waited for by the give-up's empty batch or, where that batch fails too, by a
  later `resize`, on the present role's own queue and on the graphics queue, the layer silent; a
  present refused with nothing enqueued, out of memory and an unlisted error, stale until a
  `resize`, the unlisted error's fence and semaphore destroyed and the layer silent, and after such
  a present, behind released slow work, a drop that hands the semaphores over with a stamp at or
  past the batch that waits for the present semaphore, as a test-only record of the stamp shows
  whether or not the batch has completed, the layer silent; an acquire semaphore whose batches
  failed out of host memory, injected (`submit::FAILING`), left signalled and waited for by the
  image's next present, after a present whose submit and empty batch both failed, and by the next
  `resize`, after a `resize` refused for its empty batch with its driver swapchain kept, the layer
  silent; a drop whose empty batch for an acquire fails, which leaks the device and refuses a second
  swapchain, the layer silent; a present the driver did not take whose empty batch for the present
  semaphore fails too, the present's own submit behind released slow work, on the present role's own
  queue and on the graphics queue, the chain's stamp that submit's, and a `resize`, retried while
  refused `pending`, that returns only once that submit is complete rather than destroy the
  semaphore under it, the layer silent; the encoders of a present whose submit and empty batch both
  failed dropped at once, with no later batch of the queue, where each waited until the next
  present; a graphics submit held by a host point that writes the acquired image, submitted once the
  host has seen a batch that waits for the image's acquire complete, pending across a `resize` and
  across the drop with a second swapchain made, the image's object and its driver swapchain kept
  until the submit is known complete and let go after, with no message; on the device whose present
  role the graphics queue serves, an acquired image given up by `resize` and by the drop behind a
  held submit that names no image, the driver swapchain kept by the count handed over with the
  batch's stamp alone until the point is signalled, and let go after; a frame submit pending on an
  acquired image, after the host has seen its acquire waited for, held by a host point, at a
  `resize`, its encoder dropped while a panic unwinds, the replaced driver swapchain kept until the
  point is signalled, and the layer silent; and frames presented beside a frame thread whose submit
  each present's walk finds stamped and not yet received, each undecided at least once (a test-only
  count). With no driver: the zero extent's decision, the images' usage with and without copies, and
  the display-time report's widening, watermark, order and read guard.
- On a window, in `lactose-platform`'s `tests/window.rs` (`tests/swapchain.rs` there), run on
  the M1 under MoltenVK, with Metal API and shader validation, and under KosmicKrisp, each
  with the layer: 131 frames across a resize to half the window and a rebuild right after a
  present, every present's image read back equal to its target, which the
  `swapchain-copies` feature allows, the image cleared first to a colour its target is not,
  so that only the copy makes the two equal; each id reported once and in order, the call's
  bound held at every call; with the copies turned off, the images colour attachments
  alone, which MoltenVK makes framebuffer-only, a present whose encoder copies one refused
  with `Rule::Usage`, with nothing submitted, and 36 frames presented, none stale, with no
  message and no driver line; and frames presented to a minimised window for 8 s while
  workers draw.
  `cargo xtask gpu` does not run them; the window run does, on the driver the loader finds.
- Unit tests in `src/pipeline.rs`: the workgroup a module states for an entry point, no size,
  two sizes or the built-in state no workgroup, an entry declared by its name and its stage's
  execution model, a `Bool` widened to four bytes and `Int64Atomics` refused unless enabled.
  In `src/convert.rs`: the re-take from `Undefined`, the transfer within a family as one
  layout change, the transfer between families as a release and an acquire of equal values,
  no barrier between two families naming the host, the host's barrier after an acquire into
  `HostRead` on the acquiring family alone, a family without `GRAPHICS`, and two writes in
  one writing state. In `src/encoder.rs`: `buffer_calls` puts the host barrier after an
  acquire into `HostRead` between two families in its second list, on the acquiring side
  alone, and none after a release or a change within one family; that `transition_buffers`
  records the two lists in that order, as two calls, is held by reading. In `src/raw.rs`:
  compute
  takes a family that does not draw first, and viewport bounds rounded inward to what an Apple
  GPU honours, and the swapchain's maintenance form following the surface form the instance
  enabled. In `src/records.rs`: a present's walk leaving a promise undecided while its submit
  is stamped and not received or failed, and releasing it once received, or once its point
  is reached; a reached point completing its submit and the earlier ones
  of its queue, a failed submit's promise kept until its stamp and its wait are known
  reached and its point then retried, a failed submit let go once the caller has seen its
  waits and a later point of its queue, its promise of a removed timeline completing
  nothing, its waits ordering no later submit of its queue and giving no credit at the
  top, and its wait for its own timeline met by a host signal below its promise, another
  queue's pending signal needing a wait, a wait
  ordering only the signal it waits for and the earlier submits of that signal's queue (the
  shapes null's test of the same name holds null to), a host signal below a promise; the
  interleavings of a driver read and its hold, each fixed in order against a model driver (a
  host signal or a dropped promise between a read and its hold, a pass begun during a slot
  write, a destroy at once after a wait with the queue behind, a failed submit's point
  signalled from the host after another queue's submit was ordered through its promise or
  its wait, which no device test can provoke since no driver's submit fails on demand, a
  wait-only submit 2^31 below a marked host signal, a stale read folded at a late hold, a
  wait that reaches two submits of one queue), and host signals racing on threads over one
  timeline, which stays behind the driver. In `src/table.rs`: a replaced or removed value
  dropped after its entry's lock, and a handle resolving to its own value or none across
  threads. In `src/holders.rs`: the last holder sees every other holder's `Relaxed` stores,
  ordered by the count alone. Nightly runs these under Miri ("What Miri covers" in `SAFETY.md`).

## Records

The runs on the M1 (macOS 27.2.1, Vulkan SDK 1.4.363.1), each by `cargo xtask gpu` with every
message counted; the first two are the gate, which fails on a failure of either. Under `system` the loader's first device is MoltenVK's, and the run is
MoltenVK's; both drivers name the device `Apple M1`, so `--adapter` cannot choose between them.

| Driver | Passed | Skipped | Failed |
|---|---|---|---|
| MoltenVK 1.4.2, under Metal API and shader validation | 186 | 37: 31 null alone, 4 no loss, `indirect_count` not reported, compute on its own queue | 0 |
| KosmicKrisp (SDK 1.4.363) | 147 | 75: 31 null alone, 9 no window, 17 no headless surface, 4 no loss, 11 compute sharing the graphics queue and 1 compute with no queue of its own, the canary across queues (no second queue), every optional draw capability reported | 0 |
| KosmicKrisp under GPU-assisted validation | 142 | 75: as above, but for the canary across queues, which is not this form's, and the drop that leaks the device while unwinding, which GPU-assisted validation aborts on at exit; the injected acquire-semaphore reuse skips for core validation off | 0 |

The counts are of the device tests: the suite's 186 cases, 158 contract and 28 output, and the
device's own; the Metal canary runs under MoltenVK alone. Each driver passes 27 of the 28
output cases: MoltenVK skips the count draw, and KosmicKrisp the ownership transfer, since
compute shares its one queue. Under Metal's validation every case that does not skip prints
both banners; a skipped case is exempt, since it may skip before it makes a device.

GPU-assisted validation does not run under MoltenVK 1.4.2: its instrumented shaders fail in
SPIRV-Cross (a `uint` to `ulong` `reinterpret_cast`). On an Apple GPU the device reports
viewport bounds of at most -16384 to 16383, whatever the driver reports (verification row
183).
