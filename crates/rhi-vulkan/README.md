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
  above the heap and sampler table the request asks for, and it must offer host-visible
  memory. A lack is `Unsupported`, naming it. Each optional capability the adapter offers is
  enabled and reported.
- **Queues by role.** The graphics queue is the first of a family that draws and computes.
  Compute has a queue of its own in a family that computes and does not draw, else in another
  family that computes, else as a second queue of the graphics family; with none it shares the
  graphics queue (`QueueLayout`). Present shares the graphics queue until the swapchain gives
  it one. Every queue has a timeline semaphore that each submit to it signals to the submit's
  stamp after its last command; the device reads its value before each check, so what the
  device counts as pending is completion.
- **Memory.** One `VkDeviceMemory` per buffer and per image not placed in a block, of the type
  its class prefers: `Device` a device-local type that is not host-visible where there is one,
  `Upload` a host-visible coherent one, `Readback` host-visible, cached and coherent. Host
  memory is mapped whole for the resource's life; a flush or an invalidate is rounded out to
  `nonCoherentAtomSize`. A memoryless request takes a lazily allocated type where the device
  reports the attachment's kind memoryless, and ordinary memory otherwise.
- **The heap and the sampler table** are one descriptor set (set 1), partially bound and
  updated after bind, with a placeholder image in each empty slot and a placeholder sampler in
  each free entry, so a slot is never unbound. A slot write and a sampler's free are the
  contract's two holds, under the heap's lock.
- **Pipelines** are made from cooked SPIR-V (`CookedShader` version 2), one stage module per
  creation, through one pipeline cache. Before the driver sees a stage, `CookedShader::check`
  holds it to the format's rules and, for a graphics pipeline, `check_varyings` holds the
  fragment stage's reads to the vertex stage's writes: the layer reports nothing for a `uint`
  varying read as an `int` on MoltenVK or KosmicKrisp (the GPU review of `shader-format`), so
  the format's refusal is the only guard. The module's header is checked and its `OpCapability` and `OpExtension` instructions
  are compared with what the device enabled (`capability`). Each given constant is one
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
  a family without `GRAPHICS` gets no graphics stage. A re-take from `Undefined` carries
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
call, and none across a wait or the caller's closure: a host access lends its range under the
records' lock, copies with none held, and clears the range in a second hold.

The drops that wait: an encoder's, for its pending submit, and the last holder of the device's
(the device or an encoder), for the device to idle. A drop while the thread unwinds a panic
does not wait: with work possibly in flight it leaks the device's objects rather than destroy
them under the GPU's use.

## Tests

Every test that makes a device is ignored by a plain run, which may have no driver;
`cargo xtask gpu --driver <name>` runs each ignored test of the `integration` binary in a
process of its own, under the pinned Khronos validation layer, by decision 5.10's rule
(`tools/xtask/README.md`, "GPU runs").

- `tests/conformance.rs`: every case of `lactose-rhi-api`'s suite, one test each, on the
  driver the loader finds. A case of a check null alone makes, a case that needs a device
  loss, a window or a capability the device does not report, is skipped and says why. The
  output cases read their shaders from `$LACTOSE_GPU_SHADERS`, which `cargo xtask gpu` cooks.
- `tests/device.rs`: the device's facts, sent nothing when clean; a device-only buffer filled
  through a staging copy and read back through a copy; the engine's shader,
  `shaders/surface.slang` as cooked, making its two pipelines, not run; a compute stage whose
  workgroup is not its module's, refused; each validator's canary, a misuse whose message
  alone the run must hear (core: an image declared in a state it is not in; synchronisation:
  two copies into one range; synchronisation's load-after-store check: a target stored by one
  render pass and kept by the next with no transition; GPU-assisted: a shader's read past a
  buffer, run by `--gpu-assisted` alone); a cache that fails a check (a corrupt payload, a truncated file,
  a wrong checksum, a stale header) starting empty; and a cache whose directory cannot be
  written, which the run reports and goes past.
- Unit tests in `src/pipeline.rs`: the workgroup a module states for an entry point, a
  `Bool` widened to four bytes and `Int64Atomics` refused unless enabled. In
  `src/convert.rs`: the re-take from `Undefined`, the transfer within a family
  as one layout change, the transfer between families as a release and an acquire of equal
  values, a family without `GRAPHICS`, and two writes in one writing state.

## Records

The runs on the M1 (macOS 27.2.1, Vulkan SDK 1.4.363.1), each by `cargo xtask gpu` with every
message counted. Under `system` the loader's first device is MoltenVK's, and the run is
MoltenVK's; both drivers name the device `Apple M1`, so `--adapter` cannot choose between them.

| Driver | Passed | Skipped | Failed |
|---|---|---|---|
| MoltenVK 1.4.2 | 131 | 44: 30 null alone, 8 no window, 4 no loss, `indirect_count` not reported, compute on its own queue | 0 |
| KosmicKrisp (SDK 1.4.363), a record (decision 5.7) | 126 | 49: 30 null alone, 8 no window, 4 no loss, 6 compute shared, every optional draw capability reported | 0 |
| KosmicKrisp under GPU-assisted validation, a record | 124 | 49, as above | 0 |

GPU-assisted validation does not run under MoltenVK 1.4.2: its instrumented shaders fail in
SPIRV-Cross (a `uint` to `ulong` `reinterpret_cast`). On an Apple GPU the device reports
viewport bounds of at most -16384 to 16383, whatever the driver reports (verification row
183).
