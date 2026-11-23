# lactose-rhi-vulkan

The Vulkan backend of `lactose-rhi-api`, with no window: a device made against the contract's
required set on a Vulkan 1.3 driver with `VK_KHR_dynamic_rendering_local_read`, reporting what
it offers of the optional set, that checks every rule of the contract's "Every backend" column
before the driver call the rule guards. It runs on Linux and Windows drivers, lavapipe in CI,
and on macOS through MoltenVK or KosmicKrisp. Test support: the render crates may run their
tests on a device through it (decision 5.12 of the milestone 2 plan).

| | |
|---|---|
| Tier | backend, `[test_support]` and `[server_denylist]` in `crate-tiers.toml` |
| Determinism boundary | outside; a boundary crate's test may not name it (`cargo xtask boundary`) |
| Unsafe | yes, FFI to the Vulkan driver (architecture section 21); see `SAFETY.md` |

**Status: implemented, without the swapchain.** A request with a surface is refused
`Unsupported`; the swapchain, the present and the mapping of present errors are lane
`swapchain`'s. `crates/rhi-api/README.md` is the contract, and this file states only what the
contract leaves to the backend.

## The public items

| Item | What it is |
|---|---|
| `VulkanDevice::new(DeviceRequest)` | A device on the adapter whose name contains the request's `adapter`, or on the loader's first; its sink takes every warning and error of the validation layer, the loader and the driver through the debug messenger; its pipeline cache starts from the request's bytes, empty, loaded or rejected |

`VulkanDevice` is a `Device`; its encoders are `Box<dyn Encoder>`.

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
  graphics queue (`QueueLayout`). Present shares the graphics queue until the swapchain gives
  it one. The compute family may not draw, so a `Compute` encoder's render pass and depth
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
  stage the driver marks as an application cache hit counts as a hit in
  `pipeline_cache_stats`.
- **The pipeline cache** is sealed with `lactose-rhi-api`'s seal under the vendor, the device
  and the driver's cache UUID. Where the sealed bytes are kept is the caller's. A payload that fails the seal, whose Vulkan header names
  another device, or that the driver refuses, starts the cache empty and `Rejected`.
- **The encoder** owns a command pool and one primary command buffer on its queue's family.
  It records as the caller calls, after every check of the "Every backend" column a command
  can be refused by while recording; the first breach stops the recording, and its submit is
  refused with the breach's rule. A transition is recorded as one `vkCmdPipelineBarrier2` on
  the recording queue, its scopes from the two states (`lactose-rhi-api`'s README, "States");
  its elements are unordered, so a call that names one subresource or one buffer twice is
  refused `state` at the call, before any of it is recorded, by `ImageRange::overlaps` as
  null compares; a family without `GRAPHICS` gets no graphics stage. A re-take from `Undefined` carries
  `ALL_COMMANDS` and `MEMORY_WRITE` as its source scope. Two roles on two queues of one family
  are one layout change on the releasing queue and a semaphore (D160). A local read is a
  by-region barrier from colour attachment output to fragment shader input reads inside the
  render pass, and a render pass that marks attachments for it binds an input attachment set
  from the encoder's own pools.
- **The submit** takes its queue's lock and the records', makes every check a submit can be
  refused by, stamps the submit, gives the records' lock up and calls `vkQueueSubmit2` with
  the caller's waits and signals and the queue's own timeline signal last.

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

The drops that wait: an encoder's, for its pending submit, and the last holder of the device's
(the device or an encoder), for the device to idle. A drop while the thread unwinds a panic
does not wait. An encoder's, with its submit pending, hands its pools to the device, whose
last holder destroys them once the device is idle. The last holder's, with a submit pending,
leaks the device whole, the messengers' sink and the loader included, rather than destroy
anything under the GPU's use; with nothing pending it tears down as any last holder does.

## Tests

Every test that makes a device is ignored by a plain run, which may have no driver;
`cargo xtask gpu` runs each ignored test of the library's `canary` module and of the
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
  other stage, refused; a heap and sampler table past the driver's support refused
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
- Unit tests in `src/pipeline.rs`: the workgroup a module states for an entry point, no size,
  two sizes or the built-in state no workgroup, an entry declared by its name and its stage's
  execution model, a `Bool` widened to four bytes and `Int64Atomics` refused unless enabled.
  In `src/convert.rs`: the re-take from `Undefined`, the transfer within a family as one
  layout change, the transfer between families as a release and an acquire of equal values,
  a family without `GRAPHICS`, and two writes in one writing state. In `src/raw.rs`: compute
  takes a family that does not draw first, and viewport bounds rounded inward to what an Apple
  GPU honours. In `src/records.rs`: a reached point completing its submit and the earlier ones
  of its queue, a failed submit's promise dropped and its point retried, another queue's
  pending signal needing a wait, a wait ordering only the signal it waits for and the earlier
  submits of that signal's queue (the shapes null's test of the same name holds null to), a
  host signal below a promise; the interleavings of a driver
  read and its hold, each fixed in order against a model driver (a host signal or a dropped
  promise between a read and its hold, a pass begun during a slot write, a destroy at once
  after a wait with the queue behind, a failed submit's point signalled from the host, a
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
| MoltenVK 1.4.2, under Metal API and shader validation | 155 | 47: 33 null alone, 8 no window, 4 no loss, `indirect_count` not reported, compute on its own queue | 0 |
| KosmicKrisp (SDK 1.4.363) | 145 | 56: 33 null alone, 8 no window, 4 no loss, 8 compute sharing the graphics queue and 1 compute with no queue of its own, the canary across queues (no second queue), every optional draw capability reported | 0 |
| KosmicKrisp under GPU-assisted validation | 140 | 56: as above, but for the canary across queues, which is not this form's, and the drop that leaks the device while unwinding, which GPU-assisted validation aborts on at exit | 0 |

The counts are of the device tests: the suite's 182 cases, 154 contract and 28 output, and the
device's own; the Metal canary runs under MoltenVK alone. Each driver passes 27 of the 28
output cases: MoltenVK skips the count draw, and KosmicKrisp the ownership transfer, since
compute shares its one queue. Under Metal's validation every case that does not skip prints
both banners; a skipped case is exempt, 14 of MoltenVK's 47 having made a device first.

GPU-assisted validation does not run under MoltenVK 1.4.2: its instrumented shaders fail in
SPIRV-Cross (a `uint` to `ulong` `reinterpret_cast`). On an Apple GPU the device reports
viewport bounds of at most -16384 to 16383, whatever the driver reports (verification row
183).
