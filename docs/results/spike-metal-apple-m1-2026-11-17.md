# Native Metal spike, Apple M1, 2026-11-17

The milestone 2 plan's `spike-metal` lane (decision 5.6, D139): a throwaway headless program
against Metal itself, which walks the plan's list. It answers one question: what must
`lactose-rhi-api` expose so that architecture section 12.1's list runs on Metal, with no call that
Vulkan or a null backend cannot honour?

## Answer

- **All twelve items ran and were checked on the GPU, with exact results.** The checks were made
  by readback against a CPU reference, by readback of the drawable, or by the pipeline count with
  fail-on-miss. The runs:
  - one frame on the M1;
  - a heap of 1024 textures as an argument buffer, read at slots 5, 17, 300 and 1023 with an
    index that varies inside one draw;
  - a texture reached only through an argument buffer, read after a writer made to lag, in a
    later command buffer and in the same one;
  - a memoryless G-buffer read in its pass by framebuffer fetch;
  - a memoryless depth that still holds the occluder after that read;
  - two transients aliasing one untracked placement heap, with a fence between them;
  - an indexed indirect draw and an indirect command buffer;
  - pipelines from Slang's MSL specialised by function constants, all thirteen found again in a
    second process with fail-on-miss;
  - two command buffers encoded on overlapping threads;
  - a shared event waited on across queues before it was signalled;
  - two samplers;
  - six frames of drawables from a layer in no window, presented from a second queue.

  All of it is clean under Metal API validation and Metal shader validation, alone and together.
  The deployment target is macOS 14, and no call is newer, so the program has no `#available`
  branch.
- **Metal's run-time compiler returns errors only.** A `#warning` canary was not delivered by
  either form of `makeLibrary`. Offline, `metal` 32023.922 warns twice on the frame's MSL,
  "writable resources in non-void vertex function", at each vertex function taking the read-only
  position buffer, which Slang emits `device` rather than `const device`. The other four files are
  clean. The run now compiles every file offline as well, counts the warnings and allows those two
  by text; the canary fails it.
- **The heap's shader form: the fixed-size array in a parameter block.** Its slot is the same
  number on both targets. A `DescriptorHandle` also worked here, but the value the CPU writes for
  it is an `MTLResourceID` on Metal and a two-word index into a heap Slang declares for itself on
  SPIR-V.
- **The RHI spike's fourteen points hold on Metal.** One rule is added to point 4: a memoryless
  attachment is never stored. Metal asserts on a store; Vulkan allows it. The ownership half of
  point 2 and the layouts of point 9 are Vulkan's alone. Three points are added:
  - samplers in a table the backend owns;
  - aliased transients as a placement and an alias call;
  - the drawable's acquire and present.
- **Indirect command buffers stay out of `rhi-api`.** A CPU-encoded one is a recorded command list
  the backend does not need, and Slang cannot emit the MSL that encodes one on the GPU. Neither
  validator reported its index buffer left out of the residency declaration.
- **Silent in at least one setting, so `rhi-null` or the test policy must cover them:**
  - **Missing residency, for a heap slot or for a buffer reached only through its address.**
    Shader validation reports it. Without validation both read exact here because the upload
    finished before the frame ran. The declaration also brings the resource under Metal's hazard
    tracking: a texture reached only through an argument buffer and left out of `useResource`
    read stale while its writer in an earlier command buffer was running (all 4096 values, 3 of 3
    plain runs, and under API validation, which was silent), and read exact once declared. In the
    same command buffer as its writer it read exact either way. Run after the address dispatch
    instead of before the frame, the same undeclared read was exact in 3 of 3 runs: whether an
    undeclared reader is ordered depends on earlier work in the process.
  - **A sampler made without `supportArgumentBuffers`.** It reads zeros under both validators.
  - **A rewritten heap slot that a pending frame reads.** Both validators are silent.
  - **A dropped alias fence.** It shows as wrong values under API validation, but shader
    validation serialises the work and hides it.
- **The binary archive is not safe on bad input.** With 4 KiB inverted at each sixteenth of the
  file, Metal's loader refused two, crashed the process on two (a segfault and a bus error), had
  three pipelines miss on one, and accepted eleven with every pipeline found and the frame exact.
  Five of the eleven change only the AIR slice, which a hit does not read; for the other six the
  run does not show whether the inverted bytes were code that ran. Loading the archive under
  shader validation segfaults, and an archive written under it has no AIR slice and misses every
  pipeline on reload. So `rhi-metal` neither opens nor serialises an archive while shader
  validation is on, and before Metal parses one it compares an identity written beside it (OS
  build, device and GPU family) and the facade's length and checksum over a private copy, which it
  then loads. With those checks every corrupt, truncated and foreign archive here was refused
  before the parse.

## Run header

- **Machine:** Apple M1, 4 performance and 4 efficiency cores, 16 GiB.
  - macOS 27.2.1 (26B131).
  - Xcode 27.2 (27B52) is installed, and `xcode-select` points at it.
  - The program uses the command line tools instead.
- **Commit:** `e4827d1` (clean): `a539d60` with this file's first version. The program lives
  outside the repository.
- **Versions:**
  - `xcrun metal --version`: Apple metal version 32023.922 (metalfe-32023.922.4), target
    air64-apple-darwin27.2.0. Each run calls it on every MSL file for its warnings (below).
  - Command line tools `swiftc --version`: Apple Swift version 6.0.3 (swiftlang-6.0.3.1.10
    clang-1600.0.30.1). The SDK is MacOSX15.2, so `MTLGPUFamily.metal4` and `MTLResidencySet`'s
    successors are absent.
  - `slangc -version`: `2026.17-25-gc7954ebd1`. This is the Vulkan SDK 1.4.363.1's copy, not the
    pin of decision 5.8.
- **Shaders:**
  - **Sources.** Five Slang files:
    - the frame;
    - the compute kernels;
    - the present draw;
    - the `DescriptorHandle` comparison;
    - the tracking kernels of item 2.
  - **MSL.** The build compiles each to MSL with
    `slangc <name>.slang -target metal -line-directive-mode none -warnings-as-errors all`. The
    MSL is kept beside the program and reproduces byte for byte. Each run compiles each file
    twice:
    - offline, by `xcrun metal -std=metal3.1 -mmacosx-version-min=14.0 -c`, whose warnings and
      errors are counted. Two warnings are known and allowed by file and text: `frame.metal`
      106:28 and 128:29, "writable resources in non-void vertex function", on `vs_tri` and
      `vs_full`. Any other message fails the run, and the run reports how many of the two it saw
      (2 of 2 in every run of the final set).
    - at run time, through Metal, by `makeLibrary(source:options:completionHandler:)` with
      `MTLCompileOptions.languageVersion = .version3_1`. These are the libraries the pipelines
      use.
  - **What each compiler reports.** Metal's run-time compiler returns errors only: a `#warning`
    canary was not delivered by either form (the throwing form and the completion form, in the
    review's probe). Offline, `metal` warns twice on the frame's MSL, at each vertex function
    taking the read-only position buffer (`StructuredBuffer<float4>`), which Slang emits
    `packed_float4 device*` rather than `const device`. The other four files are clean.
  - **The canary.** `--misuse msl-canary` compiles a copy of the frame's MSL with
    `#warning "spike-metal canary"` as its first line. The offline count reported it as
    unexpected and the run exited 1, plain and under API validation.
  - **SPIR-V.** Each file is also compiled to SPIR-V with the same flags. It is read with the
    SDK's `spirv-dis`, only to check the non-uniform decoration.
- **Build:**
  `build.sh <binary>` runs `slangc` as above, then
  `/Library/Developer/CommandLineTools/usr/bin/swiftc -swift-version 5 -target arm64-apple-macos14.0 -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk -Onone -g -framework Metal -framework QuartzCore -framework Foundation src/main.swift -o <binary>`.
  Frameworks: Metal, QuartzCore and Foundation only.
- **Run:**
  `spike-metal <program>/shaders <archive-file> [--reload] [--no-archive] [--misuse <what>] [--salt <n>] [--lag <n>] [--alias render|compute|split] [--track-lag <n>] [--track early|late] [--raw-archive]`.
  - The cold run makes every pipeline, adds each to an `MTLBinaryArchive`, serialises it, and
    writes `<archive-file>.identity` beside it: the OS version string, the device name, the
    highest Apple family, and the file's length and checksum.
  - `--reload` first makes the checks of item 7, then makes every pipeline with `binaryArchives`
    set and `.failOnBinaryArchiveMiss`. `--raw-archive` leaves the checks out, to show Metal's
    own behaviour.
  - `--track` runs item 2's tracking case before the frame (`early`, the default) or after the
    address dispatch (`late`).
  - Exit 0 only when every check passed, no command buffer reported an error, the run-time
    compiler returned no error and the offline compiler said nothing beyond the two known
    warnings.
  - Exit 2 when there is no device or setup fails.
  - Every command buffer is made with `errorOptions = .encoderExecutionStatus`, and its status
    and error are checked.
- **Validation settings:**
  - API validation: `MTL_DEBUG_LAYER=1 MTL_DEBUG_LAYER_WARNING_MODE=nslog`, banner "Metal API
    Validation Enabled".
  - Shader validation: `MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1`, banner
    "Metal GPU Validation Enabled".
- **Date and load:**
  - The final run set ran on the final binary from 11:42:45 to 11:45:43 UTC on 2026-11-17, 104
    processes one at a time.
  - Load averages (1, 5, 15 min) were 4.29 7.07 12.07 before the set and 8.70 6.65 10.89 after.
  - Other agents were building beside it. No figure below is a measurement.
  - A set before it, from 11:38:58 to 11:41:55 UTC (load 2.86 8.48 13.93 before, 6.26 7.82 12.62
    after), ran the same program with item 8's overlap still a check. Its outcomes were the same
    except where this file says so.

## The device

`MTLCopyAllDevices()` returned one device. The program exits 2 when it returns none.

| Property | Value |
|---|---|
| Name | Apple M1 |
| `lowPower`, `headless`, `removable` | false, false, false |
| `hasUnifiedMemory` | true |
| GPU families | apple1 to apple7, mac2 and metal3 true; apple8 and apple9 false |
| `argumentBuffersSupport` | tier 2 |
| `maxArgumentBufferSamplerCount` | 1024 |
| `readWriteTextureSupport` | tier 2 |
| `areRasterOrderGroupsSupported` | true |
| `maxBufferLength` | 9,534,832,640 |
| `recommendedMaxWorkingSetSize` | 12,713,115,648 |
| `maxThreadsPerThreadgroup` | 1024 |
| `maxThreadgroupMemoryLength` | 32,768 |

What each item needed from the device is given under the item.

## The twelve items

The terms used for each item:
- **Yes** means a GPU-side use whose result was read back and compared exactly.
- **Reported** means a query returned true.
- **State** is what the caller carried between calls, which is what a null backend must keep.
- **External synchronisation** is what the program had to order itself.

Availability is the SDK header's `API_AVAILABLE` for macOS.

### 1. A bindless heap as an argument buffer, indexed past slot 0 with a non-uniform index: yes

- **Used:**
  - `MTLTexture.gpuResourceID` (13.0), written as an `MTLResourceID` (8 bytes) into slot × 8 of
    a shared 8 KiB buffer bound with `setFragmentBuffer` at the index Slang assigned.
  - The textures are 1×1 `rgba8Uint`, private, uploaded by a blit from a staging buffer.
  - The shader is `ParameterBlock<Heap>` with `Texture2D<uint4> t[1024]`. Slang emits a
    `Heap constant*` holding `array<texture2d<uint, access::sample>, 1024>`.
  - The slot comes from a table indexed by the pixel's band of eight columns, so it varies inside
    one draw.
- **Device:**
  - Argument buffers tier 2: reported. Tier 2 is what lets a buffer hold resource IDs and an
    array of 1024 textures.
  - Metal 3 family: reported. `gpuResourceID` is a Metal 3 call.
- **Shown:** 1052 pixels of the triangle across four bands (99, 315, 477 and 161 pixels), each
  equal to 255 − (texel ⊕ salt).
- **Non-uniform marker:**
  - On Metal there is none, and no marker was needed.
  - The SPIR-V of the same source carries `NonUniform` on the access, through the inlined
    `__target_switch` helper.
  - **Heap form:** the fixed array is chosen. The reason and what the other form did are under
    "The heap's shader form".
- **State:**
  - which slot holds which texture;
  - the free slot 600, written while the frame was pending (exact);
  - the frame's declared slots, for the `uuwp-used` probe.
- **External synchronisation:** a slot write is a plain CPU store into shared memory. The caller
  orders it against pending GPU reads; Metal does not.

### 2. Residency per encoder, for the heap and for a buffer reached only through its `gpuAddress`: yes

- **Heap:** `useResources(_:usage:stages:)` (10.15) on the render encoder, for every texture in
  the heap, `.read`, `.fragment`.
- **Address:**
  - `MTLBuffer.gpuAddress` (13.0) for both buffers.
  - `useResource(_:usage:)` (10.13) on the compute encoder: `.read` for the source and `.write`
    for the destination.
  - The two addresses travel in an `AddressParams` struct (`uint device*` twice, then a count)
    passed by `setBytes`. Neither buffer is bound.
- **Shown:** the heap as in item 1, and 256 of 256 values equal to the CPU reference
  `src[i] × 2654435761 + i`.
- **Misuses:**
  - A heap slot dropped from the declaration (`heap-resident`).
  - The source dropped from the address dispatch's declaration (`address-resident`).
  - Each is reported by shader validation alone, as "Texture is not resident ..." and
    "resident:No" (quoted under "Validation runs").
  - **Without validation both read exact here because the upload finished before the frame
    ran.** The frame waits on a host signal given about 100 ms after it is committed, by which
    time the upload has completed. They do not show that a missing declaration is harmless.
- **Tracking, with a writer made to lag:**
  - A private 64×64 `r32Uint` texture holds 1. A kernel spins (`--track-lag`, 2,000,000
    iterations, about 66 ms of GPU time) and then writes 7. A second kernel reads the texture only
    through a one-slot argument buffer of `MTLResourceID`s (`ParameterBlock<TrackHeap>`) and
    writes what it read to a shared buffer.
  - Two layouts in each process: the reader in a second command buffer on the same queue, and the
    reader in the writer's own command buffer, as a later encoder.
  - The clean run declares the texture on the reader, `useResource(tracked, usage: .read)`. The
    misuse `track-undeclared` leaves it out.

  | Reader | Two command buffers | One command buffer |
  |---|---|---|
  | Declared, plain, API, shader and both (every clean run) | 4096 of 4096 read 7 | 4096 of 4096 read 7 |
  | Undeclared, run before the frame, plain | 4096 of 4096 read 1, stale: 3 of 3 runs | 4096 read 7 |
  | Undeclared, before the frame, API validation | 4096 read 1, stale, and silent | 4096 read 7 |
  | Undeclared, before the frame, shader validation | "Texture is not resident ..."; 0 read 7 and 0 read 1 | 4096 read 7 |
  | Undeclared, run after the address dispatch (`--track late`), plain | 4096 read 7: 3 of 3 runs | 4096 read 7 |

  - **The declaration also brings the resource under Metal's hazard tracking.** One left out of
    `useResource` read stale while its writer in an earlier command buffer was running. Within one
    command buffer the encoders were ordered either way.
  - **Where the undeclared case runs changes its outcome.** After the address dispatch of item 9
    it read exact. Bisecting the program placed the cause in that section's dispatch, but neither
    an event wait, a dispatch on a second queue nor `cs_address` alone reproduced it in a
    separate probe. An undeclared reader's order is therefore not defined, and is not to be
    relied on either way.
- **State:** the textures and buffers each encoder reaches, with read or write usage.
- **External synchronisation:** none beyond the declaration, which must name every resource a
  pass reaches through the heap or an address, with its exact usage.

### 3. A memoryless G-buffer read in its pass by framebuffer fetch, and a memoryless depth attachment kept across that read: yes

- **Used:**
  - `MTLStorageMode.memoryless` (11.0) for the `rgba16Float` G-buffer (usage `.renderTarget`).
  - The same for the `depth32Float` depth. Both use load clear and store `.dontCare`.
  - The output is a private `rgba16Float` colour attachment 1.
  - The resolve's fragment input is `[[color(0)]]`, which Slang emitted for a `SubpassInput` with
    input attachment index 0.
- **One pass, five draws in order:**
  1. A depth-only occluder at z 0.1.
  2. The G-buffer triangle at z 0.5 (item 5's indirect draw).
  3. The full-screen resolve, depth `always`, writing 255 − G-buffer.
  4. A full-screen marker at z 0.3 with depth `greater`, so it passes only where the stored depth
     is still 0.1.
  5. The indirect command buffer's draw.

  No call separates the G-buffer write from its read.
- **Device:** memoryless and framebuffer fetch are Apple-family features; apple7 is reported.
- **Shown:**
  - the triangle's 1052 pixels from the fetched G-buffer;
  - 480 pixels of the occluder overwritten by the marker after the read, so the memoryless depth
    kept its contents across the read;
  - 2396 background pixels at 255.

  Unlike MoltenVK, native Metal keeps a memoryless depth across a local read.
- **Misuses and mutations:**
  - `memoryless-store` (the G-buffer stored with `.store`): API validation asserts "Memoryless
    attachment content cannot be stored in memory." Vulkan allows a lazily allocated attachment
    to be stored.
  - `clear` (the G-buffer loaded `.dontCare`): 2403 pixels wrong (2405 in the first version),
    from the readback alone.
- **State:** each attachment's storage mode, load and store actions; which input index is which
  colour attachment.
- **External synchronisation:** none. The fetch is ordered by the rasteriser.

### 4. Two transients aliasing one untracked heap, with a fence between them: yes

- **Heap:**
  - `MTLHeapDescriptor` with `type = .placement` (10.15), `storageMode = .private` and
    `hazardTrackingMode = .untracked` (10.15).
  - Its size is `heapTextureSizeAndAlign` (10.13) of one 256×256 `r32Uint` texture: 262,144
    bytes, align 128.
  - Two textures are made at offset 0 by `makeTexture(descriptor:offset:)`.
  - Fences are `MTLFence` (10.13) with `updateFence` and `waitForFence`; on the render encoder
    they are `updateFence(_:after: .fragment)` and `waitForFence(_:before:)`.
- **Order:**
  1. Write A.
  2. Read A after a GPU spin. The lag gives about 75 ms of GPU time for the command buffer.
  3. Write B. This is the alias step, waiting on the fence the read of A updated.
  4. Read B.
- **Three layouts of step 3:**
  - `render`, the default: a render pass whose clear writes B.
  - `compute`: a compute write in the same command buffer.
  - `split`: steps 3 and 4 in a second command buffer on the same queue.
- **Shown:** with the fence, 65,536 of 65,536 values exact for both transients, in every layout,
  in every run of the final set that kept the fence.
- **Whether the fence, dropped with the GPU made to lag, showed:**

  | Layout | Final set (and the set before it) | Earlier runs |
  |---|---|---|
  | `render` | 3 of 3 runs wrong, all 65,536 values of A read as B's clear (3 of 3) | 6 of 6, and 3 of 3 in the first version's set |
  | `render`, under API validation | wrong and silent: all 65,536 of B (2,496 of A and 63,040 of B) | all of A in the first version's set |
  | `compute` | 0 of 3 (0 of 3) | 2 of 5, 40,960 values wrong each time; 0 of 3 before the textures gained render-target usage |
  | `split` | 0 of 3 (0 of 3) | 3 of 4, and 1 of 3 in the first version's set, all 65,536 wrong (the second command buffer started while the first ran) |
  | `render`, under shader validation | 0 wrong, silent: shader validation serialises the work | |

  The dropped fence shows reliably when the alias is written by a render pass after a compute
  read. Elsewhere it shows only sometimes. **No validator reports it.**
- **State:** which transients share memory and the order of their uses; the fence each use
  waits on and updates.
- **External synchronisation:** all of it. The heap is untracked, so Metal orders nothing between
  the aliases. The fence orders encoders within a queue, across command buffers too (`split`).

### 5. An indexed indirect draw and an indirect command buffer: yes

- **Indexed indirect draw:**
  - `drawIndexedPrimitives(type:indexType:indexBuffer:indexBufferOffset:indirectBuffer:indirectBufferOffset:)`
    (10.11).
  - The buffer holds two 20-byte argument records: a decoy at offset 0 and the triangle at 20.
    This is the same layout as `VkDrawIndexedIndirectCommand`.
- **Indirect command buffer:**
  - `MTLIndirectCommandBufferDescriptor` with `.drawIndexed`, `inheritPipelineState` and
    `inheritBuffers`, from `makeIndirectCommandBuffer(descriptor:maxCommandCount:options:)`
    (10.14).
  - One command is encoded on the CPU by `indirectRenderCommandAt(0).drawIndexedPrimitives(...)`
    at index offset 24 bytes.
  - It has its own index buffer, which no other draw binds, so the buffer's residency is the
    declaration's alone.
  - It runs through `executeCommandsInBuffer(_:range:)` (10.14), with
    `supportIndirectCommandBuffers` (10.14) on its pipeline and
    `useResource(icbIndices, usage: .read, stages: .vertex)`.
- **Shown:**
  - The triangle drew from the record at offset 20, and the decoy's area stayed at 255.
  - The indirect command buffer drew 152 pixels at 2000.
  - Mutation `draw` (the indirect draw skipped): 1052 pixels wrong.
- **Misuse `icb-resident`** (the indirect command buffer's own index buffer left out of the
  declaration): exact output and **no message from either validator**, in the plain run, under
  API validation and under shader validation. Index fetch is fixed-function, so shader
  validation likely has no shader code to instrument for it. The rule exists in the
  documentation and is not checked on this device and toolchain.
  - In the first version of this file the index buffer was also the direct indexed draw's, so
    the misuse removed nothing. The run here gives it a buffer of its own.
- **Not done:** an indirect command buffer encoded on the GPU. Slang cannot emit MSL's
  `command_buffer`, as the plan expects.
- **State:** the argument buffer and offset; the indirect command buffer's commands and the
  resources they reach.
- **External synchronisation:** the arguments are written before submission.

### 6. A pipeline from Slang-emitted MSL, specialised by a function constant: yes

- **Used:**
  - `makeFunction(name:constantValues:)` (10.12) with an `MTLFunctionConstantValues` that sets
    index 0 (`SALT`, 90) and index 1 (`BAD_SLOT`, 0), both `.uint`.
  - Slang turned `[[vk::constant_id(N)]] const uint` into `function_constant(N)` plus a default
    guarded by `is_function_constant_defined`. The ids carry over one for one.
  - `functionConstantsDictionary` reflects `fc_SALT_0` (index 0, uint, not required) and
    `fc_BAD_SLOT_0` (index 1, uint, not required).
- **Shown:** every G-buffer pixel equals 255 − (texel ⊕ 90). Mutation `salt0` specialises with no
  values set, so the default 0 applies, and 1052 pixels are wrong against the salt-90 reference.
- **Probe `unspecialised`:**
  - The function taken from `makeFunction(name:)` and used in a pipeline asserts, in the plain run
    as under API validation: "fragmentFunction fs_gbuf cannot be used to build a pipeline state.
    Use newFunctionWithName:constantValues:... to get the specialized function".
  - This happens even though every constant has a default.
  - So a function with constants is always specialised on Metal, with an empty set if the caller
    gives none.
- **Compiler:** the five libraries compiled at run time as MSL 3.1 with no error. Metal's
  run-time compiler returns errors only: a `#warning` canary was not delivered by either form.
  Offline, `metal` 32023.922 warns twice on the frame's MSL, "writable resources in non-void
  vertex function", at each vertex function taking the read-only position buffer, which Slang
  emits `device` rather than `const device`. The other four files are clean.
  - **A question for the `slang` lane's cook policy:** can Slang emit `const device` for a
    read-only buffer, or does the cook allow this warning by name?
- **State:** the constant values by id; the reflected type of each id.
- **External synchronisation:** none.

### 7. A binary archive saved and reloaded in a second process with fail-on-miss, run without shader validation: yes

- **Used:**
  - `MTLBinaryArchiveDescriptor`, `makeBinaryArchive(descriptor:)` (11.0),
    `addRenderPipelineFunctions(descriptor:)` and `addComputePipelineFunctions(descriptor:)` for
    all thirteen pipelines (six render, seven compute), and `serialize(to:)` with a file URL.
  - The file is 228,208 bytes. `file` reads it as a universal binary holding an AIR MetalLib slice
    and an `applegpu_g13g` Mach-O slice. Its header (magic `0xcbfebabe`, then the fat header's
    20-byte entries) places the AIR slice at bytes 48 to 84,940 and the GPU slice at 84,944 to
    228,208.
  - On reload, each descriptor gets `binaryArchives = [archive]` and
    `makeRenderPipelineState` or `makeComputePipelineState` with `.failOnBinaryArchiveMiss`
    (11.0).
- **Shown:**
  - 13 of 13 pipelines found, with the frame exact, in the plain reload and under API validation.
  - **A binary archive works from a library compiled from source:** every library in both
    processes is compiled at run time from the MSL text.
- **The checks `rhi-metal` makes before `makeBinaryArchive`,** since fail-on-miss acts only after
  a parse that can crash:
  - **No archive under shader validation.** While `MTL_SHADER_VALIDATION` is set, the archive is
    neither opened nor serialised. The clean shader-validation runs, cold and reload, left it alone
    and exited 0.
  - **An identity beside the archive,** written with it and compared before the parse: the OS
    version string (here "Version 27.2.1 (Build 26B131)"), the device name and the highest Apple
    GPU family. The identity changed to another OS build was refused before the parse.
  - **The length and checksum of the bytes Metal reads.** Metal loads only by URL, so the bytes
    are read once, checked, written to a private copy, and the copy is loaded. The program's
    checksum is FNV-1a over 64 bits, enough to see a corrupt file here; it is not a choice for
    `rhi-metal`, which uses the facade's.
- **Controls, plain, with the checks:**
  - Missing file: refused, "cannot read the archive".
  - Truncated to 1000 bytes: refused on length.
  - Each sixteenth of the sweep below: refused on the checksum, 16 of 16.
  - `--salt 91`: the checks pass, `fs_gbuf` alone misses ("Unable to find function "fs_gbuf"
    (render pipeline) in binary archives."), and the other twelve are found. The specialisation
    is part of the key.
  - `--misuse heap`: the same single miss.
- **Controls, plain, without the checks (`--raw-archive`):**
  - Missing file: `makeBinaryArchive` fails, "Invalid URL".
  - Truncated to 1000 bytes: "... has an invalid format."
- **Corruption sweep, without the checks.** 4 KiB inverted at each sixteenth of the file, one
  process each:

  | Sixteenths | Where | Outcome |
  |---|---|---|
  | 0 | the header and the AIR slice | `makeBinaryArchive` refuses the file as invalid |
  | 1 to 5 | the AIR slice only | Accepted: 13 of 13 found with fail-on-miss and the frame exact |
  | 6 | the GPU slice | The process segfaults (exit 139) |
  | 7, 11 to 15 | the GPU slice | Accepted: 13 of 13 found and the frame exact |
  | 8 | the GPU slice | A bus error (exit 138); a segfault in the set before |
  | 9 | the GPU slice | Three pipelines miss with "Internal error during function compilation", and Metal prints raw bytes of the archive to standard output; a bus error in the set before |
  | 10 | the GPU slice | `makeBinaryArchive` refuses the file as invalid |

  - A hit does not read the AIR slice, so sixteenths 1 to 5 say nothing about bad GPU code. For
    7 and 11 to 15 the run does not show whether the inverted bytes were code that ran: "the
    frame exact" means only that every check passed.
  - The crash stacks were read in the first version's sweep, on an archive of eleven pipelines:
    `EXC_BAD_ACCESS` at 0x30 in `MTLArchiveLinkResolverImpl::decodeAIRNTSections`, under
    `-[_MTLBinaryArchive loadFromURL:error:]`.
- **Shader validation:**
  - **The good archive reloaded under it, without the checks:** the process segfaults (exit 139).
    The first version's crash report put it in `MTLMetalScriptBuilder::initFromScript`, under
    `-[_MTLBinaryArchive loadFromURL:error:]` from
    `-[MTLGPUDebugDevice newBinaryArchiveWithDescriptor:error:]`.
  - **An archive written under it, without the rule:** 193,452 bytes, which `file` reads as
    "Mach-O 64-bit bundle applegpu_g13g", with no AIR slice. Reloaded plain, it passes the
    identity, length and checksum checks, and all 13 pipelines miss with fail-on-miss; the frame
    is exact from the pipelines compiled after the miss. Reloaded under shader validation without
    the checks, it opens without crashing, and all 13 miss again. The checks cannot see it; only
    the rule keeps it out of the cache.
- **Not measured:** pipeline creation is not compared. The system's own shader cache served the
  cold runs: all thirteen pipelines took 0.8 to 8 ms per process in the final set, cold or on
  reload, under load, and 55 ms where three missed and were compiled. Library
  compilation now includes the offline `metal` calls, about 0.9 s per run.
- **State:** the archive file and its identity file, and the descriptors exactly as added
  (functions, specialisation, formats, `supportIndirectCommandBuffers`).
- **External synchronisation:** none in one process. The file is written once, then read.

### 8. Command buffers encoded on two threads and enqueued in a fixed order: yes

- **Used:** two command buffers made on the main thread and `enqueue()`d (10.11) as upload then
  frame. Two `Thread`s meet at a barrier, each encodes one command buffer and records the time,
  and they meet again. They are committed in the reverse order, frame first.
- **Shown:**
  - The encoding-only intervals overlapped in every one of the 97 processes of the final set that
    reached item 8, for example upload [1095661, 1096094] µs and frame [1095646, 1097403] µs in
    the final cold run. The overlap is reported, not checked: in the set before, 92 of 96
    overlapped, and in the other four one thread began after the other had finished.
  - The first version of this file recorded each end after the second barrier, so its intervals
    overlapped by construction.
  - The frame read textures the upload wrote, exact, so the enqueued order held against the
    commit order.
- **State:** the enqueue order, fixed before encoding.
- **External synchronisation:**
  - Each command buffer and its encoders are touched by one thread.
  - Enqueue happens on one thread, in order.
  - Commits may come in any order once enqueued.

### 9. A shared event waited on before it is signalled: yes

- **Used:**
  - `makeSharedEvent()` (10.14).
  - `encodeWaitForEvent` and `encodeSignalEvent` (10.14).
  - `signaledValue` set by the host.
  - `wait(untilSignaledValue:timeoutMS:)` (12.0).
- **Order:**
  - Queue A's copy waits for 2 and signals 3.
  - Queue B's address dispatch waits for 1 and signals 2.
  - The waiter on queue A is committed first.
- **Shown:**
  - After 200 ms the value was 0, the copy's destination untouched and the copy not completed.
  - The host then signalled 1.
  - The host wait for 3 returned true, and the copied values matched the reference.
  - The frame also began with a wait for a host signal (value 1) and was confirmed pending and
    untouched before it.
- **State:** each event's value and who waits for and signals which value.
- **External synchronisation:** the host's signal and wait.

### 10. A sampler: yes

- **Used:**
  - Two `MTLSamplerState`s, nearest with clamp to edge and nearest with repeat, each with
    `supportArgumentBuffers = true` (10.13).
  - Their `gpuResourceID`s and the sampled texture's are written into a 24-byte parameter block
    (`Texture2D` then `SamplerState s[2]`).
  - The block is read by a compute kernel, one threadgroup per sampler, by `SampleLevel` at u
    0.25, 0.75, 1.25 and −0.25.
- **Shown:** clamp gives texels 0, 1, 1, 0 and repeat gives 0, 1, 0, 1, as exact `rgba8Unorm`
  bytes.
- **Misuse `sampler-noab`** (the flag left false):
  - The samplers' resource IDs become pointer-sized values (0x7a7f182df0 against 0x1d in the
    clean run).
  - Every sample reads zero.
  - **Neither validator reports it.** Metal's limit on this device: the flag is required and is
    not checked.
- **State:** the sampler descriptors.
- **External synchronisation:** none.

### 11. A drawable from a layer in no window, three in flight, on a second queue that waits on an event the first signals: yes

- **Layer:** a `CAMetalLayer` that is never added to a window, with `bgra8Unorm`, 64×64,
  `maximumDrawableCount = 3` and `framebufferOnly = false`.
- **Main thread, queue A:** each of six frames
  1. waits on `released` f − 2;
  2. clears offscreen target f mod 2;
  3. signals `ready` f.
- **Present thread, queue P:** each frame
  1. takes one of three slots of a semaphore released by the command buffer's completion handler;
  2. encodes a wait on `ready` f;
  3. calls `nextDrawable()`;
  4. draws the target into the drawable's texture;
  5. signals `released` f;
  6. calls `present(_:)` (`presentDrawable:`) and commits.
- **Shown:**
  - six drawables, none nil;
  - the last drawable's texture read back as BGRA `ff3c648c` at every pixel, frame 6's colour;
  - every command buffer completed without error.
- **Probe `drawable-timeout`:**
  - Eight `nextDrawable()` calls with none presented all returned a drawable.
  - The layer's cap of three did not bind for a layer in no window.
  - The three in flight here are the program's own semaphore.
- **Waits:** `nextDrawable()` took 0.5 to 10 ms per call. In some exploratory runs after the probe
  it took about 1.03 s and still returned a drawable; the review saw one block 1,034,222 µs and
  return a drawable. `allowsNextDrawableTimeout` is true: the documentation says the call can
  block for up to a second and return nil. It never returned nil here.
- **State:** the drawable until it is presented; the frame's slot; the event values.
- **External synchronisation:**
  - the frames-in-flight limit;
  - the order of `ready` and `released` across the two queues;
  - `nextDrawable` blocks the CPU and takes no caller timeout.

### 12. A readback against a CPU reference: yes

- **Method:**
  - The output attachment is copied by a blit into a shared buffer at the end of the frame.
  - Every pixel centre is classified against the geometry.
  - A pixel within 1/16 px of an edge accepts either neighbouring value; there were 16 such
    pixels.
  - All four `rgba16Float` channels must be equal and accepted.
- **Final clean runs:** 1052 drawn, 480 occluded and marked after the read, 152 from the indirect
  command buffer, 2396 outside, 16 in the margin, 0 wrong.
- **Mutations, each caught by the readback alone:**
  - `vertex` (one vertex moved one pixel): 20 wrong;
  - `draw`: 1052 wrong;
  - `clear`: 2403 wrong;
  - `salt0`: 1052 wrong.

### The heap's shader form

- **Chosen: the fixed-size array in a parameter block.**
  - Its slot is one number on both targets: the argument buffer's index on Metal and the binding
    array's index on SPIR-V (`heap.t`, set 1, binding 0, `OpTypeArray` of 1024).
  - For a heap of one resource type, the heap's capacity, null's slot rules and the RHI spike's
    single binding array apply unchanged. A heap of more than one type is under "Not shown".
  - Its marker is the inlined helper, and the SPIR-V carries `NonUniform` on the access.
- **The other form, `DescriptorHandle<Texture2D<uint4>>` in a structured buffer, also ran:** four
  slots read back exact through four handles.
  - On Metal it emits a `device` struct holding a `texture2d`, so the CPU writes an
    `MTLResourceID`.
  - On SPIR-V the same struct is a `uvec2`, an index into a runtime array named
    `__slang_resource_heap` that Slang declares itself at set 1, binding 2.
  - The value the caller writes therefore differs by backend, and on SPIR-V the binding is
    Slang's choice, not the facade's.
  - Its `nonuniform` marker decorates the SPIR-V correctly.
- **Cooker detail:** the module with a single entry point (`handle.slang`) names it `main` in
  SPIR-V. The modules with several keep their names.

### Not shown

- **An indirect dispatch.** It is in section 12.1, not in the plan's list.
- **A GPU-encoded indirect command buffer.** Slang cannot emit it.
- **A layer in a window.** The compositor's drawable cap, `nextDrawable` returning nil after its
  timeout, and present timing.
- **A metallib cooked by the cooker or the runner.** Every library here was compiled from MSL at
  run time.
- **`MTLResidencySet` and `useHeap`.** Residency was declared per resource.
- **A binary archive across an OS update or on another device.**
- **Errors at execution.** No command buffer failed; the `.encoderExecutionStatus` path ran only
  in its success form.
- **Metal 4.** `MTLGPUFamily.metal4` is not in the command line tools' SDK 15.2.
- **A good archive opened under shader validation.** The load segfaults, and `rhi-metal` opens
  none while it is on (item 7).
- **A heap of more than one resource type.** The review's probe: one parameter block holding
  `Texture2D` and `TextureCube` arrays becomes SPIR-V set 1, bindings 0 and 1, and one Metal
  argument buffer with two arrays; two blocks become sets 1 and 2, and buffers 0 and 1. A slot is
  then per type. The capacity (1024 here) is part of the cooked shader's type, so the cooked
  format names each array's type and capacity.
- **GPU-written indirect arguments,** as CREAM's cull pass writes them. Every record here was
  written by the CPU. Written by a compute pass and read by a later draw, they are item 2's
  lagging-writer case, and through an address, its residency case too.
- **Why an undeclared reader was ordered after the address dispatch** (item 2). Bisecting found
  the dispatch; no smaller probe reproduced it.

## Validation runs

Every clean run below is the final binary, exit 0, with the summary line "0 failed checks, 0
command buffer errors, 0 compiler errors, 2 offline warnings (0 unexpected)". The only lines that
are not the program's own are the validation banners.

| Run | Settings | Clean result | Misuse and what reported it |
|---|---|---|---|
| Plain | none | exit 0, cold and reload; also with `--track late` | `unspecialised`: asserts, exit 134 (item 6). `msl-canary`: "metal offline: UNEXPECTED: frame.metal:1:2: warning: "spike-metal canary" [-W#warnings]", exit 1 |
| Metal API validation | `MTL_DEBUG_LAYER=1 MTL_DEBUG_LAYER_WARNING_MODE=nslog` ("Metal API Validation Enabled") | exit 0, cold and reload | `msl-canary`: as in the plain run, exit 1. `copy` (a buffer copy past the source's end): "-[MTLDebugBlitCommandEncoder copyFromBuffer:sourceOffset:toBuffer:destinationOffset:size:]:1224: failed assertion `Copy From Buffer Validation (sourceOffset + size)(37900) must be <= [sourceBuffer length](1024).", exit 134. `memoryless-store`: "_MTLDebugValidateRenderPassDescriptorAndTrackAttachments:370: failed assertion `RenderPass Descriptor Validation Memoryless attachment content cannot be stored in memory.", exit 134. `unspecialised`: "validateWithDevice:5101: failed assertion `Render Pipeline Descriptor Validation fragmentFunction fs_gbuf cannot be used to build a pipeline state. Use newFunctionWithName:constantValues:... to get the specialized function", exit 134 |
| Metal shader validation | `MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1` ("Metal GPU Validation Enabled") | exit 0, cold and reload, the archive neither opened nor serialised (item 7) | `heap` (function constant 1 makes the last band read unwritten slot 6): "Invalid Metal usage: Null texture access MTLTexture:<resource description unavailable> used in fragment function: "fs_gbuf", pipeline: "gbuf" ... Error detected in drawIndexedPrimitives:indexType:indexBuffer:indexBufferOffset:indirectBuffer:indirectBufferOffset: in encoder with label "frame" GPU stack trace: * frame #0: fs_gbuf() - /program_source:57:87", 161 pixels wrong. `heap-resident` (slot 300 left out of `useResources`): "Invalid Metal usage: Texture is not resident or MTLResourceUsage flags specified in useResource does not match shader usage of texture MTLTexture:(label:"heap slot 300", gpuResourceID:3) used in fragment function: "fs_gbuf" ...", with the same encoder, stack trace and allocation backtrace; 477 pixels wrong. `address` (the source address moved 0x240000 past the buffer): "Invalid Metal usage: Invalid load in 'device' or 'constant' address space at offset 2360064, executing compute function: "cs_address" MTLBuffer: src, length:1024, gpuAddress: 0x..., resident:Read Only ... Error detected in dispatchThreadgroups:threadsPerThreadgroup: ... GPU stack trace: * frame #0: cs_address() - /program_source:66:50"; 0 of 256 values equal. `address-resident` (the source left out of `useResource`): "Invalid Metal usage: Invalid load in 'device' or 'constant' address space at offset 0, executing compute function: "cs_address" MTLBuffer: src, length:1024, gpuAddress: 0x..., resident:No ..."; 0 of 256 values equal. `track-undeclared`: "Invalid Metal usage: Texture is not resident or MTLResourceUsage flags specified in useResource does not match shader usage of texture ..." for the tracked texture |
| Both together | all four variables | exit 0, cold with `--no-archive` | |

Under shader validation every misuse's command buffers still completed with status completed and
no error ("0 command buffer errors"). The messages reach stderr only. API validation's errors are
assertions that abort the process. Neither validator sends anything to the program.

Probes, each in its own process. "Silent" means no message from the validators the probe ran
under.

| Probe | What it changes | Readback | Validators |
|---|---|---|---|
| `heap-resident`, plain | A heap slot not made resident | Exact, because the upload finished first (item 2) | Only shader validation reports it (above) |
| `address-resident`, plain | An address-reached buffer not made resident | Exact, as above | Only shader validation reports it (above) |
| `track-undeclared`, before the frame | A texture reached through an argument buffer left out of `useResource`, its writer made to lag | Two command buffers: all 4096 stale, 3 of 3 plain and under API validation; one command buffer: exact | Silent under API validation; shader validation reports it |
| `track-undeclared`, `--track late` | The same, after the address dispatch | Exact, 3 of 3 | Not run under validation |
| `icb-resident` | The indirect command buffer's own index buffer left out of the declaration | Exact | Silent: plain, API, shader |
| `sampler-noab` | Samplers made without `supportArgumentBuffers` | 8 of 8 samples zero | Silent: API, shader |
| `uuwp-used` | Slot 5, which the pending frame reads, rewritten while it is pending | 99 wrong | Silent: plain, API, shader |
| `no-fence` | The alias fence dropped (item 4) | `render`: all of A wrong in 3 of 3 plain runs | Silent under API validation, where the values are wrong another way (all of B in the final set; 2,496 of A and 63,040 of B in the set before); hidden under shader validation (exact) |
| `unspecialised` | A function with constants used without specialisation | Asserts | Reported in the plain run too |
| `drawable-timeout` | Eight `nextDrawable` with none presented | All returned | n/a |
| Reload under shader validation, `--raw-archive` | The archive opened with shader validation on | Segfault, exit 139 | n/a |
| Written under shader validation, `--raw-archive` | An archive serialised with shader validation on, then reloaded | 0 of 13 found, plain with the checks and under shader validation without them | n/a |
| `archive-identity-os` | The identity file names another OS build | Refused before the parse | n/a |

## The RHI spike's fourteen points

From "What `rhi-api` must expose" in `docs/results/spike-rhi-apple-m1-2026-11-15.md`.

1. **Device creation against two sets: kept.**
   - Metal's required set: argument buffers tier 2; Metal 3 (`gpuResourceID` and `gpuAddress`,
     macOS 13); MSL 3.1 (macOS 14); an Apple family for memoryless and framebuffer fetch.
   - Reported on Metal:
     - a memoryless attachment keeps its contents across a local read: yes (item 3);
     - multi-draw-indirect: no; Metal draws one indirect record per call;
     - indirect count: no; Metal has no draw-count call;
     - 64-bit atomics: not probed.
2. **Queues by role: kept; the release and acquire are Vulkan's alone.** Metal queues have no
   families and no ownership. Work crosses queues by an event value (items 9 and 11), which is the
   facade's timeline.
3. **Buffers: kept.**
   - The device address is `gpuAddress`, a `u64`.
   - Shared memory on the M1 is coherent, so flush and invalidate are no-ops.
   - A buffer reached through its address must be in the pass's declaration (point 5).
4. **Images: changed, one rule added: a memoryless attachment is never stored.**
   - Metal asserts on a store; Vulkan allows one.
   - The facade gives a memoryless attachment the store action don't-care, and null refuses a
     store.
   - The memoryless request and its queryable outcome stay as they are. Native Metal honoured it
     for a colour attachment read by framebuffer fetch and for depth.
5. **One bindless heap: kept.**
   - On Metal it is an argument buffer of `MTLResourceID`s.
   - The per-pass declaration of heap slots and buffers becomes `useResources` per encoder. It is
     required. Its absence shows under shader validation, and as stale reads while the resource's
     writer in an earlier command buffer runs: the declaration also brings the resource under
     Metal's hazard tracking (item 2). So the read or write usage it is derived with must be
     exact.
   - The `uuwp-used` rule is silent on Metal too, so null keeps it.
   - The `heap-layout` rule is Vulkan's alone, since Metal has no layouts; null keeps it for
     Vulkan.
6. **Pipelines with specialisation constants: kept.**
   - The Slang id is the Metal function-constant index, and Metal reflects each constant's type,
     so "id to bytes" holds and `rhi-metal` checks the length against the reflected type.
   - `rhi-metal` always specialises, with an empty set when the caller gives none, because Metal
     refuses an unspecialised function that has constants.
   - Write masks, formats and depth state carried over unchanged.
7. **The pipeline cache as an opaque blob with the facade's length and checksum: kept.** It is
   needed more on Metal than on MoltenVK:
   - A corrupt archive can crash the process in `makeBinaryArchive` or be accepted silently.
   - `MTLBinaryArchive` loads only from a file URL, so `rhi-metal` hands Metal a file, which the
     facade has checked.
   - Fail-on-miss is the warm-pipeline signal; specialisation values are part of the key. It acts
     only after the parse, which is where sixteenths 6 and 8 crashed.
   - Before `makeBinaryArchive`, `rhi-metal` checks two things. The facade's length and checksum
     are taken over the exact bytes Metal reads: Metal loads by URL, so `rhi-metal` checksums a
     private copy and loads that copy. An identity written beside the archive (OS build, device
     and GPU family) is compared before the parse.
   - `rhi-metal` neither opens nor serialises an archive while shader validation is on: a load
     segfaults, and one written under it misses every pipeline.
8. **Encoders, one per thread: kept.** A Metal command buffer encoded on its own thread is the
   encoder. The submit order is the enqueue order, which `rhi-metal` fixes by enqueueing a
   submit's command buffers in submit order.
9. **Resource states: kept; the layouts are Vulkan's alone.**
   - On Metal the enum yields the residency usage (read or write) for tracked resources. That
     usage must be exact, since the declaration is also what Metal's hazard tracking orders
     (point 5).
   - For untracked aliased memory it yields the fences of the alias call (new point 16).
10. **A render pass with the local read as its own call: kept.** On native Metal the local read is
    nothing, and nothing was needed (item 3).
11. **Timeline semaphores and fences: kept.**
    - `MTLSharedEvent` gives host signal, host wait with a timeout (macOS 12) and value query.
    - A submit's fence is the shared-event value it signals; `MTLFence` is `rhi-metal`'s means
      for point 16. An `MTLFence` orders encoders of one queue on the GPU and cannot be waited on
      by the host, so it is not the facade's fence.
12. **Indexed indirect draws: kept.** The argument record is the same 20 bytes on both. Indirect
    count is reported no on Metal.
13. **Copies: kept.** Buffer to buffer, buffer to texture and texture to buffer, all blits.
14. **A message sink: kept, in Metal's form.**
    - API validation's errors abort the process, so a test runs in its own process under nextest
      and an abort is a failure.
    - Shader validation's messages reach stderr only, while the command buffer completes without
      error, so the sink reads stderr.
    - Warnings are not delivered at run time; the cook's `metal` call is where they are seen.

## What `rhi-api` must expose, and must not

The fourteen points above, with point 4's rule. Three points are added; each can be honoured by
Vulkan and by null.

15. **Samplers** made from a descriptor, which the caller references by index into a sampler
    table the backend owns.
    - `rhi-metal` sets `supportArgumentBuffers` on every sampler. Without it reads are zero and
      both validators are silent.
    - The table holds at most `maxArgumentBufferSamplerCount` samplers, 1024 here.
16. **Aliased transients:**
    - a block of device memory and images placed at offsets in it;
    - an alias call, "the image placed here now begins", between the last use of one alias and
      the first use of the next. The new image's contents are undefined.
    - **Metal:** an untracked placement heap, with a fence updated after the last use and waited
      on before the first.
    - **Vulkan:** a barrier from the undefined layout.
    - **Null:** fails any use of an image whose memory another image used, with no alias call
      since.

    A dropped alias call shows as wrong pixels on Metal only sometimes, and no validator reports
    it.
17. **The drawable:**
    - an acquire that returns an image or none within a bounded time on every backend, with no
      caller timeout. Metal's `nextDrawable` has its own one-second limit; `rhi-vulkan` acquires
      with a finite timeout, so the present thread (D153) sees shutdown and resize. Here it
      always returned an image; "none" rests on Metal's documentation;
    - a present that is part of a submit;
    - the frames-in-flight limit, kept by the caller, since a layer in no window did not enforce
      its own.

`rhi-api` must not expose:
- **Indirect command buffers.** Vulkan and null have no equivalent short of device-generated
  commands. Slang cannot emit the GPU form. The CPU form is a recorded list the backend does not
  need, and its residency rule is unchecked.
- **Residency calls of any kind.** They are derived from each pass's declaration. This reads
  section 12.1's "residency calls where the backend needs them" as calls the backend makes
  itself (below).
- **`MTLResourceID`s, `DescriptorHandle` values or heap objects.** These are backend numbers; the
  facade deals in slots.
- **A timeout on the drawable acquire.**
- **A choice of hazard tracking.** Aliased memory is untracked and everything else is tracked,
  which is the backend's decision. Declared argument-buffer resources rely on Metal's tracking
  (item 2), which section 12.1 and D45 say not to rely on (below).

What `rhi-null` must check, beyond the RHI spike's five rules (`local`, `split`, `no-qfot`,
`heap-layout`, `uuwp-used`, all kept):
- a memoryless attachment with a store action other than don't-care (`memoryless-store`);
- a use of a placed image with no alias call since another image used its memory (`no-fence`).

For the conformance suite on the M1:
- the cases that touch the heap or device addresses run under Metal shader validation, the only
  thing that reports missing residency here, and once more without it, with the writer in an
  earlier command buffer made to lag;
- the pipeline-cache cases run without it.

## Against the architecture

Three conflicts, for the freeze decision at milestone 2's exit (D153) and for the plan's section 8.
No decision entry is proposed here; the freeze decision is where they are settled.

1. **Section 12.1, "one bindless heap; residency calls where the backend needs them",** against
   "residency calls of any kind" in the list above. This result reads 12.1 as calls the backend
   makes itself, derived from each pass's declaration. The freeze decision either amends 12.1 to
   say so, or keeps the sentence and records this reading.
2. **Section 12.1, "Metal backends handle aliased transients and argument-buffer residency
   explicitly rather than relying on automatic hazard tracking", and D45's premise that "automatic
   hazard tracking does not cover heap-aliased transients or argument-buffer resources".** Here a
   resource reached through an argument buffer and declared with `useResource` was ordered by
   Metal's tracking, and one left undeclared was not (item 2). Point 9 and "a choice of hazard
   tracking" rely on that tracking for every declared resource, and handle only aliased memory
   explicitly. The freeze decision amends 12.1 and D45 either way: to "argument-buffer resources
   are declared per encoder, which makes them resident and brings them under Metal's tracking;
   aliased transients are untracked and fenced", or to an untracked `rhi-metal` with its own
   fences for everything, which this spike did not show.
3. **D43 names indirect command buffers** among the Metal features that would force breaking
   changes on a `rhi-api` frozen after Vulkan alone. This result keeps them out of `rhi-api`. The
   freeze decision records that they are out, and whether `rhi-metal` may still add them as an
   amendment (for a GPU-encoded form, if Slang gains one) or D43's warning lapses.

## The program

The program is not in the repository. Its hash is taken over the sorted list of its files: the
build script, the Swift source, and the Slang and the MSL of each shader. The SPIR-V files are
excluded; they are build outputs, read only for decorations. Run from the program's directory:

```
find build.sh src shaders -type f ! -name '*.spv' | LC_ALL=C sort | xargs shasum -a 256 | shasum -a 256
```

`6b3e7d7eca225ed67d0c707f6f4f4304b5ee8b4c8c4c346a08205f76439b7703`, over 12 files: `build.sh`,
`src/main.swift`, and `shaders/compute`, `frame`, `handle`, `present` and `track`, each as
`.slang` and `.metal`. The first version's hash was over 10 files, before `track`.
