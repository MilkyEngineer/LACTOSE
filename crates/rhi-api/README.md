# lactose-rhi-api

The render hardware interface (architecture section 12.1): the one surface the render graph,
the renderer, the present thread and the client's frame are written against, and that the
null, Vulkan and Metal backends implement. A device and its queues by role; buffers with
device addresses, images and samplers; one bindless heap; pipelines from cooked shaders;
encoders one thread records; resource states the backends derive their barriers from;
timelines; the swapchain; and the shape of the conformance suite every backend runs.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented by `lactose-rhi-null`.** Every public signature is final for milestone
2's lanes. What runs here: the cooked shader's `encode` and `decode`, `seal_cache` and
`open_cache`, the rule names with `Rule::refuse` and `Rule::of`, the queue layout, the flag
sets and the indirect record, and, behind `conformance`, the suite's `run` and the bodies of
its contract cases, written with `lactose-rhi-null`, which implements the traits. The output
cases' bodies arrive with `lactose-rhi-vulkan`. This file is the contract the backends meet
(`rhi-null`, `vulkan`, `swapchain`, and `rhi-metal` at milestone 3), and every sentence
under "Behaviour the
signatures cannot express" is one a dependent crate may rely on. Architecture sections 12.1,
5.1, 19, 20 and 21 are the design, with the milestone 2 plan's decisions 5.6, 5.8, 5.10 to
5.13 and 5.17; where this file and the architecture disagree, the architecture wins and this
file is wrong. Decision D158 records the choices section 12.1 left open, and the side this
file takes on the native Metal spike's three conflicts, which its merge wrote into section
12.1 ("The three conflicts"). The two
spikes are the evidence, and are cited as **RHI** (`docs/results/spike-rhi-apple-m1-2026-11-15.md`)
and **Metal** (`docs/results/spike-metal-apple-m1-2026-11-17.md`).

The interface is unstable (D43) until `vulkan`'s merge freezes it (decision 5.13). Amending a
signature is allowed in the change that needs it. Say so in the change description and list
what changed, because `lactose-rhi-null`, `lactose-rhi-vulkan`, `lactose-render-graph`,
`lactose-renderer`, `lactose-platform` and `lactose-app` are written against this surface.
`tests/consumers.rs` is a compile-only sketch of what each of them writes, a backend
included: a change that breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `DeviceRequest`, `DeviceInfo` | What every backend's constructor takes (the adapter by name, the window, the heap's and the sampler table's capacity, the saved pipeline cache, the message sink), and what the device says it is. The facade has no constructor |
| `Device` | The `dyn` object a backend hands out, `Send + Sync`: resources, the heap, pipelines, timelines, encoders, submits, the swapchain, `is_lost` |
| `Capabilities`, `Capabilities::LEAST` | The reported set, fixed for the device's life. The required set is not a type ("Capabilities") |
| `QueueRole`, `QueuePath`, `QueueLayout` | Queues by role, graphics, compute and present, and which roles a device gives a queue of their own |
| `BufferHandle`, `BufferDesc`, `BufferUsage`, `MemoryClass`, `Sharing` | Buffers: size, usage, memory class, and whether one queue owns them |
| `ImageHandle`, `ImageDesc`, `ImageUsage`, `ImageMemory`, `Format`, `Extent`, `ImageRange` | Images: two dimensions with mips and layers, and where their memory comes from, a memoryless request and a placement included |
| `MemoryBlockHandle`, `MemoryBlockDesc`, `Placement` | A block of device memory images are placed in, where they may alias |
| `HeapSlot`, `SamplerIndex`, `SamplerDesc`, `Filter`, `AddressMode` | The heap's slots and the sampler table's entries: the numbers a shader indexes with |
| `ImageState`, `BufferState`, `StateOn`, `ImageTransition`, `BufferTransition` | Resource states, each on a queue, and the changes the caller records |
| `PassReach`, `BufferReach` | What a pass declares it reaches: heap slots, and buffers by device address |
| `Encoder` | What one thread records, `Send` and not `Sync`: transitions, the alias call, copies, passes, typed push constants, the local read, draws and dispatches |
| `RenderPass`, `ComputePass`, `ColourAttachment`, `DepthAttachment`, `Target`, `Load`, `Store`, `Viewport`, `Rect` | A pass and its attachments |
| `BufferCopy`, `BufferImageCopy`, `IndexedIndirectArgs` | Copy regions, and the twenty-byte record an indirect draw reads |
| `CookedShader`, `ShaderStage`, `ShaderTarget`, `HeapUse`, `HeapForm`, `HeapArray`, `HeapArrayKind`, `ConstantSlot`, `COOKED_SHADER_VERSION`, `HEAP_SET`, `PUSH_BYTES_MAX` | The cooked shader format, which names the heap's shader form |
| `GraphicsPipelineDesc`, `ComputePipelineDesc`, `StageDesc`, `Constant`, `ColourTargetDesc`, `WriteMask`, `DepthDesc`, `Compare`, `Cull`, the two pipeline handles | Pipelines from cooked shaders, with specialisation |
| `seal_cache`, `open_cache`, `CacheIdentity`, `CacheStart`, `PipelineStats` | The driver cache's bytes under the facade's own length and checksum, and what a device counts |
| `TimelineHandle`, `TimelinePoint`, `Submit` | Timelines, and a submit with its waits and signals |
| `SurfaceSource`, `SwapchainDesc`, `Swapchain`, `Acquired`, `Presented` | A surface from `raw-window-handle`'s types, and the swapchain the present thread holds |
| `MessageSink`, `Message`, `MessageSource`, `Severity` | Where validation and driver messages go |
| `Rule`, `Refusal` | The rules a backend checks, by name, and the value a refused call returns |
| `HEAP_SLOTS_DEFAULT`, `SAMPLERS_DEFAULT`, `OFFSCREEN_TARGET_IMAGES_MIN` | The heap's and the sampler table's capacities milestone 2's shaders are cooked for, and the least images of an offscreen target |
| `conformance` (feature `conformance`) | The suite's shape: `CASES`, `Case`, `CaseKind`, `Harness`, `run`, `Outcome` |

Hidden plumbing: none. `kind` holds the markers the handle aliases are told apart by; `Any`
is a supertrait of `Encoder` so a backend recovers its own type in a submit.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `lactose-rhi-null` (`rhi-null`) | Every call honourable by kept state, with no `unsafe`; each misuse the plan's row names as a rule with a name a test asserts; a capability set to be created with; the suite's contract cases to fill in | Handles as plain data; recording that cannot fail and a submit that checks in submission order; `write_buffer` and `read_buffer` lend a range for a call, so a store behind a lock serves; `Rule`, `Rule::refuse`, `Rule::of`; `Capabilities` as plain data; `conformance::CASES` |
| `lactose-rhi-vulkan` (`vulkan`) | The spike's fourteen points; states and queues enough to emit every barrier and transfer without tracking state itself; the cache blob's framing; output cases to fill in | `ImageTransition` and `BufferTransition` carry both states and both queues; `Sharing`; `seal_cache`, `open_cache`; `conformance::Harness::shader` |
| `lactose-rhi-metal` (milestone 3) | Residency and its usage per encoder with no call of its own; the alias relation to fence; nothing that names a layout, a family or a hazard-tracking mode | `PassReach` on every pass, each buffer with its exact state; `ImageMemory::Placed` and `Encoder::alias`; states that Metal reads as usage and otherwise ignores |
| `swapchain` (in `lactose-rhi-vulkan` and `lactose-platform`'s tests) | Acquire with no caller timeout, present as part of a submit, the present role's queue held by one thread, resize, which queue path the device took | `Swapchain`, `Acquired`, `Presented`, `Capabilities::queues` |
| `lactose-platform` (`platform`, `platform-impl`) | To hand over a window without a backend naming it; to know which thread reads the handles | `SurfaceSource`, implemented for any `HasWindowHandle + HasDisplayHandle + Send + Sync`; the handles are read once, in the constructor, on the calling thread |
| `lactose-render-graph` (`graph`, `graph-impl`) | Passes by usage mapped to states; transients that alias; queue assignment and crossings; record on frame workers, one pool for each job and frame in flight, reset after the frame's timeline value; submission in graph order | `ImageState`, `BufferState`; `ImageMemory::Placed`, `Encoder::alias`, `Device::image_placement`; `QueueLayout::same_queue`, transfers recorded on both sides; `Encoder` is its own pool, `Send`, and `reset` is refused while pending; `Submit` takes encoders in slice order with waits and signals |
| `lactose-renderer` (`renderer`, `renderer-impl`) | The staging ring written in place and never read; the scatter writing through an address; the latched camera; an indexed indirect draw; heap slots for textures | `Device::write_buffer` on `MemoryClass::Upload`; `push_address`, `BufferReach`; `draw_indexed_indirect`, `IndexedIndirectArgs`; `write_heap_slot`, `push_slot` |
| `lactose-app` (`frame-design`) and `lactose-app-client` | To drive the render side through `dyn` objects and name no backend; to learn of a device loss as a value | `Arc<dyn Device>`, `Box<dyn Encoder>`, `Box<dyn Swapchain>`; `ErrorKind::DeviceLost`, `Device::is_lost` |
| `tools/cook` (`slang`) | One format to write for every backend, which names the heap's form and its capacities without a device to ask; what to check in what Slang emitted | `CookedShader::encode`, `HeapUse`, `HEAP_SET`, `HeapForm::FixedArray`, `HEAP_SLOTS_DEFAULT`, `SAMPLERS_DEFAULT`; "Pipelines and the cooked shader format" |
| `cargo xtask gpu` and every device test | Every message counted; the suite by case name | `MessageSink`; `conformance::CASES` and `run` |

## Behaviour the signatures cannot express

**Capabilities** (RHI point 1, Metal point 1)

- A capability is required or reported. A device that lacks any of the required set is not
  created: the backend's constructor returns `ErrorKind::Unsupported` and its message names
  what is missing. Nothing is enabled by a caller: a backend turns on, at creation, every
  capability of either set its driver offers.
- The required set, by what the facade needs and how each driver names it:

  | Required | Vulkan | Metal |
  |---|---|---|
  | A device address for a buffer | `bufferDeviceAddress` | Metal 3 `gpuAddress` |
  | The heap: a partially filled array of sampled images, written while bound and while submits are pending, indexed by a value that varies | descriptor indexing: partially bound, sampled-image update-after-bind, update-unused-while-pending, sampled-image non-uniform indexing | argument buffers tier 2, `gpuResourceID` |
  | Timelines | `timelineSemaphore` | `MTLSharedEvent` |
  | States the backend turns into barriers | `synchronization2` | nothing to require |
  | A render pass begun with its attachments, and the local read | `dynamicRendering`, `dynamicRenderingLocalRead` | an Apple-family GPU (framebuffer fetch) |
  | A write mask for each attachment | `independentBlend` | nothing to require |
  | 64-bit integers in a shader: an address held as a `uint64_t` | `shaderInt64` | an Apple-family GPU |
  | The local read as Slang lowers it, an image read with no format | `shaderStorageImageReadWithoutFormat` | an Apple-family GPU |
  | An indirect draw's `first_instance` | `drawIndirectFirstInstance` | an Apple-family GPU |

  The RHI spike's list also had runtime descriptor arrays. The heap's form is a fixed array
  (below), which needs none, and the cook refuses a module that declares one (decision 5.8),
  so the facade does not require it. A backend's own platform needs (Vulkan's portability
  subset, Metal 3 and MSL 3.1) stay inside the backend.
- The reported set is `Capabilities`. It is the same value for the device's life. A call
  that needs a field reported `false` is refused (`capability`) by every backend.
  `multi_draw_indirect` gates a count above one in `draw_indexed_indirect`; `indirect_count`
  gates `draw_indexed_indirect_count`; `buffer_atomics_64` gates a pipeline whose module
  declares 64-bit atomics, which no shader of milestone 2 does ("Pipelines and the cooked
  shader format"). Native Metal reports the first two `false` (Metal point 1) and a caller
  loops instead.
- `compute_groups` is the driver's limit on a dispatch's workgroup count in each dimension,
  at least 65535, Vulkan's least, which `Capabilities::LEAST` holds: so null, under `LEAST`,
  refuses on the M1 a dispatch that only a device at the least would break (MoltenVK reports
  2^30). A `dispatch` above it is refused (`bounds`, every backend). Metal publishes no
  limit; `rhi-metal` reports what MoltenVK reports for the same GPU, which hands the counts
  to Metal unchanged, and a probe of the facade's GPU review ran 2^30 groups in each of x, y
  and z on the M1, under Metal API validation, each to its last group.
- `viewport_extent` and `viewport_bounds` are the driver's limits on a viewport: its largest
  width and height, and the range its edges lie in, Vulkan's `maxViewportDimensions` and
  `viewportBoundsRange` rounded inward. Vulkan's least, 4096 and -8192 to 8191, is what
  `LEAST` holds; MoltenVK reports 16384 and -32768 to 32767. A viewport beyond either is
  refused (`bounds`, every backend). Metal publishes neither, and `rhi-metal` reports
  MoltenVK's `viewport_extent` for the same GPU. On an Apple GPU, `rhi-metal` and `vulkan`
  report `viewport_bounds` of at most -16384 to 16383, the range the facade's GPU review's
  probe saw the M1 honour. MoltenVK reports -32768 to 32767, which the M1 does not honour
  past a far edge near 24,576: it draws nothing, with no error.
- `heap_slots` and `samplers` are the request's: a device whose limit is below either is not
  created. `HEAP_SLOTS_DEFAULT` and `SAMPLERS_DEFAULT` are what milestone 2 requests,
  `Capabilities::LEAST` holds, and the cook compiles its arrays to.

**Queues by role** (RHI point 2, Metal point 2, decision 5.6)

- A queue is named by its role: `Graphics`, `Compute`, `Present`. A device has a graphics
  queue. `Capabilities::queues` says whether the compute role and the present role have a
  queue of their own (`Own`) or are served by the graphics queue (`Shared`), and
  `QueueLayout::serving` resolves a role to the role whose queue serves it. A caller writes
  the same calls on every layout; the layout decides only whether a change of state between
  two roles is an ownership transfer.
- Submits to one queue run in the order the calls return. A backend serialises the calls
  itself with a lock for each queue, the queue that serves the role (architecture section 3
  permits the lock): a submit takes it before its walk of the records and keeps it through
  the driver's submit, so the points a queue's submits are stamped with grow in the order
  its driver receives them, and a completed value at or above a stamp means the stamped
  submit has run. The heap's lock is taken before a slot's first hold of the record's lock
  and kept through its descriptor write and its second hold, so two writes of the heap never
  interleave ("Threads"). A driver's submit that fails after the walk, the device not lost,
  has changed nothing in the driver and leaves the point to the backend. Before it lets go
  of the queue's lock, the backend submits a batch with no work that signals the queue's own
  timeline to the point, behind the queue's earlier submits, and, in the hold every submit
  takes after its driver call, drops the failed submit's promises and its points; should the
  batch fail too, the queue's next submit passes the point. On Vulkan every signal a backend
  submits, the queue's own, the caller's and the batch's, is at
  `VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT`, so that a point reached means every earlier submit
  to its queue has run. The point is never signalled from the host, and the backend never
  waits for the queue there: an earlier submit to the queue may still be pending, perhaps on
  a point the caller will signal from the host. Which of two threads' submits comes first is
  the callers' to decide. Nothing orders work on two queues but a timeline point one submit
  signals and the other waits for.
- On one queue a submit may be held by every submit made before it: one that waits for a
  point holds those behind it until the point is reached. Null holds them so, running a
  queue's submits strictly in order; a driver may let a later one pass. So a submit never
  waits for a point that only a later submit to its own queue signals: on null, and on any
  driver that holds the queue, neither completes, and no backend detects it.
- A submit is pending until it completes. For every rule that names a pending submit
  (`pending`, `uuwp-used`, `no-qfot`, a host access under a writer), null counts a submit as
  pending until the caller has seen a point that it, a later submit to its queue, or a
  submit that waits for such a point, signals: a `wait_timeline` that returned `true`, a
  `timeline_value` at or above it, or, for a change on another queue, a submit of that queue
  that waits for it. So a caller that frees or reuses what a submit uses before it has seen
  that submit's point is refused on null even where the submit has run, as it would race on
  a device. Null runs a submit's effects when its waits are reached, and a drop does not
  block on what it has run.
- A device backend's pending is completion. The completed value of each queue is read from
  the driver before the lock and folded into the value the device knows the queue's own
  timeline has reached ("Timelines"): an internal timeline semaphore every Vulkan submit
  signals, read with `vkGetSemaphoreCounterValue`; an internal shared event `rhi-metal`
  encodes before the caller's signals, read with `signaledValue`. Never a value a completion
  handler or a status updates later: on the M1 both trail the point the host has seen. Where
  that value is below a stamp, the device also compares the caller's points that the stamped
  submit and the later submits of its queue signal with the values it knows those timelines
  have reached ("Timelines"), and counts the submit complete if any is reached; it reads no
  driver value for them. On Metal this rests on a later command buffer's event signal
  landing after the earlier command buffers of its queue complete, which Apple does not
  document; verification re-checks it. A point the caller has seen through `wait_timeline`
  or `timeline_value` is known before that call returns. A failed submit's points are
  dropped with its promises: the caller may then signal them from the host, which says
  nothing of the queue's earlier submits. No backend refuses a call as pending once the
  caller has seen a point the submit signals. A device's refusal of a call made before then
  depends on timing; the call is wrong either way.

**The present thread** (decision 5.6, D153; Metal item 11)

- Every command that reads or writes a swapchain image is recorded on an encoder of the
  `Present` role and submitted through `Swapchain::present`, by the one thread that holds
  the `Swapchain`. `Device::submit` refuses the `Present` role on every backend, and null
  refuses a swapchain image on an encoder of another role (`present-queue`).
- The present role has a queue of its own where the device offers a queue, other than the
  graphics queue, in a family that can draw and present to the surface. No other thread's
  submit then reaches it or waits behind it. Otherwise the roles share the graphics queue:
  `present` takes that queue's lock like any submit, and a frame submit made while a present
  blocks waits behind it. `Capabilities::queues.present` reports which.
- The frame renders into an offscreen target, and the present thread draws it into the
  swapchain image with a pipeline made for `Swapchain::format`, sampling the target through
  a heap slot its pass declares. A copy is not offered for it: it would swap channels into a
  BGRA surface and skip the encoding into an sRGB one.
- The offscreen target is `OFFSCREEN_TARGET_IMAGES_MIN` images or more. The present thread
  holds one at a time, the newest the frame has finished. A finished image crosses to the
  present role by a transition the frame's last submit for it records and the present
  thread's encoder records again, the present's submit waiting for a point the frame's
  submit signals; or, for an image made `Sharing::Concurrent`, by a transition the present
  thread records alone.
- The frame takes an image back only after the point the present thread signals when its
  read of it is done, which the frame waits for on the host or its submit waits for, and by
  a transition from `Undefined`: it overwrites the image whole, so nothing crosses back. A
  frame submit that takes the image while a present that reads it is pending, and that
  does not wait for that point, is refused on null (`no-qfot`; "States"), which is how the
  frame is kept from writing the image the present thread holds. Where the roles share
  one queue there is no such refusal and none is needed: the queue runs the read before the
  write. A backend sees a hold only while a present that reads the image is pending; which
  image the present thread holds between its presents is the frame's protocol to keep
  (`frame-design`).

**The surface** (plan section 3, `rhi-api`)

- A device presents to the window its `DeviceRequest` names, or to none. The window is any
  type that lends `raw-window-handle` 0.6's window and display handles and is `Send + Sync`;
  no backend names `lactose-platform`.
- A backend reads the two handles once each, inside its constructor, on the thread that
  calls the constructor, and never again. A platform whose handles are only available on one
  thread (AppKit's main thread) calls the constructor there. The backend holds the source's
  `Arc` in what destroys the surface and drops it in that destroy, which is the drop of the
  last of the `Device`, its encoders and its swapchain; it keeps no clone elsewhere, so the
  window is let go when the device is.
- `Device::create_swapchain` on a device made with no window returns `Unsupported`.

**Threads**

- `Device` is `Send + Sync` and every method may be called from any thread at once.
- `Encoder` is `Send` and not `Sync`. Two encoders record on two threads with no lock
  between them: each has its own pool, and a recording call takes no lock an update of the
  device's records takes. It resolves a handle through a lock on that handle's entry alone,
  held for a compare of the generation and a count, once for each handle an encoder records
  between resets; after that the encoder reads its own copy. The entries are in a table
  whose entries do not move while the device lives (a fixed capacity, or chunks that are
  never moved), so a create that grows the table never moves an entry a recording thread
  holds. The checks made at recording (`usage`, `bounds`, `encoding`) read only what the
  handle resolved to, which does not change while the resource lives. `Swapchain` is `Send`
  and not `Sync`.
- A `MessageSink` is called from any thread, a driver's own included.
- A device backend keeps records of its own to check the rules every backend checks ("The
  rules"): what each handle resolves to, the host accesses in progress on each buffer, and
  the stamps and counts of "What a device backend keeps". Several threads reach them at once
  on the per-frame path, the extract sink's jobs through `write_buffer` among them. A
  backend guards them with one lock, and a call holds it once for its checks and the updates
  they permit together, so that no other call falls between a check and the change it
  permits: a submit for the check of every handle, buffer, slot and timeline its encoders
  and points name and then every stamp and promise, so a refused submit stamps nothing, and
  once more, under the queue's lock, after its driver call, to record that its queue's
  submits through its stamp have reached the driver and, where the call failed, to drop its
  promises and its points; a present for the same walk, over the points its submit depends
  on as well, made again after each wait for a point the walk could not decide ("The
  swapchain"); a destroy for its checks and the handle's removal; `write_buffer` and
  `read_buffer` for their checks and the insert of their range, and again for its removal
  once the closure has returned; `write_heap_slot` and `clear_heap_slot` for the check, a
  mark on the slot and the count of the new image, and again, after the descriptor write,
  for the slot's record, the release of the old image and the mark's removal; a submit whose
  walk finds a slot it declares marked is refused (`uuwp-used`); `signal_timeline` for its
  check and the mark, with its value, of its signal in progress, and again, after its driver
  call, for the mark's removal and, where the call succeeded, the fold of its value into the
  timeline's known value ("Timelines"); `wait_timeline` that returns `true` and
  `timeline_value` once each, after their driver call, to fold in the value they saw. Where
  a call must make a driver call or run the caller's closure between its check and its
  effect (a host lend, a host signal, a heap write), its first hold sets a mark that every
  conflicting check refuses, and a second hold clears it; the second hold decides nothing.
  The lock is never held across a wait, a driver call or a caller's closure, so `fill`,
  `read` and every validator's message run outside it. A value a check reads from the
  driver, a queue's completed value or a timeline's, is read before the lock is taken, and
  the hold folds it into the greatest value the records know that timeline has reached,
  which the check then takes ("Timelines"). No driver value is read under the lock: with the
  Khronos validation layer a read waits for the layer's own thread to retire the work it
  shows. The known value only grows and is never above the driver's, so a check that refuses
  below it, as every check of `pending` does, errs toward refusal; a check that a signal
  must exceed it is sound because every value the timeline is ever signalled to is known, a
  promise in the records or a marked host signal's, not because the read is recent.
- A device backend takes its locks in one order: a queue's lock or the heap's lock, then the
  record's lock, then the lock of one entry, a handle's or a heap slot's. No call takes two
  queue locks, and the heap's lock and a queue's are never held together. Each device
  backend's `SAFETY.md` lists every lock it takes, the calls that take it and this order.
  Architecture section 3 names these locks among those it permits (D158).

**Handles and lifetimes**

- A handle is an index and a generation. It means something to the device that issued it
  alone. A handle whose resource is destroyed is refused (`destroyed`) by every backend: by
  the `Device` call it is given to, and, for one recorded into an encoder, by that
  encoder's submit, which checks every handle the encoder recorded again, so a resource
  destroyed between a recording and its submit is refused there too.
- A handle another device issued is refused (`destroyed`) by null, whose devices draw
  their handles' generations from one count for the process, so no two of them issue an
  equal handle. A device backend refuses it only where it resolves to nothing of its own;
  where it resolves to a live resource of the device, the call is that resource's, under
  every check of the call, which is a wrong image and never a freed object.
- No backend defers a destroy. A destroy of something a pending submit uses is refused
  (`pending`, every backend), and the caller waits for the timeline point that frees it. The
  submit checks every handle and then stamps each, in one hold of the record's lock, before
  the driver's submit. A refused submit stamps nothing. A resource keeps the stamp of the
  last submit of each queue that names it, three at most, so a resource two queues use at
  once (`Sharing::Concurrent`, or one used off its queue, which null alone refuses) is freed
  only after both.
- A destroy makes the handle stale at once, on every thread: a lookup after it, and the
  submit of an encoder that recorded it, refuse it (`destroyed`). What the handle resolved
  to, and the memory it is placed in, stay alive while a call that resolved it before the
  destroy is in progress, and while an encoder that recorded it is not reset or dropped: a
  device backend keeps a count with the lookup and releases the driver's object with the
  last. No count is the caller's: a destroy is never refused or delayed for one. The count
  is taken by every `Device` call that resolves a handle and then calls the driver
  (`buffer_address`, the host lends, `flush_buffer`, `invalidate_buffer`, `write_heap_slot`,
  `signal_timeline`, `wait_timeline` and `timeline_value`, and `Swapchain::present` for the
  timeline its wait for an undecided point names) as by recording. A hold such a call makes
  after its driver call changes the records of what the handle resolved to only while the
  handle still resolves to it; after a destroy it folds nothing and clears nothing. A
  present whose wait's timeline was destroyed meanwhile is refused (`destroyed`) by its next
  walk, with nothing submitted.
- An image is destroyed only after no heap slot holds it, and a block only after no image
  is placed in it (`pending`, every backend).
- The device lives until the `Device` object, every encoder and the swapchain it made are
  dropped. Dropping a pending encoder, the swapchain with a present pending, or the last of
  them all blocks until the submits concerned complete. A caller that wants a prompt
  shutdown waits for its own timeline points first, and a drop under a submit whose wait is
  never signalled does not return. So encoders, the swapchain and the device are dropped off
  the frame workers, whose one sanctioned blocking wait is the physics step (architecture
  section 5.1). A drop made while the thread unwinds a panic does not block: the backend
  leaks what is pending, so a test that fails holding a pending encoder fails instead of
  hanging. A drop that finds no submit and no present pending waits for no submit, no
  present and no thread of the caller's (under the Khronos validation layer its reads may
  wait briefly for the layer's own thread, which runs nothing of the caller's but the sink),
  so it may be made on a thread that must not wait, the main thread included; a caller that
  drops there first waits for its points and the swapchain's presents, and signals every
  point a pending submit waits on that the host owes.

**Buffers and host access** (RHI point 3, Metal point 3)

- A buffer's contents are unspecified until written. Its device address is one `u64`, the
  same for the buffer's life, and no byte of another live buffer has an address inside it.
- `MemoryClass::Device` is never touched by the host and is filled by a copy.
  `Upload` is written by the host through `write_buffer` and not read; `Readback` is read by
  the host through `read_buffer` and not written. A call outside the class is refused
  (`usage`) by every backend.
- `write_buffer` and `read_buffer` lend exactly the range asked for, for the length of the
  call, and call their closure once. Calls on ranges of one buffer that do not overlap may
  run at once; a call that overlaps one in progress, a destroy of the buffer while any is in
  progress, and a submit that writes the buffer while any is in progress are refused
  (`host-overlap`), and a range outside the buffer is refused (`bounds`), all by every
  backend, since a backend's mapped memory is reached through them.
- `write_buffer` of a buffer a pending submit writes, by a copy into it or a pass that
  declares it `ShaderWrite`, and `read_buffer` and `invalidate_buffer` of one a pending
  submit writes or changes the state of, are refused (`pending`, every backend): the bytes a
  call lends are never ones the device may be writing, and an invalidate made before the
  write or the barrier lands is not the invalidate after it, so is never taken for one. A
  transition into `HostRead` is the device's last access before the host's, and on Vulkan
  the barrier that makes the writes visible to the host; a barrier touches no bytes, so a
  submit that only transitions a buffer holds no host write back. An `Upload` buffer may
  hold `COPY_DESTINATION` and `DEVICE_ADDRESS`, so the write is checked as the read is. A
  device keeps, for each buffer, the point of the last submit of each queue that writes it,
  and of the last that writes it or changes its state. The check is of the whole buffer, as
  a pass that writes through an address names no range.
  Whichever of a host call and a submit that writes its buffer takes the record's lock
  first, the other is refused: the submit by the range in progress, the host call by the
  stamp ("Threads").
- `flush_buffer` after a write and `invalidate_buffer` before a read are always called. They
  do nothing where `coherent_host_memory` is reported (the M1's shared memory: Metal point
  3). On a device that reports it `false`, null refuses a submit that declares, binds or
  copies from a buffer with a written range not flushed since, and a `read_buffer` of a
  range a submit wrote with no invalidate since (`unflushed`).
- A host write is seen by every submit made after its flush returns. The host does not
  write a range a pending submit reads; null cannot see every such read and does not check
  it. A breach gives that submit a torn read, a wrong image, which is the residual "The
  rules" names, and no host memory is reached that a reference covers.

**Images and memoryless requests** (RHI point 4, Metal point 4)

- An image has two dimensions, `mips` mips and `layers` layers, and is made in
  `Undefined`. A format or an extent the device cannot make is `Unsupported`.
- `ImageUsage::TRANSIENT` says nothing needs the contents after the pass that writes them.
  With it the usage holds only `COLOUR`, `DEPTH` and `INPUT`, and `mips` and `layers` are
  one.
- `ImageMemory::Memoryless` is a request, and needs `TRANSIENT`. The device honours it
  exactly when it reports `memoryless_colour` (for a colour attachment whose usage holds
  `INPUT`, which is the G-buffer's) or `memoryless_depth` (for a depth attachment);
  otherwise the image has memory of its own and every call on it still works.
  `Device::image_memoryless` says which happened. A backend probes `memoryless_colour` with
  an image whose usage holds `INPUT`: MoltenVK keeps its lazily allocated types from such
  an image and not from a colour image without it (RHI item 7).
- An image made with a memoryless request is never stored and never loaded: its
  attachment's store action is `DontCare` and its load is not `Load::Keep`, whether the
  request was honoured or not (`memoryless-store`, every backend). Metal API validation
  asserts on either, and Vulkan allows both. A device keeps one flag for each image and
  checks it for each attachment as the pass begins.
- A memoryless image is not used in a pass after that pass's local read where the device
  reports `memoryless_survives_local_read` as `false` (`split`, null). The render graph does
  not ask for memoryless in such a pass.

**Aliased transients** (Metal point 16, item 4)

- A memory block is device memory of a size. An image made `ImageMemory::Placed` takes
  `Device::image_placement`'s `size` bytes of it from its offset, which is a multiple of
  `alignment`; a placement that does not fit is refused (`bounds`, every backend). Two
  placed images alias when their byte ranges in one block overlap. That relation is all a
  caller states: the backend fences it.
- Every block takes any image `image_placement` accepts; a backend returns `Unsupported`
  from `image_placement` for an image no block can hold, such as one its driver gives
  memory of its own alone. On Vulkan an image's memory types depend on its tiling, on its
  format for a depth image, on the transient bit of its usage and on its create flags, so
  `vulkan` makes its blocks from the memory types a colour image's and a `Depth32Float`
  image's requirements have in common, and returns `Unsupported` for an image whose types
  its blocks do not offer.
- On Metal a placed image is untracked at every use, not at the alias call alone, so
  `rhi-metal` fences every encoder that uses one ("States", the last item): a transient
  written in one pass and read in the next is ordered as an alias is. Metal item 4's
  program fences every use so; `useResource` on the reading encoder gives residency alone.
- `Encoder::alias(image)` is the alias call: `image` now begins. It is recorded after the
  last use of every image that aliases it and before its own first use, in submission order.
  After it every subresource of `image` is `Undefined` on the encoder's queue and holds
  nothing readable, and each image that aliases it is unusable until an alias call of its
  own.
- Null refuses any use of a placed image whose bytes another image has used with no alias
  call since (`no-fence`), and a read of the new image before its first write
  (`undefined-read`). The first image ever used at a placement needs no alias call, and one
  is accepted.
- Images that alias are used on one queue, as the layout resolves it: an alias call on
  another queue's encoder is refused (`no-qfot`, null). Metal's fence orders the encoders of
  one queue only (Metal point 11).

**States** (RHI point 9, Metal point 9)

- Each subresource of an image, and each buffer, is in one state on one queue at every
  point of the submission order. Images start `Undefined` and buffers `Undefined`, on no
  queue; the first transition takes them for its `to` queue.
- A use needs a state, and null refuses a use in another (`state`):

  | Use | State |
  |---|---|
  | A colour attachment the pass does not read | `ImageState::ColourAttachment` |
  | A colour attachment with `local_read` | `ImageState::LocalRead` |
  | The depth attachment | `ImageState::DepthAttachment` |
  | The image of a heap slot a pass declares | `ImageState::SampledRead`, every subresource (`heap-layout`) |
  | A copy's source and destination | `CopySource`, `CopyDestination` |
  | The image a present shows | `ImageState::Present` |
  | A buffer a pass declares | the `BufferReach`'s state: `ShaderRead` or `ShaderWrite` |
  | The bound index buffer; an indirect draw's or dispatch's argument and count buffers | `BufferState::Index`; `BufferState::Indirect` |
  | `read_buffer` | `BufferState::HostRead`, with no pending submit that writes the buffer or changes its state (`pending`, every backend) |

- A transition's `from` is the state and queue each subresource of its range is in, or the
  transition is refused (`state`). A transition to the state and queue a subresource is
  already in changes no state. Where that state writes (`ColourAttachment`, `LocalRead`,
  `DepthAttachment`, `CopyDestination`, `ShaderWrite`), it orders the uses before it with
  those after it: Vulkan records a barrier with that state's stages and access on both
  sides. A resource a pass or a copy wrote in such a state is used by a later pass or copy
  only after such a transition, or one to another state (`state`, null): a depth pre-pass
  and the pass that keeps its depth, two passes drawing into one attachment, two copies
  into one buffer, and two compute passes that write one buffer each have one between
  them. Null keeps, for each resource, whether a pass or a copy has written it since its
  last transition, beside the writes it keeps for `undefined-read`.
- A transition of an image into a state its usage does not allow is refused (`usage`, every
  backend): `ColourAttachment` needs `COLOUR`, `LocalRead` `COLOUR` and `INPUT`,
  `DepthAttachment` `DEPTH`, `SampledRead` `SAMPLED`, and `CopySource` and
  `CopyDestination` their own bits; `Undefined` and `Present` need none. Vulkan's layout
  transitions require the usage of the layout they enter.
- A copy region lies inside both its resources, and a buffer-image copy's `buffer_offset`
  is a multiple of its format's texel (`bounds`, every backend): Vulkan's
  VUID-vkCmdCopyBufferToImage-dstImage-07975 and -07978, and their image-to-buffer
  counterparts.
- Inside a compute pass, dispatches run in recording order, each seeing the buffer writes
  of those before it: Metal's serial dispatch does it, and Vulkan records a shader-write
  barrier between two dispatches of a pass that declares a `ShaderWrite` buffer. Draws of
  one render pass are not ordered through buffers.
- A transition whose `from.state` is `Undefined` matches any state and takes the resource
  for `to.queue` with no transfer; `from.queue` is ignored. It discards an image's
  contents. A buffer's bytes it keeps, unless it moves the buffer to another queue, which
  discards them. Vulkan gives such a transition a source scope that covers every earlier
  use on the queue.
- Discarded contents, an image's after `Store::DontCare`, and a buffer nothing has written
  are undefined, and a read of them before the next write is refused (`undefined-read`,
  null): `Load::Keep`, a declared slot's image, a copy's source, an input read, a buffer
  declared `ShaderRead`, an index or indirect buffer, `read_buffer`. A write is an
  attachment a pass stores, a copy's destination, a host write, and a buffer a pass
  declares `ShaderWrite`. Null keeps this for each subresource of an image and for a
  buffer as a whole, and takes a write of any part as a write of all of it.
- A transition between two roles that one queue serves is an ordinary change of state. One
  between two queues is an ownership transfer, recorded twice with equal values: on an
  encoder of `from.queue`'s role, which releases, and on one of `to.queue`'s, which
  acquires. Between the two the resource is used by nothing, but for a transition from
  `Undefined`, which ends a release no acquire met and takes the resource with no transfer,
  its contents undefined, a buffer's as an image's; an acquire of that release is then
  refused. Null refuses an acquire with no equal release already submitted, and any use on
  a queue that does not own the resource (`no-qfot`).
- A resource made `Sharing::Concurrent` has no owner: its transitions are recorded once, the
  queue of a transition's `from` is not compared, and two queues may use it at once in one
  state.
- Two queues are ordered by timeline points alone. A submit that changes a resource's state
  or owner while a pending submit of another queue uses the resource waits for a point that
  submit, or a later submit to its queue, signals; null refuses it otherwise (`no-qfot`).
  That covers an acquire, a transition from `Undefined` and every transition of a
  `Concurrent` resource. A submit uses what its commands name: a transition's resource, an
  attachment, a declared buffer, a declared slot's image, a copy's two sides, an index or
  indirect buffer. A submit that uses a resource while a pending submit of another queue
  wrote it or changed its state or owner waits, directly or through the submits it waits
  for, for a point that submit, or a later submit to its queue, signals; null refuses it
  otherwise (`no-qfot`). It binds `Concurrent` resources, which no acquire orders, and an
  `Exclusive` one moved to another queue by a transition from `Undefined`, which has no
  acquire: an image taken back from the present thread, and a resource imported again after
  a failed submit. It binds every submit that uses the resource, not the first of its queue
  alone: an acquire or a transition in an earlier submit of the queue does not discharge a
  later submit's use. A wait for a point already reached costs no GPU wait, so a caller that
  waits in each such submit is never refused by it.
- A transition is recorded on an encoder of the role that uses the resource next; a release
  on one of the role that used it last.
- A buffer has one state at a time. A pass cannot both bind a buffer as indices or indirect
  arguments and reach it through an address; no consumer of milestone 2 does.
- Vulkan derives stages, access, layouts and queue families from the two states. Metal has
  no layouts and no ownership. It ignores every transition, but for images placed in a
  block: those are untracked, and `rhi-metal` waits on the block's fence before, and
  updates it after, every encoder that uses one (as an attachment, a declared slot's image
  or a copy's side), so their uses are ordered whether or not an alias call falls between
  them. It reads only a declared buffer's state, as the usage it is made resident with.

**The heap and the sampler table** (RHI point 5, Metal points 5 and 15, decision 5.8)

- A device has one heap of `heap_slots` slots of two-dimensional sampled images, empty at
  creation, and one sampler table of `samplers` entries. The caller allocates slot numbers;
  the device allocates sampler indices. Both are the numbers a shader indexes with, the same
  on every backend: no backend's own handle (an `MTLResourceID`, a descriptor handle) is
  ever a value in this interface.
- `write_heap_slot` puts a whole image, whose usage holds `SAMPLED`, in a slot;
  `clear_heap_slot` empties it. Either is seen by every pass that begins after it returns;
  an encoder with a pass that declared the slot before it is refused at its submit
  (`heap-layout`, every backend). Either is refused while a pass of a pending submit
  declares the slot (`uuwp-used`, every backend): a slot is reused only after the timeline
  point that frees it. A slot outside the heap is refused (`bounds`, every backend).
- A sampler's index is valid from `create_sampler` to `destroy_sampler`. A full table is
  `ErrorKind::ResourceExhausted`. The table needs no declaration and no state, so any
  pending submit may use any sampler: `destroy_sampler` is refused while a submit made
  before it is pending (`pending`, every backend). Samplers are made at the start and
  destroyed at the end, where the caller has waited for its submits anyway.
- An empty slot, from the device's creation or after `clear_heap_slot`, holds a device
  backend's placeholder image, and a free entry of the sampler table its placeholder
  sampler. A shader that reaches either through data alone then reads an unspecified value,
  a wrong image, and never an unwritten descriptor. No call names the placeholders.
  `rhi-metal` declares its placeholder image with `useResource` on every encoder of a pass
  that reaches the heap, since Metal makes no image resident that an encoder does not
  declare; a sampler needs no residency.
- In a shader the heap is `HeapForm::FixedArray`: fixed-size arrays in one parameter block,
  the images at binding 0 and the samplers at binding 1, in descriptor set `HEAP_SET` on
  SPIR-V and one argument buffer on Metal (Metal, "The heap's shader form"; the second
  array is the review's probe under its "Not shown", and no spike ran it).

**What a pass declares** (RHI point 5, Metal point 5 and item 2)

- Every render pass and compute pass carries a `PassReach`: each heap slot a shader of the
  pass may index, and each buffer a shader of the pass may reach through a device address,
  with the state it is in for the whole pass. It is required on every backend, whether the
  backend uses it or not. The index buffer and the indirect buffers a draw names are not
  declared: the call that names them is their declaration.
- A declared buffer's state is exact: `ShaderWrite` if any shader of the pass writes it,
  and `ShaderRead` if none does. A buffer listed twice is listed in one state, or the pass
  is refused (`state`, every backend): which of two would hold is no backend's to choose,
  and Vulkan's barrier and Metal's `useResource` usage each take one.
- A slot given to `push_slot` or a buffer given to `push_address` that the pass did not
  declare is refused (`undeclared`, every backend): a device backend looks it up in the
  pass's declaration, which its encoder holds. So a slot number a backend writes names a
  declared slot, and a declared slot that is empty is refused at each draw or dispatch
  (`heap-layout`, every backend); an address a backend writes names a buffer made resident
  for the pass on Metal, where a shader's access to one that is not resident faults.
- A pass resolves each slot it declares to the slot's image when it begins, through the
  slot's entry lock, and the encoder counts that image as it counts a recorded handle. The
  submit's walk compares each declared slot's image with the one the pass resolved and
  refuses a difference (`heap-layout`, every backend): on Metal the pass made the old image
  resident, and the argument buffer would name one no encoder made resident. `rhi-metal`
  resolves every declared slot to its texture for `useResource` anyway, and `vulkan` looks
  each one up once a pass.
- Null also refuses, at each draw or dispatch, a declared slot whose image is not in
  `SampledRead` (`heap-layout`), treating every declared slot as read, and a declared buffer
  in another state (`state`).
- What a shader reaches through data alone, an address, a slot number or a sampler index in
  `push_bytes` or in a buffer, no backend sees. It is the second residual "The rules" names:
  the placeholders turn an empty slot or a free sampler entry into a wrong image, and Metal
  shader validation and Vulkan's GPU-assisted validation report the rest in tests.
- The facade has no residency call. On Metal the declaration is `useResource` on the pass's
  encoder, with read or write usage from the state, which is also what brings the resource
  under Metal's hazard tracking. On Vulkan it is not used beyond the states.

**Render passes and the local read** (RHI point 10, Metal point 10, decision 5.8)

- A pass's attachments have one extent, each names one mip of one layer, no image is named
  twice, and there are at most `colour_attachments` colour attachments (`usage`, every
  backend). The scissor lies inside that extent (`bounds`, every backend): Metal API
  validation asserts on one that does not. A viewport's fields are finite, its width and
  height above zero and at most `viewport_extent`, its edges within `viewport_bounds`, and
  its near and far depths in zero to one (`bounds`, every backend): Metal API validation
  asserts on one that is not finite, and Vulkan's rules require the rest. Within those
  limits a viewport may reach past the attachments.
- Clip space is x to the right, y up and depth from zero at the near plane to one. The
  framebuffer's origin is its top left. A triangle's front is the side on which its vertices
  run anticlockwise as displayed. Triangles are lists; indices are 32 bits; a pipeline reads
  its vertices through device addresses, and there is no vertex buffer binding. A backend
  whose driver's conventions differ converts (Vulkan flips its viewport).
- A vertex shader's vertex id is the index plus `base_vertex` (indexed) or the range's
  start plus the vertex's number; its instance id is the instance range's start, or the
  record's `first_instance`, plus the instance's number, on every backend. Slang lowers
  `SV_VertexID` to SPIR-V as the index less the base vertex and to MSL with it, so the
  cook refuses `SV_VertexID` and `SV_InstanceID` and a shader reads `SV_VulkanVertexID`
  and `SV_VulkanInstanceID`, which lower to the same ids on both targets and need no
  `DrawParameters` capability.
- Input `i` of a fragment stage is colour attachment `i` of the pass, and nothing else.
  The attachment is marked `local_read`, its image's usage holds `INPUT` and it is in
  `LocalRead`. A draw whose pipeline reads input `i` in a pass with no such attachment `i`
  is refused (`local-map`, null). The depth attachment is not an input.
- `Encoder::local_read` is a call of its own: "read what this pass has written so far". It
  takes no argument and is not a barrier. A draw whose pipeline reads input `i` when a draw
  has written colour attachment `i` (a pipeline with a write mask that is not empty for it)
  since the last `local_read` of the pass is refused (`local`, null). A pipeline does not
  read input `i` and write colour attachment `i`: `create_graphics_pipeline` returns
  `InvalidData`.
- A bound pipeline's colour and depth formats are the pass's attachments' (`usage`, every
  backend): Metal API validation asserts on a pipeline of other formats than its pass.
- `Load::Clear` and `Load::DontCare` need no defined contents; `Load::Keep` does. After
  `Store::DontCare` the contents are undefined.
- A backend opens no span. The caller that records a pass opens one under the pass's
  `label` (architecture section 3).

**Pipelines and the cooked shader format** (RHI point 6, Metal point 6, decision 5.8)

- A cooked shader is one stage for one target, and its bytes are what
  `CookedShader::encode` writes: nothing else writes them and `decode` reads no other
  version. Equal shaders encode to equal bytes on every host. A backend makes a pipeline
  from the targets it runs (`SpirV` on Vulkan, `Msl` or `MetalLibrary` on Metal, `SpirV`
  on null, which runs none of it) and returns `Unsupported` for another, so the suite's
  cooked SPIR-V serves null's contract cases and `Msl` is a target null refuses.
- The format names the heap's form: `HeapUse` carries `HeapForm::FixedArray`, the set
  (`HEAP_SET`), and each array's kind, binding and capacity. `create_graphics_pipeline` and
  `create_compute_pipeline` return `InvalidData` for a shader whose image array's capacity
  is not the device's `heap_slots` or whose sampler array's is not its `samplers`.
- What the format cannot hold is the cooker's to check on what Slang emitted (decision 5.8):
  that the module declares no runtime descriptor array, that every access through the heap
  is marked non-uniform, that each fragment output has one location on both targets, that
  `inputs` and `outputs` are what the code reads and writes, and that the SPIR-V declares no
  capability outside this list, each of which the required set enables: `Shader`,
  `PhysicalStorageBufferAddresses`, `Int64`, `ShaderNonUniform`,
  `SampledImageArrayNonUniformIndexing`, `InputAttachment`, `StorageImageReadWithoutFormat`.
  Slang declares `Int64` for an address held as a `uint64_t` and
  `StorageImageReadWithoutFormat` for the local read.
- `DrawParameters`, `RuntimeDescriptorArray` and `Int64Atomics` are outside the list.
  `SV_VertexID` and `SV_InstanceID` declare `DrawParameters`, so the cook refuses both and
  requires `SV_VulkanVertexID` and `SV_VulkanInstanceID` ("Render passes and the local
  read"). A shader with 64-bit atomics declares `Int64Atomics`: from milestone 3 the
  renderer selects one by `buffer_atomics_64`, and the cooked format gains a field that
  names the capability.
- A shader's code is the cook's and is trusted. A backend checks a module's form: its magic
  and version are its target's (`InvalidData`); on SPIR-V, its `OpCapability` and
  `OpExtension` instructions are among the list above and `SPV_KHR_physical_storage_buffer`,
  each of which the device enabled (`capability`, every backend that takes SPIR-V). A Metal
  target declares none, and Metal's compiler refuses one the GPU cannot run (`InvalidData`).
  What a malformed module does is the second residual of "The rules".
- A pipeline has no layout object. Its layout is the heap's block, the fragment stage's
  input attachments (set 0, input `i` at binding `i`) and one push-constant range of the
  largest `push_bytes` among its stages, at most `PUSH_BYTES_MAX`.
- A colour target whose write mask is not empty is one the fragment stage writes (its bit
  of `outputs`) and does not read as an input (its bit of `inputs`), or the pipeline is
  `InvalidData`. A pipeline with no fragment stage writes depth alone, and each of its
  write masks is empty. A pipeline has at most `colour_attachments` colour targets
  (`usage`, every backend): VUID-VkPipelineRenderingCreateInfo-colorAttachmentCount-09533.
- A `Constant` names an id the shader declares, with bytes of the declared length, or the
  pipeline is `InvalidData`. A backend specialises every pipeline, with no values when none
  are given (Metal refuses an unspecialised function that has constants).
- The push-constant block belongs to the pass: it starts as zeros and is what each draw or
  dispatch sees whichever pipeline is bound. `push_address` writes eight bytes and
  `push_slot` four; a write outside `PUSH_BYTES_MAX` is refused (`bounds`).

**The pipeline cache** (RHI point 7, Metal point 7 and item 7)

- `Device::pipeline_cache` returns the driver's cache sealed by `seal_cache`: the facade's
  magic number and version, the backend's `CacheIdentity`, the payload's length, the
  payload and a checksum. The caller writes the bytes to disk and hands them to the next
  run's `DeviceRequest`; the facade and the backends open no file of the caller's.
- A backend hands its driver only what `open_cache` returned for its own identity. Bytes
  that fail it, and bytes the driver then refuses, start the cache empty: the constructor
  never fails for them, sends no message, and reports `CacheStart::Rejected`.
- `rhi-metal` writes the opened payload to a private file and loads that, since Metal loads
  an archive by URL; it neither loads nor returns a cache while shader validation is on
  (`pipeline_cache` is then empty).
- `stage_hits` and `stage_misses` are what the driver says of its cache. They are
  statistics. That a warm cache hits is the suite's case, shown by the driver's own signal
  (Vulkan's creation feedback, Metal's fail-on-miss) and never by time.

**Recording and submitting** (RHI point 8, Metal point 8)

- Recording does not fail and returns nothing. What a recorded command breaks is found at
  the submit, in submission order, because states have no other order: encoders are
  recorded at once on several threads.
- An encoder is recording from creation and after each `reset`, and is submitted once
  between resets. A submit that takes it makes it pending until the submit completes;
  `reset` is refused while pending (`pending`, every backend). From that submit to the next
  reset a recording call records nothing and changes nothing the submit runs, and a second
  submit with no reset between is refused (`encoding`, every backend): native Metal cannot
  commit a command buffer twice, and recording into a pending Vulkan command buffer is the
  driver's undefined behaviour. One encoder is one pool: the graph keeps one for each
  recording job and frame in flight.
- A draw whose vertex, index or instance range is empty, a dispatch with a count of zero in
  any dimension, and an indirect draw of no records record nothing, on every backend; a
  range whose start passes its end is empty. Metal API validation asserts on a zero instance
  count, a zero index count and a zero dispatch.
- A submit runs its encoders in slice order after every point of `waits` is reached, and
  completes when all have run; its `signals` are then reached. A submit with no encoders
  is valid: it waits and signals alone. A submit takes all of its encoders or none:
  refused, or failed with `DeviceLost`, it leaves them recorded, changes no state and
  signals nothing.
- A refused call changes nothing, on every backend, whichever call it is. A call that
  breaks more than one rule is refused by one of them, unspecified which.
- A command outside its scope (the `Encoder` documentation names each scope), a second
  `begin` inside a pass, a draw or dispatch with no pipeline of its kind bound, an indexed
  draw with no index buffer bound and a pass left open at the end of an encoder are
  refused (`encoding`, every backend). So is an encoder of another device or another role,
  and one already pending. A device keeps one state for each encoder, which `rhi-metal`
  needs anyway to know its open command encoder; committing with a render encoder open
  aborts inside Metal with no validation layer on.
- After a breach it records, a device backend records nothing more into its driver's
  command buffer, and the submit refuses the encoder: a draw recorded after a skipped bind
  would break the driver's own rules before any submit.

**Timelines** (RHI point 11, Metal point 11 and item 9)

- A timeline's value only grows. A point is reached once the value is at least the point's.
  A submit's `signals` give a value greater than any the timeline has reached, been given by
  a host signal or been promised, and greater than any value the same submit waits for on
  it. A submit signals a timeline only where every pending signal of it is from the submit's
  own queue or from a submit it waits for. No value is 2^31 or more away from the timeline's
  value, from a host signal's in progress, or from a value a pending submit waits for or
  signals on it (`timeline`, every backend). A device keeps, for each timeline, the greatest
  value it knows the timeline has reached, and one list of the submits that wait for or
  signal it, each with its queue, the point it is stamped with, and its wait and its signal
  on the timeline. The known value is the greatest of every value a call has read from the
  driver for a check, folded in by that call's hold; a host signal's value, folded in by its
  second hold once its driver call has succeeded; and what a `wait_timeline` that returned
  `true` and a `timeline_value` saw, folded in by a hold each makes after its driver call. A
  signal in the list is a promise, pending until the known value reaches it, not until its
  submit completes; a wait in the list is pending until its submit completes. A submit
  leaves the list once neither is pending, so the list is no longer than the submits with a
  pending wait or promise on the timeline; a failed submit leaves it in the hold it takes
  after its driver call, and its promises, which never reached the driver, are not folded
  in, so a retry of the same point is accepted. In every check the timeline's value is the
  known value after the check's own read is folded in. Every value the timeline is ever
  signalled to is known, a promise in the list or the value of a host signal in progress, so
  the driver's value is at least the known value and at most the greatest of the three: a
  host signal another thread made in full after a check's read, and a promise another call
  saw reached after it, are in the known value. The 2^31 checks take the known value, the
  value of a host signal in progress, and the least and the greatest wait and signal in the
  list.
- `signal_timeline` gives a value greater than the timeline's and less than every value a
  pending submit will signal to it. It is refused (`timeline`, every backend) where a submit
  that will signal the timeline does not wait, on that timeline, for a value greater than
  the timeline's and no greater than the host's, which only this signal can bring the
  timeline to, and while another host signal of the timeline is in progress: either could
  reach the timeline first and leave the host's value below it. A submit that signals a
  timeline is refused while a host signal of it is in progress, unless it waits for that
  signal's value. A backend marks a host signal in progress, with its value, under the
  record's lock before its driver call, and in a second hold after it clears the mark and,
  where the call succeeded, folds the value into the known value.
- Each of these is a rule of Vulkan's for a timeline semaphore whose breach is undefined
  behaviour and may hang its waiters. A host signal below a pending promise that waits for
  it is the way the host releases a submit made before its wait is met. On an Apple GPU a
  submit whose wait the GPU reaches unmet is failed by the GPU's watchdog after about five
  seconds (an M1 under macOS 27.2.1; Apple documents no figure). MoltenVK then loses the
  device, and `rhi-metal` reports the failed command buffer as a loss. A submit made before
  its wait is met is released within that time, by its host signal or by the submit whose
  promise meets it; a stop between them, such as a debugger's, loses the device. On other
  drivers each device backend's README names the deadline or says there is none. A wait the
  timeline has already met holds nothing back, and a promise outlives its submit's
  completion until the timeline reaches it: on Metal the backend's own event, encoded before
  the caller's signals, completes a submit before its promise lands.
- `wait_timeline` blocks for at most its timeout and returns whether the point was reached.
  A wait may be made before the signal that satisfies it is submitted; a present behind one
  holds, and is refused past its bound ("The swapchain").
- A fence is a timeline point a submit signals. The facade has no other fence; Metal's
  `MTLFence` is `rhi-metal`'s own means for aliases.

**The swapchain** (Metal point 17 and item 11, decision 5.6)

- `acquire` returns within a bound the backend sets, about one second, and takes no timeout:
  an image, `None`, or `Stale`. One image is acquired at a time (`swapchain`, every
  backend). An acquired image is `Undefined` on the present role's queue and is a colour
  attachment and nothing else. Its handle is the caller's to use from the acquire to its
  present and at no other time (`swapchain`, null); a later acquire may return the same
  handle or another. `VK_SUBOPTIMAL_KHR` is reported as `Acquired::Image` and
  `Presented::Shown`, never as `Stale`; `Stale` is `VK_ERROR_OUT_OF_DATE_KHR`, and
  `VK_ERROR_OUT_OF_POOL_MEMORY` after MoltenVK's drawable timeout, from `acquire` or
  `present` (decision 5.6). On macOS a backend that makes the layer it presents to keeps
  that layer's bounds and `contentsScale` equal to the view's, on the main thread, so a
  surface's current extent is the window's size in physical pixels.
- `present` is a submit and a present in one call: the submit's encoders are of the
  `Present` role, it leaves the image in `ImageState::Present`, and its waits and signals
  are a submit's. The backend owns whatever its driver needs between the two (one present
  semaphore for each image, on Vulkan). Under MoltenVK each fetch of the drawable made while
  none is held waits up to about one second for each image of the swapchain; after a timeout
  its submit fetches again at each use of the image, and its present twice more, so
  `present` returns within about three seconds for each image, or more. The backend leaves
  the layer's `allowsNextDrawableTimeout` true. On other drivers its driver calls are
  bounded by the driver alone, and each device backend's README names the bound or says
  there is none.
- A present's driver calls are made only once its submit is released: each of its waits is
  reached, by the timeline's known value, or promised by a released submit whose driver call
  has returned, and where the present role shares the graphics queue every earlier submit on
  that queue is released. A submit is released on the same terms, for each of its waits and
  every earlier submit on its queue. A host signal in progress has not been made. The
  present's walk decides this from the records, in its hold under its queue's lock: a point
  at or below the known value, or one whose least promise at or above it has a stamp at or
  below the last its queue's driver has received, and in turn that promise's submit's waits
  and the earlier submits of its queue. A greater promise never decides a point: every
  greater promise depends on the least. A walk that comes back to a submit it is still
  deciding leaves the point undecided: a submit's own promise, or a promise of a submit that
  depends on it, never meets its wait. So a submit that waits on a timeline for a point the
  host has not signalled, and promises a greater point there, as a host release does
  ("Timelines"), is not released by its own promise. Where the walk leaves a point
  undecided, it stamps nothing, and the present lets go of the record's lock and its
  queue's, waits for the point on the driver with no lock held, within what remains of its
  bound, and walks again, folding in what the wait saw. It never waits under its queue's
  lock: on a shared queue the submit that signals the point may need that lock. Until the
  submit is released `present` holds for at most the bound `acquire` has; after that bound
  it is refused (`pending`, every backend) with nothing submitted, and the image is still
  the acquired one, for the caller to retry. Vulkan requires every signal a present waits
  for, and every signal that signal depends on, to have been submitted
  (VUID-vkQueuePresentKHR-pWaitSemaphores-03268), and the backend's present semaphore
  depends on every earlier submit on its queue.
- The number of presents in flight is the caller's to bound, by the points its presents
  signal. No backend bounds it (Metal item 11).
- After `Acquired::Stale` or `Presented::Stale` the caller waits for the points its presents
  signal and calls `resize`; every handle the swapchain gave before it is destroyed
  (`destroyed`). A present is pending until the driver no longer reads its image, which a
  submit's completion does not show: on Vulkan until its present fence signals. `resize`
  first waits, within the bound `acquire` has, for the swapchain's own presents, its present
  fences; the caller's wait before `resize` is for its own points alone, and a refusal as
  `pending` after that bound (every backend) is the caller's to retry. A Vulkan device made
  with a window requires `VK_KHR_swapchain_maintenance1` or `VK_EXT_swapchain_maintenance1`,
  with its feature and the instance's surface maintenance extension, and is not created
  without them (`Unsupported`): a queue's idle wait does not cover a present. MoltenVK
  offers both forms.
- `create_swapchain` and `resize` refuse an extent with a zero dimension (`bounds`, every
  backend). At a surface whose current extent is empty (a minimised window on Windows
  reports 0×0), neither calls the driver: `create_swapchain` returns a swapchain with no
  images, whose `extent` is 0×0 and `image_count` 0, and `resize` keeps the images it has.
  Either leaves the swapchain stale, and `acquire` returns `Stale` until a `resize` finds
  the surface not empty and makes images. The window can be minimised between the caller's
  read of its size and the call, so neither is a refusal. No extent outside the surface's
  minimum and maximum, as the backend's query found them just before the call, and none with
  a zero dimension, reaches the driver.

**Device loss** (architecture section 19, verification row 192)

- A loss is a value. Any call that reaches the device may return an error of kind
  `ErrorKind::DeviceLost`; from the first that does, `Device::is_lost` is true and stays
  true, and every later call that reaches the device returns `DeviceLost` too.
- On Metal a command buffer that ends in error (`MTLCommandBufferStatusError`, the GPU
  watchdog's timeout among them) is a loss. From then on `rhi-metal` returns `DeviceLost` as
  MoltenVK does, since nothing says the failed command buffer's signals land.
- After a loss: pending submits never complete and their signals are never made;
  `wait_timeline` returns at once, `Ok(true)` for a point already reached and `DeviceLost`
  otherwise; `acquire` and `present` return `DeviceLost`; every `destroy_*`, every
  `Encoder::reset` and every drop succeeds, so a caller tears down as it would otherwise;
  `info`, `capabilities`, `is_lost` and `pipeline_stats` never fail.
- Recovery is a new device from the backend's constructor. Nothing of the lost device means
  anything to it: every handle, encoder and swapchain is made again.
- The facade has no call that causes a loss. Null has one of its own, which the suite
  reaches through `Harness::lose`.

**Messages** (RHI point 14, Metal point 14, decision 5.10)

- Every warning and error a backend receives from a validator or its driver goes to the
  request's sink, from the constructor's start to the device's drop. A backend sends
  nothing of its own, and a clean run sends none.
- A sink is called inside the backend's driver calls, on the calling thread or a driver's
  own, and a queue's lock or the heap's may be held while it runs: it makes no call to the
  device or to anything the device made, and waits for no thread that may.
- A message is not an error: it changes no call's result. A rule a backend checks itself is
  returned from the call and is not also sent.
- What reaches the process's standard error alone, Metal's API and shader validation, is
  read there by the command that runs the tests. Metal API validation aborts the process:
  a device test runs in a process of its own.

**The rules**

A call that breaks a rule a backend checks returns `Rule::refuse`'s error: kind
`InvalidData`, a message that begins with the rule's name, and a `Refusal` as its source,
which `Rule::of` reads. Null checks every rule. A device backend checks the "Every backend"
column: each clause whose breach on a device would be undefined behaviour in the process, a
GPU fault, or an abort inside the driver, Metal API validation's asserts included. Whatever
a caller passes, no call reaches freed host memory, a freed driver object or one a pending
submit still uses, or another device's object, through a backend's own code, and no command
its driver forbids outright reaches the driver's command buffer. The methods are safe Rust,
and must be: their callers are crates that forbid `unsafe`, which is why no call may reach
undefined behaviour, not a licence for one to. Two residuals are accepted (D158), and each
device backend's `SAFETY.md` names both.

The first is the "Null alone" column: the clauses whose breach on a device gives a wrong
image, wrong pixels, a stale read, a race between two passes. A device backend leaves them
unchecked. Null catches them in every consumer's tests, where every rule is checked, and the
validators of decision 5.10 catch them on a GPU. In Vulkan's own terms such a breach still
breaks a valid-usage rule, whose effect the specification leaves undefined; what both spikes
saw of it is a wrong image.

The second is a shader's code and what a shader reaches through data alone. A shader's code
is the cook's and is trusted. A backend checks a module's form: its magic and version, and
on SPIR-V its `OpCapability` and `OpExtension` instructions against what the device enabled
(`capability`, every backend that takes SPIR-V; a Metal target declares none, and Metal's
compiler refuses what the GPU cannot run). What a malformed module does, and what a shader
reaches through data alone (an address, a slot number or a sampler index in push bytes or in
a buffer, an indirect record's contents or a dispatch's counts), may fault the GPU or race a
host access. The cook's checks guard it, and Metal shader validation and Vulkan's
GPU-assisted validation catch it in tests. Every backend checks what the arguments show of
it: a `push_slot` or `push_address` the pass did not declare and a declared slot that is
empty are refused, and a placeholder in every empty slot and free sampler entry turns a
stale number into a wrong image ("The heap and the sampler table").

| Rule | Name | Every backend | Null alone |
|---|---|---|---|
| `State` | `state` | a buffer a pass declares in two states | a transition's `from`, a use's state, by subresource; a use after a write in a writing state with no transition since |
| `Queue` | `no-qfot` | | a use off the owning queue; an acquire with no release, or of a release a transition from `Undefined` ended; a change of state or owner under another queue's pending submit, or a use under another queue's pending write or change of state or owner, with no wait for it, direct or through the submits it waits for; an alias call across queues |
| `Destroyed` | `destroyed` | a stale handle, at the `Device` call it is given to, or at the submit of the encoder it was recorded into, a resource destroyed since the recording included | a handle another device issued that resolves to a live resource |
| `Pending` | `pending` | `Encoder::reset` while pending; `resize` with a present pending past its wait; `present` behind a wait that no signal made or submitted meets, past `acquire`'s bound; a destroy under a pending submit of any queue; of an image a slot holds; of a block an image is placed in; of a sampler while any submit made before it is pending; `write_buffer` of a buffer a pending submit writes; `read_buffer` and `invalidate_buffer` of one a pending submit writes or changes the state of | a submit counted pending until the caller has seen its point ("Queues by role") |
| `Capability` | `capability` | every call that needs an unreported capability; a SPIR-V module whose `OpCapability` or `OpExtension` instructions name one the device did not enable | |
| `Usage` | `usage` | host access outside the memory class; `buffer_address` without `DEVICE_ADDRESS`; a descriptor no device could honour; every other use outside a usage, a transition into a state the image's usage does not allow included; a pass's attachments; a pipeline's formats, and colour targets past `colour_attachments` | |
| `Bounds` | `bounds` | host ranges; heap slots; placements; push-constant offsets; copy regions, outside their resources or two of one call that write one byte of a buffer or one texel of an image, a buffer offset of a buffer-image copy that is not a multiple of the texel included; the scissor; the viewport, against `viewport_extent` and `viewport_bounds`; the index buffer's offset and the indices a direct draw reads; indirect offsets and strides, their alignment and the records they span; a dispatch's counts against `compute_groups`; a swapchain extent with a zero dimension | |
| `Undeclared` | `undeclared` | `push_slot`, `push_address` | |
| `Unflushed` | `unflushed` | | a written range at a submit; a read with no invalidate |
| `HostOverlap` | `host-overlap` | overlapping host access; a destroy of a buffer under a host access; a submit that writes a buffer under one | |
| `Local` | `local` | | an input read with no local read since the write |
| `LocalMap` | `local-map` | | an input with no attachment of its index marked |
| `Split` | `split` | | a memoryless attachment after a local read, where lost |
| `MemorylessStore` | `memoryless-store` | a memoryless attachment stored, or loaded with `Load::Keep` | |
| `HeapLayout` | `heap-layout` | a declared slot that is empty; one written since its pass began | a declared slot not in `SampledRead` |
| `HeapWrite` | `uuwp-used` | a slot written under a pending declaration; a submit that declares a slot whose write is in progress | |
| `Alias` | `no-fence` | | a placed image used with no alias call since its bytes were another's |
| `UndefinedRead` | `undefined-read` | | a read of undefined contents |
| `PresentQueue` | `present-queue` | `Device::submit` to `Present` | a swapchain image on another role's encoder |
| `Swapchain` | `swapchain` | a second acquire; a present of another image; a second swapchain | a swapchain image used outside its acquire and its present |
| `Timeline` | `timeline` | a submit's signal that does not grow past the timeline's value, its host signals and promises and the values the submit waits for, or that another queue's pending signal could pass; a host signal not below every pending promise, or one a promise whose wait it does not alone release or another host signal could pass; a value 2^31 or more from the timeline's, a host signal's in progress, or a pending wait or signal | |
| `Encoding` | `encoding` | a foreign, wrong-role or pending encoder in a submit; one submitted again with no reset; a command outside its scope; a draw or dispatch with nothing bound to it; a pass left open | |

**What a device backend keeps**

The "Every backend" column costs a device backend a few records, each cheap beside the
driver call it guards:

| Clauses | What a device keeps | Cost |
|---|---|---|
| `destroyed` at a call, and at a submit for every handle the encoder recorded | Its resource tables, by handle, whose entries do not move while the device lives (a fixed capacity, or chunks never moved), each live entry behind a lock of its own; the encoder's copy of what each handle it recorded resolved to, with a count, walked again at the submit | One lookup a handle an encoder, under that handle's entry lock, and a count kept to the encoder's reset; the walk the submit makes anyway, over the encoder's distinct handles |
| `pending`: a reset, a resize, a destroy under a pending submit, a sampler's destroy | The point each queue's last submit is stamped with, taken under the queue's lock in the order its driver receives them, and the completed value of each queue, read from the driver before the lock and folded into the known value of the queue's own timeline ("Queues by role"); for each queue, the caller's points its pending submits signal, a failed submit's dropped with its promises; for each queue, the stamp of its last submit whose driver call has returned; for each resource, the last submit of each queue that names it, three stamps at most, stamped in the walk; a present's fence for each image the swapchain presents | A stamp a handle and queue at the submit; a read before the lock, folded, and a compare at the destroy; where the queue is behind, a compare of the caller's points with their known values, with no driver read; a hold after each submit's driver call |
| `pending`: an image a slot holds, a block an image is placed in | A count of the slots holding each image and of the images placed in each block | An increment and a decrement |
| `pending`: `write_buffer` under a writer; `read_buffer` and `invalidate_buffer` under a writer or a transition | For each buffer, the last submit of each queue that writes it, and the last that writes it or changes its state, stamped in the walk | A compare a host call |
| `state`: a buffer declared in two states | The open pass's declaration, on the encoder | A compare of the declaration's buffers as the pass begins, with no lock |
| `host-overlap`: overlapping host access, a destroy or a submit that writes under one | For each buffer, the ranges of the host accesses in progress | An insert and a removal a call, and a lookup for each buffer a submit writes, under the record's lock ("Threads") |
| `uuwp-used`; `heap-layout`'s empty slot and its slot written since the pass began | For each slot, its image, behind the slot's entry lock, and the last submit of each queue that declares it, stamped in the walk; a mark while a write of the slot is in progress, and the image it writes, held; on the encoder, the image each declared slot resolved to when its pass began, counted | A lookup and a compare for each declared slot of each pass at the submit, at most the heap's slots a pass; a lookup for each declared slot as its pass begins, which `rhi-metal` makes for `useResource` anyway |
| `undeclared` | The open pass's declaration, on the encoder | A lookup a push of a slot or an address, at recording, with no lock |
| `usage`, `bounds` | Each resource's description, in what the encoder resolved; the device's capabilities | A compare a command, at recording, against what the encoder resolved, with no lock |
| `capability`: a SPIR-V module's capabilities and extensions | The capabilities and extensions the device enabled | A scan of the module's header at each pipeline's creation |
| `memoryless-store` | One flag for each image | A compare for each attachment as its pass begins |
| `encoding` | One state for each encoder, and the breach it recorded first | Free: `rhi-metal` needs the state to know its open command encoder |
| `timeline` | For each timeline, the greatest value the device knows it has reached; one list of the submits that wait for or signal it, each with its queue, its stamp and its wait and signal on it, kept while its wait or promise is pending; a mark, with its value, for a host signal in progress; for each queue, the stamp of its last submit whose driver call has returned | A compare a signal or a wait against the known value, the mark's value and the list; the read before the lock folded in the hold; the mark set under the record's lock before a host signal's driver call, and cleared after it, with the value folded in where the call succeeded; a hold after `wait_timeline` and `timeline_value` to fold what they saw; a hold after each submit's driver call, which records its stamp and drops a failed submit's entries; a present's walk over the points its submit depends on, again after each wait |

A breach a device finds while recording is carried to the submit as the encoder's first
breach, and the backend records nothing more into its driver's command buffer.

## How each call is honoured

Every call is one the null backend honours by the state it keeps, and one native Metal
honours by a call the Metal spike made. The Vulkan column is the RHI spike's.

| Calls | Null keeps | Native Metal | Vulkan | Evidence |
|---|---|---|---|---|
| The constructor against the two sets; `capabilities`, `info` | The set it is created with | Argument buffers tier 2, Metal 3, an Apple family, or `Unsupported`; reports no multi-draw and no indirect count | The features of the required table, or `Unsupported` | RHI item 1, point 1; Metal "The device", point 1 |
| Roles and `QueueLayout` | The layout of its set, one queue or several | A command queue for each role; no families, no ownership | A second family that draws and presents (MoltenVK has four), or one queue (lavapipe) | RHI item 1, point 2; Metal item 11, point 2 |
| `create_buffer`, `destroy_buffer`, `buffer_address` | A table by handle; addresses it invents, distinct | `makeBuffer`, `gpuAddress` | A buffer with `SHADER_DEVICE_ADDRESS` | RHI item 2, point 3; Metal item 2, point 3 |
| `write_buffer`, `read_buffer`, `flush_buffer`, `invalidate_buffer` | A byte store for each buffer, lent for the call with the device's records unlocked; the range recorded as in progress; written and flushed ranges under a set that is not coherent | `contents()` of shared memory; flush and invalidate do nothing | Persistently mapped memory; `vkFlushMappedMemoryRanges`, `vkInvalidateMappedMemoryRanges` | RHI point 3, question 8; Metal point 3 |
| `create_image`, `destroy_image`, `image_memoryless` | Usage, state for each subresource, the request and whether its set honours it | `makeTexture`; `.memoryless` storage for colour and depth | An image; lazily allocated memory where offered | RHI item 7, point 4; Metal item 3, point 4 |
| `create_memory_block`, `image_placement`, `ImageMemory::Placed`, `Encoder::alias` | Byte ranges for each block; which image last used each; refuses `no-fence` | An untracked placement heap; `heapTextureSizeAndAlign`; the block's fence waited on before, and updated after, every encoder that uses a placed image, at the alias call and at every other use | Bound memory at an offset; a barrier from undefined | Metal item 4, point 16 |
| `write_heap_slot`, `clear_heap_slot` | The image for each slot; the declarations of pending submits; refuses `uuwp-used` | Under the heap's lock, the slot marked, a write of the texture's resource id into the argument buffer, the placeholder's on a clear, and the mark cleared | Under the heap's lock, the slot marked, an update-after-bind descriptor write, and the mark cleared | RHI item 3, point 5; Metal item 1, point 5 |
| `create_sampler`, `destroy_sampler` | A table of descriptors | A sampler with `supportArgumentBuffers`, its id in the table's argument buffer, the placeholder's in a free entry; at most `maxArgumentBufferSamplerCount` | A sampler in the table's descriptor array, the placeholder in a free entry | Metal item 10, point 15 |
| `CookedShader`, `create_graphics_pipeline`, `create_compute_pipeline`, the destroys | The cooked fields of a `SpirV` target: stages, constants' lengths, inputs, outputs, heap capacity, and the capabilities the module declares. It runs no code | A library from MSL 3.1 or a metallib; functions always specialised; a pipeline state | Shader modules with specialisation info | RHI item 4, point 6; Metal item 6, point 6 |
| `pipeline_cache`, the request's bytes, `pipeline_stats`, `seal_cache`, `open_cache` | A payload of its own under the same seal | A binary archive loaded from a private copy of the opened payload; fail-on-miss; never under shader validation | `VkPipelineCache` data; creation feedback | RHI item 5, point 7; Metal item 7, point 7 |
| `create_encoder`, `Encoder::reset`, recording on a thread | A command list, its scope, whether it is submitted or pending, and its first breach | A command buffer made on first use; `reset` drops it; the open command encoder, the state device backends keep | A command pool and one command buffer; `vkResetCommandPool` | RHI item 6, point 8; Metal item 8, point 8 |
| `transition_images`, `transition_buffers` | The state and owner of each subresource, applied at the submit in order; whether a pass or copy wrote it since | Nothing, but for placed images, whose every use the block's fence orders | Barriers of synchronisation 2, a same-state one in a writing state included; a release and an acquire across families | RHI item 7, point 9; Metal point 9 |
| `begin_render_pass`, `begin_compute_pass`, `end_pass`, `PassReach` | The attachments, what is declared, what was written since the last local read | A render or compute command encoder; `useResource` for each declared slot's texture, the placeholder image and each declared buffer | `vkCmdBeginRendering` with the input mapping; the declaration unused | RHI item 7, points 5 and 10; Metal items 2 and 3, point 5 |
| `local_read` | Clears "written since"; refuses `local`, `split` | Nothing: framebuffer fetch reads the tile | The local-read barrier (which MoltenVK turns into a pass split) | RHI item 7, point 10; Metal item 3, point 10 |
| `bind_*`, `set_viewport`, `set_scissor`, `push_bytes`, `push_address`, `push_slot` | The bound pipeline; typed pushes against the declaration | State on the encoder; the block as bytes set for each stage | Dynamic state; `vkCmdPushConstants` | RHI items 2 to 4; Metal items 1 and 2 |
| `draw`, `draw_indexed`, `draw_indexed_indirect`, `draw_indexed_indirect_count` | Checks states, declarations and capabilities; draws nothing | `drawPrimitives`, `drawIndexedPrimitives`, one indirect record a call; no count form | The draw calls; multi-draw and indirect count where reported | RHI item 9, point 12; Metal item 5, point 12 |
| `dispatch`, `dispatch_indirect` | Checks; runs nothing | `dispatchThreadgroups`, and its indirect form, on a serial compute encoder | `vkCmdDispatch`, `vkCmdDispatchIndirect`; a shader-write barrier between two dispatches of a pass that declares a `ShaderWrite` buffer | `dispatch`: RHI items 2 and 8, Metal items 2 and 9. `dispatch_indirect`: architecture 12.1 alone; both files list it under "Not shown" (see "Proof") |
| `copy_buffer`, `copy_buffer_to_image`, `copy_image_to_buffer` | Buffer to buffer moves bytes in its stores; the image forms are checked and move nothing | Blit encoder copies | The copy commands | RHI item 10, point 13; Metal item 12, point 13 |
| `create_timeline`, `timeline_value`, `signal_timeline`, `wait_timeline` | A counter and a condition variable | `MTLSharedEvent`: `signaledValue`, a listener, a wait with a timeout | A timeline semaphore | RHI item 8, point 11; Metal item 9, point 11 |
| `submit` with waits and signals | Checks every command in order, applies the states, completes when its waits are reached; pending until the caller sees its point | Command buffers enqueued in slice order, `encodeWaitForEvent` and `encodeSignalEvent`, the queue's own shared event before the caller's; the handles each names stamped with its point under the queue's lock | `vkQueueSubmit2` with timeline semaphores, the queue's own among the signals; the handles stamped likewise | RHI items 6 and 8; Metal items 8 and 9 |
| `create_swapchain`, `acquire`, `present`, `resize` | Images with no window; an acquire that can be made to return `None` | A `CAMetalLayer`, its bounds and `contentsScale` the view's; `nextDrawable`, which has its own one-second limit; `present(drawable)` before the commit | A swapchain, with the swapchain maintenance extension's present fence for each present, waited for by `resize` within `acquire`'s bound; an acquire with a finite timeout; `vkQueuePresentKHR` after the submit | Metal item 11, point 17; decision 5.6 |
| `is_lost`, `DeviceLost` | A flag its own call sets | A command buffer's error status | `VK_ERROR_DEVICE_LOST` | Architecture 19; verification row 192 |
| `MessageSink` | Never called | The driver's log where it has one; validators write to standard error | The debug messenger | RHI "Validation runs", point 14; Metal "Validation runs", point 14 |

## The conformance suite

`conformance::CASES` lists every case by name, each a *contract* case or an *output* case,
and `conformance::run` runs one against a `Harness`. The bodies arrive with the backends:
the contract cases' with `rhi-null`, the output cases' and their Slang sources with
`vulkan`. A backend's tests carry one test for each case, named after it. There are 161: 136
contract cases and 25 output cases.

- A **contract** case asserts what the contract says of calls and results. It needs no
  shader to run and no pixel, so null passes it as a device does. A case that names a rule
  (`Case::refuses`) fails unless the call is refused by exactly that rule: a backend built
  without the rule fails it, which is the mutation proof the plan asks of null.
- `Case::devices` says whether a device backend runs a case. It is `false` for the cases
  that provoke a check null alone makes, whose breach on a device gives a wrong image and
  proves nothing, and for the one that holds null's own meaning of a pending submit
  (`destroy_after_an_unobserved_submit_is_refused`), since a device's pending is completion
  and its refusal there depends on timing.

The refusal cases a device backend also runs are those of the rules table's "Every backend"
column, one or more for each of its clauses, 64 in all: `unreported_capability_is_refused`,
`submit_to_the_present_role_is_refused`, `overlapping_host_access_is_refused`,
`destroy_during_host_access_is_refused`, `host_range_outside_the_buffer_is_refused`,
`host_access_outside_the_memory_class_is_refused`,
`address_of_a_buffer_without_the_usage_is_refused`,
`read_under_a_pending_writer_is_refused`,
`read_under_a_pending_transition_to_host_read_is_refused`,
`write_under_a_pending_writer_is_refused`,
`submit_writing_under_a_host_access_is_refused`, `buffer_use_outside_its_usage_is_refused`,
`image_use_outside_its_usage_is_refused`, `memoryless_request_without_transient_is_refused`,
`memoryless_attachment_stored_is_refused`, `memoryless_attachment_kept_is_refused`,
`placement_outside_the_block_is_refused`,
`destroy_of_a_block_an_image_is_placed_in_is_refused`,
`heap_slot_outside_the_heap_is_refused`, `declared_slot_empty_is_refused`,
`slot_written_after_its_pass_began_is_refused`,
`heap_write_under_a_pending_reader_is_refused`,
`destroy_of_an_image_a_slot_holds_is_refused`,
`sampler_destroyed_under_a_pending_submit_is_refused`,
`undeclared_slot_argument_is_refused`, `undeclared_address_argument_is_refused`,
`buffer_declared_in_two_states_is_refused`,
`shader_needing_an_unenabled_capability_is_refused`,
`pipeline_of_other_formats_than_its_pass_is_refused`,
`encoder_reset_while_pending_is_refused`, `encoder_in_two_submits_is_refused`,
`encoder_resubmitted_without_reset_is_refused`,
`encoder_of_another_role_or_device_is_refused`, `command_outside_its_scope_is_refused`,
`draw_with_nothing_bound_is_refused`, `pass_left_open_is_refused`,
`push_outside_the_block_is_refused`, `copy_region_outside_its_resource_is_refused`,
`copy_regions_overlapping_in_the_destination_are_refused`,
`dispatch_beyond_the_group_limit_is_refused`,
`timeline_value_that_does_not_grow_is_refused`,
`host_signal_above_a_pending_signal_is_refused`,
`host_signal_below_a_promise_that_does_not_wait_for_it_is_refused`,
`host_signal_below_a_promise_whose_wait_is_met_is_refused`,
`submit_signalling_no_more_than_it_waits_for_is_refused`,
`timeline_value_beyond_the_difference_limit_is_refused`,
`timeline_signalled_from_two_queues_without_a_wait_is_refused`,
`destroy_under_a_pending_submit_is_refused`,
`concurrent_resource_destroyed_while_another_queue_uses_it_is_refused`,
`use_after_destruction_is_refused`, `destroyed_after_recording_is_refused_at_submit`,
`attachments_of_unequal_extent_are_refused`, `scissor_outside_the_attachments_is_refused`,
`viewport_outside_its_rules_is_refused`,
`index_buffer_offset_outside_or_unaligned_is_refused`,
`indexed_draw_past_its_index_buffer_is_refused`,
`indirect_arguments_outside_or_unaligned_are_refused`,
`second_acquire_before_a_present_is_refused`, `present_of_an_image_not_acquired_is_refused`,
`present_behind_an_unmet_wait_is_refused`, `second_swapchain_is_refused`,
`swapchain_extent_with_a_zero_dimension_is_refused`,
`resize_with_a_present_pending_is_refused` and `image_of_before_a_resize_is_refused`.

- A device runs each of them as null does, holding the earlier submit pending where the case
  needs one by a wait on a point it signals only after the refused call. A present is never
  held so, since a present behind an unmet wait is refused.
- The five race cases start two calls at once on two threads, round after round. Each but
  the race of host signals holds a submit pending by a wait on a point not yet signalled, so
  that both accepted is a defect on any backend however fast its device.
  `destroy_racing_a_submit_refuses_one` races a destroy of a buffer against a submit that
  names it, and exactly one is refused, `pending` or `destroyed`;
  `submit_racing_a_host_access_refuses_one` races a submit that writes an upload buffer
  against a `write_buffer` of it whose `fill` stays until the submit has returned, and
  exactly one is refused, `host-overlap` or `pending`;
  `submits_racing_on_one_queue_keep_their_order` races two submits to one queue, one held by
  its wait, and a destroy of what the held one names is refused `pending` until the wait is
  reached; `heap_write_racing_a_submit_refuses_one` races a `write_heap_slot` against the
  submit of an encoder whose pass declared the slot before the race, and exactly one is
  refused: the write as `uuwp-used`, or the submit as `uuwp-used` or `heap-layout`;
  `host_signals_racing_on_one_timeline_keep_it_growing` races two host signals of one
  timeline, to two values above its own, and where both are accepted the timeline ends at
  the greater. The race cases catch a backend whose check and update are split only where
  something falls between them. A device backend's records are a type of its own that makes
  no driver call, and its tests run the same interleavings under Miri with many seeds and
  under stress, with fixed interleavings among them: a host signal made in full between
  another host signal's driver read and its hold, and between a submit's read and its hold;
  a promise reached, and dropped by a third call's check, in the same two windows; a pass
  that begins between a heap write's first hold and its descriptor write, with its submit's
  walk before the write's second hold; a destroy made at once after a `wait_timeline` that
  returned `true`, with the queue's own value behind the stamp, which is accepted; and a
  failed submit's point signalled from the host while an earlier submit of its queue is
  pending, after which a destroy of what that submit uses is refused; and a submit that only
  waits, made between a host signal's two holds, 2^31 below its value, which is refused.
  Each asserts after every hold that the known value is at most the driver's.
- `destroy_at_once_after_the_wait_is_accepted` holds a device to "Queues by role": a lagging
  submit, a wait for its point, then at once a destroy and a reset, repeated, each accepted.
- `dispatch_beyond_the_group_limit_is_refused` and `viewport_outside_its_rules_is_refused`
  go one past the limits the device reports; null runs them under `Capabilities::LEAST`, and
  a device under `LEAST` would need to report below its driver's limits, which no device
  backend does, so a device runs them against its own.
  `empty_draws_and_dispatches_record_nothing` runs with Metal API validation on.
- `shader_needing_an_unenabled_capability_is_refused` creates a pipeline from a SPIR-V
  module that declares `Int64Atomics` on a device that does not report `buffer_atomics_64`.
  `rhi-metal` skips it: a Metal target declares no capability, and Metal's compiler refuses
  a shader the GPU cannot run, as `InvalidData`.
- `host_signal_below_a_promise_whose_wait_is_met_is_refused` holds a submit that waits for a
  value the timeline has reached and signals above the host's;
  `submit_signalling_no_more_than_it_waits_for_is_refused` waits for a value on a timeline
  and signals one no greater on it; `timeline_value_beyond_the_difference_limit_is_refused`
  gives a value 2^31 from the timeline's, one 2^31 above a pending submit's wait, and a wait
  2^31 below a pending submit's greater wait; `slot_written_after_its_pass_began_is_refused`
  begins a pass that declares a slot, writes the slot, and submits.
  `swapchain_extent_with_a_zero_dimension_is_refused` makes a swapchain with a zero width
  and resizes one to a zero height; `resize_with_a_present_pending_is_refused` holds a
  present pending through null's `Harness::stall`, which a device does not offer; on a
  device it presents, waits for the points its present signals, resizes at once, and finds
  the resize succeeds.
  `present_behind_an_unmet_wait_is_refused` presents with a submit that waits for a point no
  signal has been made or submitted for, and is refused after the bound with the image still
  acquired; it then signals the point from the host and presents the same image, which is
  shown. Under a set whose present role shares the graphics queue, the unmet wait is instead
  in a frame submit made before the present. On every set it also presents behind a frame
  submit that waits, on the timeline it signals, for a point the host has not signalled, the
  present waiting for the frame's point; that present is refused too, and is shown once the
  host signals.

- An **output** case draws, dispatches or copies on a device, reads the result back and
  compares it with a reference computed on the CPU, never with a stored image (decision
  5.10). Null skips it.
- Every device a case makes has a sink that counts, and a case fails on any message it did
  not provoke. `each_validator_reports_its_canary` is the one that provokes them.
- A harness names the capability sets it runs under. Null runs every contract case under
  each set its rules need: one queue and several, coherent memory and not, memoryless kept
  across a local read and lost, each optional capability on and off.
- On the M1 the output cases that touch the heap or an address run under Metal shader
  validation and once without it, and `declared_reader_follows_a_lagging_writer` counts as
  evidence only in a process where the same case with its declaration dropped reads stale
  (Metal, "What `rhi-api` must expose"). The cache's cases run without shader validation.

Some output cases count as evidence only in the shape, or on the device, named here:

- `aliased_transients_render_in_turn` cannot fail under MoltenVK, whose heaps are tracked
  and keep depth out. It counts on `rhi-metal` with a lagging writer of one transient, read
  in the next pass through a declared slot before the alias call, run without shader
  validation, which serialises the work.
- `memoryless_g_buffer_is_lit_in_its_pass` reports `image_memoryless` for its G-buffer, as
  the RHI result did, and is not memoryless under MoltenVK ("Images").
- `ownership_transfer_keeps_the_contents` cannot fail on lavapipe, which has one queue, or
  on MoltenVK, whose families share Metal resources: only a discrete GPU shows it, and
  plan decision 5.17 leaves transfers to null.
- `sampler_filters_as_described` picks among two or more samplers by a non-uniform index
  past zero, in the draw that indexes the image array past slot zero, and compares exactly
  where filtering is exact, with ±1 at a linear midpoint.
- `dispatch_indirect_reads_its_counts` has a dispatch write the counts, at an offset that
  is not zero and all three different, through a `ShaderWrite` to `Indirect` transition,
  with a lagging writer; a mutation that drops the transition shows on a discrete GPU
  alone, since lavapipe runs each dispatch to completion before the next command,
  synchronisation validation does not track indirect buffers and Metal's tracking hides the
  miss on the M1.
- `dispatches_of_one_pass_see_each_others_writes` has a lagging first dispatch; a mutation
  that drops Vulkan's barrier between the two shows on a discrete GPU alone, since lavapipe
  runs each dispatch to completion before the next command, and on the M1 Metal's serial
  compute encoder orders the two, so the case cannot fail there.
- `viewport_and_scissor_place_the_triangles` draws with a viewport off the centre and a
  scissor strictly inside the attachment, triangles of both windings under `Cull::Back`,
  compared exactly; `clear_and_keep_load_as_described` also orders two passes into one
  attachment through a same-state transition ("States").
- `draw_parameters_reach_the_shader` draws indexed with a `base_vertex` and a first
  instance that are not zero, and writes the ids the shader reads.

| Area | Contract cases | Output cases |
|---|---|---|
| Capabilities and queues | `capabilities_are_fixed`, `unreported_capability_is_refused`, `roles_resolve_as_the_layout_reports`, `submit_to_the_present_role_is_refused` | |
| Buffers and host access | `buffer_write_then_copy_then_read_returns_the_bytes`, `buffer_address_is_stable_and_distinct`, `host_ranges_that_do_not_overlap_are_written_at_once`, `overlapping_host_access_is_refused`, `destroy_during_host_access_is_refused`, `host_range_outside_the_buffer_is_refused`, `host_access_outside_the_memory_class_is_refused`, `address_of_a_buffer_without_the_usage_is_refused`, `read_under_a_pending_writer_is_refused`, `read_under_a_pending_transition_to_host_read_is_refused`, `write_under_a_pending_writer_is_refused`, `submit_writing_under_a_host_access_is_refused`, `submit_racing_a_host_access_refuses_one`, `unflushed_range_at_a_submit_is_refused`, `flush_and_invalidate_are_accepted_where_memory_is_coherent`, `buffer_use_outside_its_usage_is_refused` | `device_memory_is_filled_through_a_staging_copy` |
| Images, memoryless, aliases | `image_use_outside_its_usage_is_refused`, `memoryless_outcome_is_queryable_and_as_reported`, `memoryless_request_without_transient_is_refused`, `memoryless_attachment_stored_is_refused`, `memoryless_attachment_kept_is_refused`, `memoryless_attachment_after_a_local_read_is_refused_where_lost`, `placement_outside_the_block_is_refused`, `destroy_of_a_block_an_image_is_placed_in_is_refused`, `aliased_image_used_without_the_alias_call_is_refused`, `alias_read_before_its_first_write_is_refused`, `aliases_used_in_turn_with_the_alias_call_pass` | `memoryless_g_buffer_is_lit_in_its_pass`, `aliased_transients_render_in_turn`, `image_round_trips_through_copies` |
| States and queues | `transition_from_another_state_is_refused`, `use_in_another_state_is_refused`, `use_after_a_write_without_a_transition_is_refused`, `state_is_kept_for_each_mip_and_layer`, `state_follows_submission_order_not_recording_order`, `cross_queue_use_without_a_transfer_is_refused`, `acquire_without_a_release_is_refused`, `release_and_acquire_cross_a_resource`, `shared_and_discarded_resources_cross_without_a_transfer`, `roles_on_one_queue_need_no_transfer`, `image_taken_while_another_queue_reads_it_is_refused`, `read_of_contents_left_undefined_is_refused`, `use_under_another_queues_pending_write_is_refused`, `use_in_a_later_submit_than_the_waiting_change_is_refused`, `use_waiting_through_another_submit_passes`, `acquire_of_a_release_a_retake_ended_is_refused`, `read_after_a_retake_of_an_unmet_release_is_refused` | `ownership_transfer_keeps_the_contents` |
| The heap, samplers, declarations | `heap_slot_outside_the_heap_is_refused`, `declared_slot_empty_is_refused`, `slot_written_after_its_pass_began_is_refused`, `declared_slot_not_in_sampled_read_is_refused`, `heap_write_under_a_pending_reader_is_refused`, `heap_write_after_its_reader_completes_passes`, `heap_write_racing_a_submit_refuses_one`, `destroy_of_an_image_a_slot_holds_is_refused`, `sampler_destroyed_under_a_pending_submit_is_refused`, `undeclared_slot_argument_is_refused`, `undeclared_address_argument_is_refused`, `declared_buffer_in_another_state_is_refused`, `buffer_declared_in_two_states_is_refused`, `sampler_table_is_bounded` | `heap_is_sampled_past_slot_zero`, `address_is_read_in_a_shader`, `declared_reader_follows_a_lagging_writer`, `sampler_filters_as_described` |
| Cooked shaders, pipelines, the cache | `cooked_shader_round_trips`, `cooked_shader_of_another_version_is_refused`, `cooked_shader_corrupt_or_truncated_is_refused`, `shader_for_another_heap_is_refused`, `shader_target_the_backend_does_not_run_is_unsupported`, `constant_of_another_length_is_refused`, `shader_needing_an_unenabled_capability_is_refused`, `pipeline_of_other_formats_than_its_pass_is_refused`, `sealed_cache_round_trips`, `cache_corrupt_truncated_or_foreign_starts_empty` | `specialised_constant_changes_the_output`, `write_mask_keeps_a_channel`, `depth_state_orders_two_triangles`, `warm_cache_hits_every_stage` |
| Encoders and submits | `encoder_recorded_on_another_thread_submits`, `encoder_reset_while_pending_is_refused`, `pending_encoder_drop_waits_for_its_submit`, `encoder_in_two_submits_is_refused`, `encoder_resubmitted_without_reset_is_refused`, `recording_while_pending_records_nothing`, `encoder_of_another_role_or_device_is_refused`, `command_outside_its_scope_is_refused`, `draw_with_nothing_bound_is_refused`, `pass_left_open_is_refused`, `push_outside_the_block_is_refused`, `copy_region_outside_its_resource_is_refused`, `copy_regions_overlapping_in_the_destination_are_refused`, `refused_submit_changes_nothing`, `submit_of_no_encoders_waits_and_signals`, `submit_is_pending_until_its_wait_is_reached`, `empty_draws_and_dispatches_record_nothing` | `two_threads_record_at_once` |
| Timelines, destroys and races | `timeline_is_signalled_waited_and_read_by_the_host`, `timeline_wait_times_out`, `waits_are_woken_by_the_submit_that_reaches_them`, `wait_is_woken_by_a_host_signal`, `timeline_value_that_does_not_grow_is_refused`, `host_signal_above_a_pending_signal_is_refused`, `host_signal_below_a_promise_that_does_not_wait_for_it_is_refused`, `host_signal_below_a_pending_signal_releases_its_waiter`, `host_signal_below_a_promise_whose_wait_is_met_is_refused`, `submit_signalling_no_more_than_it_waits_for_is_refused`, `timeline_value_beyond_the_difference_limit_is_refused`, `timeline_signalled_from_two_queues_without_a_wait_is_refused`, `destroy_under_a_pending_submit_is_refused`, `concurrent_resource_destroyed_while_another_queue_uses_it_is_refused`, `destroy_after_an_unobserved_submit_is_refused`, `destroy_at_once_after_the_wait_is_accepted`, `destroy_racing_a_submit_refuses_one`, `submits_racing_on_one_queue_keep_their_order`, `host_signals_racing_on_one_timeline_keep_it_growing`, `use_after_destruction_is_refused`, `destroyed_after_recording_is_refused_at_submit`, `handle_of_another_device_is_refused` | |
| Render passes and the local read | `input_read_without_a_local_read_is_refused`, `input_read_of_another_attachment_is_refused`, `attachments_of_unequal_extent_are_refused`, `scissor_outside_the_attachments_is_refused`, `viewport_outside_its_rules_is_refused` | `triangle_matches_the_reference`, `local_read_sees_what_the_pass_wrote`, `clear_and_keep_load_as_described`, `viewport_and_scissor_place_the_triangles`, `draw_parameters_reach_the_shader` |
| Indirect draws and dispatches | `index_buffer_offset_outside_or_unaligned_is_refused`, `indexed_draw_past_its_index_buffer_is_refused`, `indirect_arguments_outside_or_unaligned_are_refused`, `dispatch_beyond_the_group_limit_is_refused` | `indexed_indirect_draw_matches_the_reference`, `indirect_count_draws_where_reported`, `dispatch_writes_through_an_address`, `dispatches_of_one_pass_see_each_others_writes`, `dispatch_indirect_reads_its_counts` |
| The swapchain and the present role | `second_acquire_before_a_present_is_refused`, `present_of_an_image_not_acquired_is_refused`, `present_behind_an_unmet_wait_is_refused`, `present_of_an_image_not_in_present_is_refused`, `swapchain_image_on_a_frame_encoder_is_refused`, `swapchain_image_used_outside_its_acquire_is_refused`, `second_swapchain_is_refused`, `swapchain_extent_with_a_zero_dimension_is_refused`, `resize_with_a_present_pending_is_refused`, `image_of_before_a_resize_is_refused`, `frame_submits_complete_while_a_present_is_pending` | |
| Device loss and messages | `loss_is_returned_and_stays`, `waits_return_at_once_after_a_loss`, `wait_is_woken_by_a_loss`, `teardown_succeeds_after_a_loss`, `clean_run_sends_no_message` | `each_validator_reports_its_canary` |

The swapchain's cases run on null's swapchain, which needs no window. What needs a window or
the present thread is `lactose-platform`'s to test (`swapchain`'s row), and is no case here:
`Acquired::None` on native Metal is shown by `lactose-platform`'s window test with the
drawable made to block, and MoltenVK cannot show it, as it takes the drawable inside the
submit (plan decision 5.6). `tests/tables.rs` holds the table to itself: names distinct, a
rule only on a contract case, every rule with a case that provokes it, the device-run
refusals exactly the 62 named above, and every case named in this file. Every clause of the
rules table's "Every backend" column has a case. A clause of its "Null alone" column with
none here gets its case from `rhi-null`, with the check, as an amendment to list.

## The three conflicts

The native Metal spike found three sentences of the architecture and the decision log that
its result contradicts (Metal, "Against the architecture"). This crate's merge amends the
two of section 12.1 to the side the facade takes, and D158 records that side for all three;
the freeze decision at `vulkan`'s merge keeps or amends it (decision 5.13).

1. **Section 12.1, "residency calls where the backend needs them".** The facade has no
   residency call. A pass declares what it reaches, and a backend that needs residency
   derives its calls from the declaration.
2. **Section 12.1 and D45, "explicitly rather than relying on automatic hazard tracking".**
   The facade exposes no choice of hazard tracking. A declared resource is under the
   backend's tracking on Metal, which is why a declared buffer's state must be exact. An
   image placed in a block is untracked, and the backend fences every use of one, not the
   alias call alone ("States").
3. **D43, indirect command buffers.** They are not in the facade. `draw_indexed_indirect`
   is the draw path on every backend.

## Not in this crate yet

Each is absent because no lane of milestone 2 needs it. Adding one is an amendment of the
kind section 20 allows, a new field, variant or call, and changes what no call above means.

- A backend-neutral constructor trait. Each backend exports its own constructor over
  `DeviceRequest`; null's also takes the `Capabilities` it reports. Who makes the device
  again after a loss is milestone 3's, with the recovery test (verification row 192).
- Blend factors, stencil, multisampling, depth bias, a read-only depth attachment, lines
  and points, three-dimensional and cube images, block-compressed formats.
- Views of part of an image in a heap slot, storage images, and buffers placed in a memory
  block.
- More than one window for a device, present modes other than the surface's default, and
  HDR surface formats.
- Indirect command buffers, mesh shaders and ray tracing.
- A buffer in two read states at once.
- Queries, and labels and markers inside an encoder for a GPU debugger.

## Proof

`tests/consumers.rs` compiles each
consumer's use of the surface, and a backend's implementation of every trait with no
`unsafe`. The static assertions in `src/lib.rs` hold `Device`, `MessageSink` and
`SurfaceSource` object-safe and `Send + Sync`, `Encoder` and `Swapchain` object-safe, `Send`
and not `Sync`, every handle plain data of eight bytes, and the indirect record twenty
bytes. `tests/tables.rs` runs the rule names, the refusal's round trip (wrapped in other
errors too), the queue layout, the indirect record's bytes, the suite table's consistency,
and this file's naming of every case and of the 62 a device runs. The bodies the crate has,
the cooked shader's `encode` and `decode`, `seal_cache` and `open_cache`, and the suite's
contract cases, run in `lactose-rhi-null`'s tests: every contract case under its five
capability sets, the cooked shader's round trip and refusals, and a device started from
sealed, corrupt, truncated, foreign and absent cache bytes. For each rule null keeps, a
build with that rule removed fails one of them with a message; `rhi-null`'s README holds
the table of rule, mutation and failing test.

What neither spike showed and the facade still includes, because section 12.1 or decision
5.6 names it or a consumer cannot do without it: `dispatch_indirect` (both files' "Not
shown"; `vulkan`'s output case is the first evidence); the sampler array beside the image
array in the heap's block (the Metal file's "Not shown" has the review's probe of one block
with two arrays, which became bindings 0 and 1 of set 1 and one argument buffer);
`Load::Keep` (the RHI spike's passes cleared or did not care), `set_viewport` and
`set_scissor` (neither file names them); `Acquired::None` on Metal, which rests on Apple's
documentation (Metal point 17); and the swapchain on Vulkan, which is `swapchain`'s to show.
