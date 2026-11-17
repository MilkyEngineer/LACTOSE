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

**Status: facade.** Every public signature is final for milestone 2's lanes and every body
that decides anything is `todo!()`: the cooked shader's `encode` and `decode`, `seal_cache`
and `open_cache`, and the suite's `run`. The traits have no implementation yet. What runs is
plain data: the rule names and `Rule::refuse` and `Rule::of`, the queue layout, the flag sets
and the indirect record. This file is the contract the backends meet (`rhi-null`, `vulkan`,
`swapchain`, and `rhi-metal` at milestone 3), and every sentence under "Behaviour the
signatures cannot express" is one a dependent crate may rely on. Architecture sections 12.1,
5.1, 19, 20 and 21 are the design, with the milestone 2 plan's decisions 5.6, 5.8, 5.10 to
5.13 and 5.17; where this file and the architecture disagree, the architecture wins and this
file is wrong, except on the three sentences "The three conflicts" names, where this file
takes a side and says so. Decision D158 records the choices section 12.1 left open. The two
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
| `HEAP_SLOTS_DEFAULT`, `OFFSCREEN_TARGET_IMAGES_MIN` | The heap capacity milestone 2's shaders are cooked for, and the least images of an offscreen target |
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
| `tools/cook` (`slang`) | One format to write for every backend, which names the heap's form | `CookedShader::encode`, `HeapUse`, `HEAP_SET`, `HeapForm::FixedArray` |
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

  The RHI spike's list also had runtime descriptor arrays. The heap's form is a fixed array
  (below), which needs none, and the cook refuses a module that declares one (decision 5.8),
  so the facade does not require it. A backend's own platform needs (Vulkan's portability
  subset, Metal 3 and MSL 3.1) stay inside the backend.
- The reported set is `Capabilities`. It is the same value for the device's life. A call
  that needs a field reported `false` is refused (`capability`) by every backend.
  `multi_draw_indirect` gates a count above one in `draw_indexed_indirect`; `indirect_count`
  gates `draw_indexed_indirect_count`; `buffer_atomics_64` gates no call. Native Metal
  reports the first two `false` (Metal point 1) and a caller loops instead.
- `heap_slots` and `samplers` are the request's: a device whose limit is below either is not
  created.

**Queues by role** (RHI point 2, Metal point 2, decision 5.6)

- A queue is named by its role: `Graphics`, `Compute`, `Present`. A device has a graphics
  queue. `Capabilities::queues` says whether the compute role and the present role have a
  queue of their own (`Own`) or are served by the graphics queue (`Shared`), and
  `QueueLayout::serving` resolves a role to the role whose queue serves it. A caller writes
  the same calls on every layout; the layout decides only whether a change of state between
  two roles is an ownership transfer.
- Submits to one queue run in the order the calls return. A backend serialises the calls
  itself (architecture section 3 permits the lock); which of two threads' submits comes
  first is the callers' to decide. Nothing orders work on two queues but a timeline point
  one submit signals and the other waits for.
- On one queue a submit may be held by every submit made before it: one that waits for a
  point holds those behind it until the point is reached. Null holds them so, completing a
  queue's submits strictly in order; a driver may let a later one pass. So a submit never
  waits for a point that only a later submit to its own queue signals: on null, and on any
  driver that holds the queue, neither completes, and no backend detects it.

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
  thread (AppKit's main thread) calls the constructor there. The backend keeps the `Arc`
  until its surface is destroyed, so the window outlives every use of its handles.
- `Device::create_swapchain` on a device made with no window returns `Unsupported`.

**Threads**

- `Device` is `Send + Sync` and every method may be called from any thread at once.
- `Encoder` is `Send` and not `Sync`. Two encoders record on two threads with no lock
  between them: each has its own pool. `Swapchain` is `Send` and not `Sync`.
- A `MessageSink` is called from any thread, a driver's own included.

**Handles and lifetimes**

- A handle is an index and a generation. It means something to the device that issued it
  alone. A handle whose resource is destroyed, or that the device never issued, is refused
  (`destroyed`) by every backend: by the `Device` call it is given to, and, for one
  recorded into an encoder, by that encoder's submit. A backend resolves a handle when the
  command is recorded, so a resource destroyed between a recording and its submit is found
  by null alone.
- No backend defers a destroy. A destroy of something a pending submit uses is refused on
  null (`pending`), and the caller waits for the timeline point that frees it. A device
  backend does not check it, and there it is the driver's undefined behaviour.
- An image is destroyed only after no heap slot holds it, and a block only after no image
  is placed in it (`pending`, null).
- The device lives until the `Device` object, every encoder and the swapchain it made are
  dropped. Dropping a pending encoder, the swapchain with a present pending, or the last
  of them all blocks until the submits concerned complete. A caller that wants a prompt
  shutdown waits for its own timeline points first, and a drop under a submit whose wait
  is never signalled does not return.

**Buffers and host access** (RHI point 3, Metal point 3)

- A buffer's contents are unspecified until written. Its device address is one `u64`, the
  same for the buffer's life, and no byte of another live buffer has an address inside it.
- `MemoryClass::Device` is never touched by the host and is filled by a copy.
  `Upload` is written by the host through `write_buffer` and not read; `Readback` is read by
  the host through `read_buffer` and not written. A call outside the class is refused
  (`usage`) by every backend.
- `write_buffer` and `read_buffer` lend exactly the range asked for, for the length of the
  call, and call their closure once. Calls on ranges of one buffer that do not overlap may
  run at once; a call that overlaps one in progress is refused (`host-overlap`), and a range
  outside the buffer is refused (`bounds`), both by every backend, since a backend's mapped
  memory is reached through them.
- `flush_buffer` after a write and `invalidate_buffer` before a read are always called. They
  do nothing where `coherent_host_memory` is reported (the M1's shared memory: Metal point
  3). On a device that reports it `false`, null refuses a submit that declares, binds or
  copies from a buffer with a written range not flushed since, and a `read_buffer` of a
  range a submit wrote with no invalidate since (`unflushed`).
- A host write is seen by every submit made after its flush returns. The host does not
  write a range a pending submit reads; null cannot see every such read and does not check
  it.

**Images and memoryless requests** (RHI point 4, Metal point 4)

- An image has two dimensions, `mips` mips and `layers` layers, and is made in
  `Undefined`. A format or an extent the device cannot make is `Unsupported`.
- `ImageUsage::TRANSIENT` says nothing needs the contents after the pass that writes them.
  With it the usage holds only `COLOUR`, `DEPTH` and `INPUT`, and `mips` and `layers` are
  one.
- `ImageMemory::Memoryless` is a request, and needs `TRANSIENT`. The device honours it
  exactly when it reports `memoryless_colour` (for a colour attachment) or
  `memoryless_depth` (for a depth attachment); otherwise the image has memory of its own and
  every call on it still works. `Device::image_memoryless` says which happened.
- An image made with a memoryless request is never stored: its attachment's store action is
  `DontCare`, whether the request was honoured or not (`memoryless-store`, null). Metal
  asserts on a store and Vulkan allows one.
- A memoryless image is not used in a pass after that pass's local read where the device
  reports `memoryless_survives_local_read` as `false` (`split`, null). The render graph does
  not ask for memoryless in such a pass.

**Aliased transients** (Metal point 16, item 4)

- A memory block is device memory of a size. An image made `ImageMemory::Placed` takes
  `Device::image_placement`'s `size` bytes of it from its offset, which is a multiple of
  `alignment`; a placement that does not fit is refused (`bounds`, every backend). Two
  placed images alias when their byte ranges in one block overlap. That relation is all a
  caller states: the backend fences it.
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
  | `read_buffer` | `BufferState::HostRead`, with no pending submit writing the buffer |

- A transition's `from` is the state and queue each subresource of its range is in, or the
  transition is refused (`state`). A transition to the state and queue a subresource is
  already in does nothing.
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
  acquires. Between the two the resource is used by nothing. Null refuses an acquire with no
  equal release already submitted, and any use on a queue that does not own the resource
  (`no-qfot`).
- A resource made `Sharing::Concurrent` has no owner: its transitions are recorded once, the
  queue of a transition's `from` is not compared, and two queues may use it at once in one
  state.
- Two queues are ordered by timeline points alone. A submit that changes a resource's state
  or owner while a pending submit of another queue uses the resource waits for a point that
  submit, or a later submit to its queue, signals; null refuses it otherwise (`no-qfot`).
  That covers an acquire, a transition from `Undefined` and every transition of a
  `Concurrent` resource. A submit uses what its commands name: a transition's resource, an
  attachment, a declared buffer, a declared slot's image, a copy's two sides, an index or
  indirect buffer.
- A transition is recorded on an encoder of the role that uses the resource next; a release
  on one of the role that used it last.
- A buffer has one state at a time. A pass cannot both bind a buffer as indices or indirect
  arguments and reach it through an address; no consumer of milestone 2 does.
- Vulkan derives stages, access, layouts and queue families from the two states. Metal has
  no layouts and no ownership: it ignores every transition, and reads only a declared
  buffer's state, as the usage it is made resident with.

**The heap and the sampler table** (RHI point 5, Metal points 5 and 15, decision 5.8)

- A device has one heap of `heap_slots` slots of two-dimensional sampled images, empty at
  creation, and one sampler table of `samplers` entries. The caller allocates slot numbers;
  the device allocates sampler indices. Both are the numbers a shader indexes with, the same
  on every backend: no backend's own handle (an `MTLResourceID`, a descriptor handle) is
  ever a value in this interface.
- `write_heap_slot` puts a whole image, whose usage holds `SAMPLED`, in a slot;
  `clear_heap_slot` empties it. Either is seen by every submit made after it returns. Either
  is refused while a pass of a pending submit declares the slot (`uuwp-used`, null): a slot
  is reused only after the timeline point that frees it. A slot outside the heap is refused
  (`bounds`, every backend).
- A sampler's index is valid from `create_sampler` to `destroy_sampler`. A full table is
  `ErrorKind::ResourceExhausted`. The table needs no declaration and no state. An index is
  freed only after every submit whose shaders may use it has completed, which no backend
  can check.
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
  and `ShaderRead` if none does.
- Null checks what the interface's arguments show: a slot given to `push_slot` or a buffer
  given to `push_address` that the pass did not declare (`undeclared`); at each draw or
  dispatch, a declared slot that is empty or whose image is not in `SampledRead`
  (`heap-layout`), null treating every declared slot as read; a declared buffer in another
  state (`state`). What a shader reaches through data alone, an address or a slot number in
  `push_bytes` or in a buffer, no backend sees: it is Metal shader validation's to report.
- The facade has no residency call. On Metal the declaration is `useResource` on the pass's
  encoder, with read or write usage from the state, which is also what brings the resource
  under Metal's hazard tracking. On Vulkan it is not used beyond the states.

**Render passes and the local read** (RHI point 10, Metal point 10, decision 5.8)

- A pass's attachments have one extent, each names one mip of one layer, no image is named
  twice, and there are at most `colour_attachments` colour attachments (`usage`, null).
- Clip space is x to the right, y up and depth from zero at the near plane to one. The
  framebuffer's origin is its top left. A triangle's front is the side on which its vertices
  run anticlockwise as displayed. Triangles are lists; indices are 32 bits; a pipeline reads
  its vertices through device addresses, and there is no vertex buffer binding. A backend
  whose driver's conventions differ converts (Vulkan flips its viewport).
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
- A bound pipeline's colour and depth formats are the pass's attachments' (`usage`, null).
- `Load::Clear` and `Load::DontCare` need no defined contents; `Load::Keep` does. After
  `Store::DontCare` the contents are undefined.
- A backend opens no span. The caller that records a pass opens one under the pass's
  `label` (architecture section 3).

**Pipelines and the cooked shader format** (RHI point 6, Metal point 6, decision 5.8)

- A cooked shader is one stage for one target, and its bytes are what
  `CookedShader::encode` writes: nothing else writes them and `decode` reads no other
  version. Equal shaders encode to equal bytes on every host. A backend makes a pipeline
  from the targets it runs (`SpirV` on Vulkan, `Msl` or `MetalLibrary` on Metal, none on
  null) and returns `Unsupported` for another.
- The format names the heap's form: `HeapUse` carries `HeapForm::FixedArray`, the set
  (`HEAP_SET`), and each array's kind, binding and capacity. `create_graphics_pipeline` and
  `create_compute_pipeline` return `InvalidData` for a shader whose image array's capacity
  is not the device's `heap_slots` or whose sampler array's is not its `samplers`.
- What the format cannot hold is the cooker's to check on what Slang emitted (decision
  5.8): that the module declares no runtime descriptor array, that every access through the
  heap is marked non-uniform, that each fragment output has one location on both targets,
  and that `inputs` and `outputs` are what the code reads and writes.
- A pipeline has no layout object. Its layout is the heap's block, the fragment stage's
  input attachments (set 0, input `i` at binding `i`) and one push-constant range of the
  largest `push_bytes` among its stages, at most `PUSH_BYTES_MAX`.
- A colour target whose write mask is not empty is one the fragment stage writes (its bit
  of `outputs`) and does not read as an input (its bit of `inputs`), or the pipeline is
  `InvalidData`. A pipeline with no fragment stage writes depth alone, and each of its
  write masks is empty.
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
- An encoder is recording from creation and after each `reset`. A submit that takes it
  makes it pending until the submit completes; `reset` is refused while pending (`pending`,
  every backend). One encoder is one pool: the graph keeps one for each recording job and
  frame in flight.
- A submit runs its encoders in slice order after every point of `waits` is reached, and
  completes when all have run; its `signals` are then reached. A submit takes all of its
  encoders or none: refused, or failed with `DeviceLost`, it leaves them recorded, changes
  no state and signals nothing.
- A refused call changes nothing, on every backend, whichever call it is. A call that
  breaks more than one rule is refused by one of them, unspecified which.
- A command outside its scope, a pass left open at the end of an encoder, and a second
  `begin` inside a pass are refused (`encoding`, null). An encoder of another device or
  another role, and one already pending, are refused by every backend.

**Timelines** (RHI point 11, Metal point 11 and item 9)

- A timeline's value only grows. A point is reached once the value is at least the
  point's. `signal_timeline` and a submit's `signals` each give a value greater than any
  the timeline has reached or been promised by a submit not yet complete (`timeline`, null).
- `wait_timeline` blocks for at most its timeout and returns whether the point was reached.
  A wait may be made before the signal that satisfies it is submitted.
- A fence is a timeline point a submit signals. The facade has no other fence; Metal's
  `MTLFence` is `rhi-metal`'s own means for aliases.

**The swapchain** (Metal point 17 and item 11, decision 5.6)

- `acquire` returns within a bound the backend sets, about one second, and takes no
  timeout: an image, `None`, or `Stale`. One image is acquired at a time (`swapchain`, every
  backend). An acquired image is `Undefined` on the present role's queue and is a colour
  attachment and nothing else. Its handle is the caller's to use from the acquire to its
  present and at no other time (`swapchain`, null); a later acquire may return the same
  handle or another.
- `present` is a submit and a present in one call: the submit's encoders are of the
  `Present` role, it leaves the image in `ImageState::Present`, and its waits and signals
  are a submit's. The backend owns whatever its driver needs between the two (one present
  semaphore for each image, on Vulkan).
- The number of presents in flight is the caller's to bound, by the points its presents
  signal. No backend bounds it (Metal item 11).
- After `Acquired::Stale` or `Presented::Stale` the caller waits for its presents and calls
  `resize`, which is refused while one is pending (`pending`, every backend); every handle
  the swapchain gave before it is destroyed (`destroyed`).

**Device loss** (architecture section 19, verification row 192)

- A loss is a value. Any call that reaches the device may return an error of kind
  `ErrorKind::DeviceLost`; from the first that does, `Device::is_lost` is true and stays
  true, and every later call that reaches the device returns `DeviceLost` too.
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
- A message is not an error: it changes no call's result. A rule a backend checks itself is
  returned from the call and is not also sent.
- What reaches the process's standard error alone, Metal's API and shader validation, is
  read there by the command that runs the tests. Metal API validation aborts the process:
  a device test runs in a process of its own.

**The rules**

A call that breaks a rule a backend checks returns `Rule::refuse`'s error: kind
`InvalidData`, a message that begins with the rule's name, and a `Refusal` as its source,
which `Rule::of` reads. Null checks every rule. A device backend checks the ones its own
code cannot run without, the "Every backend" column: whatever a caller passes, no call
reaches host memory, another device's object or a freed pool through the backend's own code.
The rest it hands to its driver unchecked, and what a driver does with a broken rule is
undefined: wrong pixels, a lost device, or a fault inside the driver. The functions are safe
to call all the same, because their callers are crates that forbid `unsafe` (architecture
section 21). What holds a caller to the rules is null, where every rule is checked and every
consumer's tests run, and on a device the validators of decision 5.10.

| Rule | Name | Every backend | Null alone |
|---|---|---|---|
| `State` | `state` | | a transition's `from`, a use's state, by subresource |
| `Queue` | `no-qfot` | | a use off the owning queue; an acquire with no release; a change of state or owner under another queue's pending submit with no wait for it; an alias call across queues |
| `Destroyed` | `destroyed` | a stale or foreign handle, at the `Device` call it is given to or at the submit of the encoder it was recorded into | a resource destroyed between a recording that names it and that encoder's submit |
| `Pending` | `pending` | `Encoder::reset` while pending; `resize` with a present pending | a destroy under a pending submit; of an image a slot holds; of a block an image is placed in |
| `Capability` | `capability` | every call that needs an unreported capability | |
| `Usage` | `usage` | host access outside the memory class; `buffer_address` without `DEVICE_ADDRESS`; a descriptor no device could honour | every other use outside a usage; a pass's attachments; a pipeline's formats |
| `Bounds` | `bounds` | host ranges; heap slots; placements; push-constant offsets | copy regions; index and indirect offsets |
| `Undeclared` | `undeclared` | | `push_slot`, `push_address` |
| `Unflushed` | `unflushed` | | a written range at a submit; a read with no invalidate |
| `HostOverlap` | `host-overlap` | overlapping host access | |
| `Local` | `local` | | an input read with no local read since the write |
| `LocalMap` | `local-map` | | an input with no attachment of its index marked |
| `Split` | `split` | | a memoryless attachment after a local read, where lost |
| `MemorylessStore` | `memoryless-store` | | a store of a memoryless attachment |
| `HeapLayout` | `heap-layout` | | a declared slot empty or not in `SampledRead` |
| `HeapWrite` | `uuwp-used` | | a slot written under a pending declaration |
| `Alias` | `no-fence` | | a placed image used with no alias call since its bytes were another's |
| `UndefinedRead` | `undefined-read` | | a read of undefined contents |
| `PresentQueue` | `present-queue` | `Device::submit` to `Present` | a swapchain image on another role's encoder |
| `Swapchain` | `swapchain` | a second acquire; a present of another image; a second swapchain | a swapchain image used outside its acquire and its present |
| `Timeline` | `timeline` | | a value that does not grow |
| `Encoding` | `encoding` | a foreign, wrong-role or pending encoder in a submit | commands outside their scope; a pass left open |

## How each call is honoured

Every call is one the null backend honours by the state it keeps, and one native Metal
honours by a call the Metal spike made. The Vulkan column is the RHI spike's.

| Calls | Null keeps | Native Metal | Vulkan | Evidence |
|---|---|---|---|---|
| The constructor against the two sets; `capabilities`, `info` | The set it is created with | Argument buffers tier 2, Metal 3, an Apple family, or `Unsupported`; reports no multi-draw and no indirect count | The features of the required table, or `Unsupported` | RHI item 1, point 1; Metal "The device", point 1 |
| Roles and `QueueLayout` | The layout of its set, one queue or several | A command queue for each role; no families, no ownership | A second family that draws and presents (MoltenVK has four), or one queue (lavapipe) | RHI item 1, point 2; Metal item 11, point 2 |
| `create_buffer`, `destroy_buffer`, `buffer_address` | A table by handle; addresses it invents, distinct | `makeBuffer`, `gpuAddress` | A buffer with `SHADER_DEVICE_ADDRESS` | RHI item 2, point 3; Metal item 2, point 3 |
| `write_buffer`, `read_buffer`, `flush_buffer`, `invalidate_buffer` | A byte store behind a lock, lent for the call; written and flushed ranges under a set that is not coherent | `contents()` of shared memory; flush and invalidate do nothing | Persistently mapped memory; `vkFlushMappedMemoryRanges`, `vkInvalidateMappedMemoryRanges` | RHI point 3, question 8; Metal point 3 |
| `create_image`, `destroy_image`, `image_memoryless` | Usage, state for each subresource, the request and whether its set honours it | `makeTexture`; `.memoryless` storage for colour and depth | An image; lazily allocated memory where offered | RHI item 7, point 4; Metal item 3, point 4 |
| `create_memory_block`, `image_placement`, `ImageMemory::Placed`, `Encoder::alias` | Byte ranges for each block; which image last used each; refuses `no-fence` | An untracked placement heap; `heapTextureSizeAndAlign`; a fence updated after the last use and waited on before the first | Bound memory at an offset; a barrier from undefined | Metal item 4, point 16 |
| `write_heap_slot`, `clear_heap_slot` | The image for each slot; the declarations of pending submits; refuses `uuwp-used` | A write of the texture's resource id into the argument buffer | An update-after-bind descriptor write | RHI item 3, point 5; Metal item 1, point 5 |
| `create_sampler`, `destroy_sampler` | A table of descriptors | A sampler with `supportArgumentBuffers`, its id in the table's argument buffer; at most `maxArgumentBufferSamplerCount` | A sampler in the table's descriptor array | Metal item 10, point 15 |
| `CookedShader`, `create_graphics_pipeline`, `create_compute_pipeline`, the destroys | The cooked fields: stages, constants' lengths, inputs, outputs, heap capacity. It runs no code | A library from MSL 3.1 or a metallib; functions always specialised; a pipeline state | Shader modules with specialisation info | RHI item 4, point 6; Metal item 6, point 6 |
| `pipeline_cache`, the request's bytes, `pipeline_stats`, `seal_cache`, `open_cache` | A payload of its own under the same seal | A binary archive loaded from a private copy of the opened payload; fail-on-miss; never under shader validation | `VkPipelineCache` data; creation feedback | RHI item 5, point 7; Metal item 7, point 7 |
| `create_encoder`, `Encoder::reset`, recording on a thread | A command list and whether it is pending | A command buffer made on first use; `reset` drops it | A command pool and one command buffer; `vkResetCommandPool` | RHI item 6, point 8; Metal item 8, point 8 |
| `transition_images`, `transition_buffers` | The state and owner of each subresource, applied at the submit in order | Nothing | Barriers of synchronisation 2; a release and an acquire across families | RHI item 7, point 9; Metal point 9 |
| `begin_render_pass`, `begin_compute_pass`, `end_pass`, `PassReach` | The attachments, what is declared, what was written since the last local read | A render or compute command encoder; `useResource` for each declared slot's texture and each declared buffer | `vkCmdBeginRendering` with the input mapping; the declaration unused | RHI item 7, points 5 and 10; Metal items 2 and 3, point 5 |
| `local_read` | Clears "written since"; refuses `local`, `split` | Nothing: framebuffer fetch reads the tile | The local-read barrier (which MoltenVK turns into a pass split) | RHI item 7, point 10; Metal item 3, point 10 |
| `bind_*`, `set_viewport`, `set_scissor`, `push_bytes`, `push_address`, `push_slot` | The bound pipeline; typed pushes against the declaration | State on the encoder; the block as bytes set for each stage | Dynamic state; `vkCmdPushConstants` | RHI items 2 to 4; Metal items 1 and 2 |
| `draw`, `draw_indexed`, `draw_indexed_indirect`, `draw_indexed_indirect_count` | Checks states, declarations and capabilities; draws nothing | `drawPrimitives`, `drawIndexedPrimitives`, one indirect record a call; no count form | The draw calls; multi-draw and indirect count where reported | RHI item 9, point 12; Metal item 5, point 12 |
| `dispatch`, `dispatch_indirect` | Checks; runs nothing | `dispatchThreadgroups`, and its indirect form | `vkCmdDispatch`, `vkCmdDispatchIndirect` | `dispatch`: RHI items 2 and 8, Metal items 2 and 9. `dispatch_indirect`: architecture 12.1 alone; both files list it under "Not shown" (see "Proof") |
| `copy_buffer`, `copy_buffer_to_image`, `copy_image_to_buffer` | Buffer to buffer moves bytes in its stores; the image forms are checked and move nothing | Blit encoder copies | The copy commands | RHI item 10, point 13; Metal item 12, point 13 |
| `create_timeline`, `timeline_value`, `signal_timeline`, `wait_timeline` | A counter and a condition variable | `MTLSharedEvent`: `signaledValue`, a listener, a wait with a timeout | A timeline semaphore | RHI item 8, point 11; Metal item 9, point 11 |
| `submit` with waits and signals | Checks every command in order, applies the states, completes when its waits are reached | Command buffers enqueued in slice order, `encodeWaitForEvent` and `encodeSignalEvent` | `vkQueueSubmit2` with timeline semaphores | RHI items 6 and 8; Metal items 8 and 9 |
| `create_swapchain`, `acquire`, `present`, `resize` | Images with no window; an acquire that can be made to return `None` | A `CAMetalLayer`; `nextDrawable`, which has its own one-second limit; `present(drawable)` before the commit | A swapchain; an acquire with a finite timeout; `vkQueuePresentKHR` after the submit | Metal item 11, point 17; decision 5.6 |
| `is_lost`, `DeviceLost` | A flag its own call sets | A command buffer's error status | `VK_ERROR_DEVICE_LOST` | Architecture 19; verification row 192 |
| `MessageSink` | Never called | The driver's log where it has one; validators write to standard error | The debug messenger | RHI "Validation runs", point 14; Metal "Validation runs", point 14 |

## The conformance suite

`conformance::CASES` lists every case by name, each a *contract* case or an *output* case,
and `conformance::run` runs one against a `Harness`. The bodies arrive with the backends:
the contract cases' with `rhi-null`, the output cases' and their Slang sources with `vulkan`.
A backend's tests carry one test for each case, named after it.

- A **contract** case asserts what the contract says of calls and results. It needs no
  shader to run and no pixel, so null passes it as a device does. A case that names a rule
  (`Case::refuses`) fails unless the call is refused by exactly that rule: a backend built
  without the rule fails it, which is the mutation proof the plan asks of null.
- `Case::devices` says whether a device backend runs a case. It is `false` for the cases
  that provoke a check null alone makes: on a device the same calls are the driver's
  undefined behaviour and prove nothing. The nineteen refusal cases a device backend also
  runs are those of the rules table's "Every backend" column, one or more for each of its
  clauses: `unreported_capability_is_refused`, `submit_to_the_present_role_is_refused`,
  `overlapping_host_access_is_refused`, `host_range_outside_the_buffer_is_refused`,
  `host_access_outside_the_memory_class_is_refused`,
  `address_of_a_buffer_without_the_usage_is_refused`,
  `memoryless_request_without_transient_is_refused`,
  `placement_outside_the_block_is_refused`, `heap_slot_outside_the_heap_is_refused`,
  `encoder_reset_while_pending_is_refused`, `encoder_in_two_submits_is_refused`,
  `encoder_of_another_role_or_device_is_refused`, `push_outside_the_block_is_refused`,
  `use_after_destruction_is_refused`, `second_acquire_before_a_present_is_refused`,
  `present_of_an_image_not_acquired_is_refused`, `second_swapchain_is_refused`,
  `resize_with_a_present_pending_is_refused` and `image_of_before_a_resize_is_refused`.
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

| Area | Contract cases | Output cases |
|---|---|---|
| Capabilities and queues | `capabilities_are_fixed`, `unreported_capability_is_refused`, `roles_resolve_as_the_layout_reports`, `submit_to_the_present_role_is_refused` | |
| Buffers and host access | `buffer_write_then_copy_then_read_returns_the_bytes`, `buffer_address_is_stable_and_distinct`, `host_ranges_that_do_not_overlap_are_written_at_once`, `overlapping_host_access_is_refused`, `host_range_outside_the_buffer_is_refused`, `host_access_outside_the_memory_class_is_refused`, `address_of_a_buffer_without_the_usage_is_refused`, `unflushed_range_at_a_submit_is_refused`, `flush_and_invalidate_are_accepted_where_memory_is_coherent`, `buffer_use_outside_its_usage_is_refused` | `device_memory_is_filled_through_a_staging_copy` |
| Images, memoryless, aliases | `image_use_outside_its_usage_is_refused`, `memoryless_outcome_is_queryable_and_as_reported`, `memoryless_request_without_transient_is_refused`, `memoryless_attachment_stored_is_refused`, `memoryless_attachment_after_a_local_read_is_refused_where_lost`, `placement_outside_the_block_is_refused`, `aliased_image_used_without_the_alias_call_is_refused`, `alias_read_before_its_first_write_is_refused`, `aliases_used_in_turn_with_the_alias_call_pass` | `memoryless_g_buffer_is_lit_in_its_pass`, `aliased_transients_render_in_turn`, `image_round_trips_through_copies` |
| States and queues | `transition_from_another_state_is_refused`, `use_in_another_state_is_refused`, `state_is_kept_for_each_mip_and_layer`, `state_follows_submission_order_not_recording_order`, `cross_queue_use_without_a_transfer_is_refused`, `acquire_without_a_release_is_refused`, `release_and_acquire_cross_a_resource`, `shared_and_discarded_resources_cross_without_a_transfer`, `roles_on_one_queue_need_no_transfer`, `image_taken_while_another_queue_reads_it_is_refused` | `ownership_transfer_keeps_the_contents` |
| The heap, samplers, declarations | `heap_slot_outside_the_heap_is_refused`, `declared_slot_not_in_sampled_read_is_refused`, `heap_write_under_a_pending_reader_is_refused`, `heap_write_after_its_reader_completes_passes`, `undeclared_slot_argument_is_refused`, `undeclared_address_argument_is_refused`, `declared_buffer_in_another_state_is_refused`, `sampler_table_is_bounded` | `heap_is_sampled_past_slot_zero`, `address_is_read_in_a_shader`, `declared_reader_follows_a_lagging_writer`, `sampler_filters_as_described` |
| Cooked shaders, pipelines, the cache | `cooked_shader_round_trips`, `cooked_shader_of_another_version_is_refused`, `cooked_shader_corrupt_or_truncated_is_refused`, `shader_for_another_heap_is_refused`, `shader_target_the_backend_does_not_run_is_unsupported`, `constant_of_another_length_is_refused`, `sealed_cache_round_trips`, `cache_corrupt_truncated_or_foreign_starts_empty` | `specialised_constant_changes_the_output`, `write_mask_keeps_a_channel`, `depth_state_orders_two_triangles`, `warm_cache_hits_every_stage` |
| Encoders, submits, timelines | `encoder_recorded_on_another_thread_submits`, `encoder_reset_while_pending_is_refused`, `encoder_in_two_submits_is_refused`, `encoder_of_another_role_or_device_is_refused`, `command_outside_its_scope_is_refused`, `push_outside_the_block_is_refused`, `refused_submit_changes_nothing`, `submit_is_pending_until_its_wait_is_reached`, `timeline_is_signalled_waited_and_read_by_the_host`, `timeline_wait_times_out`, `timeline_value_that_does_not_grow_is_refused`, `destroy_under_a_pending_submit_is_refused`, `use_after_destruction_is_refused` | `two_threads_record_at_once` |
| Render passes and the local read | `input_read_without_a_local_read_is_refused`, `input_read_of_another_attachment_is_refused`, `attachments_of_unequal_extent_are_refused` | `triangle_matches_the_reference`, `local_read_sees_what_the_pass_wrote`, `clear_and_keep_load_as_described` |
| Indirect draws and dispatches | | `indexed_indirect_draw_matches_the_reference`, `indirect_count_draws_where_reported`, `dispatch_writes_through_an_address`, `dispatch_indirect_reads_its_counts` |
| The swapchain and the present role | `second_acquire_before_a_present_is_refused`, `present_of_an_image_not_acquired_is_refused`, `present_of_an_image_not_in_present_is_refused`, `swapchain_image_on_a_frame_encoder_is_refused`, `swapchain_image_used_outside_its_acquire_is_refused`, `second_swapchain_is_refused`, `resize_with_a_present_pending_is_refused`, `image_of_before_a_resize_is_refused`, `frame_submits_complete_while_a_present_is_pending` | |
| Device loss and messages | `loss_is_returned_and_stays`, `waits_return_at_once_after_a_loss`, `teardown_succeeds_after_a_loss`, `clean_run_sends_no_message` | `each_validator_reports_its_canary` |

The swapchain's cases run on null's swapchain, which needs no window. What needs a window or
the present thread is `lactose-platform`'s to test (`swapchain`'s row), and is no case here.
`tests/tables.rs` holds the table to itself: names distinct, a rule only on a contract case,
and every rule with a case that provokes it. Every clause of the rules table's "Every
backend" column has a case. A clause of its "Null alone" column with none here gets its case
from `rhi-null`, with the check, as an amendment to list.

## The three conflicts

The native Metal spike found three sentences of the architecture and the decision log that
its result contradicts (Metal, "Against the architecture"). The facade takes a side on each,
for now; the freeze decision at `vulkan`'s merge keeps or amends it (decision 5.13).

1. **Section 12.1, "residency calls where the backend needs them".** The facade has no
   residency call. A pass declares what it reaches, and a backend that needs residency
   derives its calls from the declaration.
2. **Section 12.1 and D45, "explicitly rather than relying on automatic hazard tracking".**
   The facade exposes no choice of hazard tracking. A declared resource is under the
   backend's tracking on Metal, which is why a declared buffer's state must be exact; only
   aliased memory is handled explicitly, through the alias call.
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

There is nothing to run against a facade but its tables. `tests/consumers.rs` compiles each
consumer's use of the surface, and a backend's implementation of every trait with no
`unsafe`. The static assertions in `src/lib.rs` hold `Device`, `MessageSink` and
`SurfaceSource` object-safe and `Send + Sync`, `Encoder` and `Swapchain` object-safe, `Send`
and not `Sync`, every handle plain data of eight bytes, and the indirect record twenty bytes.
`tests/tables.rs` runs the rule names, the refusal's round trip, the queue layout, the
indirect record's bytes and the suite table's consistency.

What neither spike showed and the facade still includes, because section 12.1 or decision
5.6 names it or a consumer cannot do without it: `dispatch_indirect` (both files' "Not
shown"; `vulkan`'s output case is the first evidence); the sampler array beside the image
array in the heap's block (the Metal file's "Not shown" has the review's probe of one block
with two arrays, which became bindings 0 and 1 of set 1 and one argument buffer);
`Load::Keep` (the RHI spike's passes cleared or did not care), `set_viewport` and
`set_scissor` (neither file names them); `Acquired::None` on Metal, which rests on Apple's
documentation (Metal point 17); and the swapchain on Vulkan, which is `swapchain`'s to show.
