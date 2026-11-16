# Milestone 2 plan: Pixels

Status: draft, not approved. Written at main `6f0508a`, architecture revision 3.3, on 16 November
2026, ahead of milestone 1's exit by the owner's ruling of that day. Revisions 2 and 3, at main
`0810f9f`, answer its two rounds of review, each a general one and a GPU one. Revision 4 folds
in the owner's answers of 16 November 2026 and the closing reviews' six findings. It is re-based
on main and approved only when milestone 1 exits, and the coordinator applies it then.
This plan maps each milestone 2 criterion of architecture section 22, and every
`docs/verification.md` row that gates milestone 2, to a lane, the coordinator or the owner. It
orders the lanes and takes the decisions that order needs, the two of section 22.1 that are due by
milestone 2 among them. It proposes three `docs/decisions-log.md` entries and the verification row
changes that go with them (section 6). It amends no architecture sentence: revision 3.4 is a
separate pass, and section 8 lists what it must settle. Section 9 names every place where this
plan depends on how milestone 1 exits. Section 10 says what it could not verify.

Conventions:

- A verification row is cited by its line number in `docs/verification.md` at `0810f9f` and by its
  opening words, since line numbers move. D148, D149 and the coordinator's exit pass are on main
  there.
- A name in code font such as `rhi-null` is this plan's handle for a lane (section 3). Where it is
  also a crate's short name, the lane is that crate's.
- Cost is counted in agent runs, as `docs/plans/milestone-1.md` counts it. An implementation lane
  is seven: the lane, its review, about two fix rounds with a diff review each, and the merge. A
  second or third reviewer beside the first review adds one each: `reviewer-gpu` on work that
  records, submits or synchronises GPU work, defines the surface it goes through or produces what
  it runs, and `reviewer-concurrency` on work that runs on several threads (`docs/agents.md`,
  section 2). A third fix round this plan expects adds two. A small lane is five. A facade is six,
  and seven where `reviewer-gpu` reads it too. A spike that delivers a result file is seven, as
  the RHI spike was. The totals are a floor.
- "The discrete GPU" is the Windows laptop's. "The desktop" is the owner's x86 desktop. "The M1"
  in milestone 2 is the development Mac on macOS 26: the owner upgrades it once milestone 1's exit
  is recorded and before milestone 2's first lane (section 9).
- A probe is a command run on the development Mac on 16 November 2026, outside the tree: this
  plan's, or one of its reviews'. Section 10 lists them.

## 1. The criteria

Section 22 gives milestone 2 seven proofs. At `0810f9f` none of the eight crates it delivers
exists, nor the client binary; `crate-tiers.toml` already names each of them. The RHI spike has
run under MoltenVK (`docs/results/spike-rhi-apple-m1-2026-11-15.md`).

| # | Criterion | What proves it | Machine | File or run | Closed by |
|---|---|---|---|---|---|
| 1 | A triangle through the graph on Windows and macOS | The client binary in its fixed-frame mode: a fixed clock, a scripted input stream, a local world whose rows move, extract each frame, and instanced triangles at the extracted transforms, drawn by a `render-graph` pass after the scatter pass into an offscreen target, which the present thread draws or blits to the swapchain image. The run dumps what it drew the last frame with. A CPU reference is rasterised from the dump and compared with the offscreen target's readback, each validator is shown on, and none reports anything else (decision 5.1) | Windows: the discrete GPU. macOS: the M1 under MoltenVK (decision 5.7) | `triangle` | `frame-impl`, `measure-m2` |
| 2 | MoltenVK, KosmicKrisp and native Metal spikes recorded, and `rhi-api` frozen or amended on their findings | Three records and a decision. MoltenVK: the RHI spike's file; the one shape it still owes, where the drawable blocks, shown by `swapchain`; and its open cost figures through `rhi-vulkan` (decision 5.6). Native Metal: a throwaway program on the M1. KosmicKrisp: `rhi-vulkan`'s conformance suite, the swapchain's cases and the cost figures under it on the M1, beside MoltenVK's (decision 5.7). The planner's exit decision that records the freeze (decision 5.13) | The M1, on macOS 26 | `spike-rhi` (on main), `spike-metal`, `rhi-conformance` and `gpu-cost` under both drivers; `docs/plans/rhi-freeze-decision.md` | `spike-metal`, `vulkan`, `swapchain`, `measure-m2`, the exit |
| 3 | Overlap with exclusive extract | Tests of the client's frame, by ordering and never by timing. (a) No stage of a tick starts before extract's pass has resolved, and no frame reads the snapshot buffer or the ring region extract is writing. (b) Frame N's record and tick N+1's stages are in flight at once: each is held at a barrier until the other has started, so the test hangs if they run in turn. (c) A recording test steps one world with the render side attached, on `rhi-null`, and without it, and SOUR finds the hashes equal in every mode | The M1 at the merge; the three CI platforms for (c) | The nightly's run id, in the row | `frame-design`, `frame-impl` |
| 4 | Extract at most 1 ms for 50k dirty rows | A cost test: 100,000 rows with a `Transform`, 50,000 of them moved every tick, extract's pass alone, the median of 60 passes, writing the device's ring (decision 5.2) | A quiet M1; the desktop | `extract-50k` | `present-impl`, `renderer-impl`, `measure-m2` |
| 5 | Remote transform upload at most 1 ms for 5k entities at 144 Hz | A cost test: 5,000 slots of the interpolation buffer written at 30 Hz, sampled in one batch and written to the staging ring every frame, paced at 144 Hz for ten seconds, the median frame (decision 5.3) | A quiet M1; the desktop | `remote-upload-5k` | `presentation`, `measure-m2` |
| 6 | lavapipe CI | A job in the pull-request workflow on linux-x64: `cargo xtask gpu --driver lavapipe`, the conformance suite and the graph's and the renderer's device tests under a pinned Khronos validation layer, each validator's canary seen and no other message (decision 5.10) | The linux-x64 runner | The first green run's id, in the row | `vulkan` |
| 7 | macOS shader runner | A job on macos-arm64 that cooks every engine shader to SPIR-V, MSL and a metallib with the pinned Slang and the runner's `metal` compiler, as MSL 3.1 for macOS 14, the oldest client, loads each metallib and makes a pipeline state from it with its function constants. The linux-x64 job cooks SPIR-V and MSL, and the two hosts' hashes are equal. One metallib the runner cooked is loaded on the M1, and each on the oldest macOS image the hosted runners offer, if it has a Metal device (decision 5.8) | The macos-arm64 and linux-x64 runners; the M1 once | The first green run's id, in the row | `slang` |

Section 22.1 puts two decisions at milestone 2. The Apple interim path is decision 5.7. Crash
reporting is decision 5.9.

Milestone 1 sent three measurements here. The stall the physics phase imposes on render recording
(row 21, architecture section 16) is `record-stall`, recorded with no threshold (decision 5.16).
The presentation snapshot's budget (section 5.3) is reported inside `extract-50k`, as the
snapshot's part of the pass. The remote upload's cost at 144 Hz (section 5.3) is criterion 5.

One outcome is not section 22's and binds the milestone all the same: every check job of the
pull-request workflow stays inside ten of its fifteen minutes, by the median of its three most
recent warm runs of main (decision 5.15).

## 2. The verification rows

Every row whose gate names milestone 2 has an owner below, or is re-gated with its reason. "Pass"
is the coordinator's verification pass. Section 6 gives each row's change as its gate and the gist
of its new text.

| Row | Opening words | Owner | What closes it |
|---|---|---|---|
| 15 | MoltenVK and KosmicKrisp | `vulkan`, `swapchain`, `measure-m2` | Both drivers on the M1 under macOS 26. The conformance suite under each gives the capability answers: indirect-draw behaviour, memoryless attachments and local reads. `gpu-cost` under each gives the encode time for ten thousand draws and the pipeline creation cost |
| 16 | Extract cost | `present-impl`, `measure-m2` | `extract-50k`: the pass's time, and the cost per dirty row on one worker |
| 18 | Native Metal | `spike-metal`, then the freeze decision | The spike's result file: argument buffers, residency, memoryless attachments, framebuffer fetch, indirect draws. `rhi-api` freezes at `vulkan`'s merge, and the exit decision records what is frozen and what was amended |
| 19 | Remote transform upload | `presentation`, `measure-m2` | `remote-upload-5k` |
| 21 | Physics phase stall | `frame-impl`, `measure-m2` | `record-stall`, recorded |
| 34 | MoltenVK multi-draw-indirect is a CPU loop | `measure-m2` | `gpu-cost`: one multi-draw of N commands against N draws under MoltenVK, beside a reading of the pinned MoltenVK |
| 35 | Slang's Metal target supports argument buffers, function constants and framebuffer fetch | `spike-metal`, `slang` | The spike's shaders are MSL the Slang compiler emitted and use all three. The cooker's are then held by a pipeline state made from each: on the M1 from the cooked MSL, compiled through Metal at run time, and on the runner from the metallib |
| 63 | Windows efficiency-core steering | `eco-qos` | The benchmark's rows with two background threads, before and after, on the laptop, which is a hybrid part |
| 95 | `lactose-input` is implemented | `presentation` | The camera rig's forward and yaw equal `PlayerInput::aim` and `Yaw::rotation` for pinned samples |
| 97 | Artist-defined surfaces behind fixed pipelines | Re-gated to 3 | Decision 5.14. `slang` proves the mechanism only |
| 99 | `lactose-predict` is implemented | `presentation` | A re-spawned body without `Teleported` interpolates between two snapshots, and one with it does not |
| 102 | Interpolation slot footprint | `interp-loom`, `measure-m2` | A loom model of the sequence lock. The reader's retry rate under one writer at 30 Hz and the footprint are in `remote-upload-5k` |
| 109 | The line-ending rule on a Windows host (D148) | Pass; the owner | Its CI half is met: run 43795852109 at `8869d2c` read "line-endings: 889 files read, none refused" on the three hosts, and the Windows cache key's last hash equals the other two's. What remains is the owner's refresh of the laptop's checkouts |
| 134 | The milestone 1 crash hook | `crash-hook` | Decision 5.9: the hook is `lactose-app`'s, and the report also goes to a file |
| 136 | SILO's load call against section 5.1's 0.5 ms slice | `slang`; the rest re-gated to 3 | Milestone 2 cooks no mesh pool and no texture. The shader kind's load is measured against the slice (decision 5.14) |
| 139 | Mounted packs are never unmounted and published versions never unloaded | Re-gated to 3 | The owner's answer 4: the directory watch moves to milestone 3, and its dev session with it |
| 140 | Mount cost | Re-gated to 3 | The owner's answer 4: D149 sends the Linux and Windows figure to the watch loop's dev session |
| 143 | The cooked pack set of a fresh process | Re-gated to 3 | The owner's answer 4: the watch's half |
| 150 | The RHI spike ran under MoltenVK alone | `vulkan` | The suite under lavapipe settles the spike's eight design questions where lavapipe can: it has one coherent memory type and one queue. The four questions the spike left to hardware are probed on the laptop's devices and recorded, and a run that shows nothing discharges no rule of null (decision 5.17). Question 2 needs a lazily allocated memory type, which lavapipe and the PC drivers surveyed do not offer: KosmicKrisp answers it on the M1 if it offers one, and `spike-metal` answers it for Metal |
| 164 | The keeper's timed wait on Windows | `frame-design` | Its second half asks that the unmeasured list be read where frame pacing on Windows is designed. `frame-design` reads it. An item its design uses becomes a measurement it names for `measure-m2`, and the rest is re-gated with its reason |

## 3. The lanes

One lane of milestone 1 comes first and is not counted here: `test-binaries`, D125's lever, which
builds each existing package's integration tests as one binary and writes the rule a new crate
follows. No milestone 2 lane starts before its merge, nor before the Mac is on macOS 26
(decision 5.15, section 9). Each row below is the size of a brief.

**Queue 1, the RHI.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `spike-metal` | The native Metal spike: a throwaway program on the M1 against Metal itself, never checked in, as the milestone 1 plan's decision 5.10 ruled for the RHI spike. Its question: what must `rhi-api` expose for section 12.1's list to run on Metal with no call Vulkan or a null backend cannot honour? It walks: a bindless heap as an argument buffer indexed past slot 0 with a non-uniform index, in a shader form the Slang compiler emits and Metal accepts; residency declared per encoder, for the heap and for a buffer a shader reaches only through its `gpuAddress`; a memoryless G-buffer read in its pass by framebuffer fetch, and a memoryless depth attachment kept across that read; two transients that alias one untracked heap with the fence between them; an indexed indirect draw and an indirect command buffer; a pipeline from MSL the Slang compiler emitted (the SDK's copy, its version in the file), specialised by a function constant; a binary archive saved and reloaded in a second process with fail-on-miss, run without shader validation; command buffers encoded on two threads and enqueued in a fixed order; a shared event waited on before it is signalled; a sampler; a drawable from a layer in no window, with `nextDrawable`, a render into its texture and `presentDrawable`, three in flight, submitted on a second command queue that waits on an event the first signals, for the call's shape alone; a readback against a CPU reference | The Mac on macOS 26 (section 9). `reviewer-gpu` reviews it alone | Result file `spike-metal`. Each item yes or no with the state the caller carried. Metal API validation and Metal shader validation each shown on by a misuse it reports, a heap slot and a buffer dereference without residency among them. Whether the fence, dropped with the GPU made to lag, showed. Whether a binary archive works from a library compiled from source. The heap's shader form it chose, a fixed-size array in a parameter block or `DescriptorHandle`. The device comes from `MTLCopyAllDevices`, and the run fails when there is none. Each of the RHI spike's fourteen points marked kept, changed or Vulkan's alone. Rows 18 and 35 | 7 |
| `rhi-api` | Architect: the facade of `lactose-rhi-api`, from the RHI spike's fourteen points, `spike-metal`'s result and section 12.1. Capabilities are required or reported. The local read is a call of its own, and input i reads colour attachment i and nothing else. Memoryless is a request. A pass declares the heap slots and the buffers it reaches. Aliasing is a relation the backend fences. Device loss is a value. A surface is made from `raw-window-handle`'s types, so no backend names `lactose-platform`. Queues are named by role: graphics, compute and present. Every command that reads or writes a swapchain image is submitted by the present thread, on the present queue, which no other thread submits to where the device offers one in a family that can draw and present; otherwise the roles share the graphics queue. The offscreen target is two images or more, and the frame never writes the one the present thread holds (decision 5.6). The cooked shader format names the heap's shader form. The conformance suite's shape: its cases by name, each marked contract, which null passes too, or output, which only a device can | `spike-metal` merged | A facade, unstable (D43). `reviewer` and `reviewer-gpu` both read it. Every call is one a null backend and native Metal can honour. Decision text for what section 12.1 left open | 7 |
| `rhi-null` | `lactose-rhi-null` against the facade, and the bodies of the suite's contract cases in `lactose-rhi-api` behind a dev-only feature, as `lactose-asset-api`'s is. Null keeps the state the caller carries between calls and refuses each misuse the contract names, under the capability set it is created with | `rhi-api` merged | Every contract case passes on null. For every rule null keeps, a build with that rule removed fails a test: a state that does not match, by subresource and by queue; a use after destruction; a slot, a pool or a transient reused before the timeline value that frees it; a feature or capability not enabled; an alias read before its first write; a heap slot or a device address handed to a pass through `rhi-api`'s arguments that the pass did not declare; a range of memory that is not coherent left unflushed at a submit, under a capability set that has such memory; a local read mapped to another attachment; and the five misuses the RHI spike's validators missed (`local`, `split`, `no-qfot`, `heap-layout`, `uuwp-used`). What a shader reaches through data alone null cannot see: that is Metal shader validation's under `rhi-metal`, at milestone 3. Null is run under a one-queue capability set and under a several-queue one. Null injects a device loss. Null stays test support and outside the server (`cargo xtask tiers`) | 8 |
| `vulkan` | `lactose-rhi-vulkan` without a window: device creation against the required and reported sets, queues by role, buffers, images, samplers, the heap, pipelines from cooked SPIR-V with specialisation, the cache blob, an encoder per thread, states to barriers, the local read, memoryless requests, timeline semaphores and fences, indirect draws and dispatch, copies, the message sink. `SAFETY.md`. The bodies of the suite's output cases and their Slang sources. To `cargo xtask gpu`, which `slang` adds with one step, the drivers, the suite's run and the rule of decision 5.10. The `gpu (lavapipe)` job and its nightly form with GPU-assisted validation. `lactose-rhi-vulkan` joins `[test_support]` (decision 5.12) | `rhi-null` and `slang` merged; it may start when `rhi-api` merges. The facts of the coordinator's `gpu-probe` | The output cases, their shaders cooked by `cargo xtask gpu` before it runs them, pass under lavapipe in CI and under MoltenVK on the M1 by decision 5.10's rule. A buffer in device-only memory filled through a staging copy is a case and runs on the M1, where MoltenVK's first memory type is device-local alone. The pipeline cache falls back to an empty one on a corrupt payload, a truncated file, a wrong checksum and a stale header, and a hit is shown by the creation feedback's bit, never by time. After the push, the suite on each of the laptop's devices, selected by name, as labelled data, with what they show of the spike's four hardware questions (decision 5.17). The spike's eight design questions answered from the lavapipe run (row 150). The suite under KosmicKrisp on the M1, as that driver's record: the capabilities it reports and the cases it passes, a failure there recorded and no failed gate (decision 5.7); it answers the spike's question 2 if it offers a lazily allocated memory type. A cache directory that cannot be written is reported and the run goes on. `reviewer-gpu` and `reviewer-concurrency` read beside the first review. Pushed alone | 11 |
| `swapchain` | The swapchain and present, in `lactose-rhi-vulkan` and in `lactose-platform`'s tests: acquire; the present queue, in a family that can draw and present; two offscreen targets or more; the present thread's submit, which draws or blits the newest finished target into the swapchain image by the surface's format, and its present; a finished target's crossing to the present queue, by a timeline semaphore and an ownership transfer or by concurrent sharing; one present semaphore for each image; resize and out-of-date; shutdown with frames in flight; null's swapchain | `vulkan` and `platform-impl` merged | The tests that need a window or the present thread are `lactose-platform`'s, which may dev-depend on `lactose-rhi-vulkan` as a test-support crate, and run as a subprocess on macOS; the headless ones are `lactose-rhi-vulkan`'s. On null: with the present queue's submit held at a barrier, every frame submit on the graphics queue completes, each writing a target the present thread does not hold; a frame write to the held target is refused. Under MoltenVK: the same with the drawable made to block, by a minimised window or a sleeping display, whichever the lane shows blocks. The case counts only if the present thread recorded a block of a second or more; MoltenVK's "CAMetalDrawable could not be acquired" error is that case's expected message; the frame submits that complete while the drawable blocks write the targets, each within a frame, and no frame worker's job is held (decision 5.6). After that message the present thread treats `VK_ERROR_OUT_OF_POOL_MEMORY` from present or acquire as out of date and rebuilds the swapchain; the case shows the rebuild and a present after it. If neither way blocks, the lane says so and the row stays open. The same cases run under KosmicKrisp, recorded: whether that driver takes the drawable inside a submit (decision 5.7). On the M1 and the laptop, a fixed number of frames presented across a resize by decision 5.10's rule, an out-of-date swapchain rebuilt, and on the M1 the swapchain image read back once and equal to the offscreen target. An acquire semaphore is reused only after the submit that waited on it has completed, and one injected reuse is reported by a layer at 1.4.313 or later. The present fence of `VK_EXT_swapchain_maintenance1` is used where the driver offers it. A shutdown with frames in flight ends cleanly, and so does a device loss null injects. The headless tests run under nextest on the M1, since MoltenVK offers a headless surface, and under lavapipe if the runner's driver does; where a device has no second queue that can draw and present, the roles share the graphics queue under its lock. Both specialist reviewers read beside the first review | 9 |

**What the RHI spike left open, and where it lands.** The spike settled `rhi-api`'s Vulkan
shape: its fourteen points, and five rules for null. Its "Not shown" list is owned as follows.

| Left open by the spike | Lands in |
|---|---|
| The swapchain and present, and where the drawable blocks | `swapchain`; the call's shape on Metal in `spike-metal` |
| Residency, heaps, indirect command buffers, Metal libraries and the binary archive, aliased transients on Metal, parallel Metal encoding | `spike-metal` |
| A buffer in device-only memory filled through a staging copy | `vulkan`, as a case of the suite, on the M1 |
| An ownership transfer's effect; a heap image in the wrong layout | Null's rules. Probed on the laptop's devices and under GPU-assisted validation, as labelled data (decision 5.17) |
| A heap indexed without the non-uniform decoration | `slang`: the cook reads the SPIR-V it emitted |
| Memory that is not coherent | Null's rule, under a capability set that has it. No machine in reach has such memory |
| The eight design questions for a conformant driver; GPU-assisted validation of the address and heap misuses | `vulkan`, under lavapipe |
| Samplers; indirect dispatch | `vulkan`, as cases of the suite |
| Transients that alias memory on Vulkan; a missing barrier with the GPU made to lag | `graph-impl`, `renderer-impl`, by class of hazard (decision 5.17) |
| SPIR-V cooked by Slang | `slang` |
| Multi-draw-indirect's cost, the encode time for ten thousand draws, pipeline creation cost | `measure-m2`, as `gpu-cost` |
| 64-bit atomics | Milestone 3 (row 36) |

**Queue 2, the frame.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `platform` | Architect: the facade of `lactose-platform` [PAIL]: the window and its handles, the event pump on the main thread over `lactose-task`'s `LanePump`, the present thread and its pump, which carries every submit that touches a swapchain image, the `Platform` the client passes as `Workers::platform`, what a close and a resize report. Device sources and action mapping stay milestone 5's | No lane editing `lactose-app` in flight: it edits one sentence of `crates/app/README.md` | A facade and decision text. `reviewer` and `reviewer-gpu` both read it, since GPU work goes through the present thread. `crates/app/README.md`'s sentence on `Workers::platform` agrees | 7 |
| `platform-impl` | Implements it over winit on Windows and macOS. It builds on linux-x64, where CI lints it. `unsafe` for the present thread's QoS and the window handles, with `SAFETY.md` | `platform` merged | A test that runs a subprocess: it opens a window, pumps events, runs one call on the main thread and one on the present thread, and closes, on the M1 and the laptop. A worker that waits on the main thread fails a test. The Windows and Linux cross-target clippy. `reviewer-concurrency` reads beside the first review. Pushed alone | 8 |
| `graph` | Architect: the facade of `lactose-render-graph` [GOUDA]: passes with reads and writes by usage, imported and transient resources, the history ping-pong, compile and its cache key, record on frame workers, queue assignment, submission in graph order | `rhi-api` merged | A facade. `reviewer` and `reviewer-gpu` both read it | 7 |
| `renderer` | Architect: the facade of the minimal `lactose-renderer` of decision 5.4 and no more: the render slot table by `StableKey`, the staging ring's writer, which offers no read, the scatter, the instanced triangle pass, the latched camera buffer, `RenderFeature` | The `graph` and `present-api` facades merged | A facade. `reviewer` and `reviewer-gpu` both read it. Any math crate it takes leaves the shipped binaries' boundary feature trees identical (`cargo xtask features`) | 7 |
| `frame-design` | Architect: the client's frame in `lactose-app`, and what `lactose-app-client` wires. The frame's order: the ticks, `Simulation::run_passes`, the snapshot's publication, then the next ticks beside the presentation schedule, graph compile, record, latch and submit. What the frame renders into: two offscreen targets or more, of which the present thread holds the newest finished one and draws or blits it to the swapchain image on the present queue (decision 5.6). Who paces it against the present thread. The traits of `present-api` and `rhi-api` through which `app` drives the render side without naming a backend (decision 5.11). The physics reservation for render recording. Where criterion 3's recording test lives. Frame pacing on Windows, read against the keeper's unmeasured list (row 164), with any measurement it needs named for `measure-m2` | The `platform`, `graph`, `renderer` and `present-api` facades and `server-closure` merged. Milestone 1's exit step 7 routed (section 9) | A facade and decision text that amends D131's surface. `lactose-app`'s normal-dependency closure reaches no `[server_denylist]` crate, by the rule `server-closure` adds. `reviewer` and `reviewer-gpu` both read it | 7 |
| `graph-impl` | Implements the graph: order, culling, transient lifetimes, aliasing, transitions, queues, the capped compile cache, record on frame workers with one span for each pass, and one command pool for each recording job and frame in flight | `graph` and `rhi-null` merged; `vulkan` merged for its device tests | On null: a build that drops a derived transition fails a test, by null's state rule; an unreachable pass is culled and a pass whose only output is a readback or an import is kept; each input of the cache key, changed alone, forces a recompile; two transients alias, the second starts undefined, and a read before its first write is refused. On devices, by class of hazard (decision 5.17): a build that drops the barrier on a tracked attachment or buffer yields a synchronisation-validation message the gate counts; a hazard between the host and the GPU yields a wrong readback with the GPU made to lag; aliasing, reads through the heap, indirect and index buffers and ownership transfers rest on null's rules. A command pool is reset only after its frame's timeline value, held with the GPU made to lag (`VUID-vkResetCommandPool-commandPool-00040`). Where a device has one queue the roles share it: null runs the graph under both capability sets, the M1's four families run the several-queue path, and lavapipe's one queue never does. `reviewer-gpu` and `reviewer-concurrency` read beside the first review | 11 |
| `renderer-impl` | Implements the minimal renderer | `renderer`, `graph-impl`, `vulkan` and `slang` merged | Slots are allocated on the CPU, freed from the structural log and reconciled after a restore, shown with a prediction world's rollback. A batch names each slot once: the batch's builder resolves duplicates on the CPU in batch order and asserts it, a test on null holds it, a build without the resolution fails that test, and a device test puts many duplicates in different workgroups of a large batch. With the GPU made to lag, the ring never overwrites a region a frame in flight reads, and frame N's readback shows N's camera after N+1's has been written. An instance and the camera far from the origin in one cell render the pixels they render at the origin. A device test steps a world, extracts, renders and compares the readback with the CPU reference. Steering: the ring's write cost for 50k rows. Both specialist reviewers read beside the first review | 9 |
| `frame-impl` | The frame in `lactose-app` and the binary `lactose-app-client`: a local world, the cooked pack set mounted by the binary through `lactose_asset::pack_set`, since `lactose-app` takes no dependency on `lactose-asset` before milestone 3 (D136), the pipeline cache file, resize, and the fixed-frame mode of decision 5.1 | `frame-design`, `present-impl`, `presentation`, `renderer-impl`, `swapchain` and `crash-hook` merged | Criterion 3's three tests; the package that holds the recording test is a `[harness]` package. Criterion 1's run on the M1 and on the laptop, and each of its four mutations fails it. `lactose-app-client` joins `[feature_roots]`, and its boundary feature tree equals `lactose-app-headless`'s. `lactose-app`'s closure still reaches no `[server_denylist]` crate. Both specialist reviewers read beside the first review, and a third fix round is expected | 11 |

**Queue 3, shaders, presentation and tools.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `slang` | The Slang toolchain in `tools/cook` [RICOTTA]: the shader kind, cooked by the pinned Slang release to SPIR-V and MSL on every host and to a metallib on macOS (decision 5.8). The pin, a tracked file with the version and each host's archive checksum, and one command that installs it on a host. The cooked format `rhi-api` names, with the heap's shader form. Two checks on what the cook emitted: every access through the heap carries the non-uniform decoration in the SPIR-V, and every fragment output has one location on both targets. Slang's warnings are errors. On macOS the cook runs `metal` itself on the MSL it emitted, under a stable file name, as MSL 3.1 with macOS 14 as the deployment target. The `shaders` job. One engine shader is written against an interface specialised statically and a link-time constant, for both targets | `rhi-api` merged; `test-binaries` merged | Cooking twice writes the same SPIR-V and MSL, whatever the checkout's path; whether two cooks write the same metallib is recorded and nothing rests on it. Each check has its mutations for the heap form `spike-metal` chose, and the cook refuses each: the marker removed; for a fixed array, the marker inside a helper that is not inlined; for `DescriptorHandle`, `NonUniformResourceIndex` in place of `nonuniform`, which Slang only warns of; a shader whose one output is `SV_Target1`. Every cooked MSL is compiled on the M1 through Metal at run time and a pipeline state made from it with its function constants, by a Swift script under `tools/xtask`, run with the command line tools' `swift` by `cargo xtask gpu`, which this lane adds with that step alone: no `unsafe` outside section 21. It takes its device from `MTLCopyAllDevices` and fails when there is none. The cook's `metal` step runs on the M1 too, with the Xcode the owner installs after the upgrade. Criterion 7, with each metallib also loaded on the oldest macOS image the hosted runners offer, if it has a Metal device. A pipeline from the cooked SPIR-V passes the suite's pipeline case, once `vulkan` runs it. The shader kind's load against the 0.5 ms slice (row 136). The Slang behaviours the GPU reviews found on the SDK's copy are run again on the pin. `reviewer-gpu` reads beside the first review, and a third fix round is expected. Pushed alone | 10 |
| `present-api` | Architect: the facade of `lactose-present-api`: the mesh, light, decal and camera rig components, VFX and audio request events, UI view-model resources, the debug-draw buffer, and the presentation snapshot, triple-buffered. Extract is its pass system, as the gather is `net-api`'s (D121), and writes transforms through a sink the renderer's ring implements. It names no RHI, platform or `net-host` type, so it stays what `[boundary_internal]` admits | Nothing | A facade and decision text. The consumer rows of `crates/anim/README.md`, `crates/physics-api/README.md` and `crates/predict/README.md` agree | 6 |
| `present-impl` | The snapshot and extract | `present-api` merged. No other lane edits `lactose-ecs` while it does | By counts: the rows dirty in this tick or the one before are written and no other; a row that stopped moving has its previous transform set; a teleport carries the no-interpolate flag; the pass after a restore or a load is full. A reader that interpolates two snapshots never reads the third while extract writes it, shown by a loom model or under Miri, whichever the facade's design admits. `reviewer-concurrency` reads beside the first review. Steering: extract's time at 50k dirty rows on null | 8 |
| `presentation` | `lactose-presentation`: the schedule per frame on the snapshot, interpolation between two snapshots, the camera rig's orbit and boom, the batch sample of the interpolation buffer and the remote upload through the ring, presentation events delivered once (decision 5.5) | `present-impl` and the `renderer` facade merged | Rows 95 and 99. Each remote slot is sampled and written once a frame. A delivered event is not delivered again after a rollback. Steering: the upload's time for 5k slots. `reviewer-concurrency` reads beside the first review | 8 |
| `measure-m2` | `cargo xtask measure` gains `extract-50k`, `remote-upload-5k`, `record-stall`, `gpu-cost` and `triangle`, their tests in `lactose-app-client`, and the header of section 4 for a file taken on a device | `frame-impl` merged | One command writes every file on the M1. The measurements that need no device run short in the `measure` workflow, their numbers discarded. Times are printed, never asserted. `reviewer-gpu` reads beside the first review | 8 |
| `server-closure` | `cargo xtask tiers` applies `[server_denylist]` to `lactose-app`'s normal-dependency closure whether or not `lactose-app-server` exists. Today `server_denylist` in `tools/xtask/src/tiers.rs` returns when that package is absent, and it arrives at milestone 4 | `test-binaries` merged; no lane editing `tools/xtask/src` in flight | A fixture in which `lactose-app` reaches a denied crate fails with no server package, and passes with the edge removed. The tree at its base passes. `crate-tiers.toml`'s comment on `[server_denylist]` and `tools/xtask/README.md`'s sentence on the server denylist state the new rule, in the same change | 5 |
| `interp-loom` | A loom model of `lactose-net-host`'s sequence lock (row 102), behind an optional feature as `lactose-input-host`'s is; `LOOM_MODELS` in `tools/xtask/src/features.rs` gains the crate | `test-binaries` merged; no lane editing `tools/xtask/src` in flight; milestone 1's exit step 7 routed | The model passes, and fails without each ordering the crate states. The nightly loom job runs it. `tools/xtask/README.md`'s sentence on the loom rule names the crate, in the same change | 6 |
| `crash-hook` | The crash hook moves from the headless binary into `lactose-app`, shared by every engine binary, and writes its report to a file as well as to stderr (decision 5.9). RICOTTA keeps its own hook: it does not link `lactose-app` | Question 3's answer | A subprocess test: a panic in the headless binary writes the report to both and aborts, and a directory that cannot be written costs only the file. This closes the row's "checked by inspection only". Its report records, for the M1 and the laptop, whether the system keeps a crash report; the desktop's setting is recorded in session C | 5 |
| `eco-qos` | Windows efficiency-core steering (row 63): `ThreadClass::Background` sets the power-throttling thread setting, in `crates/core/src/thread_class.rs`. Whether `ThreadClass::Io` follows is reported and not decided | `keeper` merged or closed (section 9) | On the laptop, the WHISK benchmark's rows with two background zstd threads, before and after. Miri for `lactose-core` | 5 |

**The coordinator's passes.**

- After the upgrade and before the first lane, with no lane (section 9): the Vulkan SDK, the
  validation layer, the SDK's Slang and Xcode's `metal` re-checked on macOS 26; `vulkaninfo`
  under MoltenVK and under KosmicKrisp; the RHI spike's program run again; then the quiet
  window that re-takes milestones 0 and 1's M1 files.
- `gpu-probe`, pushed alone: a manually dispatched job that prints what each runner offers. On
  linux-x64, the Mesa version and lavapipe's `vulkaninfo`: the local read, its memory types and
  queues, indirect count, a headless surface, and whether the pinned validation layer and
  GPU-assisted validation load. On macos-arm64, the Xcode version, whether `xcrun metal`
  exists, whether a Metal device is present, and the same for the oldest macOS image the
  hosted runners offer. Beside it, the laptop's `vulkaninfo` for each of its devices: the
  queue families that have the graphics bit, and which of them can present. Its output goes
  into the rows section 6 adds. The io_uring probe of the nightly is the precedent.
- The verification pass, after the `vulkan` merge and again at the exit: section 2's Done marks
  and section 6's rows.
- The merge brief of every branch that changes `lactose-rhi-api`, `lactose-rhi-null`,
  `lactose-rhi-vulkan`, `lactose-render-graph`, `lactose-renderer`, `lactose-presentation`,
  `lactose-platform`, `tools/cook` or the client binary names one extra gate,
  `cargo xtask gpu --driver moltenvk`, passed by decision 5.10's rule. KosmicKrisp is no gate
  (decision 5.7). After its push, the laptop runs `cargo xtask gpu --driver system` on each of
  its devices and sends the files back.
- The machine notes: the pinned Slang installed on the M1 and the laptop by `slang`'s command,
  and what `crash-hook` recorded of each machine's crash reports.

**Order.** Three tracks run at once, with the document work beside them. Each queue is in
dependency order, and the coordinator refills a free slot from any queue whose head is unblocked.

| Queue | Lanes, in order | Runs |
|---|---|---|
| 1, the RHI | `spike-metal`, `rhi-api`, `rhi-null`, `vulkan`, `swapchain` | 42 |
| 2, the frame | `platform`, `platform-impl`, `graph`, `renderer`, `frame-design`, `graph-impl`, `renderer-impl`, `frame-impl` | 67 |
| 3, shaders, presentation and tools | `present-api`, `slang`, `present-impl`, `presentation`, `measure-m2`; beside them `server-closure`, `interp-loom`, `crash-hook` and `eco-qos` | 61 |

The 22 lanes are 170 runs. Document work beside them is about 22: this plan's two rounds of
review and their answers (6), its re-base at approval and that diff's review (2), revision 3.4
(3), the freeze decision (3) and the exit (8, as milestone 1 counted its own: two criteria
reviewers, and the milestone 3 plan with its reviews and answers). The whole is about 190 runs
at two fix rounds a lane, and about 230 if a third of the lanes take four or five. The
owner's other answers add to it: the directory watch about sixteen runs, a minidump writer about
nine, a split of the check jobs five.

The longest chain is `spike-metal`, `rhi-api`, `slang`, `vulkan`, `renderer-impl`, `frame-impl`,
`measure-m2`: 63 runs, and about 65 where `graph-impl`'s device tests wait on `vulkan`. `slang`
is on it because `vulkan`'s output cases run cooked shaders: it takes the first slot that frees
once `rhi-api` has merged. `present-api` needs nothing and starts before it. The first slot
that frees goes to the chain's head. `server-closure`, `interp-loom`, `crash-hook` and `eco-qos`
need nothing in their queue and fill a slot no head can take; `server-closure` merges before
`frame-design` starts, and `crash-hook` before `frame-impl`.

Five facades come first: `rhi-api`, `graph`, `renderer`, `platform` and `present-api`, and
`frame-design` amends a sixth, `lactose-app`'s. `lactose-rhi-null` and `lactose-rhi-vulkan`
implement `rhi-api`. `lactose-presentation`'s one consumer is the client's frame, which
`frame-design` covers. `tools/cook` is a leaf.

Seven rules bind whatever the coordinator reorders:

- One lane at a time edits each of `lactose-ecs`, `lactose-app`, `tools/xtask/src` and
  `tools/cook`. Edits to `crate-tiers.toml` and `ci-policy.toml` are ordered by merge. In
  `tools/xtask/src` the order is `server-closure`, `interp-loom`, `slang`, `vulkan`,
  `measure-m2`.
- A backend's lane may amend `lactose-rhi-api` in the change that needs it, called out in its merge
  (section 20, decision 5.13). While `rhi-null`, `vulkan` and `graph-impl` are in flight together,
  the amendment merges first and the others take main.
- A push that changes the cache key, which is `Cargo.lock`, any `Cargo.toml`,
  `.cargo/config.toml` or `rust-toolchain.toml`, is pushed alone, and nothing follows it until its
  Windows check completes (D125). A push that changes a workflow is pushed alone too. That is
  every lane that creates a crate or adds a dependency: `rhi-api`, `rhi-null`, `vulkan`,
  `swapchain`, `platform`, `platform-impl`, `graph`, `renderer`, `frame-design`, `graph-impl`,
  `renderer-impl`, `frame-impl`, `slang`, `present-api`, `presentation` and `interp-loom`, and
  `present-impl`, `eco-qos` or `measure-m2` if they touch a manifest; and `gpu-probe`. So at
  least seventeen of the milestone's pushes each hold the push slot for one Windows check: 9.5
  to 11.7 minutes over three runs of one code, about three hours across the milestone at that
  figure, and less after `test-binaries`.
- After a push that changes a GPU crate, the next such push waits for a green `gpu (lavapipe)`
  job too.
- A crate that milestone 2 adds follows the rule `test-binaries` writes for its integration
  tests (decision 5.15). A file that is not text is named `binary` in `.gitattributes` by the
  change that adds the first of its kind (D148).
- The measurement commit is the first commit of main that holds `frame-impl` and `measure-m2`. The
  quiet window, the laptop's session and session C run there (section 4). A lane that merges after
  it and edits a measured crate runs the measurements it touches and reports them as steering.
- A device test compares against a reference the test computes on the CPU, at a margin it
  justifies. No image is checked in (decision 5.10).

## 4. Machines, workloads and result files

| Machine | What it can show | What it cannot |
|---|---|---|
| The development Mac: Apple M1, four performance and four efficiency cores, on macOS 26 from milestone 2's first lane, with the Vulkan SDK's MoltenVK and KosmicKrisp, its validation layer, and the Xcode current for macOS 26, each re-checked after the upgrade (section 9) | MoltenVK and KosmicKrisp under the Khronos layer, Metal API validation and Metal shader validation. A window and a present, and a headless surface. The native Metal spike. Cooked MSL compiled through Metal at run time, and a metallib built with Xcode's `metal`. A buffer in device-only memory. A hazard between the host and the GPU, with the GPU made to lag. Timing in a quiet window | macOS 14, the oldest client the cook targets: nothing in reach runs it once the Mac is upgraded. A missing barrier between two GPU uses, by pixels: Metal orders tracked resources itself. GPU-assisted validation under MoltenVK, which cannot compile it; under KosmicKrisp it is not known |
| The Windows laptop: Intel i7-12650H, six performance and four efficiency cores, an RTX 4070 laptop GPU and, it is expected, an Intel integrated one, Windows 11, driven through its own agent session | Vulkan on a discrete GPU and the Win32 swapchain. The triangle on Windows. GPU-assisted validation. The cost of a write-combined ring. The power-throttling setting on a hybrid part. Frame pacing on Windows | Section 20's x86 desktop figures, which are session C's (the owner's answer 2). Memory that is not coherent, or lazily allocated: no PC driver surveyed offers either. Its figures are labelled |
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until milestone 1's session A returns | The x86 desktop figures section 20 asks for. A second discrete GPU | Anything without the owner at it |
| linux-x64 runner: no GPU | lavapipe, a CPU rasteriser: the conformance suite and the device tests under the pinned validation layer on every pull request, and under GPU-assisted validation nightly | Timing. Tile memory. A vendor's driver. A window. Two queues or an ownership transfer: Mesa's source gives lavapipe one queue and one coherent memory type |
| windows-x64 runner: no GPU, no Vulkan driver | The build, the lints and the tests on null | Any Vulkan call. The Win32 path runs on the laptop alone |
| macos-arm64 runner: Xcode, a virtual GPU | Cooking metallibs (criterion 7). Its oldest image is the one place a metallib can be loaded on an older macOS | Timing. Whether it runs Metal or MoltenVK tests, and what its oldest image offers, is `gpu-probe`'s to find |

**Four kinds of sitting produce every file a criterion rests on.**

- *At each merge of a GPU crate*, on the M1: the extra gate, under MoltenVK and the three
  validators. A run that presents holds the display awake with `caffeinate -d`: under MoltenVK a
  frame into a sleeping display may hold the present thread's submit for seconds (decision 5.6).
  Then, after the push, the laptop's run on its devices.
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s -d`, and `cargo xtask measure --all` runs once. It writes milestone 2's
  five files beside milestone 1's, `gpu-cost` under MoltenVK and under KosmicKrisp.
- *The laptop's session*, at the same commit, with nothing else running there: the same command,
  and `cargo xtask gpu --driver system`. Its `triangle` file is criterion 1's proof on Windows
  (the owner's answer 2).
- *Session C*, the owner's, at the same commit, on the desktop under each system it boots: the
  same command. Its `extract-50k` and `remote-upload-5k` files are the x86 desktop's figures.
  Whatever of milestone 1's sessions A and B is still owed runs in the same sitting, at their own
  commits. Under a system that is not a client platform, the measurements that need a window
  are skipped and the file says so.

Only session C needs the owner at a desk. The laptop's runs need the laptop awake with its agent
session open, and a run that opens a window needs a person logged in at it.

**What each measurement runs, and against what.**

| Measurement | Workload | Read against |
|---|---|---|
| `extract-50k` | 100,000 rows with a `Transform` and a mesh component, in four archetypes and sixteen regions, as milestone 1's cost scene is laid out. A kernel moves 50,000 of them every tick. One tick, then `Simulation::run_passes` with extract alone registered, sixty times, on N frame workers, N the performance cores up to eight. The ring is the device's mapped buffer, and the run is repeated on null. The file gives the pass's median, the ring's part and the snapshot's part, the cost per dirty row on one worker, and the ring's memory type and flags. Each pass ends with an empty submit that signals its timeline value, so the ring's regions come free; a wait for a free region is left out of the median and reported | Criterion 4: the median on the device at most 1 ms, on the M1 and on the desktop |
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer. One writer thread pushes a state to every slot at 30 Hz. The presentation schedule's remote upload runs every frame, paced at 144 Hz by a timer for ten seconds: one `sample_batch`, the conversion to render space, the write to the device's ring. Each frame ends with an empty submit that signals its timeline value, so the ring's regions come free as they do in a frame. The file gives the median, P99 and worst frame, the reader's retries, the buffer's bytes and the ring's memory type | Criterion 5: the median at most 1 ms, on the M1 and on the desktop. P99 is recorded; over 2 ms it is a finding |
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without | Nothing: recorded. It sets `lactose-app`'s default reservation (decision 5.16) |
| `gpu-cost` | Through `rhi-vulkan` under a named driver, MoltenVK and KosmicKrisp on the M1 and the system's on the laptop and the desktop, with the validators off: ten thousand indexed draws as draws, as one multi-draw and as one indirect draw over a compacted buffer, with the record time and the submit time given apart and the thread of each, since MoltenVK encodes Metal inside the submit; the creation of 96 pipelines three ways, with the hit bits: source-cold, from shaders cooked with a link-time constant the run salts, so the SPIR-V and the MSL are new; specialisation-cold, under a salted specialisation constant, which under MoltenVK leaves the MSL source the same; and from a reloaded cache. GPU time is from timestamps across whole render passes, by the queue's valid bits | Nothing: recorded. Rows 15 and 34, and the comparison of decision 5.7 |
| `triangle` | The client binary's fixed-frame mode for 600 frames, seven instances, under every validator the host has, the last frame's offscreen target read back; each validator's canary runs in its own process beside it | Criterion 1: no pixel outside the margin wrong, each canary seen, no other message, on Windows and on macOS |

`extract-50k`'s arithmetic, not a measurement: with a tenth of a millisecond for dispatch and the
snapshot's publication, four workers have about 70 ns for each dirty row. No extract figure
exists. The nearest is the replication gather's 0.46 ms for 4,000 changed rows among 100k, taken
before the ECS hot-path lanes (row 98).

**How a result is checked in.** As `docs/results/README.md` states for milestone 1. A file taken
on a device adds to its header what the RHI spike's run header carries: the adapter's name, vendor
and device ids, the driver's name and version, the API version, the SDK's and the layer's
versions, the validation settings, the present mode and the display's refresh rate.
`cargo xtask gpu --driver <name>` writes `rhi-conformance-<driver>-<machine>-<date>.md`: the cases
run, the capabilities reported, each canary's message, and the count of other messages by source.

## 5. Decisions this plan takes

Each is a recommendation the owner approves with the plan. Each names what would change it.

**5.1 The triangle is the client binary's frame, in a mode that can be reproduced.** Criterion 1
is proven by `lactose-app-client` drawing instanced triangles at extracted transforms, through
passes the render graph compiled and recorded, on a hardware Vulkan driver on Windows and under
MoltenVK on macOS. The client at milestone 2 runs a local world, as the headless binary does:
`Client` gains its replicated world only at milestone 4 (`crates/app/README.md`). The RHI spike's
margin rested on integer vertices and a fixed camera. This frame has moving rows, an
interpolation fraction, a latched camera and 32-bit transforms, so the mode fixes what a
reference needs. It steps a fixed clock and a scripted input stream. It dumps the instance
transforms, the fraction and the camera of the last frame. The reference is rasterised from that
dump and compared at pixel centres a quarter of a pixel or more inside or outside every edge, a
margin the test justifies from 32-bit error at the scene's extent and four bits of sub-pixel
snapping. Every colour is zero or one in each channel, the clear colour is its own, and each
instance has its own, which makes the scene seven instances. The image read is the offscreen
target the graph rendered, before the present thread's blit or draw. The run is under the
Khronos layer on both hosts and under Metal API validation and Metal shader validation on macOS,
by decision 5.10's rule. Four mutations fail it:
a dropped draw, a vertex moved one pixel, a skipped clear, two instances swapped. On macOS a
window test cannot run on a test harness's thread, since winit needs the main thread, so the
proof runs the binary. This rules out a triangle drawn by a test program beside the graph, a
criterion met by a window somebody looked at, and a reference that guesses what was drawn. What
would change it: `frame-design` finding that the local world needs a second assembly path. The
proof then moves to a device test of the frame without the binary, and the binary's run is
recorded beside it.

**5.2 Extract's criterion is read on the device's ring, at 50,000 rows moved among 100,000.** A
row is dirty when its change tick is newer than extract's mark or than the tick before (section
5.3), so a scene whose 50,000 rows move every tick has 50,000 dirty rows at every pass. The pass
is what `Simulation::run_passes` runs for extract: the fork over dirty chunks, the write of the
previous and current transform to the staging ring in render space, and the presentation
snapshot. The ring is a mapped buffer of the device. On the M1 it is cached shared memory, and on
a discrete GPU it is write-combined, where a read is very slow. So the criterion is read on the
device, the file records the memory type, the ring's writer offers no read, each pass ends with
a submit so the ring's regions come free, and the null backend's figure separates the ECS's
cost from the memory's. It is read as the median of sixty
passes, as milestone 1's criteria are (D139), on the M1's four performance cores and on the
desktop. This rules out a criterion met on null alone, and one read without the snapshot. What
would change it: the `present-api` facade keeping transforms out of the snapshot, in which case
the file says what the snapshot holds.

**5.3 Remote upload's criterion is the median frame of a ten-second run at 144 Hz, with a writer
running.** The path is the one section 5.3 names as the client's dominant one: the batch sample
through `InterpolationReader::sample_batch`, the conversion through `RenderPosition::from_sim`,
and the write through the same ring as extract. A writer at 30 Hz runs beside it, since the
sequence lock's retries are part of the cost and row 102 asks for their rate. Each frame ends
with a submit, so the ring's regions are freed by timeline values as in a real frame. The run is
paced by a timer and needs no 144 Hz display. The median is read, as milestone 1's are, and P99
is recorded beside it. This rules out a figure taken with no writer, one that stops before the
ring, and a ring that never frees. What would change it: a P99 over twice the criterion, which
goes to the planner as a finding about the frame's tail.

**5.4 The minimal renderer is the path the two timing criteria end at, and a triangle on it.** In
milestone 2: the render slot table (D49); the staging ring that extract and the remote upload
write; its upload and a scatter pass into a persistent instance buffer in the cell-plus-offset
format (D41); triangles drawn by instance from that buffer; the latched camera buffer (D48),
written before submit from the snapshot's pivot and `lactose-input-host`'s `LatestSample`;
pipelines from cooked shaders and the pipeline cache file; `RenderFeature` with no provider. The
scatter and the cell-plus-offset format are milestone 3's by section 22 and are pulled forward on
purpose: the ring's row is then in its final format, so criterion 4's figure is of the path
milestone 3 keeps, and the ring's contract, frames in flight and a slot named twice in a batch,
is tested now. Not in milestone 2: meshes and materials in the GPU scene, cluster submission,
culling, deferred lighting, the heap's use by materials, skinning, GPU VFX, light binning, the
cascade pads and the boom's recheck at the latch. Device-lost recovery is tested with the GPU
scene it rebuilds, at milestone 3; `rhi-api` reports the loss as a value now and null injects it.
This rules out a staging format that milestone 3 would rewrite, and any other milestone 3 pass
built early. What would change it: the `renderer` facade showing the scatter cannot be specified
without the GPU scene's layout. The ring then uploads into the instance buffer directly, and the
scatter and its hazards are milestone 3's.

**5.5 The presentation schedule at milestone 2 runs what has a source.** In: the schedule per
frame on the snapshot, interpolation of local transforms with the no-interpolate flag, the camera
rig's orbit and boom, the remote sample and upload, and the delivery of presentation events to a
sink a test counts. Not in: boom collision, which needs the static collision asset of milestone
3; pose evaluation, which section 22 gives milestone 3; UI animation, the audio listener and
markers, whose crates are milestone 5's; the visual blend of a correction, which nothing shows
before a client meets a server at milestone 4; and the prediction world's copy shadowing the
replicated one, which is tested when `Client` holds both worlds, at milestone 4. Device sources
are milestone 5's: the camera is driven by a scripted sample stream in every proof. This rules
out stubs for crates that do not exist. What would change it: the static collision asset
reaching presentation before milestone 3, which brings boom collision with it.

**5.6 `rhi-api`'s facade follows the native Metal spike alone, and MoltenVK owes one shape: the
present thread has a queue of its own.** D139 put the facade behind the MoltenVK, KosmicKrisp and
native Metal spikes. The MoltenVK spike has given what shapes a surface: fourteen points, and
five rules for null. Its open figures, the encode time and the pipeline creation cost, are costs,
which `rhi-api` reports or budgets and does not bake into a signature. One thing it left open is
a shape. MoltenVK 1.4.2 never takes a drawable at acquire: it calls `nextDrawable` when a command
first uses the swapchain image's texture, a copy out of it included, and it encodes inside
`vkQueueSubmit` on the calling thread (its source, read by the GPU review). Vulkan has the caller
serialise every use of a queue. So while that submit waits for a drawable, up to a second for
each image, the queue is held, and a frame submit on the same queue waits behind it whichever
thread makes it. Two rules follow, and `rhi-api` states them for every backend.

- *Every command that reads or writes a swapchain image is submitted by the present thread.* The
  frame renders into an offscreen target, and the present thread draws or blits it into the
  swapchain image by the surface's format: a plain copy would swap channels into a BGRA surface
  and skip the encoding into an sRGB one. The offscreen target is two images or more. The
  present thread holds one at a time, the newest the frame has finished. The frame renders only
  into one the present thread does not hold, after the timeline value the present thread signals
  when its read of it is done. It renders from undefined, since it overwrites the image whole,
  so nothing crosses back. With one image the next frame's write would wait on the present
  thread's read, and the frame would stand behind a blocked drawable again, through the target.
- *The present thread submits on a present queue that no other thread submits to, in a family
  that can draw and present to the surface, where the device offers one; otherwise the roles
  share the graphics queue under its lock.* A draw or a blit needs a family with the graphics
  bit. Under MoltenVK that is a second family: it has four, each able to draw and present, with
  one queue each. A finished target crosses to the present queue by a timeline semaphore and an
  ownership transfer, which null's rule holds, or by concurrent sharing. The graph assigns queues
  and derives such a crossing for the compute queue (section 12.3). The present thread's pass is
  not recorded in graph order, so the crossing is forward only, and null's rule of state by
  queue refuses a frame write to a target the present thread holds. lavapipe has one queue, and
  a PC driver may have one graphics queue: `rhi-api` reports which path a device takes.

The other remedy was to switch off MoltenVK's synchronous submits. It is not taken. It is a
setting of the interim driver alone, it moves the wait onto MoltenVK's own dispatch queue and
leaves the frame and the present on one queue, and the RHI spike's file is of MoltenVK's
defaults. The present queue serves the other backends too: `vkQueuePresentKHR` takes
a queue the caller must serialise, so a FIFO present that blocks, which section 5.1 already
expects, holds its queue on any driver. `swapchain` shows the rule on null with the present
queue held, and under MoltenVK with the drawable made to block. On native Metal `nextDrawable`
is a call the engine places, so `spike-metal` walks it, headless, for its shape. KosmicKrisp is a
conformant Vulkan driver, so it is the same surface; it is measured through `rhi-vulkan` once
the Mac is on macOS 26 (decision 5.7). The native Metal spike is the one that can change a
signature (D43): residency, argument buffers, heaps, indirect command buffers and the binary
archive. So the facade waits for it alone, and the cost figures are taken through `rhi-vulkan`,
on the path that ships, under each driver with one command. This rules out a second throwaway
Vulkan program, a frame worker that submits into a swapchain image, a present that holds the
frame's queue, and a frame that waits on the present thread through its one target. What would
change it: `spike-metal` reporting that an item cannot be decided without a MoltenVK cost. That
cost is then measured with the RHI spike's program first. Or a PC driver whose one graphics
queue a blocking present holds, shown on the laptop: the present alone then moves to a queue of
another family that can present, after the blit on the graphics queue.

**5.7 The Apple interim path is MoltenVK, and KosmicKrisp is measured beside it on the same
Mac.** Section 22.1 asks for the path by milestone 2. Every item of the RHI spike ran under
MoltenVK 1.4.2 with a GPU-side use read back exact. KosmicKrisp is built on Metal 4 and needs
macOS 26 (Mesa's driver documentation); on macOS 14.7.4 the SDK's driver enumerated no device.
The owner has ruled that the Mac moves to macOS 26 once milestone 1's exit is recorded (answers
1 and 6), so from milestone 2's first lane both drivers run on one machine. The path is still
MoltenVK, for three reasons that do not rest on what KosmicKrisp could not do. The decision is
needed before the first lane, and the only evidence in hand is MoltenVK's: the spike, the
facade's fourteen points and null's rules come from it, and nothing has run under KosmicKrisp.
MoltenVK runs on macOS 14 and 15, where KosmicKrisp cannot, and macOS 14 is the oldest client the
cook targets (decision 5.8). And the path lasts one milestone: `rhi-metal` lands in milestone 3.
So the M1's gate runs under MoltenVK, and KosmicKrisp is no gate. It is recorded three times
through `rhi-vulkan`, by one command each and with no lane of its own: the conformance suite at
`vulkan`'s merge, the swapchain's cases at `swapchain`'s, and `gpu-cost` in the quiet window,
beside MoltenVK's figures. A case that fails under it is recorded and fails no merge. Two costs
of the path are accepted for one milestone. The G-buffer in tile memory is not shown on Apple
before `rhi-metal`, since MoltenVK's local read restarts the Metal pass. GPU-assisted validation
does not run under MoltenVK, so it runs under lavapipe and on the laptop. One shape the path
showed is adopted for every backend and outlasts it: the present thread's submits and its queue
(decision 5.6). This rules out a gate under a driver nothing has run on, a second gate driver at
every merge, and a KosmicKrisp record without timings. What would change it: KosmicKrisp passing
every output case of the suite on the M1 by decision 5.10's rule, and offering one thing
MoltenVK does not: a memoryless attachment read in its pass and kept across a local read,
GPU-assisted validation that runs, or a drawable taken outside a submit. The planner then brings
the owner a switch of the gate's driver, as an addendum to D<N+1>, with MoltenVK run beside it
for macOS 14. A KosmicKrisp that does not enumerate the M1 on macOS 26 is recorded as that, and
the question returns to the owner.

**5.8 Slang is a pinned release that the cooker runs, checked by what it emitted, and a metallib
is cooked on macOS alone.** The cooker runs `slangc` of one Slang release and refuses another.
The pin is a tracked file: the version and each host's archive checksum. One command installs it
on a host by fetching that archive, checking it and unpacking it under `target/`; the runners,
the M1, the laptop and the desktop all get it that way, so an owner's session stays one command.
The SDK's copy on the Mac is a build 25 commits past release 2026.17 and is not the pin; releases
2026.17 to 2026.22 were listed on 16 November 2026. Section 21 permits FFI to Slang and nothing
requires it: a subprocess needs no `unsafe`, no link step on three hosts and no build script, and
milestone 2 cooks a handful of shaders. The GPU review's probes of the SDK's copy change what
the cook must check, because a check on the source can pass while the output is wrong:

- *The heap.* `NonUniformResourceIndex` fails Slang's Metal target, and a runtime array emits MSL
  that Metal refuses. Two forms cook for both targets: a fixed-size array in a parameter block,
  and `DescriptorHandle`. `spike-metal` chooses, and the cooked format names the form. The marker
  differs by form. For a fixed array it is a helper that switches on the target, and it emits
  SPIR-V with no decoration unless it is inlined. For `DescriptorHandle` it is `nonuniform`;
  `NonUniformResourceIndex` on a handle only warns and emits no decoration. So the cook reads the
  SPIR-V, where every access through the heap must be decorated, and treats Slang's warnings as
  errors.
- *Outputs.* An output struct whose one field is `SV_Target1` gets location 0 in SPIR-V and
  colour 1 in MSL. The cook requires each fragment output to have one location on both targets.
- *The local read.* Slang turns an input attachment into a fragment input at the colour index
  of the same number, and can emit nothing else, so the cook has no check for it. The facade's
  local read is that identity, and `rhi-api` and null hold it at run time.

SPIR-V and MSL are cooked on every host and their bytes are equal across hosts, which the
`shaders` job compares. The cook turns line directives off, since the MSL carries the source's
path when the compiler is given an absolute one. Cooked MSL is compiled on the M1 through Metal
at run time and a pipeline state made from it, by a Swift script that `cargo xtask gpu` runs
with the command line tools' `swift`: that needs no Xcode, and no `unsafe` outside section 21,
which exempts `tools/cook` for Slang and compression alone. A metallib needs Apple's `metal`
compiler, so it is cooked on macOS: on the runner, and on the M1 once the owner has installed
the Xcode current for macOS 26 (answer 5), so a wrong flag is found on the branch. Slang's own
call of that compiler passes `-std=metal3.1`, no deployment target and an input file whose name
changes at every run (the GPU review's stand-in compiler). So the cook runs `metal` itself on
the MSL it emitted, under a stable name, as MSL 3.1 with macOS 14 as the deployment target. MSL
3.1 needs macOS 14, and that makes macOS 14 the oldest client the cook targets. Once the Mac is
on macOS 26 no machine in reach runs macOS 14. That a metallib cooked for it loads there is then
shown only on a hosted runner's oldest image, if that has a Metal device, and that the engine
runs there is not shown at all. Equal bytes are claimed for SPIR-V and MSL and not for a
metallib: whether two cooks agree is recorded. One metallib the runner cooked is loaded on the
M1. This rules out a Slang taken from whichever SDK a host has, a check that reads the source
alone, a check that cannot fail, and cooked bytes that depend on a checkout's path. What would
change it: process starts dominating the cook at milestone 3's 48 pipelines, or the cooker
needing reflection the command does not give. The FFI is taken then.

**5.9 Crash reporting is the panic hook's text report, in `lactose-app`, until a build leaves the
owner's machines.** Section 22.1 asks for minidumps and symbol upload by milestone 2. The hook is
in the headless binary today, and its README and row 134 give it to `lactose-platform`, a crate
`[server_denylist]` denies the server. `lactose-app` is what every engine binary links, and the
hook is safe code. RICOTTA has a hook of its own and keeps it: a tool does not link the
assembly. The report goes to stderr and to a file. A hardware fault, which a driver call makes
possible from this milestone on, is read from the operating system's own report. macOS writes
one by default. Windows keeps a local dump only once Windows Error Reporting's `LocalDumps` key
is set, which is off by default and an administrator's change (Microsoft's documentation), so
the owner sets it once on the laptop and the desktop; `crash-hook`'s report records the M1's
and the laptop's setting, and session C the desktop's. A minidump writer that catches such a
fault runs in a signal handler, which is `unsafe` that section 21 admits in no crate every
binary links, or in a second process, which is a monitor to build and ship. An upload needs a
service and a symbol store, and nothing reads either before a build goes to a tester. So neither
is built, and the row of section 22.1 moves to milestone 5. This rules out a hook the server
cannot link. What would change it: a fault on the owner's machines that the system's report does
not explain, or a build given to a tester sooner.

**5.10 One command runs the device tests by one rule, lavapipe runs them on every pull request,
and no GPU runner is bought.** `cargo xtask gpu --driver <name>` sets the loader's environment
for its child as the RHI spike's run header set it by hand, cooks the suite's shaders, runs the
conformance suite and every device test, and writes a result file. The drivers are `lavapipe`,
`moltenvk`, `kosmickrisp` and `system`; under `system` the device is selected by name and
recorded, since a laptop has two. A device test is an ignored test that the command selects, as a
measurement is, so a job with no driver does not run it, and under a named driver a missing
device fails the run. The rule a run passes by:

- *What is counted.* Every message from four sources: the Vulkan debug messenger; Metal API
  validation's and Metal shader validation's lines on stderr; MoltenVK's own error and warning
  lines.
- *Which layer.* The Khronos layer of one SDK, pinned by version and checksum on every host, with
  synchronisation and thread-safety validation on, all recorded in the file. Ubuntu 24.04's own
  package is 1.3.275, and the swapchain's semaphore checks exist only from 1.4.313.
- *The canary.* Every run has one case for each validator, a known misuse that must yield
  exactly its known message. Each runs in a process of its own beside the run: Metal API
  validation asserts and aborts by default, and a shader-validation misuse fails its command
  buffer. That default is kept and recorded, so a real misuse aborts the run it is in. A run in
  which a canary is silent fails: a layer that did not load reports nothing and would otherwise
  pass.
- *Everything else.* Any other message fails the run, except one a case names as the message it
  exists to provoke, as the blocked drawable's case does.

Section 20 runs RHI conformance "on a GPU runner nightly". No such runner exists. A hosted one is
a paid larger runner, and a self-hosted one on a public repository runs what a stranger's pull
request sends it. Hardware is covered where it is: the M1 at every merge of a GPU crate, the
laptop after each such push, the desktop at the measurement commit. This rules out a spend on a
runner, a device test that compares against a checked-in image, and a run that passes because
nothing was listening. What would change it: a defect that reaches main because the laptop was
not in reach at a merge. The owner is then asked about a runner.

**5.11 `lactose-app` drives the render side through `-api` traits, the client binary wires the
backends, and the server's denylist binds `lactose-app` now.** `[server_denylist]` denies the
server the render crates directly or transitively (D83), and the server will link `lactose-app`.
So `app` names `present-api` and `rhi-api` alone and is handed the backends as `dyn` resources,
the wiring D64 gives every backend service, and `lactose-app-client` constructs the platform,
the device, the graph, the renderer and the presentation schedule. Extract's sink takes a
chunk's rows at a call, so the `dyn` boundary is crossed once for each chunk and not for each
row. The check behind this is empty today: `cargo xtask tiers` applies the denylist only when a
package named `lactose-app-server` exists, and that package arrives at milestone 4. The general
review's probe gave `lactose-app` an edge to a stub renderer and the check passed. So
`server-closure` applies the denylist to `lactose-app`'s own closure, before `frame-design`
gives `app` its first render-side edges. This rules out a feature of `lactose-app` that turns
backend edges on, and a rule that first bites two milestones after it is broken. What would
change it: `frame-design` measuring a call across the boundary that criterion 4 cannot afford.

**5.12 `lactose-rhi-vulkan` is a test-support crate, and five crates get a facade.**
`lactose-render-graph` and `lactose-renderer` must run on a device in their own tests, and a
backend crate may dev-depend only on itself and on `[test_support]`. So `lactose-rhi-vulkan` joins
that list beside `lactose-rhi-null`, in the `vulkan` merge. The server denylist is unchanged. The
lane shows that `cargo xtask boundary` refuses a boundary crate's test that names it, or adds
that rule. The device tests are then Vulkan's alone: what only Metal derives, the fences between
aliases and residency, gets a device test when milestone 3 adds `lactose-rhi-metal` to the same
list. `rhi-api`, `render-graph`, `renderer`, `platform` and `present-api` are crates other
crates are written against and get a facade first (section 20). This rules out a
backend-to-backend exception for tests. What would change it: the `rhi-api` facade placing a
backend-neutral test harness in a leaf crate.

**5.13 `rhi-api` freezes as section 20 says, when `vulkan` passes the suite, and the exit
decision records it.** Section 20 freezes an interface once its first real backend passes its
conformance suite, and keeps `rhi-api` unstable until the native Metal spike has run. The spike
is the first lane, so `rhi-api` freezes at `vulkan`'s merge. Section 20's next sentence lets a
lane amend an interface in the change that needs it, called out at its merge, and makes no
exception for a frozen one: D143 amended `physics-api` that way. So `swapchain`, `graph-impl` and
`renderer-impl` amend `rhi-api` in their own changes, called out, and no sentence of section 20
is replaced. Revision 2 of this plan set the trigger aside on the premise that an amendment
after a freeze takes a decision. The premise was wrong. Criterion 2 asks that `rhi-api` be
frozen or amended on the spikes' findings, and the planner's exit decision records it: what the
suite covers and is frozen, each amendment made since `vulkan`'s merge with its reason, and the
calls only `rhi-metal` exercises, which its lane may still amend. It is written from
`spike-metal`, the conformance files under lavapipe, MoltenVK and the laptop's devices, and
KosmicKrisp's if there is one, as the scalar decision was written from its probe. This rules out
a freeze by silence, and a decision for each amendment. What would change it: an amendment that
changes what an existing backend must do, and not only what the interface offers. That one goes
to the planner before its merge.

**5.14 Five things leave milestone 2.**

- *The directory watch goes to milestone 3* (the owner's answer 4). D136 and D147 place it in
  milestone 2.
  No criterion reads it, it needs an `io` request on three platforms, and its other half, pack
  retirement, is already milestone 3's (row 139). A re-cooked shader is seen after a restart,
  which is section 18's iteration model. Rows 139, 140 and 143 follow it.
- *Artist surfaces are decided at milestone 3* (row 97). The proposal's measurement is a G-buffer
  pass over 1, 16 and 128 surfaces, on native Metal and on MoltenVK, and it freezes
  `SurfaceSample` before the first cooked surface. `spike-metal` is a native Metal run, but a
  throwaway program has no G-buffer layout and no surface set, so its figure would be of its own
  shader, and nothing would consume a frozen `SurfaceSample` before milestone 3's materials.
  `slang` proves the mechanism the proposal rests on, an interface specialised statically and a
  link-time constant, for both targets.
- *The mesh and texture load against the 0.5 ms slice goes to milestone 3* (row 136), with the
  kinds. Milestone 2 measures the kind it adds.
- *Device-lost recovery is tested at milestone 3* (decision 5.4).
- *The visual blend of a correction and the shadowing of the replicated copy are milestone 4's*
  (decision 5.5).

**5.15 The test-binary conversion is milestone 1's, and milestone 2's CI budget is read after
it, as a median over every check job.** D125 called one integration-test binary for each crate
the next lever, due at once. Revision 1 of this plan deferred it on the workflow's comment, 5.2
to 5.7 minutes for the Windows check, and that figure was out of date. Three runs of one code
show the level and the noise. No Rust source, manifest or lock file changed between `6f0508a`
and `0810f9f`, and the Windows check took 11.7, 11.4 and 9.5 minutes (runs 43781979172,
43795852109 and 43799365983; the first and the last were full cache matches) and the macOS check
11.0, 10.7 and 8.8. Over the last thirty runs of main the Windows check took 6.6 to 12.0
minutes. Both jobs straddle the workflow's own signal, ten of fifteen minutes, and milestone 2
adds eight crates and a binary. So the lever is carried out now, by lane `test-binaries`, as
milestone 1's work: every existing package's integration tests as one binary, and the rule a new
crate follows. Every crate milestone 2 adds follows that rule; where it leaves a recording
target apart, a new crate's is apart too, since a target may carry a codename and a module may
not (section 3). One run reads either side of ten by chance. So the figure is the median, for
each check job, of the three most recent runs of main that were full cache matches. It is read
from the runs main makes anyway. No run is made again for it: the workflow's concurrency group
cancels a run in progress when another run of main starts, and a first run cancelled after a
cache-key change leaves a partial cache entry (D125). The first run after a push that changes
the key is not a full match and is not counted, so through a stretch of such pushes the figure
lags until three counted runs exist. It is read after `test-binaries` merges (section 9), and
again whenever a counted run arrives. While every median is ten minutes or less, nothing more is
done. When one passes ten, the owner is asked question 6 before the next lane that adds a crate
is pushed. This rules out pushing crate lanes onto a job two crates from its timeout, a trigger
pulled by a runner's noise, and hours of main held for re-runs. What would change it: the
conversion gaining on the runners what it gained for `lactose-ecs` on the M1, 5.2 s in place of
14, in which case the question is never asked.

**5.16 The stall is recorded and sets the reservation.** Section 16 asks milestone 2 to measure
the stall the physics phase imposes on render recording. It has no criterion. `record-stall`
gives the record phase's time beside the physics phase at a reservation of one frame worker and
of two, on the M1's four performance cores and on the desktop. If the file shows
`lactose-app`'s default reservation wrong, a follow-up commit changes it; the file is cited
either way. This rules out a threshold nobody set. What would change it: a stall over a
millisecond at both reservations on the M1, which goes to the planner.

**5.17 A hazard is proven by the evidence its class has on the machines in reach.** Revision 1
had a dropped barrier fail "with the GPU made to lag". That holds for one class only.

- *A tracked attachment or buffer.* A build that drops its barrier yields a synchronisation
  validation message, which the gate counts. On the M1 it yields no wrong pixel: Metal orders
  tracked resources itself.
- *The host and the GPU*: the ring, a readback, the camera buffer. A wrong readback with the GPU
  made to lag, on the M1, where memory is coherent and shared.
- *Aliased memory, reads through the heap, indirect and index buffers, ownership transfers,
  memory that is not coherent.* Null's rules alone. Synchronisation validation tracks neither
  aliased memory, nor descriptors a shader indexes, nor indirect and index buffers (its
  documentation), and a transfer dropped whole was silent in the RHI spike. MoltenVK builds
  tracked placement heaps, so Metal orders aliases too, and it keeps depth images out of them
  (its source). lavapipe has one queue and one coherent memory type (Mesa's source), so the
  graph's several-queue path never runs in CI: null runs it under a several-queue capability
  set, and the M1's four families run it on a device. Every host-visible memory type on the PC
  drivers one survey lists is coherent, and none is lazily allocated; what KosmicKrisp offers on
  the M1 is not known until it runs. So no machine in reach is known to show these by pixels,
  and each of null's rules is held by a mutation (`rhi-null`).
- *Device evidence for that class* is labelled data from the laptop's devices and from
  GPU-assisted validation. A run that shows nothing is recorded as that and discharges no rule.
  The first device evidence for an aliasing fence is `rhi-metal`'s, at milestone 3.

This rules out a proof that cannot fail on the machine it runs on, and a "no effect" recorded as
an answer. What would change it: a machine with untracked aliasing or memory that is not coherent
coming within reach.

**For the owner.** Answered on 16 November 2026, through the coordinator's questions, and written
into this plan:

| Question | Answer | What it settles |
|---|---|---|
| 1, KosmicKrisp | "Upgrade the Mac to macOS 26", which was not the recommended option | KosmicKrisp is measured beside MoltenVK on the M1, with timings (decision 5.7). Every M1 result file of milestones 0 and 1 is taken again after the upgrade (section 9) |
| 2, Windows GPU | "Laptop, one desktop sitting" | As recommended (section 4) |
| 3, Crash dumps | "Text report now" | As recommended (decision 5.9) |
| 4, Asset watch | "Milestone 3" | As recommended (decision 5.14) |
| 5, Xcode, put again after answer 1 | "Install Xcode": the Xcode current for macOS 26, after the upgrade | The cook's `metal` step runs on the M1 before a merge (decision 5.8) |
| Added by the coordinator after answer 1: when is the Mac upgraded? | "At milestone 1's exit": after the exit is recorded and before milestone 2's first lane | Milestone 1 closes on the system its figures were taken on, and every milestone 2 figure on the M1 is of macOS 26 (section 9) |
| 6, CI budget | Not asked | It stays conditional (decision 5.15) |

The questions follow as the record of what was asked. The first block is as this plan put it.
The second is the Xcode question as the coordinator put it again after answer 1, in this plan's
words; as first written it offered Xcode 16.2, the newest for macOS 14. The added question had no
block here.

```json
{"questions":[
{"header":"KosmicKrisp","question":"Section 22 asks for a KosmicKrisp spike recorded in milestone 2. KosmicKrisp is built on Metal 4 and needs macOS 26. The development Mac runs macOS 14.7, and the SDK's KosmicKrisp driver finds no GPU there. MoltenVK is the interim Apple path either way. How is the KosmicKrisp spike recorded?","multiSelect":false,"options":[
{"label":"Hosted macOS 26 runner (Recommended)","description":"The Mac stays on macOS 14. Once `rhi-vulkan` exists, its conformance suite runs under KosmicKrisp in a manually dispatched job on a hosted macOS 26 runner, if that runner's GPU offers Metal 4. Capability answers are recorded and timings are not, since a runner's GPU is virtual. If the runner cannot run it, the record is one line: it needs macOS 26. The criterion is restated to say so. No extra lane."},
{"label":"Upgrade the Mac to macOS 26","description":"KosmicKrisp is measured beside MoltenVK on the M1, with timings. Every M1 result file of milestones 0 and 1 was taken on macOS 14.7 and the tick-start question is open on it, so the benchmark and `cargo xtask measure --all` run again in a quiet window after the upgrade."},
{"label":"Record it as not runnable","description":"No runner job. The record is one line: KosmicKrisp needs macOS 26. The criterion is restated, and MoltenVK is the path with no comparison."}]},
{"header":"Windows GPU","question":"Milestone 2's Windows proofs need a GPU: the conformance suite on a discrete GPU after each GPU merge, the triangle on Windows, and the x86 figures for extract and remote upload. Which machine gives them?","multiSelect":false,"options":[
{"label":"Laptop, one desktop sitting (Recommended)","description":"The laptop's GPUs run the suite after each GPU merge and its RTX 4070 proves the triangle on Windows, through its own agent session. You run one sitting on the desktop at the measurement commit, `cargo xtask measure --all` under each system it boots, for the x86 desktop figures architecture section 20 asks for. Whatever of milestone 1's sessions is still owed runs in the same sitting."},
{"label":"Laptop only","description":"No desktop sitting for milestone 2. The laptop's figures, taken with nothing else running, stand as the x86 figures, and section 20's desktop is restated for this milestone. It has six performance cores."},
{"label":"Desktop only","description":"You run the suite, the triangle and the measurements on the desktop. A defect only a discrete GPU shows is then found at the measurement commit and not at the merge that made it."}]},
{"header":"Crash dumps","question":"Section 22.1 wants crash reporting, minidumps and symbol upload, decided by milestone 2. Nothing leaves your machines before milestone 5. macOS writes its own report of a hardware fault by default; Windows keeps a local dump only once you set a registry key as administrator. What is built now?","multiSelect":false,"options":[
{"label":"Text report now (Recommended)","description":"The panic hook moves into `lactose-app`, so every engine binary shares it, and writes its report to stderr and to a file. Hardware faults are read from the system's own crash reports: you set Windows Error Reporting's `LocalDumps` key once on the laptop and on the desktop. No minidump writer, monitor process or upload service is built until a build goes to a tester. One small lane, five runs."},
{"label":"Build minidumps now","description":"A minidump writer for panics and hardware faults, in a new crate that section 21 must admit, since the server cannot link `lactose-platform`. About nine runs, and still no upload until a service is chosen."}]},
{"header":"Asset watch","question":"Decisions D136 and D147 place the cooker's directory watch, hot reload without a restart, in milestone 2. No milestone 2 criterion needs it. It costs an `io` watch request on three platforms and SILO's watch, about sixteen runs, and its pack-retirement half is already milestone 3's. Where does it go?","multiSelect":false,"options":[
{"label":"Milestone 3 (Recommended)","description":"It is designed once, with streaming, residency and pack retirement. In milestone 2 a re-cooked shader is seen after the client restarts. The rows on the watch, the long dev session and the mount cost on Linux and Windows move with it."},
{"label":"Keep it in milestone 2","description":"Two more lanes, about sixteen runs, and a decision on unmounting taken before streaming designs residency."}]}
]}
```

```json
{"questions":[
{"header":"Xcode on M1","question":"The Mac moves to macOS 26 (answer 1). It has the command line tools only. The cooker's MSL can still be compiled and run there, through Metal at run time. What cannot be done there is the last step, building a metallib, which needs Apple's `metal` compiler from Xcode: the Slang compiler fails without it. The macOS CI runner has it, but no branch reaches CI before its merge. Do you install Xcode on the Mac, the one current for macOS 26, after the upgrade?","multiSelect":false,"options":[
{"label":"Install Xcode (Recommended)","description":"Lanes and reviewers then build and load metallibs before a merge, so a wrong language version or compiler flag is found on the branch, and the native Metal spike can test a binary archive built from a precompiled library. It takes several gigabytes of disk. The runner's Xcode stays the one whose metallibs count."},
{"label":"Runner only","description":"Nothing to install. A metallib is first built in CI after the merge that adds it, so a failure there is found on main and fixed by a follow-up, and GPU pushes wait meanwhile. The native Metal spike compiles its MSL at run time and reports whether a binary archive works that way."}]}
]}
```

Question 6 is asked only when a check job's median over its three most recent warm runs of main
passes ten minutes (decision 5.15). The coordinator writes the job or jobs, the medians and the
spread where the angle brackets stand.

```json
{"questions":[
{"header":"CI budget","question":"The <job or jobs> check job takes <median> minutes of its 15, the median of its three most recent warm runs of main, which spread from <low> to <high>, past the workflow's own signal of 10. Before the test-binary conversion (D125) three runs of one code gave the Windows job 9.5 to 11.7 minutes and the macOS job 8.8 to 11.0. Each crate milestone 2 adds costs about a minute on Windows by D125's estimate. What is done before the next lane that adds a crate is pushed?","multiSelect":false,"options":[
{"label":"Split the check jobs (Recommended)","description":"Each check job becomes two, by package group, with a check in `cargo xtask` that every package is in exactly one group, so a new crate cannot go untested. One small lane, five runs. Each run then uses twice the runners, and each job has room for the milestone's crates."},
{"label":"Raise the budget to 20 minutes","description":"No lane. Architecture section 20's fifteen minutes is restated, every job's timeout follows, and a cold run after a cache-key change takes longer still."}]}
]}
```

The owner's actions, in order: the upgrade and the Xcode install, before the first lane (section
9); the laptop's checkouts refreshed once (D148) and `LocalDumps` set on the laptop, before its
first milestone 2 run; session C at the measurement commit, with `LocalDumps` set on the desktop.

## 6. Text for the coordinator

The numbers are allocated at application; `D<N>` follows D149 unless another decision is applied
first. Exact OLD and NEW text is not given: this plan is re-based on main when it is approved, and
the row changes below are each row's gate and the gist of its new text. The entries are written
for the owner's answers of 16 November 2026. The paragraph after them says what question 6
changes, if it is asked.

### Decision entry: the plan's rulings

```
**D<N>. The milestone 2 plan: how its criteria are read, which machine proves each, the order the render track is built in, and what is re-gated** (§4, §5.1, §5.3, §12.1, §12.2, §12.3, §12.4, §13, §14, §16, §18, §19, §20, §22, `crate-tiers.toml`, `tools/xtask`, `docs/plans/milestone-2.md`)
- Why: section 22 gives milestone 2 seven proofs and names no workload, machine or order for
  them; none of the crates it delivers existed at `0810f9f`; twenty verification rows gated the
  milestone. `docs/plans/milestone-2.md` is the map and gives each decision's reasons; these are
  the decisions.
  - *The triangle is the client binary's frame, in a fixed-frame mode.* `lactose-app-client`
    steps a fixed clock and scripted input, extracts a local world each frame and draws
    instanced triangles at the extracted transforms through passes the render graph compiled
    and recorded, on a hardware Vulkan driver on Windows and under MoltenVK on macOS. It dumps
    what it drew; a CPU reference rasterised from the dump is compared with the offscreen
    target's readback, in result file `triangle`. Rules out a triangle drawn by a test program
    beside the graph, and a reference that guesses what was drawn.
  - *Extract's 1 ms is the median of extract's pass on N frame workers, at 50,000 rows moved every
    tick among 100,000*, writing the previous and current transform to the device's mapped
    staging ring in render space and filling the presentation snapshot; the null backend's figure
    and the ring's memory type are recorded beside it. It is read on a quiet Apple M1 and on the
    x86 desktop. Rules out a criterion met on null alone or without the snapshot.
  - *Remote upload's 1 ms is the median frame of ten seconds paced at 144 Hz*: one batch sample
    of 5,000 slots, the conversion to render space and the write to the ring, with one writer
    pushing every slot at 30 Hz and a submit each frame. P99 is recorded. Rules out a figure with
    no writer, or with a ring that never frees.
  - *The minimal renderer is the render slot table, the staging ring, its scatter into a
    persistent instance buffer in the cell-plus-offset format, triangles drawn by instance, the
    latched camera buffer and `RenderFeature`.* The scatter and the format are milestone 3's,
    pulled forward so that the ring's row is final. The GPU scene's meshes and materials,
    cluster submission, culling, deferred lighting, skinning and device-lost recovery stay
    milestone 3's. Rules out a staging format that milestone 3 would rewrite.
  - *The presentation schedule at milestone 2 is interpolation, the camera rig's orbit and boom,
    the remote sample and upload, and event delivery.* Boom collision and pose evaluation are
    milestone 3's, the visual blend of a correction and the shadowing of the replicated copy
    milestone 4's, UI animation, the audio listener and markers milestone 5's. Rules out stubs
    for crates that do not exist.
  - *`rhi-api`'s facade follows the native Metal spike alone*, a throwaway program on the M1,
    never checked in. MoltenVK's open cost figures are taken through `rhi-vulkan` by
    `cargo xtask measure gpu-cost`, since a cost is reported or budgeted and changes no
    signature. Rules out a second throwaway Vulkan program. This replaces D139's order, the
    facade behind three spikes.
  - *Every command that reads or writes a swapchain image is submitted by the present thread, on
    a present queue no other thread submits to, in a family that can draw and present, where the
    device offers one; otherwise the roles share the graphics queue.* Under MoltenVK the
    drawable is taken inside such a submit, on the calling thread with the queue held, and a
    present that blocks holds its queue on any driver. The offscreen target is two images or
    more. The present thread holds one at a time, the newest the frame has finished. The frame
    renders only into one the present thread does not hold, after the timeline value the present
    thread signals when its read of it is done, and from undefined, so nothing crosses back. A
    finished target crosses to the present queue by a semaphore and an ownership transfer or by
    concurrent sharing, and is drawn or blitted into the swapchain image. Rules out a frame
    worker blocked on a drawable, a frame submit waiting behind a present, a frame waiting on
    the present thread through its target, and switching off MoltenVK's synchronous submits.
  - *Slang is one pinned release, which the cooker runs as `slangc` and refuses any other, and
    one command installs it on a host.* The cook checks what it emitted: the non-uniform
    decoration on every access through the heap, and one location for each fragment output on
    both targets; Slang's warnings are errors. SPIR-V and MSL are cooked on every host with equal
    bytes. A metallib is cooked on macOS by the cook's own call of `metal`, as MSL 3.1 for macOS
    14, which is thereby the oldest client. Rules out a Slang taken from a host's SDK, a check
    that reads the source alone, and FFI to Slang before process starts cost the cook something.
  - *One command runs the device tests by one rule: `cargo xtask gpu --driver <name>`.* It
    counts the debug messenger, both Metal validations' lines and MoltenVK's own; the Khronos
    layer is pinned on every host; each run has a canary for each validator, in a process of
    its own, and a silent canary or any other message fails it. A device test is an ignored
    test the command selects, a missing device fails the run, and each test compares against a
    reference computed on the
    CPU. lavapipe runs them on every pull request on linux-x64. Rules out a checked-in image
    and a run that passes because no validator was on.
  - *A hazard is proven by the evidence its class has.* A tracked resource: a synchronisation
    validation message. The host and the GPU: a wrong readback with the GPU made to lag.
    Aliasing, reads through the heap, indirect and index buffers, ownership transfers and
    memory that is not coherent: the null backend's rules, each held by a mutation, since no
    machine in reach shows them. Rules out a proof that cannot fail where it runs, and a run
    that showed nothing recorded as an answer.
  - *No GPU runner.* Hardware runs are the M1's at every merge of a GPU crate, as an extra gate
    under MoltenVK and the validators, the Windows laptop's devices after each such push, and
    the desktop's at the measurement commit. Rules out a paid hosted runner and a self-hosted
    runner on a public repository.
  - *`lactose-app` drives the render side through `present-api` and `rhi-api`, and
    `lactose-app-client` wires the backends.* `cargo xtask tiers` applies `[server_denylist]` to
    `lactose-app`'s normal-dependency closure whether or not the server binary exists. Rules out
    a feature of `lactose-app` that turns backend edges on, and a denylist that first bites at
    milestone 4.
  - *`lactose-rhi-vulkan` joins `[test_support]`*, so the graph and the renderer run on a device
    in their own tests. Five crates get a facade first: `rhi-api`, `render-graph`, `renderer`,
    `platform`, `present-api`. Rules out a backend-to-backend exception for tests.
  - *`rhi-api` freezes as section 20 says, at the Vulkan backend's first pass of the suite.* A
    later lane amends it in the change that needs it, called out at its merge, as section 20
    allows and D143 did. A planner's decision at the exit records what is frozen, what was
    amended and what `rhi-metal` may still amend. Rules out a freeze by silence, and a decision
    for each amendment.
  - *A crate added from milestone 2 on has one integration-test binary*, by the rule lane
    `test-binaries` wrote under D125, and no check job's median over its three most recent warm
    runs of main passes ten of its fifteen minutes. Rules out crate lanes pushed onto a job at
    its timeout, a budget read from one run, and runs made again to read it.
  - *The physics phase's stall on render recording is recorded with no threshold* and sets
    `lactose-app`'s default reservation. Rules out a threshold nobody set.
  - *Re-gated*: the directory watch, with its dev session and the mount cost on Linux and
    Windows, to milestone 3; the artist-surfaces decision to milestone 3, where a G-buffer pass
    and `rhi-metal` exist; the mesh and texture load against the 0.5 ms slice to milestone 3;
    device-lost recovery's test to milestone 3; the visual blend of a correction to milestone 4.
- Driven by: architecture section 22 and its exit step in `docs/plans/milestone-1.md`; the RHI
  spike's result (`docs/results/spike-rhi-apple-m1-2026-11-15.md`); the plan's probes of 16
  November 2026 and its two first reviews, whose probes found the empty denylist check, the
  Windows check's real time, MoltenVK's drawable and Slang's three mismatches; the owner's
  ruling of that day that the plan be drafted ahead of milestone 1's exit, and the owner's
  answers of 16 November 2026.
- Replaced: D139's "`lactose-rhi-api`'s facade and `lactose-rhi-null` move to milestone 2,
  behind its MoltenVK, KosmicKrisp and native Metal spikes", in its order alone. D136's and
  D147's placing of the directory watch in milestone 2. Section 20's "on a GPU runner nightly".
  Section 22's placing of the scatter and the cell-plus-offset format wholly in milestone 3,
  and of the client `app` wholly in milestone 4: the client binary arrives at milestone 2 with
  a local world. The status line of `docs/proposals/artist-surfaces.md`. The next revision
  restates each.
```

### Decision entry: the Apple interim path

```
**D<N+1>. The Apple interim path until native Metal is MoltenVK; KosmicKrisp is measured beside it on the development Mac, which moves to macOS 26** (§2 Graphics, §5.1, §12.1, §20, §22, §22.1, `lactose-rhi-vulkan`, `docs/results`)
- Why: section 22.1 asks for the interim path by milestone 2, and section 22 for a MoltenVK and
  a KosmicKrisp spike behind it.
  - *MoltenVK carries section 12.1's list.* Under MoltenVK 1.4.2 on the M1 every item of the RHI
    spike ran with a GPU-side use read back exact, clean under the Khronos layer, Metal API
    validation and Metal shader validation
    (`docs/results/spike-rhi-apple-m1-2026-11-15.md`). What it does not offer, `rhi-api` reports
    as capabilities: whether an attachment of a given usage can be memoryless, whether a
    memoryless attachment keeps its contents across a local read, and indirect count.
  - *Two costs of the path are accepted for one milestone, and one shape it showed is adopted
    for every backend.* Its local read restarts the Metal pass, so the G-buffer in tile memory
    is not shown on Apple before `rhi-metal`. GPU-assisted validation does not run under it, so
    that runs under lavapipe and on the laptop. It takes the drawable inside the submit of any
    command that reads or writes a swapchain image, on the calling thread with the queue held;
    so every such command is the present thread's, on a queue of its own (D<N>).
  - *KosmicKrisp is measured, not adopted.* It is built on Metal 4, which needs macOS 26 (Mesa's
    driver documentation); on macOS 14.7.4 its driver in Vulkan SDK 1.4.363.1 enumerated no
    device. The owner moves the development Mac to macOS 26 once milestone 1's exit is recorded
    and before milestone 2's first lane, so milestone 1 closes on the system its figures were
    taken on and every milestone 2 figure on the M1 is of macOS 26. `rhi-vulkan`'s conformance
    suite, the swapchain's cases and `cargo xtask measure gpu-cost` then run under KosmicKrisp
    beside MoltenVK, on one machine, with timings. Section 22's "KosmicKrisp ... recorded" is
    those files. A case that fails under it is recorded and fails no merge.
  - *MoltenVK stays the driver the M1's gate runs under.* The decision is taken before anything
    has run under KosmicKrisp. MoltenVK also runs on macOS 14 and 15, the oldest of which the
    cook targets. And the path lasts one milestone: `rhi-metal` lands in milestone 3 and carries
    the GPU-driven path, which MoltenVK cannot (D42, D43). A switch of the gate's driver is an
    addendum here, brought to the owner if KosmicKrisp passes every output case of the suite and
    offers a memoryless attachment kept across a local read, GPU-assisted validation that runs,
    or a drawable taken outside a submit.
  - *The upgrade re-bases the M1's figures.* Every M1 result file of milestones 0 and 1 was
    taken on macOS 14.7. The WHISK benchmark and `cargo xtask measure --all` run again in a
    quiet window straight after the upgrade, and the RHI spike's program is run again by its
    file's commands. A figure that crosses a milestone 1 criterion there is a finding for the
    owner; milestone 1 is not reopened. After the upgrade no machine in reach runs macOS 14.
  - Rules out: a gate under a driver nothing has run on; a second gate driver at every merge; a
    KosmicKrisp record taken on a hosted runner's virtual GPU, without timings.
- Driven by: architecture section 22.1; the RHI spike; the plan's probe of 16 November 2026 and
  the GPU review's reading of MoltenVK 1.4.2's source; the owner's answers of 16 November 2026:
  the Mac is upgraded to macOS 26, at milestone 1's exit.
- Replaced: sections 2 and 12.1, which leave the interim path open between the two; the next
  revision names it.
```

### Decision entry: crash reporting

```
**D<N+2>. Crash reporting until a build leaves the owner's machines is the panic hook's text report, shared by every engine binary through `lactose-app`; minidumps and symbol upload are decided at milestone 5** (§3, §5.1, §19, §21, §22.1, `lactose-app`, `lactose-app-headless`, `lactose-app-client`)
- Why: section 22.1 asks for crash reporting, minidumps and symbol upload, by milestone 2, and
  milestone 2 adds a second binary and the first calls into a graphics driver.
  - *The hook is `lactose-app`'s.* It was the headless binary's
    (`crates/app-headless/src/main.rs`), and that binary's README and the verification row "The
    milestone 1 crash hook" gave its successor to `lactose-platform`, which `[server_denylist]`
    denies the server. `lactose-app` is what every engine binary links, and the hook is safe
    code. RICOTTA keeps the hook it has (`tools/cook/src/main.rs`): a tool does not link the
    assembly.
  - *The report is text*: the thread, the panic and a forced backtrace, to stderr and to a file
    in a directory the binary names. A directory that cannot be written costs the file alone
    (`docs/verification.md`, section 3). A subprocess test holds the abort path.
  - *A hardware fault is read from the operating system's report.* macOS writes one by default.
    Windows keeps a local dump once Windows Error Reporting's `LocalDumps` key is set, which is
    off by default and needs an administrator; the owner sets it on the laptop and the desktop.
    The validation layers and these are what diagnose a fault inside a driver call on the
    owner's machines.
  - *No minidump writer, no monitor process and no symbol upload are built.* A writer that
    catches a hardware fault runs in a signal handler, and section 21 admits `unsafe` for that
    in no crate every binary links, or in a second process, which is a monitor to build and
    ship. An upload needs a service and a symbol store, and nothing reads either before a build
    goes to a tester.
  - Rules out: a hook in a crate the server cannot link; a spend on a crash service before
    there is a tester.
- Driven by: architecture section 22.1; `crate-tiers.toml`; the owner's answer of 16 November 2026.
- Replaced: section 22.1's "Crash reporting: minidumps and symbol upload" by milestone 2, which
  moves to milestone 5; sections 3 and 5.1's "crash dump", which the next revision states as
  the text report; the row's and the headless README's "`lactose-platform` ... whose hook
  replaces this one".
```

### If question 6 is asked

The split adds one small lane ahead of the next crate lane and a rule to D<N>'s test-binary
bullet. The raised budget restates section 20 in revision 3.4.

### Verification rows

Each change is its gate and the gist of its new text. Rows are cited at `0810f9f`.

| Row | Gate | Gist of the new text |
|---|---|---|
| 15, MoltenVK and KosmicKrisp | 2 | Both drivers run on the M1 once it is on macOS 26 (D<N+1>): KosmicKrisp needs Metal 4, and on macOS 14.7.4 the SDK's driver enumerated no device. The conformance suite and the swapchain's cases under each give the capability answers, and result file `gpu-cost` under each the encode time and the pipeline creation cost, through `rhi-vulkan` |
| 16, Extract cost | 2 | The workload and machines of D<N>; result file `extract-50k` |
| 18, Native Metal | 2 | Result file `spike-metal`; the freeze decision at the exit |
| 19, Remote transform upload | 2 | The workload of D<N>; result file `remote-upload-5k` |
| 21, Physics phase stall | 2 | Result file `record-stall`, recorded with no threshold; it sets the default reservation |
| 34, MoltenVK multi-draw-indirect is a CPU loop | 2 | Closed by `gpu-cost`'s multi-draw figure and the reading of the pinned MoltenVK |
| 35, Slang's Metal target | 2 | Shown by `spike-metal`'s shaders, then held by a pipeline state made from each cooked shader, on the M1 from MSL and on the runner from the metallib |
| 63, Windows efficiency-core steering | 2 | Lane `eco-qos`; measured on the laptop, a hybrid part, and on the desktop if it is one |
| 95, `lactose-input` is implemented | 2 | The camera rig's check is `lactose-presentation`'s test |
| 97, Artist-defined surfaces | 2 to 3 | Decided with deferred lighting on `rhi-metal`; milestone 2's cooker proves static specialisation and a link-time constant for both targets (D<N>) |
| 99, `lactose-predict` is implemented | 2 | The presentation check is `lactose-presentation`'s test. The visual blend of a correction is milestone 4's |
| 102, Interpolation slot footprint | 2 | The loom model is lane `interp-loom`; the retry rate and the footprint are in `remote-upload-5k` |
| 109, The line-ending rule on a Windows host | 2 | The CI half is met in run 43795852109: 889 files read and none refused on three hosts, and the Windows cache key's last hash equal to the others'. The laptop's refresh remains |
| 134, The milestone 1 crash hook | 2 | The hook is `lactose-app`'s and writes a file too (D<N+2>); a subprocess test holds the abort path; RICOTTA keeps its own; minidumps are decided at milestone 5 |
| 136, SILO's load call | "3 (the baseline, a mesh pool and a texture), 2 (the shader kind)" | Milestone 2 cooks no mesh pool and no texture (D<N>) |
| 139, Mounted packs are never unmounted | 2 and 3 to 3 | The dev session waits for the watch loop, at milestone 3 (D<N>) |
| 140, Mount cost | 2 and 3 to 3 | The Linux and Windows figure is taken with the watch loop's dev session, at milestone 3 |
| 143, The cooked pack set | 2 to 3 | The watch's half is milestone 3's |
| 150, The RHI spike ran under MoltenVK alone | 2 | Closed by lane `vulkan`: the design questions lavapipe can answer from its run; the four hardware questions probed on the laptop's devices and recorded, a run that shows nothing discharging no rule of null; device-only staging as a case on the M1; question 2 answered under KosmicKrisp on the M1 if that driver offers a lazily allocated type, and open on Vulkan if not |
| 164, The keeper's timed wait on Windows | 1 and 2 | The list is read by `frame-design`: an item its design uses becomes a measurement it names, and the rest is re-gated with its reason |

Rows to add:

| Gate | Gist |
|---|---|
| 2 | Milestone 2's criteria are proven by result files and CI runs: `triangle`, `spike-metal`, `gpu-cost`, `rhi-conformance`, `extract-50k`, `remote-upload-5k`, and the first green `gpu (lavapipe)` and `shaders` runs (D<N>) |
| 2 | The development Mac moves to macOS 26 between milestone 1's exit and milestone 2's first lane (D<N+1>). Every M1 result file of milestones 0 and 1 was taken on macOS 14.7: the WHISK benchmark and `cargo xtask measure --all` are run again in a quiet window straight after the upgrade, at main's head. A figure that crosses a milestone 1 criterion there is a finding for the owner, and milestone 1 is not reopened |
| 2 | The RHI spike ran on macOS 14.7.4. Its program is run again on macOS 26, by its result file's commands, before `rhi-api` is briefed: its ten items and its five misuses read as they did, or the difference goes to the planner |
| 2 | KosmicKrisp on the M1 (D<N+1>): that it enumerates the device on macOS 26, the output cases it passes, whether it offers a lazily allocated type and keeps a memoryless attachment across a local read, whether GPU-assisted validation runs under it, and whether it takes the drawable inside a submit. Recorded at `vulkan`'s and `swapchain`'s merges and in `gpu-cost`. Passing every output case and offering one of those brings a switch of the gate's driver to the owner |
| 2 | Which queue families of the laptop's devices have the graphics bit and can present is `gpu-probe`'s to record. Where only the graphics queue's does, the present thread shares that queue under its lock (D<N>), and a blocking present there is a finding for the planner |
| 2 | The pull-request check jobs against ten of their fifteen minutes, by the median of each job's three most recent full-match runs of main, read from the runs main makes anyway. Before lane `test-binaries`, three runs of one code gave the Windows check 9.5 to 11.7 minutes and the macOS check 8.8 to 11.0 (runs 43781979172, 43795852109, 43799365983). When a median passes ten, the owner is asked for a split or a restated budget (D<N>) |
| 2 | What lavapipe on the linux-x64 runner offers is not known: its Mesa version, the local read, indirect count, a headless surface, GPU-assisted validation. Mesa's source at an older commit gives it one coherent memory type and one queue, so the graph's several-queue path runs on null and on the M1 and never in CI. `gpu-probe` records it before `vulkan` is briefed. A runner image change is a driver change |
| 2 | What the macos-arm64 runner offers is not known: its Xcode and `metal` compiler, a Metal device, and the same on its oldest macOS image. `gpu-probe` records it. A metallib's bytes depend on the Xcode that cooked it |
| 2 | The validation layer is pinned by SDK version and checksum on every host (D<N>): Ubuntu 24.04's package is 1.3.275, and the swapchain's semaphore checks need 1.4.313 |
| 2 | Under MoltenVK 1.4.2 the drawable is taken in the submit of a command that uses the swapchain image, with the queue held, by a reading of its source. The present thread has a queue of its own and the frame two targets or more (D<N>). `swapchain` shows that on null, and under MoltenVK only if a minimised window or a sleeping display makes the drawable block for a second: which does, if either, is not known. After MoltenVK's "could not be acquired" the image returns `VK_ERROR_OUT_OF_POOL_MEMORY` until the swapchain is rebuilt, by the same reading; the case shows the rebuild. Section 5.1's present-thread sentence is restated |
| 2 | The development Mac gets the Xcode current for macOS 26 after the upgrade (the owner's answer 5); until then `slangc -target metallib` fails there. Whether that Xcode's `metal` compiler is a component to install apart is not known: the coordinator's re-check after the upgrade records it |
| 2 | SPIR-V and MSL cooked by the pinned Slang are expected equal on linux-x64 and macos-arm64. The `shaders` job is the proof; a difference is a finding for the planner. The heap's two forms and their markers, the outputs' locations and the input attachment's index were found on the SDK's 2026.17-25 build and are run again on the pin |
| 2 | The metallib's language: Slang's own call of `metal` passes `-std=metal3.1`, no deployment target and a changing file name, so the cook calls `metal` itself, as MSL 3.1 for macOS 14, the oldest client it targets. Whether two cooks write equal metallib bytes is recorded. Once the Mac is on macOS 26 nothing in reach runs macOS 14: a load on a hosted runner's oldest image, if it has a Metal device, is the one check left, and that the engine runs on macOS 14 is unshown |
| 2 | A window opened from an agent's shell on the M1, and from the laptop's agent session, is assumed. If either cannot present, that run is the owner's |
| 2 | Windows keeps no local crash dump until `LocalDumps` is set, by an administrator (D<N+2>): the owner sets it on the laptop and the desktop, and `crash-hook`'s report records each machine |
| none | Hardware runs of the device tests are made at merges on the M1 and after pushes on the laptop, since no GPU runner exists (D<N>). A defect that reaches main for want of one reopens the question |
| none | No machine in reach is known to have host-visible memory that is not coherent, or untracked aliasing; a lazily allocated type on Vulkan is KosmicKrisp's to show on the M1. The null backend's rules are the guard, each held by a mutation (D<N>). A machine that has one reopens the row |
| 3 | Device-lost recovery (architecture section 19) is tested when the GPU scene exists. `rhi-api` reports the loss as a value and null injects it from milestone 2 |
| 3 | The device tests of milestone 2 are Vulkan's alone: `lactose-rhi-metal` joins `[test_support]` with its lane, so that fences between aliases and residency get a device test |
| 4 | The prediction world's copy of the local player shadowing the replicated one for extract and the slot table (section 9.4) is tested when `Client` holds both worlds |
| 5 | Minidumps and symbol upload (D<N+2>) |

### Other text

The directory watch's move changes one sentence in each of `crates/asset/README.md`,
`crates/io/README.md` and `tools/cook/README.md`, where "milestone 2" becomes "milestone 3".
`docs/proposals/artist-surfaces.md`'s status line and its two rows gated at 2 name milestone 3.
`crates/app-headless/README.md`'s sentence on the minidump follows D<N+2>, with the `crash-hook`
merge. `crates/app/README.md` says "`app-client` and `app-server` at milestone 4" in three
places: the client binary arrives at milestone 2 with a local world, and those sentences change
with `frame-design`.

## 7. The exit

Milestone 2 exits when these have happened, in this order.

1. The measurement commit exists. The quiet window, the laptop's session and session C have run
   on it, and their result files are on main.
2. Every queue of section 3 has drained, so section 22's delivers are on main: `platform`,
   `rhi-api`, `rhi-null`, `rhi-vulkan`, `render-graph`, the minimal `renderer`, the Slang
   toolchain, `present-api`, and the presentation snapshot and schedule.
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and a nightly that holds the
   client's recording test has compared it equal on three platforms. The run ids are in the
   rows. No check job's median over its three most recent warm runs of main is past ten
   minutes, or the owner has ruled on question 6.
4. The coordinator's pass has left every row of section 2 Done or re-gated.
5. The planner has written the freeze decision (decision 5.13), and the owner has approved it.
6. A `reviewer` and a `reviewer-gpu` read each of the seven criteria against its test and its
   result file. They report one verdict for each: met; met as restated, naming the decision; or
   not met. They re-run what they can on the M1 and state what they could not.
7. The owner rules on every "not met": another lane, or a restatement recorded in the decisions
   log. Milestone 2 does not exit on an unrecorded miss. Whether a project review reads the
   render seams at that commit, as one read milestone 1's, is asked then.
8. The planner writes the milestone 3 plan.

## 8. For revision 3.4

Revision 3.4 is the planner's pass after this plan is approved. It must settle these.

| Sentence | What 3.4 decides |
|---|---|
| Section 2, Graphics: "MoltenVK and KosmicKrisp spiked in milestone 2 as interim Apple paths" | The interim path is MoltenVK, with KosmicKrisp measured beside it on macOS 26 (D<N+1>) |
| Section 12.1: "with every claimed common feature verified in the milestone 2 spike", and its Apple path paragraph | The RHI spike, the native Metal spike and the conformance suite, by name; KosmicKrisp's requirement |
| Section 5.1: "A dedicated present thread calls present so a blocking `nextDrawable` or FIFO present never stalls the event pump or a worker" | The present thread also submits every command that reads or writes a swapchain image, on a queue of its own where the device has one that can draw and present, from one of two or more offscreen targets (D<N>) |
| Section 20: "`rhi-api` stays unstable until the native Metal spike has run" | The spike has run and the Vulkan backend has passed the suite: what the exit decision recorded as frozen, and what `rhi-metal` may amend |
| Section 20: "RHI conformance runs under lavapipe on Linux CI and on a GPU runner nightly" | lavapipe on every pull request; hardware at merges and at the measurement commit (D<N>) |
| Section 20's CI line and section 4's `xtask` line | The `gpu (lavapipe)` and `shaders` jobs; `cargo xtask gpu` and its rule |
| Section 22, milestones 2, 3 and 4 | A paragraph under the table on how milestone 2's criteria are read, as milestone 1 has: the triangle, the two workloads, the machines, KosmicKrisp. The scatter and the cell-plus-offset row format arrive in milestone 2; the GPU scene stays milestone 3's. The client binary arrives in milestone 2 with a local world; the networked client stays milestone 4's |
| Section 22.1 | Crash reporting moves to 5; the Apple interim path is decided |
| Sections 3 and 5.1, "crash dump" | The text report, and that the hook is `lactose-app`'s (D<N+2>) |
| Section 4 | The denylist binds `lactose-app`'s closure, with or without the server binary. `crate-tiers.toml`'s comment changes with lane `server-closure` |
| Section 5.3 | The snapshot's measured budget; what the presentation schedule runs before milestones 3 and 5 |
| Sections 2 and 12.2 | Slang as a pinned release; what the cook checks in what it emitted; a metallib cooked on macOS alone, as MSL 3.1; macOS 14 as the oldest client the cook targets, which no machine in reach runs once the Mac is on macOS 26 |
| Sections 12.4 and 19 | What the minimal renderer is; device-lost recovery tested at milestone 3 |
| Section 14 and D136, D147 | The directory watch at milestone 3 |
| Section 20, the CI budget and results | How the budget is read, the median of each check job's three most recent warm runs of main; its figure, if the owner raises it (question 6). The M1's figures are of macOS 26 from milestone 2 on, and milestones 0 and 1's are taken again there |
| `CLAUDE.md` | The revision marker |

## 9. What depends on milestone 1's exit

This plan assumes milestone 1's criteria are met as written, with one exception a file on main
already shows: tick start on the M1 is 27.0 µs at P99 against 20
(`docs/results/whisk-apple-m1-2026-11-15.md`), which `keeper` fixes or D138 restates at 35 µs.
Nothing in milestone 2 reads the difference.

**Between milestone 1's exit and milestone 2's first lane**, in this order (the owner's answers 1,
5 and the added one):

1. Milestone 1's exit is recorded, on macOS 14.7, the system its figures were taken on. `keeper`
   has run or tick start is restated by then, since its question is of that system.
2. The owner upgrades the Mac to macOS 26.
3. The owner installs the Xcode current for macOS 26.
4. The coordinator re-checks what the upgrade can break, with no lane: the Vulkan SDK's loader,
   MoltenVK and the validation layer; `vulkaninfo` under MoltenVK, and under KosmicKrisp, which
   must now enumerate the M1; the SDK's `slangc`; `xcrun metal`; main's build and its gates. It
   runs the RHI spike's program again by its result file's commands: the ten items and the five
   misuses read as they did on macOS 14.7.4, or the difference goes to the planner before
   `rhi-api` is briefed.
5. The coordinator's quiet window: the WHISK benchmark and `cargo xtask measure --all` at main's
   head, their files checked in beside the macOS 14.7 ones. A figure that moved is read against
   the lanes merged since the old file's commit, and where that does not explain it, that one
   measurement is run again at the old commit. A figure that crosses a milestone 1 criterion is a
   finding for the owner. Milestone 1 is not reopened.
6. `test-binaries` has merged, and the check jobs' medians are read (decision 5.15).

If KosmicKrisp does not enumerate the M1 on macOS 26, the record is that line, decision 5.7
stands with no comparison, and the owner is told.

| Exit outcome | What it changes here |
|---|---|
| Lane `test-binaries` merges (D125), and each check job's median over its three most recent warm runs of main is read | No milestone 2 lane starts before the merge: it edits every crate's tests and `tools/xtask`. The medians are the CI budget milestone 2 starts from; if one is past ten minutes, question 6 is asked before the first crate lane is pushed. Its rule, and what it rules for a recording target, bind every crate milestone 2 adds |
| The re-run on macOS 26 moves a figure across a milestone 1 criterion. The margins are thin for the prediction slot, 90.0 µs against 100, and the wake from an idle pool, 31.6 µs against 35 | The owner rules: a restatement, or a lane. A lane in `lactose-task`, `lactose-predict` or `lactose-physics-rapier` runs beside milestone 2's and edits none of its crates |
| Criterion 1, the 100k tick on eight performance cores, is not proven: session B has not run. If it misses, the owner rules for another ECS lane or a restatement | An ECS lane edits `lactose-ecs` first; `present-impl` waits behind it. Section 4's arithmetic for extract assumes the ECS at the speed `tick-100k` measured on the M1 under macOS 14.7 |
| Tick start: `keeper` runs after session A's files, or the criterion is restated | `keeper` edits `lactose-task`. `eco-qos` waits behind it, and `frame-design` reads its outcome for frame pacing |
| Criterion 9, edit-to-playing on Windows and Linux, is session B's. Over thirty seconds brings a build-time proposal | That proposal can change the workspace's build layout, and decision 5.15's figure is read again with it |
| Session A returns the desktop's CPU, GPU and second system | Section 4's desktop row, and what session C can run under the second system |
| Rows that wait on the desktop or on `keeper`: 64, 94, 123, 133, 149, and 77, 79, 80, 81 and 164's first half. The owner may re-gate any of them at exit step 6 | A row re-gated to 2 joins section 2's table. Session C can carry sessions A and B's commands |
| Row 167, the physics hash, waits on the first nightly at or after `10cf89e` | A difference between platforms is a finding in `lactose-physics-rapier` and reaches no milestone 2 lane |
| Exit step 7, the follow-up project review of the determinism and readiness seams | Its findings in `lactose-app`, `lactose-predict`, `lactose-ecs` or `lactose-net-host` are fixed or routed before `frame-design`, `present-impl`, `presentation` and `interp-loom` start |
| Another decision is applied before this plan | The decision numbers of section 6 |
| Main moves | Every line number here. The plan is re-based before it is approved |

One piece of work depends on nothing the exit can change: the coordinator's `gpu-probe`, which
reads the runners and the laptop. The owner may release it before the exit. `spike-metal`
touches no file of the tree, but it waits for macOS 26, since the facade is written from it.

## 10. What this plan could not verify

Checked against documentation for revision 2, on 16 November 2026, through summarising fetches:

- **Xcode.** Apple's support page lists Xcode 16.2 as the newest that runs on macOS 14, and Xcode
  26 as running on macOS 15.6 to 26. Which Xcode is current when the Mac is upgraded, its size,
  and whether its `metal` compiler is a component to install apart, were not checked.
- **The runner's layer.** Ubuntu's package index gives noble `vulkan-validationlayers`
  1.3.275.0.
- **The command pool rule.** The Vulkan specification's page for `vkResetCommandPool` carries
  `VUID-vkResetCommandPool-commandPool-00040`: no command buffer of the pool may be pending.
- **Windows local dumps.** Microsoft's page on collecting user-mode dumps: off by default, set
  under `HKEY_LOCAL_MACHINE`, an administrator's change, with a key for each application.
- **Memory types on PC drivers.** One survey article lists NVIDIA's, AMD's and Intel's types on
  Windows: every host-visible type is coherent, and none is lazily allocated. It is a survey,
  not the laptop's `vulkaninfo`, which `gpu-probe` takes.
- **lavapipe.** Mesa's `lvp_device.c`, at an older commit on a mirror: one memory type, device
  local, host visible, coherent and cached; one queue family with one queue; indirect count
  offered. That commit lists neither the local read nor a headless surface, and says nothing of
  the runner's version.
- **Slang's releases.** The project's release page lists v2026.17 to v2026.22. The archives for
  macOS and Windows were not seen on the page, and no release was run.
- **The `metal` compiler's default language version** follows its deployment target, by a
  search's summary of its manual page; the page itself was not fetched. The cook does not rest
  on it, since it passes the version itself (decision 5.8).

Still from memory, a reviewer's or this plan's, and not checked:

- **NVIDIA's handling of image layouts and ownership transfers**, and whether a missing
  non-uniform decoration shows on it. Decision 5.17 does not rest on the answer.
- **lavapipe executing in order**, so that a dropped barrier rarely shows there.
- **GPU-assisted validation reporting a heap image in the wrong layout.**
- **The laptop's second device.** An Intel integrated GPU is expected of its CPU and was not
  listed.
- **GPU runners.** That a hosted GPU runner is paid and that a self-hosted runner is unsafe on a
  public repository are GitHub's documentation as remembered.

Not verified at all:

- **No GPU program was built for this plan.** Every fact about the RHI is the RHI spike's result
  file and its reviews. No crate was compiled: `ash`, winit and a Metal binding are assumed to
  build on the pinned toolchain and to pass `cargo deny`.
- **MoltenVK's drawable** (decision 5.6) is the GPU review's reading of MoltenVK 1.4.2's source.
  Its probe took drawables from a layer in no window, which does not block. Whether a windowed
  layer blocks from an agent's shell was not probed: it opens a window on the owner's display.
  Whether a minimised window or a sleeping display makes `nextDrawable` block for a second is
  not known, so `swapchain`'s MoltenVK case may show nothing and say so. That an image whose
  drawable was not acquired returns `VK_ERROR_OUT_OF_POOL_MEMORY` until the swapchain is rebuilt
  is a reading of the same source, not a run.
- **The present queue on other drivers.** That a PC driver offers a second queue that can draw
  and present is `gpu-probe`'s to record. From a reviewer's memory, NVIDIA's graphics family has
  several queues and AMD's and Intel's have one. That a blocking present holds its queue is
  Vulkan's rule that the caller serialises a queue's uses, not a measurement of any driver's
  present. What MoltenVK does with its synchronous submits switched off was not run.
- **Slang's call of `metal`** is the GPU review's probe with a stand-in compiler on the SDK's
  copy. Whether a metallib's bytes carry its input's name was not checked, which is why the cook
  calls `metal` itself and claims no equal bytes.
- **The cold pipeline figure.** That Metal's system cache serves the unchanged MSL source across
  salted specialisation constants is suspected, not shown; `gpu-cost` gives both figures.
- **KosmicKrisp.** Its requirement is Mesa's driver documentation, beside one probe on macOS
  14.7.4: with the loader pointed at the SDK's KosmicKrisp driver alone, `vulkaninfo --summary`
  fails with "Failed to detect any valid GPUs in the current config". Nothing has run under it.
  That it enumerates the M1 on macOS 26, passes any case, offers a lazily allocated type, runs
  GPU-assisted validation or takes the drawable outside a submit is unknown until the upgrade.
- **macOS 26.** The RHI spike, both GPU reviews' probes and every figure of milestones 0 and 1
  are of macOS 14.7.4. That MoltenVK 1.4.2 and the installed SDK behave there as they did, and
  that the M1's figures hold, is assumed until the coordinator's re-check and the quiet window.
- **The oldest client.** Once the Mac is upgraded nothing in reach runs macOS 14 or 15. That a
  metallib cooked for macOS 14 loads there rests on a hosted runner's oldest image having a Metal
  device, which was not looked at; that the engine runs there is not shown by anything.
- **The macOS runner** was not looked at: its Xcode, its `metal` compiler and whether its virtual
  GPU runs Metal or MoltenVK tests.
- **Slang's behaviour** is that of the SDK's `slangc`, version 2026.17-25-gc7954ebd1, in this
  plan's probe and the GPU review's: SPIR-V and MSL from one source; the metallib target failing
  with "failed to load downstream compiler 'metal'"; the heap, the outputs and the input
  attachment as decision 5.8 states them; a link-time constant folding into both targets; MSL
  equal between two checkouts with line directives off. None of it was run on the pin, and
  equality across hosts was not run. Slang's licence was not read against `deny.toml`, which a
  subprocess does not consult.
- **A window.** Whether the laptop's agent session can open a window and present was not
  checked, nor whether the M1 can from an agent's shell.
- **The desktop.** Its CPU model, its GPU and its second system are in no document.
- **The CI budget after the conversion.** The check jobs' figures are the general reviews'
  tables. Runs 43781979172 and 43799365983 were confirmed full cache matches and 43795852109 a
  partial restore; the cause of the spread between them is not known. What `test-binaries`
  gains on the runners is not measured: `lactose-ecs` on a loaded M1 is the one figure. That a
  run made again joins the workflow's concurrency group is GitHub's documented behaviour, not
  tried.
- **Crash handling.** No minidump crate was read. Decision 5.9 rests on where such a writer must
  run, not on one crate's API. What macOS and Linux write for a fault by default was not tested
  on the machines.
- **The run counts** are this plan's estimates by milestone 1's conventions. Milestone 1's lanes
  took two to five review rounds.
- **The 70 ns for each dirty row** is arithmetic. No extract exists to measure.
