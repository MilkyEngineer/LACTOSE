# RHI spike, Apple M1 under MoltenVK, 2026-11-15

Milestone 1's RHI spike (plan decision 5.10, D139): a throwaway headless `ash` program that walks
decision 5.10's ten items of architecture section 12.1's list under MoltenVK alone. The program is
not in the repository; its hash is below. The conformant reference run, the same items under
lavapipe, is milestone 2's.

## Answer

- **Every item of the decision's list ran with a GPU-side use read back, exact.** One frame, two
  queues, a device-address dispatch, a heap indexed at slots 5, 17, 300 and 1023 with a
  non-uniform index, a specialised pipeline, a pipeline cache reloaded in a second process with
  every stage reporting a cache hit, two encoders recorded on overlapping threads, a transient
  memoryless depth attachment, a render-pass-local read, a timeline wait-before-signal across
  queues, and an indexed indirect draw. Clean under the Khronos layer with synchronisation and
  thread-safety validation, under Metal API validation and under Metal shader validation.
- **Four kinds of undefined behaviour, and one MoltenVK defect on legal Vulkan, pass every
  validator here,** so `rhi-null` must check them: a missing local-read barrier, a memoryless
  attachment across a local-read barrier on MoltenVK, an ownership transfer dropped whole, a heap
  image in the wrong layout, and a heap slot the pending draw reads rewritten. A missing
  `nonuniformEXT` is invisible as well, to null too.
- **Four "no"s on the list, all MoltenVK's.** MoltenVK 1.4.2 offers no lazily allocated memory
  to an image with input-attachment usage, so a memoryless G-buffer read in the pass cannot
  exist. Its local read is not in tile memory: the in-pass barrier ends the Metal render pass
  with stores and restarts it with loads. A memoryless depth attachment loses its contents across
  that restart, which is legal Vulkan producing wrong pixels. `drawIndirectCount` is not
  supported, as D42 already expects.
- **A fifth, MoltenVK's with the layer: GPU-assisted validation does not run under MoltenVK
  1.4.2.** Its instrumented shaders fail MoltenVK's MSL compilation and the layer falls back to
  the uninstrumented pipelines. Metal shader validation is what reported the heap and address
  misuses here.
- **What it means for `rhi-api`:** the list under "What `rhi-api` must expose". The local read is
  its own call, not a general barrier; memoryless is a request the backend may decline per usage;
  the pipeline cache blob is wrapped with a length and checksum, because MoltenVK fails cache
  creation on a corrupt payload behind a valid header. What the architecture requires and this
  run could not show (residency, aliased transients, heaps, indirect command buffers, samplers,
  Metal libraries and binary archives) is listed there too.

## Run header

- Machine: Apple M1 (4 performance, 4 efficiency cores, 16 GiB), macOS 14.7.4 (23H420).
- Commit: `437fcf6` (clean). The program lives outside the repository and its workspace.
- Driver: MoltenVK 1.4.2, "supporting Vulkan version 1.4.357"; the device reports driver name
  "MoltenVK", driver info "1.4.2", conformance version 1.4.4.0, driver ID `MOLTENVK`, vendor
  `0x106b`, device `0xe070207`. The instance requests API 1.3; the device reports 1.3.357.
- Vulkan SDK: LunarG 1.4.363.1 at `$HOME/VulkanSDK/1.4.363.1`; validation layer 1.4.363.
- `MVK_CONFIG_` variables: none set, so MoltenVK's defaults apply
  (`MVK_CONFIG_PREFILL_METAL_COMMAND_BUFFERS` 0, `MVK_CONFIG_LOG_LEVEL` 3).
- Toolchain: rustc 1.99.0, `dev` profile with `panic = "abort"`. Dependencies: `ash = "=0.38.0"`
  (0.38.0+1.3.281), which locks `libloading` 0.8.9, `cfg-if` 1.0.5 and `windows-link` 0.2.1. No
  allocator crate: memory is allocated and sub-allocated by hand.
- Shaders: GLSL compiled ahead of the run by the SDK's `glslc` (shaderc v2026.4, glslang
  11.1.0-1566, SPIRV-Tools v2026.4), `glslc --target-env=vulkan1.3 -O <name> -o <name>.spv`; the
  program loads the `.spv` files at run time. The six `.spv` files reproduce byte for byte.
- Environment, set by hand instead of `setup-env.sh`. Under `setup-env.sh` (with
  `VK_DRIVER_FILES` pinned to MoltenVK) the loader finds a second, system-wide copy of the layers
  and reports seven "Removing layer ... because it is a duplicate" warnings, which the program's
  strict message sink counts: "0 failed checks, 0 validation errors, 7 validation warnings",
  exit 1. The hand-set environment:
  `VULKAN_SDK=$HOME/VulkanSDK/1.4.363.1/macOS`, `DYLD_LIBRARY_PATH=$VULKAN_SDK/lib`,
  `VK_LAYER_PATH=$VULKAN_SDK/share/vulkan/explicit_layer.d`,
  `VK_DRIVER_FILES=$VULKAN_SDK/share/vulkan/icd.d/MoltenVK_icd.json`; `VK_ICD_FILENAMES`,
  `VK_ADD_LAYER_PATH`, `VK_INSTANCE_LAYERS` and `VK_LOADER_LAYERS_ENABLE` unset.
- Build: `cd <program> && CARGO_TARGET_DIR=<scratch>/target-spike-rhi cargo build`.
- Run: `<scratch>/target-spike-rhi/debug/spike-rhi <program>/shaders <cache-file> [--reload]
  [--validate] [--misuse <what>]`, once without `--reload` (empty cache, saved to the file), then
  once with it. `--validate` enables `VK_LAYER_KHRONOS_validation`, configured by the environment
  below. The exit status is 0 only when every check passed and the debug messenger saw no warning
  and no error, from the layer or from MoltenVK.
- Date: the first runs were on 2026-11-15 (UTC), which names this file. The final run set, every
  number below, ran on the final binary from 23:59 UTC on 2026-11-15 to 00:00 UTC on 2026-11-16.
- Load averages (1, 5, 15 min): 2.47 4.35 6.75 before the final run set, 2.43 4.22 6.64 after.
  Nothing here is a timing claim.

## The ten items

"Yes" means a GPU-side use whose result was read back and compared exactly. "Reported" means a
query returned true. "State" is what the caller carried between calls, which is what a null
backend must keep. "External synchronisation" is what the program had to order itself.

What MoltenVK reports, for all items: four queue families, each `GRAPHICS | COMPUTE | TRANSFER`
with one queue and `timestampValidBits` 64. Three memory types on one 16 GiB heap: 0
`DEVICE_LOCAL`; 1 `DEVICE_LOCAL | HOST_VISIBLE | HOST_COHERENT | HOST_CACHED`; 2
`DEVICE_LOCAL | LAZILY_ALLOCATED`. Limits read: `nonCoherentAtomSize` 16,
`bufferImageGranularity` 16, `maxPushConstantsSize` 4096, `timestampPeriod` 1,
`maxPerStageDescriptorUpdateAfterBindSampledImages` and
`maxDescriptorSetUpdateAfterBindSampledImages` 1,000,000. `VK_KHR_portability_subset` features:
`constantAlphaColorBlendFactors` 1, `events` 1, `imageViewFormatReinterpretation` 1,
`imageViewFormatSwizzle` 1, `imageView2DOn3DImage` 1, `multisampleArrayImage` 1,
`mutableComparisonSamplers` 1, `pointPolygons` 0, `samplerMipLodBias` 0, `separateStencilMaskRef`
1, `shaderSampleRateInterpolationFunctions` 1, `tessellationIsolines` 0, `tessellationPointMode`
0, `triangleFans` 1, `vertexAttributeAccessBeyondStride` 1; property
`minVertexInputBindingStrideAlignment` 1. None of the zeros touches the list. MoltenVK's own log:
"Vulkan semaphores using MTLEvent", "Descriptor sets binding resources using Metal3 argument
buffers", and nothing it says it emulates or drops for this program.

### 1. A device and its queues: yes

- Used: a Vulkan 1.3 instance with `VK_KHR_portability_enumeration` and its flag, and
  `VK_EXT_debug_utils`; device extensions `VK_KHR_portability_subset` and
  `VK_KHR_dynamic_rendering_local_read`. Features enabled, each queried first: Vulkan 1.2
  `bufferDeviceAddress`, `descriptorIndexing`, `runtimeDescriptorArray`,
  `descriptorBindingPartiallyBound`, `descriptorBindingSampledImageUpdateAfterBind`,
  `descriptorBindingUpdateUnusedWhilePending`, `shaderSampledImageArrayNonUniformIndexing`,
  `timelineSemaphore`; Vulkan 1.3 `synchronization2`, `dynamicRendering`;
  `dynamicRenderingLocalRead`; core `independentBlend`.
- MoltenVK reports every one of them. Queue A is family 0 and queue B family 1.
- Shown: both queues ran work that was read back (items 2 and 8 on both, the frame on A).
- State: the enabled feature set, so a use of a feature not enabled is refused; each queue's
  family and role.
- External synchronisation: each `VkQueue` is submitted to from one thread.

### 2. A buffer with a device address that a shader dereferences: yes

- Used: `bufferDeviceAddress`; one host-visible allocation with
  `MemoryAllocateFlags::DEVICE_ADDRESS`, sub-allocated by hand at each buffer's alignment and
  persistently mapped; buffers with `SHADER_DEVICE_ADDRESS` usage; GLSL `GL_EXT_buffer_reference`.
  A compute shader on queue B gets two addresses and a count in 20 bytes of push constants and
  writes `dst[i] = src[i] * 2654435761 + i` through them.
- MoltenVK reports `bufferDeviceAddress` 1. The allocation landed in type 1, which is coherent;
  under MoltenVK every host-visible type is.
- Shown: 256 of 256 values, copied to a third buffer through queue A, equal the CPU reference.
- State: each buffer's address and range, its usage and its liveness; the arena's offsets.
- External synchronisation: host writes before the submit; the host reads only after the timeline
  wait for the copy's value and a `COPY` to `HOST` barrier. The program flushes and invalidates
  ranges rounded to `nonCoherentAtomSize` when memory is not coherent; on this device that path
  never ran.
- Seen on the way: MoltenVK gives each `VkBuffer` its own `MTLBuffer`. Metal reports the length of
  the 1024-byte sub-allocation, not of the allocation. A dereference outside the `VkBuffer` is
  outside any buffer Metal knows, which is how Metal shader validation caught the address misuse.

### 3. One bindless heap, indexed past slot 0 with a non-uniform index: yes

- Used: one set layout, binding 0, `SAMPLED_IMAGE` × 1024, with `PARTIALLY_BOUND`,
  `UPDATE_AFTER_BIND` and `UPDATE_UNUSED_WHILE_PENDING`, the `UPDATE_AFTER_BIND_POOL` layout flag
  and an update-after-bind pool. GLSL `uniform utexture2D heap[]` read with `texelFetch` and
  `nonuniformEXT`, so no sampler. Four 1×1 `R8G8B8A8_UINT` images.
- MoltenVK reports every feature named in item 1 and an update-after-bind limit of 1,000,000.
- Shown: the fragment shader picks the slot by the pixel's band, `(x / 8) % 4`, among slots 5, 17,
  300 and 1023, so the index varies inside one draw. Every drawn pixel of each band read its
  texel; drawn pixels per band 160, 471, 349 and 166. Slots 5, 17 and 300 were written before
  recording; slot 1023 was written on the recording thread after the set was bound, which
  update-after-bind allows until submission.
- Shown, update-unused-while-pending: the frame's submit waits on a timeline value only the host
  signals. While it is pending (its fence not signalled, checked), the host writes slot 600, which
  no draw reads, then signals; the pending frame's readback stays exact. Without the
  `UPDATE_UNUSED_WHILE_PENDING` flag (`uuwp-noflag`) the same write is silent under every
  validator, so the run does not show that the flag is what makes it legal. The binding keeps
  `UPDATE_AFTER_BIND`, and `VUID-vkUpdateDescriptorSets-None-03047` binds only a binding with
  neither flag, so the layer has no rule to report here. The flag's role rests on the
  specification's definition of update-unused-while-pending, not on this run. Slot 600 is not
  read afterwards.
- Not shown: that `nonuniformEXT` is present where it is needed. With the decoration removed and
  `shaderSampledImageArrayDynamicIndexing` enabled (`uniform`), the readback is exact and every
  validator is silent. Leaving it out is undefined behaviour that works on this GPU; a desktop GPU
  may show it, and nothing here can, the null backend included. The cooker is where it can be
  checked: every heap index carries the `NonUniform` decoration (Slang's
  `NonUniformResourceIndex`).
- State: the slot table (which slots hold which image, written when, freed when) and the set's
  binding. A null backend cannot see a shader's index, so it checks writes, the image state of
  each slot a pass declares, and lifetimes.
- Undefined behaviour no validator here reports: the heap images left in
  `TRANSFER_DST_OPTIMAL` while their descriptors say `SHADER_READ_ONLY_OPTIMAL` (`heap-layout`)
  read back exact; slot 5, which the pending draw reads, rewritten while the frame is pending
  (`uuwp-used`) gives 165 wrong pixels. Both are silent under the layer with synchronisation and
  thread-safety validation, Metal API validation and Metal shader validation.
- External synchronisation: one thread at a time on the set and the pool; the slot writes before
  the submit and, for slot 600, while it is pending; the images' upload ordered before the draw by
  a barrier to `SHADER_READ_ONLY_OPTIMAL` at `FRAGMENT_SHADER`. Synchronisation validation does
  not track descriptors a shader indexes, so its clean run says nothing about the heap.
- Residency: not shown. The program makes no residency call, Vulkan has none, and MoltenVK handles
  its argument buffers' residency itself.

### 4. A pipeline from precompiled SPIR-V with a specialisation constant: yes

- Used: `VkShaderModule` from the `.spv` files; `VkSpecializationInfo` setting constant 0 (`SALT`,
  default 0) to `0x5A` and constant 1 to 0. Graphics pipelines use dynamic rendering
  (`VkPipelineRenderingCreateInfo`, two colour formats and a depth format).
- MoltenVK translates the SPIR-V to MSL at pipeline creation.
- Shown: every drawn pixel is its texel XOR `0x5A`; none of the 1146 matches the unspecialised
  value.
- State: the module until the pipelines exist (the program destroys the modules after creation),
  the pipeline layout and the specialisation data.
- External synchronisation: none.

### 5. A pipeline cache saved and reloaded in a second process: yes

- Used: `VkPipelineCache`, `vkGetPipelineCacheData`, `VkPipelineCreationFeedbackCreateInfo` on
  all four pipelines. The program checks the file's header (length, version `ONE`, vendor, device,
  `pipelineCacheUUID`) before handing it over.
- MoltenVK's cache holds MSL, so a stage's hit bit means the SPIR-V-to-MSL conversion was skipped.
- Shown: the second process, from the 7561-byte file, reports `APPLICATION_PIPELINE_CACHE_HIT` on
  every stage of every pipeline, in every reload of the final set (with and without the layer,
  under both Metal validations). Five of the six stage hits are evidence. The depth-only
  pipeline's vertex stage hits in any run, from the G-buffer pipeline's in-process conversion of
  the same shader. MoltenVK never sets the pipeline-level hit bit. Creation times are not
  compared: Metal's system shader cache confounds them.
- Controls: a missing file, and a file the program's own header check drops for one changed UUID
  byte, give no hit outside that stage. A hit survives a changed specialisation constant: the
  plain run's file reloaded with `--misuse heap` reports every stage's hit while the output
  follows the new constant (166 pixels wrong, as the `heap` misuse gives). MoltenVK passes
  specialisation constants to Metal as function constants (`MVKShaderModule.mm`,
  `MTLFunctionConstantValues`), so the bit marks the conversion as skipped, not a warm Metal
  pipeline. A file with a valid header and a corrupt or truncated payload, data
  `vkGetPipelineCacheData` did not return
  (VUID-VkPipelineCacheCreateInfo-initialDataSize-00769), makes `vkCreatePipelineCache` fail with
  `VK_ERROR_INITIALIZATION_FAILED` ("Error reading pipeline cache data: Failed to read 4 bytes from
  input stream!"). That code is not among the command's documented results, the layer cannot
  report the misuse, and the program aborts on it.
- State: the blob, its header and, in an engine, a length and checksum of its own.
- External synchronisation: none needed; the cache is internally synchronised and every pipeline
  was created on one thread.

### 6. Command buffers recorded on two threads, intervals overlapping: yes

- Used: one `VkCommandPool` per thread; the two threads meet at a barrier after `begin` and before
  `end`, so the intervals must overlap, and each logs its own. Thread 1 records the heap images'
  upload; thread 2 records the frame. Both command buffers go to queue A in one submit after both
  threads have joined.
- Shown: the intervals overlap in every run, for example upload over [250025, 250988] µs and frame
  over [250029, 250975] µs under the layer. With `MVK_CONFIG_PREFILL_METAL_COMMAND_BUFFERS` at its
  default 0, MoltenVK encodes Metal inside `vkQueueSubmit2` on the submitting thread. This shows
  Vulkan's external-synchronisation rules only, and nothing about parallel Metal encoding or its
  cost.
- State: each pool's owning thread, each command buffer's recording state, what is bound on it.
- External synchronisation: a pool and its command buffers used by one thread at a time. Sharing
  one pool between the threads is reported (below).

### 7. A transient attachment from a lazily allocated type, and a local read: yes, with three "no"s

- Used: `VK_KHR_dynamic_rendering_local_read`. The G-buffer (`R16G16B16A16_SFLOAT`, usage
  `COLOR_ATTACHMENT | INPUT_ATTACHMENT | TRANSIENT_ATTACHMENT`) is colour attachment 0 in
  `RENDERING_LOCAL_READ_KHR` for the whole pass, cleared, written by the G-buffer draw, stored
  `DONT_CARE`. The resolve draw reads it as input attachment 0 (default mappings, no
  `VkRenderingAttachmentLocationInfoKHR` or `VkRenderingInputAttachmentIndexInfoKHR`) and writes
  colour attachment 1. Each pipeline writes only its own attachment through per-attachment colour
  write masks, which need `independentBlend`. Between the two draws, inside the pass:
  `vkCmdPipelineBarrier2` with `BY_REGION`, `COLOR_ATTACHMENT_OUTPUT`/`COLOR_ATTACHMENT_WRITE` to
  `FRAGMENT_SHADER`/`INPUT_ATTACHMENT_READ`. The depth attachment (`D32_SFLOAT`,
  `DEPTH_STENCIL_ATTACHMENT | TRANSIENT_ATTACHMENT`, cleared to 1.0, stored `DONT_CARE`) is
  written by a depth-only occluder at z 0.1 and tested by the triangle at z 0.5.
- MoltenVK reports `dynamicRenderingLocalRead` 1 and memory type 2 lazily allocated. The depth
  image's `memoryTypeBits` include type 2; the G-buffer's do not.
- Shown, the local read: all 4096 output pixels equal 255 minus the G-buffer value, read inside
  the pass.
- Shown, the lazily allocated transient attachment: the depth attachment lives in type 2, which
  MoltenVK maps to memoryless storage, and its 330 pixels under both the occluder and the triangle
  kept the clear values. Its last use comes before the pass's local-read barrier.
- **No, MoltenVK's: a memoryless G-buffer that is read in the pass.** MoltenVK 1.4.2 removes the
  lazily allocated types from any image with input-attachment usage (`MVKImage.mm`, whose comment
  says memoryless as an input attachment needs shader framebuffer fetch, which MoltenVK does not
  support yet). Vulkan allows it; it is the use lazily allocated memory exists for.
- **No, MoltenVK's: the G-buffer in tile memory.** On an Apple GPU, a `BY_REGION` barrier inside
  the pass whose destination is an input-attachment read makes MoltenVK store the attachments, end
  the Metal render pass and begin a new one that loads them (`MVKCmdPipeline.mm`). The local read
  is a texture read from memory. Native Metal's framebuffer fetch is verification row 18's.
- **No, MoltenVK's: a memoryless attachment across that restart.** The `split` probe adds a second
  local-read barrier between the occluder and the triangle: legal Vulkan, silent under every
  validator. The 330 occluded pixels come out wrong. MoltenVK gives a memoryless attachment a
  `DontCare` store even when it restarts the pass (`MVKRenderPass.mm`), so the depth is gone after
  the barrier. Vulkan keeps an attachment's contents for the whole render pass instance. The cause
  is isolated by two more probes: the same split over a depth attachment from a device-local type
  (`split-nolazy`) gives 0 wrong pixels, and over a memoryless one stored with `STORE`
  (`split-store`) still 330. It is the memoryless storage, whatever the store op. Not checked
  against MoltenVK's issue tracker.
- Missing barrier: dropping the local-read barrier (`local`) makes all 4096 pixels wrong, and
  neither synchronisation validation nor either Metal validation reports it. Without the barrier
  MoltenVK does not split the pass and the resolve reads a texture not yet stored.
- State: each attachment's layout through the pass, which attachments are memoryless, the input
  index to colour attachment mapping, each pipeline's write masks, and whether a local-read
  barrier was recorded since an attachment read as input was last written.
- External synchronisation: barriers into the pass (G-buffer `UNDEFINED` to
  `RENDERING_LOCAL_READ_KHR`, output to `COLOR_ATTACHMENT_OPTIMAL`, depth to
  `DEPTH_ATTACHMENT_OPTIMAL`), the in-pass barrier, and the output to `TRANSFER_SRC_OPTIMAL` after
  it.

### 8. A timeline semaphore: one submit waits on a value another signals: yes

- Used: `timelineSemaphore`, `vkQueueSubmit2` with `VkSemaphoreSubmitInfo`, `vkSignalSemaphore`,
  `vkWaitSemaphores`, `vkGetSemaphoreCounterValue`; a release on queue B and an acquire on queue A
  for the buffer the dispatch wrote. One fence on the frame's submit, waited with a five-second
  timeout.
- MoltenVK reports `timelineSemaphore` 1 and implements semaphores with `MTLEvent`.
- Shown: queue A's copy (wait 2, signal 3) is submitted before queue B's dispatch (wait 1 at
  `COMPUTE_SHADER`, signal 2). 200 ms later the counter is 0 and the copy's destination is
  untouched. The host signals 1, waits for 3, and the copy holds the dispatch's 256 values. The
  fence wait returned before the frame's readback.
- State: each semaphore's value and the highest value a pending submit will signal; each submit's
  waits and signals; each fence's state.
- External synchronisation: the queues, one thread each; the host signal.
- Not shown: the effect of the ownership transfer. Metal has no queue-family ownership, and one
  `MTLBuffer` serves every queue, so a missing release or acquire changes nothing visible here.
  Only the pairing is checked, by the layer: dropping the release alone (`no-release`) is reported
  as `VUID-vkQueueSubmit2-commandBuffer-03879`, but dropping both halves (`no-qfot`) reads back
  exact and is silent under every validator here, as undefined behaviour.

### 9. An indexed indirect draw: yes

- Used: `vkCmdDrawIndexedIndirect` at offset 20 of a host-written argument buffer (stride 20,
  count 1). The arguments at offset 0 describe a decoy triangle elsewhere on screen.
- MoltenVK reports `multiDrawIndirect` 1 (reported; its CPU-loop encoding is verification row
  34's, not measured here). **No, MoltenVK's: `drawIndirectCount` is 0.** Vulkan 1.2 makes it
  optional and MoltenVK 1.4.2 does not support it. D42 already treats indirect count as a reported
  capability.
- Shown: the triangle at offset 20 drew; the 152 pixels surely inside the decoy and outside the
  triangle hold the clear value.
- State: the argument buffer, offset, count and stride.
- External synchronisation: host writes before the submit. Synchronisation validation does not
  track indirect or index buffers.

### 10. A readback of the triangle against a CPU reference: yes

- Used: the 64×64 `R16G16B16A16_SFLOAT` output copied to a buffer after a barrier to
  `TRANSFER_SRC_OPTIMAL`, then a `COPY` to `HOST` barrier and the fence. Every written value is an
  integer below 256, exact in a 16-bit float.
- Shown: each pixel centre is classified against the triangle and the occluder with a margin of
  1/16 pixel. The vertices are on integer coordinates and Vulkan requires at least four bits of
  sub-pixel precision, so no conformant rasteriser can change a pixel outside the margin. Result:
  1146 surely drawn, 330 surely occluded, 2596 surely outside, 24 inside the margin (either value
  accepted), 0 wrong.
- State: the image's layout before the copy, the buffer's host-read state.
- External synchronisation: the fence before the host read.

### Not shown

- The swapchain and present: the program is headless.
- A queue family ownership transfer's effect: item 8.
- Metal residency: item 3.
- Metal libraries and `MTLBinaryArchive`: the program is Vulkan only.
- Transients that alias memory, and the fences between them: no two resources share memory.
- A buffer in device-only memory filled through a staging copy: every buffer sat in memory type 1.
- SPIR-V cooked by Slang (D44): the program's shaders are GLSL compiled by `glslc`.
- Also not exercised: sampler objects (the heap reads integer texels with `texelFetch`), indirect
  dispatch (not in the decision's list), the flush and invalidate path of non-coherent memory
  (none on this device), parallel Metal encoding (item 6), 64-bit atomics (MoltenVK reports
  `shaderBufferInt64Atomics` 0, as D44 expects of the M1), multi-draw-indirect's cost, and
  GPU-assisted validation of items 2 and 3 (below).
- A missing barrier with the GPU made to lag: the spike has one frame, and the `sync` misuse below
  passes its readback, hidden by Metal's hazard tracking.

## Validation runs

Every clean run below is the final binary, cold then reload, exit 0, with the summary line
"0 failed checks, 0 validation errors, 0 validation warnings". Each validator was shown to be on
by a misuse it reported.

| Run | Settings | Clean result | Misuse and what reported it |
|---|---|---|---|
| Plain | none | exit 0, cold and reload | |
| Khronos layer 1.4.363 | `--validate`, `VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true`, `VK_KHRONOS_VALIDATION_THREAD_SAFETY=true`; the layer lists Core Checks, Synchronization, Stateless Parameter, Object lifetime, Thread Safety, Handle Wrapping | 0 errors, 0 warnings, cold and reload | `sync` (the output's transition after the pass with source stage `NONE`): `SYNC-HAZARD-WRITE-AFTER-WRITE` on `vkCmdPipelineBarrier2`, layout transition after the attachment storeOp; the readback still passes. `thread` (both threads on one pool, recording 20,000 push-constant calls): `UNASSIGNED-Threading-MultipleThreads-Write` on `VkCommandPool`, 10 errors. `wait` (queue A waits on 0): `VUID-vkQueueSubmit2-commandBuffer-03879`, `VUID-VkSubmitInfo2-semaphore-03882`, `VUID-VkSemaphoreSignalInfo-value-03258`, and items 2 and 8 fail |
| Metal API validation | `MTL_DEBUG_LAYER=1 MTL_DEBUG_LAYER_WARNING_MODE=nslog` ("Metal API Validation Enabled") | exit 0, cold and reload | `copy` (a buffer copy past the source's end): Metal asserts "Copy From Buffer Validation (sourceOffset + size)(37900) must be <= [sourceBuffer length](1024)" and aborts |
| Metal shader validation | `MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1` ("Metal GPU Validation Enabled") | exit 0, cold and reload | `heap` (specialisation constant 1 makes one band read unwritten slot 6): "Null texture access executing fragment function main0, encoder vkCmdBeginRendering RenderEncoder, draw: 1", 166 pixels wrong. `address` (the source address moved past the allocation): "Invalid device load at offset 2360960, executing kernel function main0, encoder vkCmdDispatch ComputeEncoder", items 2 and 8 fail. These messages reach stderr only, not the debug messenger: the logs were read for them |
| GPU-assisted validation | `--validate`, `VK_KHRONOS_VALIDATION_GPUAV_ENABLE=true`; also tried with `..._GPUAV_BUFFER_ADDRESS_OOB=false`, with `..._GPUAV_DESCRIPTOR_CHECKS=false` and `..._GPUAV_POST_PROCESS_DESCRIPTOR_INDEXING=false`, and with `..._GPUAV_SAFE_MODE=true` | Does not run: 4 errors, 4 warnings | MoltenVK cannot compile the instrumented shaders: the address check fails with "reinterpret_cast from 'uint' to 'ulong' is not allowed", the descriptor check with "no member named 'm_266' in 'spvDescriptorSetBuffer7'" and "use of undeclared identifier '_237'". The layer falls back to the uninstrumented pipelines, so the clean run's checks still pass, and neither the `heap` nor the `address` misuse is reported. Its own warnings: `VALIDATION-SETTINGS` (GPU-AV beside core checks) and three `WARNING-Setting-Limit-Adjusted`. **No, MoltenVK's** (with the layer): GPU-AV does not run where items 2 and 3 run |

Mutations, each run on the final binary, each caught by the readback alone: `vertex` (one
triangle vertex moved one pixel) 15 pixels wrong; `clear` (the G-buffer loaded `DONT_CARE`) 2940;
`draw` (the indirect draw skipped) 1146; `wait` (above) 0 of 256 values.

Probes, each run under the Khronos layer (synchronisation and thread safety), Metal API validation
and Metal shader validation. "Silent" means no message from any of the three.

| Probe | What it changes | Readback | Validators |
|---|---|---|---|
| `local` | The in-pass local-read barrier dropped | 4096 wrong | Silent |
| `split` | A second local-read barrier between the depth-only draw and the triangle, over the memoryless depth (legal) | 330 wrong | Silent |
| `split-nolazy` | As `split`, the depth from a device-local type (legal) | Exact | Silent |
| `split-store` | As `split`, the memoryless depth stored with `STORE` (legal) | 330 wrong | Silent |
| `no-release` | The ownership transfer's release dropped | Exact | Layer: `VUID-vkQueueSubmit2-commandBuffer-03879`; Metal silent |
| `no-qfot` | Release and acquire both dropped | Exact | Silent |
| `heap-layout` | The heap images left in `TRANSFER_DST_OPTIMAL` | Exact | Silent |
| `uniform` | The heap indexed without `nonuniformEXT` (`shaderSampledImageArrayDynamicIndexing` enabled, reported 1) | Exact | Silent |
| `uuwp-used` | Slot 5, which the pending draw reads, rewritten while the frame is pending | 165 wrong | Silent |
| `uuwp-noflag` | The normal run's pending write of free slot 600, without `UPDATE_UNUSED_WHILE_PENDING` | Exact | Silent |

Cache controls, plain: a missing file and a changed UUID byte, no hit outside the depth-only
vertex stage; a corrupt payload and a truncated one, `VK_ERROR_INITIALIZATION_FAILED` and exit 134;
the plain run's file reloaded with `--misuse heap`, every stage hits and 166 pixels are wrong.

Before the attachments moved to `R16G16B16A16_SFLOAT`, MoltenVK warned "Blending is enabled for
attachment with format R8G8B8A8_UINT, which does not support it" on every pipeline writing an
integer colour attachment, with blending disabled (`MVKPipeline.mm` warns for any format Metal
cannot blend). The layer reported `VUID-VkPipelineColorBlendStateCreateInfo-pAttachments-00605`
until `independentBlend` was enabled. Both were fixed, not noted.

## What `rhi-api` must expose

For milestone 2's facade. Every call below is one the null backend can honour by keeping the state
named in its item.

1. **Device creation against two sets.** Required: buffer device address; descriptor indexing
   (runtime arrays, partially bound, sampled-image update-after-bind, update-unused-while-pending,
   sampled-image non-uniform indexing); timeline semaphores; synchronisation2; dynamic rendering
   and its local read; independent blend. Reported: multi-draw-indirect, indirect count, 64-bit
   buffer atomics, whether an attachment of a given usage can be memoryless, and whether a
   memoryless attachment keeps its contents across a local-read call (no on MoltenVK 1.4.2).
   Portability enumeration and the portability subset stay inside `rhi-vulkan`.
2. **Queues by role.** A graphics queue and a compute queue, each with its own submit. A
   resource's state names its queue, and the backend emits the release and acquire. Null tracks
   the owning queue and fails an acquire with no release queued, as the layer did. Null also
   fails a use on a queue other than the owner with no release and acquire between, unless the
   resource is shared concurrently or its contents are discarded: a transfer dropped whole
   (`no-qfot`) is silent under the layer and MoltenVK.
3. **Buffers.** Size, usage (device address, index, indirect, copy source and destination) and a
   memory class (device, upload, readback). A device address as a `u64`, valid while the buffer
   lives. Mapped host access with an explicit flush and invalidate of a range, a no-op where the
   memory is coherent.
4. **Images.** Format, extent, usage (sampled, colour, depth, input attachment, transient, copy)
   and a memoryless request the backend may decline per usage; the outcome is queryable. The
   render graph, which knows the pass, does not ask for memoryless where that capability
   (point 1, contents kept across a local-read call) is no.
5. **One bindless heap of sampled images.** A fixed capacity under the device's update-after-bind
   limit. A slot written between bind and submit, or while a submit is pending if no pending
   submit can read it. A slot reused only after the timeline value that freed it. Each pass
   declares the heap slots and buffers it reaches, so `rhi-metal` makes them resident per encoder
   (section 12.1, D45). MoltenVK ignores the declaration, so this run did not exercise it; null
   keeps it. At each draw or dispatch that binds the heap, null fails unless every slot the pass
   declares holds an image in the sampled-read state, in submission order (`heap-layout`). Null
   fails a write to a slot that a pass of a still-pending submit declared (`uuwp-used`). Null
   cannot see the index, so it treats every declared slot as read and checks nothing else. Both
   were silent under every validator here.
6. **Pipelines from cooked shaders with specialisation constants** (id to bytes). Layouts made of
   the heap, one input-attachment set per pass and a push-constant range. Graphics pipelines carry
   the attachment formats, the depth format, per-attachment write masks and the depth state.
7. **A pipeline cache as an opaque blob,** handed back only after the facade's own length and
   checksum match and the backend's identity check passes (Vulkan: vendor, device,
   `pipelineCacheUUID`; Metal: an `MTLBinaryArchive` with
   `MTLPipelineOptionFailOnBinaryArchiveMiss`). A creation failure on accepted data falls back to
   an empty cache. Per-stage hits are a statistic, and on MoltenVK not a warm-pipeline signal.
8. **Encoders, one per thread,** each from its own pool, `Send` and not `Sync`. One submit takes
   encoders recorded on several threads, with its timeline waits and signals.
9. **Resource states** as a backend-neutral enum mapping to stages, access, layout and queue, and
   a subresource range (mips and layers). The
   spike needed `UNDEFINED`, the render-pass-local-read layout, colour and depth attachment,
   fragment-shader sampled read, copy source and destination, host read, and the release and
   acquire.
10. **A render pass begun with its attachments** (load clear or don't-care, store or don't-care,
    clear values, depth) and the mapping from input index to colour attachment. **The local read is
    a call of its own**, "read what this pass has written so far", not a general barrier: on
    MoltenVK it ends and restarts the Metal pass, and on native Metal with framebuffer fetch it is
    nothing.
11. **Timeline semaphores** with host signal, host wait with a timeout and a value query, and
    fences (section 12.1).
12. **Indexed indirect draws** (buffer, offset, count, stride); indirect count only behind its
    capability.
13. **Copies:** buffer to buffer, buffer to image, image to buffer.
14. **A message sink.** Every validation and driver message is counted, and a test fails on any;
    Metal shader validation's output reaches stderr only and is read from there.

Not shown by this run, owed by section 12.1 and D43 to D45: transients that alias memory, as a
relation the backend fences, the second starting undefined; heaps; indirect command buffers;
samplers; Metal libraries and `MTLBinaryArchive` beside `VkPipelineCache`. They come from the
architecture, unstable until the native Metal spike (D43).

What `rhi-null` must check, because every validator here missed it (each probe above):

- an input-attachment read with no local-read call since the attachment's last write (`local`);
- a memoryless attachment used after a local-read call in its pass, when the capability set null
  was created with says its contents do not survive (`split`);
- a use on a queue other than the owner with no release and acquire between, unless the resource
  is shared concurrently or its contents are discarded (`no-qfot`);
- a draw or dispatch whose pass declares a heap slot whose image is not in the sampled-read state
  (`heap-layout`);
- a write to a heap slot that a pass of a still-pending submit declared (`uuwp-used`).

A missing `nonuniformEXT` (`uniform`) is invisible to null too; it is the cooker's check.

## Design questions for the lavapipe run

1. A conformance test with the in-pass local-read barrier dropped between a colour write and its
   input-attachment read: whether synchronisation validation reports it there. If it is silent
   too, `rhi-null`'s rule is the only guard.
2. A conformance test with a second local-read barrier between a depth-only draw and a
   depth-tested draw over a memoryless depth attachment: expected exact on a conformant driver. If
   it is, `rhi-vulkan` on MoltenVK reports that a memoryless attachment does not keep its contents
   across a local-read call (point 1 of the facade list). It needs a lazily allocated type: on
   lavapipe or the desktop GPU, if either offers one; otherwise KosmicKrisp on the M1
   (verification row 15).
3. Memoryless as request or requirement: whether lavapipe offers a lazily allocated type at all,
   and what the facade reports when it does not.
4. The pipeline cache: lavapipe's response to a corrupt payload behind a valid header, which
   decides whether the facade's checksum is the only guard.
5. GPU-assisted validation of items 2 and 3, with the `heap` and `address` misuses, which MoltenVK
   cannot run.
6. The ownership transfer's effect is the desktop GPU's; lavapipe checks the pairing under the
   layer.
7. Indirect count: exercise it where it is reported.
8. Non-coherent memory: whether lavapipe has it; if not, the flush and invalidate path is the
   desktop GPU's to show.

For the desktop GPU, beside questions 6 and 8: a heap indexed without `nonuniformEXT` (item 3),
and a heap image in the wrong layout, which a GPU with compressed images may show.

## The program

The program is not in the repository. Its hash, over the sorted list of its source files (manifest,
lock file, Rust source, GLSL and the compiled SPIR-V), from the program's directory:

```
find Cargo.toml Cargo.lock src shaders -type f | LC_ALL=C sort | xargs shasum -a 256 | shasum -a 256
```

`bf62df81af96639fa9e44c1832db9b4ff1fb187c5dc094af4e5acf5baf60ddb0`, over 15 files:
`Cargo.lock`, `Cargo.toml`, `shaders/bda.comp` and the GLSL of `fullscreen.vert`,
`gbuf-uniform.frag`, `gbuf.frag`, `resolve.frag` and `tri.vert`, each with its `.spv`, and
`src/main.rs`.
