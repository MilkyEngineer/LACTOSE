# Milestone 2 plan: Pixels

Status: draft, not approved. Written at main `6f0508a`, architecture revision 3.3, on 16 November
2026, ahead of milestone 1's exit by the owner's ruling of that day. It is reviewed now. It is
re-based on main and approved only when milestone 1 exits, and the coordinator applies it then.
This plan maps each milestone 2 criterion of architecture section 22, and every
`docs/verification.md` row that gates milestone 2, to a lane, the coordinator or the owner. It
orders the lanes and takes the decisions that order needs, the two of section 22.1 that are due by
milestone 2 among them. It proposes three `docs/decisions-log.md` entries and the verification row
changes that go with them (section 6). It amends no architecture sentence: revision 3.4 is a
separate pass, and section 8 lists what it must settle. Section 9 names every place where this
plan depends on how milestone 1 exits. Section 10 says what it could not verify.

Conventions:

- A verification row is cited by its line number in `docs/verification.md` at `6f0508a` and by its
  opening words, since line numbers move.
- Two approved texts are not applied at `6f0508a`: the line-ending rule (D148) and milestone 1's
  exit rows (D149). The coordinator's exit pass is a draft. Where a row's gate comes from one of
  them, this plan reads the row as that text leaves it and says so.
- A name in code font such as `rhi-null` is this plan's handle for a lane (section 3). Where it is
  also a crate's short name, the lane is that crate's.
- Cost is counted in agent runs, as `docs/plans/milestone-1.md` counts it. An implementation lane
  is seven: the lane, its review, about two fix rounds with a diff review each, and the merge. A
  second or third reviewer beside the first review adds one each: `reviewer-gpu` on work that
  records, submits or synchronises GPU work, defines the surface it goes through or produces what
  it runs, and `reviewer-concurrency` on work that runs on several threads (`docs/agents.md`,
  section 2). A third fix round this plan expects adds two. A small lane is five. A facade is six,
  and seven where `reviewer-gpu` reads it too. A spike that delivers a result file is six: the RHI
  spike took seven, with three review rounds. The totals are a floor.
- "The discrete GPU" is the Windows laptop's. "The desktop" is the owner's x86 desktop.
- A probe of this plan is a command run for it on the development Mac on 16 November 2026, outside
  the tree. Section 10 lists them.

## 1. The criteria

Section 22 gives milestone 2 seven proofs. At `6f0508a` none of the eight crates it delivers
exists, nor the client binary; `crate-tiers.toml` already names each of them. The RHI spike has
run under MoltenVK (`docs/results/spike-rhi-apple-m1-2026-11-15.md`).

| # | Criterion | What proves it | Machine | File or run | Closed by |
|---|---|---|---|---|---|
| 1 | A triangle through the graph on Windows and macOS | The client binary run for a fixed number of frames: a local world whose rows move, extract each frame, instanced triangles at the extracted transforms drawn by a `render-graph` pass after the scatter pass, presented to a swapchain. The last frame's colour target is read back and compared with a CPU reference at a sub-pixel margin, as the RHI spike compared its triangle, with no message from any validator (decision 5.1) | Windows: the discrete GPU (question 2). macOS: the M1 under MoltenVK (decision 5.7) | `triangle` | `frame-impl`, `measure-m2` |
| 2 | MoltenVK, KosmicKrisp and native Metal spikes recorded, and `rhi-api` frozen or amended on their findings | Three records and a decision. MoltenVK: the RHI spike's file, and its open cost figures taken through `rhi-vulkan`. Native Metal: a throwaway program on the M1. KosmicKrisp: `rhi-vulkan`'s conformance suite under it where macOS 26 runs (question 1). The planner's freeze decision at the exit (decision 5.13) | The M1; a hosted macOS 26 runner for KosmicKrisp | `spike-rhi` (on main), `spike-metal`, `gpu-cost`, `rhi-conformance`; `docs/plans/rhi-freeze-decision.md` | `spike-metal`, `vulkan`, `measure-m2`, the exit |
| 3 | Overlap with exclusive extract | Tests of the client's frame, by ordering and never by timing. (a) No stage of a tick starts before extract's pass has resolved, and no frame reads the snapshot buffer or the ring region extract is writing. (b) Frame N's record and tick N+1's stages are in flight at once: each is held at a barrier until the other has started, so the test hangs if they run in turn. (c) A recording test steps one world with the render side attached, on `rhi-null`, and without it, and SOUR finds the hashes equal in every mode | The M1 at the merge; the three CI platforms for (c) | The nightly's run id, in the row | `frame-design`, `frame-impl` |
| 4 | Extract at most 1 ms for 50k dirty rows | A cost test: 100,000 rows with a `Transform`, 50,000 of them moved every tick, extract's pass alone, the median of 60 passes (decision 5.2) | A quiet M1; the desktop | `extract-50k` | `present-impl`, `renderer-impl`, `measure-m2` |
| 5 | Remote transform upload at most 1 ms for 5k entities at 144 Hz | A cost test: 5,000 slots of the interpolation buffer written at 30 Hz, sampled in one batch and written to the staging ring every frame, paced at 144 Hz for ten seconds, the median frame (decision 5.3) | A quiet M1; the desktop | `remote-upload-5k` | `presentation`, `measure-m2` |
| 6 | lavapipe CI | A job in the pull-request workflow on linux-x64: `cargo xtask gpu --driver lavapipe`, the conformance suite and the graph's and the renderer's device tests under the Khronos validation layer, with no message (decision 5.10) | The linux-x64 runner | The first green run's id, in the row | `vulkan` |
| 7 | macOS shader runner | A job on macos-arm64 that cooks every engine shader to SPIR-V, MSL and a metallib with the pinned Slang and the runner's `metal` compiler, and loads each metallib. The linux-x64 job cooks SPIR-V and MSL, and the two hosts' hashes are equal (decision 5.8) | The macos-arm64 and linux-x64 runners | The first green run's id, in the row | `slang` |

Section 22.1 puts two decisions at milestone 2. The Apple interim path is decision 5.7. Crash
reporting is decision 5.9.

Milestone 1 sent three measurements here. The stall the physics phase imposes on render recording
(row 21, architecture section 16) is `record-stall`, recorded with no threshold (decision 5.16).
The presentation snapshot's budget (section 5.3) is reported inside `extract-50k`, as the
snapshot's part of the pass. The remote upload's cost at 144 Hz (section 5.3) is criterion 5.

## 2. The verification rows

Every row whose gate names milestone 2 has an owner below, or is re-gated with its reason. "Pass"
is the coordinator's verification pass. Section 6 gives each row's change as its gate and the gist
of its new text.

| Row | Opening words | Owner | What closes it |
|---|---|---|---|
| 15 | MoltenVK and KosmicKrisp | `measure-m2`; question 1 | MoltenVK's open figures, the encode time for ten thousand draws and the pipeline creation cost, are `gpu-cost` through `rhi-vulkan` on the M1. KosmicKrisp is the conformance suite's run where macOS 26 runs, or one line that it needs macOS 26 |
| 16 | Extract cost | `present-impl`, `measure-m2` | `extract-50k`: the pass's time, and the cost per dirty row on one worker |
| 18 | Native Metal | `spike-metal`, then the freeze decision | The spike's result file: argument buffers, residency, memoryless attachments, framebuffer fetch, indirect draws. `rhi-api` is frozen or amended at the exit |
| 19 | Remote transform upload | `presentation`, `measure-m2` | `remote-upload-5k` |
| 21 | Physics phase stall | `frame-impl`, `measure-m2` | `record-stall`, recorded |
| 34 | MoltenVK multi-draw-indirect is a CPU loop | `measure-m2` | `gpu-cost`: one multi-draw of N commands against N draws under MoltenVK, beside a reading of the pinned MoltenVK |
| 35 | Slang's Metal target supports argument buffers, function constants and framebuffer fetch | `spike-metal`, `slang` | The spike's shaders are MSL the Slang compiler emitted and use all three. The cooker's are held by the conformance suite's pipeline case |
| 63 | Windows efficiency-core steering | `eco-qos` | The benchmark's rows with two background threads, before and after, on the laptop, which is a hybrid part |
| 95 | `lactose-input` is implemented | `presentation` | The camera rig's forward and yaw equal `PlayerInput::aim` and `Yaw::rotation` for pinned samples |
| 97 | Artist-defined surfaces behind fixed pipelines | Re-gated to 3 | Decision 5.14. `slang` proves the mechanism only |
| 99 | `lactose-predict` is implemented (gate 2 once D149 is applied) | `presentation` | A re-spawned body without `Teleported` interpolates between two snapshots, and one with it does not |
| 102 | Interpolation slot footprint | `interp-loom`, `measure-m2` | A loom model of the sequence lock. The reader's retry rate under one writer at 30 Hz and the footprint are in `remote-upload-5k` |
| 133 | The milestone 1 crash hook | `crash-hook` | Decision 5.9: the hook is `lactose-app`'s, and the report also goes to a file |
| 135 | SILO's load call against section 5.1's 0.5 ms slice | `slang`; the rest re-gated to 3 | Milestone 2 cooks no mesh pool and no texture. The shader kind's load is measured against the slice (decision 5.14) |
| 138 | Mounted packs are never unmounted and published versions never unloaded | Re-gated to 3 | Question 4: the directory watch moves to milestone 3, and its dev session with it |
| 139 | Mount cost (gate 2 and 3 once D149 is applied) | Re-gated to 3 | Question 4: D149 sends the Linux and Windows figure to the watch loop's dev session |
| 142 | The cooked pack set of a fresh process | Re-gated to 3 | Question 4: the watch's half |
| 149 | The RHI spike ran under MoltenVK alone | `vulkan` | The suite under lavapipe settles the spike's eight design questions. The discrete GPU settles the four it leaves to hardware. Question 2 needs a lazily allocated memory type: if neither lavapipe nor the discrete GPU offers one, it stays open on Vulkan, and `spike-metal` answers it for Metal |
| 163 | The keeper's timed wait on Windows (gate 1 and 2 once D149 is applied) | `frame-design`, `measure-m2` | The unmeasured list is read where frame pacing on Windows is designed, and what that design uses is measured on the laptop |
| new | The line-ending rule on a Windows host (D148) | Pass | The first green run of the line-ending check on the Windows runner, and the owner's refresh of the laptop's checkouts |

If the owner keeps the directory watch in milestone 2 (question 4), rows 138, 139 and 142 keep
their gates and are owned by the two lanes that question names.

## 3. The lanes

No lane is in flight for milestone 2. Each row is the size of a brief.

**Queue 1, the RHI.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `spike-metal` | The native Metal spike: a throwaway program on the M1 against Metal itself, never checked in, as the milestone 1 plan's decision 5.10 ruled for the RHI spike. Its question: what must `rhi-api` expose for section 12.1's list to run on Metal with no call Vulkan or a null backend cannot honour? It walks a bindless heap as an argument buffer indexed past slot 0 with a non-uniform index; residency declared per encoder; a memoryless G-buffer read in its pass by framebuffer fetch, and a memoryless depth attachment kept across that read; two transients that alias one heap with the fence between them; an indexed indirect draw and an indirect command buffer; a pipeline from MSL the Slang compiler emitted (the SDK's copy, its version in the file), specialised by a function constant; a binary archive saved and reloaded in a second process with fail-on-miss; encoders recorded on two threads; a shared event waited on before it is signalled; a sampler; a readback against a CPU reference | Nothing. `reviewer-gpu` reviews it alone | Result file `spike-metal`. Each item yes or no with the state the caller carried. Metal API validation and Metal shader validation each shown on by a misuse it reports, a use without residency among them. Each of the RHI spike's fourteen points marked kept, changed or Vulkan's alone. Rows 18 and 35 | 6 |
| `rhi-api` | Architect: the facade of `lactose-rhi-api`, from the RHI spike's fourteen points, `spike-metal`'s result and section 12.1. Capabilities are required or reported. The local read is a call of its own. Memoryless is a request. A pass declares what it reaches through the heap. Aliasing is a relation the backend fences. Device loss is a value. A surface is made from `raw-window-handle`'s types, so no backend names `lactose-platform`. The conformance suite's shape: its cases by name, each marked contract, which null passes too, or output, which only a device can | `spike-metal` merged | A facade, unstable (D43). `reviewer` and `reviewer-gpu` both read it. Every call is one a null backend can honour. Decision text for what section 12.1 left open | 7 |
| `rhi-null` | `lactose-rhi-null` against the facade, and the bodies of the conformance suite in `lactose-rhi-api` behind a dev-only feature, as `lactose-asset-api`'s is. Null keeps the state the caller carries between calls and refuses each misuse the contract names | `rhi-api` merged | Every contract case passes on null. For each of the five misuses the RHI spike's validators missed (`local`, `split`, `no-qfot`, `heap-layout`, `uuwp-used`) null refuses the call, and a build with that rule removed fails its test. Null stays test support and outside the server (`cargo xtask tiers`) | 8 |
| `vulkan` | `lactose-rhi-vulkan` without a window: device creation against the required and reported sets, queues by role, buffers, images, samplers, the heap, pipelines from cooked SPIR-V with specialisation, the cache blob, an encoder per thread, states to barriers, the local read, memoryless requests, timeline semaphores and fences, indirect draws and dispatch, copies, the message sink. `SAFETY.md`. `cargo xtask gpu` (decision 5.10). The `gpu (lavapipe)` job and its nightly form with GPU-assisted validation. `lactose-rhi-vulkan` joins `[test_support]` (decision 5.12) | `rhi-null` merged, for the suite; it may start when `rhi-api` merges. The facts of the coordinator's `gpu-probe` | The output cases pass under lavapipe in CI and under MoltenVK on the M1 with no message, each validator shown on by an injected misuse. After the push, the suite on the discrete GPU, as labelled data and as the answers to the spike's four hardware questions. The spike's eight design questions answered from the lavapipe run (row 149). A cache directory that cannot be written is reported and the run goes on. Pushed alone: it changes `Cargo.lock` and a workflow | 10 |
| `swapchain` | The swapchain and present in `lactose-rhi-vulkan`, from `lactose-platform`'s window: acquire, present on the present thread, one present semaphore for each image, resize and out-of-date, shutdown with frames in flight | `vulkan` and `platform-impl` merged | On the M1 and the laptop, a fixed number of frames presented across a resize with no message, and an out-of-date swapchain rebuilt. Under lavapipe on a headless surface if the runner's driver offers one; if not, the lane says that the path runs on the two machines alone | 8 |

**What the RHI spike left open, and where it lands.** The spike settled `rhi-api`'s Vulkan
shape: its fourteen points, and five rules for null. Its "Not shown" list is owned as follows.

| Left open by the spike | Lands in |
|---|---|
| The swapchain and present | `swapchain` |
| Residency, heaps, indirect command buffers, Metal libraries and the binary archive, aliased transients on Metal, parallel Metal encoding | `spike-metal` |
| An ownership transfer's effect; memory that is not coherent; a buffer in device-only memory filled through a staging copy; a heap indexed without the non-uniform decoration; a heap image in the wrong layout | `vulkan`, on the discrete GPU |
| The eight design questions for a conformant driver; GPU-assisted validation of the address and heap misuses | `vulkan`, under lavapipe |
| Samplers; indirect dispatch | `vulkan`, as cases of the suite |
| Transients that alias memory on Vulkan, and the barrier between them; a missing barrier with the GPU made to lag | `graph-impl`, `renderer-impl` |
| SPIR-V cooked by Slang | `slang` |
| Multi-draw-indirect's cost, the encode time for ten thousand draws, pipeline creation cost | `measure-m2`, as `gpu-cost` |
| 64-bit atomics | Milestone 3 (row 36) |

**Queue 2, the frame.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `platform` | Architect: the facade of `lactose-platform` [PAIL]: the window and its handles, the event pump on the main thread over `lactose-task`'s `LanePump`, the present thread and its pump, the `Platform` the client passes as `Workers::platform`, what a close and a resize report. Device sources and action mapping stay milestone 5's | Nothing | A facade and decision text. `crates/app/README.md`'s sentence on `Workers::platform` agrees | 6 |
| `platform-impl` | Implements it over winit on Windows and macOS. It builds on linux-x64, where CI lints it. `unsafe` for the present thread's QoS and the window handles, with `SAFETY.md` | `platform` merged | A test that runs a subprocess: it opens a window, pumps events, runs one call on the main thread and one on the present thread, and closes, on the M1 and the laptop. A worker that waits on the main thread fails a test. The Windows and Linux cross-target clippy. `reviewer-concurrency` reads beside the first review. Pushed alone | 8 |
| `graph` | Architect: the facade of `lactose-render-graph` [GOUDA]: passes with reads and writes by usage, imported and transient resources, the history ping-pong, compile and its cache key, record on frame workers, queue assignment, submission in graph order | `rhi-api` merged | A facade. `reviewer` and `reviewer-gpu` both read it | 7 |
| `renderer` | Architect: the facade of the minimal `lactose-renderer` of decision 5.4 and no more: the render slot table by `StableKey`, the staging ring's writer, the scatter, the instanced triangle pass, the latched camera buffer, `RenderFeature` | The `graph` and `present-api` facades merged | A facade. `reviewer` and `reviewer-gpu` both read it. Any math crate it takes leaves the shipped binaries' boundary feature trees identical (`cargo xtask features`) | 7 |
| `frame-design` | Architect: the client's frame in `lactose-app`, and what `lactose-app-client` wires. The frame's order: the ticks, `Simulation::run_passes`, the snapshot's publication, then the next ticks beside the presentation schedule, graph compile, record, latch and submit. Who paces it against the present thread. The traits of `present-api` and `rhi-api` through which `app` drives the render side without naming a backend (decision 5.11). The physics reservation for render recording. Where criterion 3's recording test lives. Frame pacing on Windows, read against the keeper's unmeasured list (row 163) | The `platform`, `graph`, `renderer` and `present-api` facades merged. Milestone 1's exit step 7 routed (section 9) | A facade and decision text that amends D131's surface. `cargo xtask tiers` passes with the server's edges unchanged | 6 |
| `graph-impl` | Implements the graph: order, culling, transient lifetimes, aliasing, transitions, queues, the capped compile cache, record on frame workers with one span for each pass | `graph` and `rhi-null` merged; `vulkan` merged for its device tests | On null: a build that drops a derived transition fails a test; an unreachable pass is culled and a pass whose only output is a readback or an import is kept; each input of the cache key, changed alone, forces a recompile; two transients alias and the second starts undefined. On devices through `cargo xtask gpu`: the same graphs with no message under synchronisation validation, and a build that drops a barrier fails with the GPU made to lag. `reviewer-gpu` and `reviewer-concurrency` read beside the first review | 11 |
| `renderer-impl` | Implements the minimal renderer | `renderer`, `graph-impl`, `vulkan` and `slang` merged | Slots are allocated on the CPU, freed from the structural log and reconciled after a restore, shown with a prediction world's rollback. A slot named twice in one frame's batch, freed and reallocated or written by extract and by the remote upload, holds the later write, on a device. The ring never overwrites a region a frame in flight reads, with the GPU made to lag. A device test steps a world, extracts, renders and compares the readback with the CPU reference. Steering: the ring's write cost for 50k rows. Both specialist reviewers read beside the first review | 9 |
| `frame-impl` | The frame in `lactose-app` and the binary `lactose-app-client`: a local world, the cooked pack set mounted through `lactose_asset::pack_set`, the pipeline cache file, resize, and a mode that runs a fixed number of frames and reads the last one back | `frame-design`, `present-impl`, `presentation`, `renderer-impl`, `swapchain` and `crash-hook` merged | Criterion 3's three tests; the package that holds the recording test is a `[harness]` package. Criterion 1's run on the M1 and on the laptop. The server denylist holds, and the client's boundary feature tree equals the server's. Both specialist reviewers read beside the first review, and a third fix round is expected | 11 |

**Queue 3, presentation, shaders and tools.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `present-api` | Architect: the facade of `lactose-present-api`: the mesh, light, decal and camera rig components, VFX and audio request events, UI view-model resources, the debug-draw buffer, and the presentation snapshot, triple-buffered. Extract is its pass system, as the gather is `net-api`'s (D121), and writes transforms through a sink the renderer's ring implements. It names no RHI, platform or `net-host` type, so it stays what `[boundary_internal]` admits | Nothing | A facade and decision text. The consumer rows of `crates/anim/README.md`, `crates/physics-api/README.md` and `crates/predict/README.md` agree | 6 |
| `present-impl` | The snapshot and extract | `present-api` merged. No other lane edits `lactose-ecs` while it does | By counts: the rows dirty in this tick or the one before are written and no other; a row that stopped moving has its previous transform set; a teleport carries the no-interpolate flag; the pass after a restore or a load is full. A reader that interpolates two snapshots never reads the third while extract writes it, shown by a loom model or under Miri, whichever the facade's design admits. `reviewer-concurrency` reads beside the first review. Steering: extract's time at 50k dirty rows on null | 8 |
| `interp-loom` | A loom model of `lactose-net-host`'s sequence lock (row 102), behind an optional feature as `lactose-input-host`'s is; the rule of `cargo xtask features` that names the crates allowed to name loom gains it | Nothing | The model passes, and fails without each ordering the crate states. The nightly loom job runs it | 6 |
| `slang` | The Slang toolchain in `tools/cook` [RICOTTA]: the shader kind, cooked by the pinned Slang release to SPIR-V and MSL on every host and to a metallib on macOS (decision 5.8); the cooked format `rhi-api` names; the check that every heap index carries the non-uniform decoration; the `shaders` job. One engine shader is written against an interface specialised statically and a link-time constant, for both targets | `rhi-api` merged; question 5's answer | Cooking twice writes the same bytes, whatever the checkout's path. Criterion 7. A pipeline from the cooked SPIR-V passes the suite's pipeline case. The cook refuses a heap index without the decoration. The shader kind's load against the 0.5 ms slice (row 135). `reviewer-gpu` reads beside the first review. Pushed alone | 8 |
| `presentation` | `lactose-presentation`: the schedule per frame on the snapshot, interpolation between two snapshots, the camera rig's orbit and boom, the batch sample of the interpolation buffer and the remote upload through the ring, presentation events delivered once (decision 5.5) | `present-impl` and the `renderer` facade merged | Rows 95 and 99. Each remote slot is sampled and written once a frame. A delivered event is not delivered again after a rollback. Steering: the upload's time for 5k slots. `reviewer-concurrency` reads beside the first review | 8 |
| `eco-qos` | Windows efficiency-core steering (row 63): `ThreadClass::Background` sets the power-throttling thread setting, in `crates/core/src/thread_class.rs`. Whether `ThreadClass::Io` follows is reported and not decided | `keeper` merged or closed (section 9) | On the laptop, the WHISK benchmark's rows with two background zstd threads, before and after. Miri for `lactose-core` | 5 |
| `crash-hook` | The crash hook moves from the headless binary into `lactose-app`, shared by every binary, and writes its report to a file as well as to stderr (decision 5.9) | Question 3's answer | A subprocess test: a panic in the headless binary writes the report to both and aborts, and a directory that cannot be written costs only the file. This closes the row's "checked by inspection only" | 5 |
| `measure-m2` | `cargo xtask measure` gains `extract-50k`, `remote-upload-5k`, `record-stall`, `gpu-cost` and `triangle`, their tests in `lactose-app-client`, and the header of section 4 for a file taken on a device | `frame-impl` merged | One command writes every file on the M1. The measurements that need no device run short in the `measure` workflow, their numbers discarded. Times are printed, never asserted. `reviewer-gpu` reads beside the first review | 8 |

**The coordinator's passes.**

- `gpu-probe`, first, pushed alone: a manually dispatched job that prints what each runner offers.
  On linux-x64, the Mesa version and lavapipe's `vulkaninfo`: the local read, a lazily allocated
  memory type, indirect count, a headless surface, and whether the validation layer and
  GPU-assisted validation load. On macos-arm64, the Xcode version, whether `xcrun metal` exists,
  whether a Metal device is present, and on a macOS 26 image whether KosmicKrisp enumerates it.
  Its output goes into the rows section 6 adds. The io_uring probe of the nightly is the
  precedent.
- The verification pass, after the `vulkan` merge and again at the exit: section 2's Done marks
  and section 6's rows.
- The merge brief of every branch that changes `lactose-rhi-vulkan`, `lactose-render-graph`,
  `lactose-renderer`, `lactose-presentation` or the client binary names one extra gate,
  `cargo xtask gpu --driver moltenvk`. After its push, the laptop runs
  `cargo xtask gpu --driver system` and sends the file back.

**Order.** Three tracks run at once, with the document work beside them. Each queue is in
dependency order, and the coordinator refills a free slot from any queue whose head is unblocked.

| Queue | Lanes, in order | Runs |
|---|---|---|
| 1, the RHI | `spike-metal`, `rhi-api`, `rhi-null`, `vulkan`, `swapchain` | 39 |
| 2, the frame | `platform`, `platform-impl`, `graph`, `renderer`, `frame-design`, `graph-impl`, `renderer-impl`, `frame-impl` | 65 |
| 3, presentation, shaders and tools | `present-api`, `present-impl`, `slang`, `presentation`, `measure-m2`; beside them `interp-loom`, `crash-hook` and `eco-qos` | 54 |

The lanes are 158 runs. Document work beside them is about 15: this plan's review and answer (2),
its re-base at approval and that diff's review (2), revision 3.4 (3), the freeze decision (3) and
the exit (5). The whole is about 175 runs at two fix rounds a lane, and about 210 if a third of the
lanes take four or five. The owner's other answers add to it: the directory watch about sixteen
runs, a minidump writer about nine, converting the existing crates' tests seven.

The longest chain is `spike-metal`, `rhi-api`, `rhi-null`, `graph-impl`, `renderer-impl`,
`frame-impl`, `measure-m2`: about sixty runs deep. The first slot that frees goes to that chain's
head. `interp-loom`, `crash-hook` and `eco-qos` need nothing in their queue and fill a slot no
head can take; `crash-hook` merges before `frame-impl` starts.

Five facades come first: `rhi-api`, `graph`, `renderer`, `platform` and `present-api`, and
`frame-design` amends a sixth, `lactose-app`'s. `lactose-rhi-null` and `lactose-rhi-vulkan`
implement `rhi-api`. `lactose-presentation`'s one consumer is the client's frame, which
`frame-design` covers. `tools/cook` is a leaf.

Seven rules bind whatever the coordinator reorders:

- One lane at a time edits each of `lactose-ecs`, `lactose-app`, `tools/xtask/src` and
  `tools/cook`. Edits to `crate-tiers.toml` and `ci-policy.toml` are ordered by merge.
- A backend's lane may amend `lactose-rhi-api` in the change that needs it, called out in its merge
  (section 20). While `rhi-null`, `vulkan` and `graph-impl` are in flight together, the amendment
  merges first and the others take main.
- After a push that changes a GPU crate, the next such push waits for a green `gpu (lavapipe)`
  job, as a push that changes the cache key waits for its Windows check (D125).
- A merge that changes `Cargo.lock` or a workflow is pushed alone (D125): `vulkan`,
  `platform-impl`, `slang`, and any other lane that adds a third-party crate.
- A crate that milestone 2 adds has one integration-test target, and a second, `tests/sour.rs`,
  where it records (decision 5.15). A file that is not text is named `binary` in `.gitattributes`
  by the change that adds the first of its kind (D148).
- The measurement commit is the first commit of main that holds `frame-impl` and `measure-m2`. The
  quiet window, the laptop's session and session C run there (section 4). A lane that merges after
  it and edits a measured crate runs the measurements it touches and reports them as steering.
- A device test compares against a reference the test computes on the CPU, at a margin that no
  conformant rasteriser can cross. No image is checked in (decision 5.10).

## 4. Machines, workloads and result files

| Machine | What it can show | What it cannot |
|---|---|---|
| The development Mac: Apple M1, four performance and four efficiency cores, macOS 14.7.4, Vulkan SDK 1.4.363.1 with MoltenVK 1.4.2, the command line tools without Xcode | MoltenVK under the Khronos layer, Metal API validation and Metal shader validation. A window and a present. The native Metal spike. Timing in a quiet window | KosmicKrisp, which needs macOS 26: the SDK's KosmicKrisp driver enumerates no device there (a probe of this plan). A metallib: `slangc` fails for want of the `metal` compiler (a probe of this plan). What only a discrete GPU shows: memory that is not coherent, an ownership transfer's effect, a missing non-uniform decoration |
| The Windows laptop: Intel i7-12650H, six performance and four efficiency cores, an RTX 4070 laptop GPU, Windows 11, driven through its own agent session | Vulkan on a discrete GPU and the Win32 swapchain. The triangle on Windows. The power-throttling setting on a hybrid part. Frame pacing on Windows | Section 20's x86 desktop figures, unless the owner rules so (question 2). Its figures are labelled |
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until milestone 1's session A returns | The x86 desktop figures section 20 asks for. A second discrete GPU | Anything without the owner at it |
| linux-x64 runner: no GPU | lavapipe, a CPU rasteriser: the conformance suite and the device tests under the validation layer on every pull request, and under GPU-assisted validation nightly | Timing. Tile memory. A vendor's driver. A window |
| windows-x64 runner: no GPU, no Vulkan driver | The build, the lints and the tests on null | Any Vulkan call. The Win32 path runs on the laptop alone |
| macos-arm64 runner: Xcode, a virtual GPU | Cooking metallibs (criterion 7) | Timing. Whether it runs Metal or MoltenVK tests, and KosmicKrisp on a macOS 26 image, is `gpu-probe`'s to find |

**Four kinds of sitting produce every file a criterion rests on.**

- *At each merge of a GPU crate*, on the M1: the extra gate, under MoltenVK and the three
  validators. A run that presents holds the display awake with `caffeinate -d`, since a present
  into a sleeping display can block for up to a second (D18). Then, after the push, the laptop's
  run on the discrete GPU.
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s -d`, and `cargo xtask measure --all` runs once. It writes milestone 2's
  five files beside milestone 1's.
- *The laptop's session*, at the same commit, with nothing else running there: the same command,
  and `cargo xtask gpu --driver system`. Its `triangle` file is criterion 1's proof on Windows if
  the owner answers question 2 as recommended.
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
| `extract-50k` | 100,000 rows with a `Transform` and a mesh component, in four archetypes and sixteen regions, as milestone 1's cost scene is laid out. A kernel moves 50,000 of them every tick. One tick, then `Simulation::run_passes` with extract alone registered, sixty times, on N frame workers, N the performance cores up to eight. The ring is the device's mapped buffer, and the run is repeated on null. The file gives the pass's median, the ring's part and the snapshot's part, and the cost per dirty row on one worker | Criterion 4: the median on the device at most 1 ms, on the M1 and on the desktop |
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer. One writer thread pushes a state to every slot at 30 Hz. The presentation schedule's remote upload runs every frame, paced at 144 Hz by a timer for ten seconds: one `sample_batch`, the conversion to render space, the write to the device's ring. The file gives the median, P99 and worst frame, the reader's retries and the buffer's bytes | Criterion 5: the median at most 1 ms, on the M1 and on the desktop. P99 is recorded; over 2 ms it is a finding |
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without | Nothing: recorded. It sets `lactose-app`'s default reservation (decision 5.16) |
| `gpu-cost` | Through `rhi-vulkan` under a named driver: the encode and submit time of ten thousand indexed draws, as draws, as one multi-draw and as one indirect draw over a compacted buffer; the creation of 96 pipelines with an empty cache and with a reloaded one | Nothing: recorded. Rows 15 and 34 |
| `triangle` | The client binary for 600 frames with the validation layer on, the last frame read back | Criterion 1: no wrong pixel, no message, on Windows and on macOS |

`extract-50k`'s arithmetic, not a measurement: with a tenth of a millisecond for dispatch and the
snapshot's publication, four workers have about 70 ns for each dirty row. No extract figure
exists. The nearest is the replication gather's 0.46 ms for 4,000 changed rows among 100k, taken
before the ECS hot-path lanes (row 98).

**How a result is checked in.** As `docs/results/README.md` states for milestone 1. A file taken
on a device adds to its header what the RHI spike's run header carries: the adapter's name, vendor
and device ids, the driver's name and version, the API version, the SDK and layer versions, the
validation settings, the present mode and the display's refresh rate.
`cargo xtask gpu --driver <name>` writes `rhi-conformance-<driver>-<machine>-<date>.md`: the cases
run, the capabilities reported, and the count of messages by validator.

## 5. Decisions this plan takes

Each is a recommendation the owner approves with the plan. Each names what would change it.

**5.1 The triangle is the client binary's frame.** Criterion 1 is proven by `lactose-app-client`
drawing instanced triangles at extracted transforms, through passes the render graph compiled and
recorded, to a swapchain, on a hardware Vulkan driver on Windows and under MoltenVK on macOS. Its
evidence is a readback against a CPU reference and a clean run under every validator, as the RHI
spike's was. The client at milestone 2 runs a local world, as the headless binary does: `Client`
gains its replicated world only at milestone 4 (`crates/app/README.md`). On macOS a window test
cannot run on a test harness's thread, since winit needs the main thread, so the proof runs the
binary. This rules out a triangle drawn by a test program beside the graph, and a criterion met
by a window somebody looked at. What would change it: `frame-design` finding that the local world
needs a second assembly path. The proof then moves to a device test of the frame without the
binary, and the binary's run is recorded beside it.

**5.2 Extract's criterion is read on the device's ring, at 50,000 rows moved among 100,000.** A
row is dirty when its change tick is newer than extract's mark or than the tick before (section
5.3), so a scene whose 50,000 rows move every tick has 50,000 dirty rows at every pass. The pass
is what `Simulation::run_passes` runs for extract: the fork over dirty chunks, the write of the
previous and current transform to the staging ring in render space, and the presentation
snapshot. The ring is a mapped buffer of the device, which on a discrete GPU is not ordinary
memory, so the criterion is read there and the null backend's figure separates the ECS's cost
from the memory's. It is read as the median of sixty passes, as milestone 1's criteria are (D139),
on the M1's four performance cores and on the desktop. This rules out a criterion met on null
alone, and one read without the snapshot. What would change it: the `present-api` facade keeping
transforms out of the snapshot, in which case the file says what the snapshot holds.

**5.3 Remote upload's criterion is the median frame of a ten-second run at 144 Hz, with a writer
running.** The path is the one section 5.3 names as the client's dominant one: the batch sample
through `InterpolationReader::sample_batch`, the conversion through `RenderPosition::from_sim`,
and the write through the same ring as extract. A writer at 30 Hz runs beside it, since the
sequence lock's retries are part of the cost and row 102 asks for their rate. The run is paced by
a timer and needs no 144 Hz display. The median is read, as milestone 1's are, and P99 is
recorded beside it. This rules out a figure taken with no writer, and one that stops before the
ring. What would change it: a P99 over twice the criterion, which goes to the planner as a
finding about the frame's tail.

**5.4 The minimal renderer is the path the two timing criteria end at, and a triangle on it.** In
milestone 2: the render slot table (D49); the staging ring that extract and the remote upload
write; its upload and a scatter pass into a persistent instance buffer in the cell-plus-offset
format (D41); triangles drawn by instance from that buffer; the latched camera buffer (D48),
written before submit from the snapshot's pivot and `lactose-input-host`'s `LatestSample`;
pipelines from cooked shaders and the pipeline cache file; `RenderFeature` with no provider. The
ring's row is in its final format, so criterion 4's figure is of the path milestone 3 keeps. The
scatter is in because the ring's contract, frames in flight and a slot named twice in a batch, is
otherwise untested until milestone 3. Not in milestone 2: meshes and materials in the GPU scene,
cluster submission, culling, deferred lighting, the heap's use by materials, skinning, GPU VFX,
light binning, the cascade pads and the boom's recheck at the latch. Device-lost recovery is
tested with the GPU scene it rebuilds, at milestone 3; `rhi-api` reports the loss as a value now
and null can inject it. This rules out a staging format that milestone 3 would rewrite and any
milestone 3 pass built early. What would change it: the `renderer` facade showing the scatter
cannot be specified without the GPU scene's layout. The ring then uploads into the instance
buffer directly, and the scatter and its hazards are milestone 3's.

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

**5.6 `rhi-api`'s facade follows the native Metal spike alone.** D139 put the facade behind the
MoltenVK, KosmicKrisp and native Metal spikes. The MoltenVK spike has given what shapes a
surface: fourteen points, and five rules for null. What it left open are costs, the encode time
and the pipeline creation cost, which `rhi-api` reports or budgets and does not bake into a
signature. KosmicKrisp is a conformant Vulkan driver, so it is the same surface, and it cannot
run on the development Mac (decision 5.7). The native Metal spike is the one that can change a
signature (D43): residency, argument buffers, heaps, indirect command buffers and the binary
archive. So the facade waits for it alone, and the cost figures are taken through `rhi-vulkan`,
on the path that ships, under each driver with one command. This rules out a second throwaway
Vulkan program. What would change it: `spike-metal` reporting that an item cannot be decided
without a MoltenVK cost. That cost is then measured with the RHI spike's program first.

**5.7 The Apple interim path is MoltenVK.** Section 22.1 asks for it by milestone 2. Every item
of the RHI spike ran under MoltenVK 1.4.2 with a GPU-side use read back exact. KosmicKrisp is
built on Metal 4 and needs macOS 26 (Mesa's driver documentation). The development Mac runs macOS
14.7.4, and the KosmicKrisp driver of the installed SDK enumerates no device there (a probe of
this plan). The interim path lasts one milestone: `rhi-metal` lands in milestone 3. So the path
is MoltenVK, at the SDK version each result file names, and KosmicKrisp is recorded where it runs
(question 1). The G-buffer in tile memory is not shown on Apple before `rhi-metal`, since
MoltenVK's local read restarts the Metal pass. This rules out an operating system upgrade for a
path that lasts one milestone, and KosmicKrisp as the path on a client below macOS 26. What would
change it: the owner moving the Mac to macOS 26 (question 1), and KosmicKrisp then keeping a
local read in tile memory where MoltenVK does not.

**5.8 Slang is a pinned release that the cooker runs, and a metallib is cooked on macOS alone.**
The cooker runs `slangc` of one Slang release, pinned by version and archive checksum in a
tracked file, and refuses another. The SDK's copy on the Mac is a build 25 commits past release
2026.17 and is not the pin. Section 21 permits FFI to Slang and nothing requires it: a subprocess
needs no `unsafe`, no link step on three hosts and no build script, and milestone 2 cooks a
handful of shaders. SPIR-V and MSL are cooked on every host and their bytes are equal across
hosts, which the `shaders` job compares. A metallib needs Apple's `metal` compiler, so it is
cooked on macOS, and the runner's Xcode version is part of what its bytes depend on. The cook
strips line directives, since the MSL a probe of this plan produced carries the source's path.
This rules out a Slang taken from whichever SDK a host has, and cooked bytes that depend on a
checkout's path. What would change it: process starts dominating the cook at milestone 3's 48
pipelines, or the cooker needing reflection the command does not give. The FFI is taken then.

**5.9 Crash reporting is the panic hook's text report, in `lactose-app`, until a build leaves the
owner's machines.** Section 22.1 asks for minidumps and symbol upload by milestone 2. The hook is
in the headless binary today, and its README and row 133 give it to `lactose-platform`, a crate
`[server_denylist]` denies the server. `lactose-app` is what every binary links, and the hook is
safe code. The report goes to stderr and to a file. A hardware fault, which a driver call makes
possible from this milestone on, is read from the operating system's own report, on a machine
set to keep one: Windows Error Reporting's local dumps, macOS's crash report, a core dump on
Linux. A minidump writer that catches such a fault runs in a signal handler or a second
process, which is `unsafe` in a crate every binary links, and section 21 has none. An upload
needs a service and a symbol store, and nothing reads either before a build goes to a tester.
So neither is built, and the row of section 22.1 moves to milestone 5. This rules out a hook
the server cannot link. What would change it: a fault on the owner's machines that the system's
report does not explain, or a build given to a tester sooner (question 3).

**5.10 One command runs the device tests, lavapipe runs them on every pull request, and no GPU
runner is bought.** `cargo xtask gpu --driver <name>` sets the loader's environment for its child
as the RHI spike's run header set it by hand, runs the conformance suite and every device test
with the validators on, counts their messages and writes a result file. The drivers are
`lavapipe`, `moltenvk`, `kosmickrisp` and `system`. A device test is an ignored test that the
command selects, as a measurement is, so a job with no driver does not run it, and under a named
driver a missing device fails the run: a device test never passes by reading back nothing.
Section 20 runs RHI conformance "on a GPU runner nightly". No such runner exists. A hosted one is
a paid larger runner, and a self-hosted one on a public repository runs what a stranger's pull
request sends it. Hardware is covered where it is: the M1 at every merge of a GPU crate, the
laptop after each such push, the desktop at the measurement commit. This rules out a spend on a
runner and a device test that compares against a checked-in image. What would change it: a defect
that reaches main because the laptop was not in reach at a merge. The owner is then asked about a
runner.

**5.11 `lactose-app` drives the render side through `-api` traits, and the client binary wires
the backends.** `[server_denylist]` denies the server the render crates directly or transitively,
and the server links `lactose-app`. So `app` names `present-api` and `rhi-api` alone, as section 4
says of every backend service a foundation crate reaches, and `lactose-app-client` constructs the
platform, the device, the graph, the renderer and the presentation schedule and hands them in.
Extract's sink takes a chunk's rows at a call, so the `dyn` boundary is crossed once for each
chunk and not for each row. This rules out a feature of `lactose-app` that turns backend edges
on. What would change it: `frame-design` measuring a call across the boundary that criterion 4
cannot afford.

**5.12 `lactose-rhi-vulkan` is a test-support crate, and five crates get a facade.**
`lactose-render-graph` and `lactose-renderer` must run on a device in their own tests, and a
backend crate may dev-depend only on itself and on `[test_support]`. So `lactose-rhi-vulkan` joins
that list beside `lactose-rhi-null`, in the `vulkan` merge. The server denylist is unchanged. The
lane shows that `cargo xtask boundary` refuses a boundary crate's test that names it, or adds
that rule. `rhi-api`, `render-graph`, `renderer`, `platform` and `present-api` are crates other
crates are written against and get a facade first (section 20). This rules out a
backend-to-backend exception for tests. What would change it: the `rhi-api` facade placing a
backend-neutral test harness in a leaf crate.

**5.13 `rhi-api` is frozen or amended by a decision at the exit.** Section 20 freezes an
interface on its first real backend's conformance run and keeps `rhi-api` unstable until the
native Metal spike has run. Both will have happened. The planner writes the decision from
`spike-metal`, the conformance files under lavapipe, MoltenVK and the discrete GPU, and
KosmicKrisp's if there is one, as the scalar decision was written from its probe. The expected
outcome is a freeze of what the suite covers, with the calls only `rhi-metal` exercises named as
the ones its lane may still amend. This rules out a freeze by silence. What would change the
expectation: a backend lane amending the facade in its last fix round.

**5.14 Five things leave milestone 2.**

- *The directory watch goes to milestone 3* (question 4). D136 and D147 place it in milestone 2.
  No criterion reads it, it needs an `io` request on three platforms, and its other half, pack
  retirement, is already milestone 3's (row 138). A re-cooked shader is seen after a restart,
  which is section 18's iteration model. Rows 138, 139 and 142 follow it.
- *Artist surfaces are decided at milestone 3* (row 97). The proposal's measurement is a G-buffer
  pass on native Metal and on MoltenVK. Milestone 2 has no G-buffer and no `rhi-metal`. `slang`
  proves the mechanism the proposal rests on, an interface specialised statically and a link-time
  constant, for both targets.
- *The mesh and texture load against the 0.5 ms slice goes to milestone 3* (row 135), with the
  kinds. Milestone 2 measures the kind it adds.
- *Device-lost recovery is tested at milestone 3* (decision 5.4).
- *The visual blend of a correction and the shadowing of the replicated copy are milestone 4's*
  (decision 5.5).

**5.15 A new crate has one integration-test target.** The deferred lever is one integration-test
binary for each crate: `lactose-ecs`'s 22 test binaries built as one took 5.2 s in place of 14
(D125; 13.9 to 14.4 s and two links in place of 23 in that lane's report). D125 called it due
at once, when the Windows check job ran 14.8 of its 15 minutes. That reason has eased: the job
takes 5.2 to 5.7 minutes since lld-link and the job split (`.github/workflows/ci.yml`). The
workspace holds about 180 such targets in 21 packages. Every crate milestone 2 adds is written
with one target, at no cost, and a second, `tests/sour.rs`, where it records: a target may carry
a codename and a module may not (section 3). The existing crates are converted when a check job
passes 8 minutes warm, by one lane of seven runs, pushed alone, since it touches every crate's
tests, the `[harness]` list, the nightly's Miri list and `cargo xtask measure`. This rules out
the conversion as a milestone 2 lane. What would change it: the owner taking it now (question
6), while no lane edits those tests.

**5.16 The stall is recorded and sets the reservation.** Section 16 asks milestone 2 to measure
the stall the physics phase imposes on render recording. It has no criterion. `record-stall`
gives the record phase's time beside the physics phase at a reservation of one frame worker and
of two, on the M1's four performance cores and on the desktop. If the file shows
`lactose-app`'s default reservation wrong, a follow-up commit changes it; the file is cited
either way. What would change it: a stall over a millisecond at both reservations on the M1,
which goes to the planner.

**For the owner.** Six questions, the recommended option first. The rest is folded into the
decisions above.

```json
{"questions":[
{"header":"KosmicKrisp","question":"Section 22 asks for a KosmicKrisp spike recorded in milestone 2. KosmicKrisp is built on Metal 4 and needs macOS 26. The development Mac runs macOS 14.7, and the SDK's KosmicKrisp driver finds no GPU there. MoltenVK is the interim Apple path either way. How is the KosmicKrisp spike recorded?","multiSelect":false,"options":[
{"label":"Hosted macOS 26 runner (Recommended)","description":"The Mac stays on macOS 14. Once `rhi-vulkan` exists, its conformance suite runs under KosmicKrisp in a manually dispatched job on a hosted macOS 26 runner, if that runner's GPU offers Metal 4. Capability answers are recorded and timings are not, since a runner's GPU is virtual. If the runner cannot run it, the record is one line: it needs macOS 26. The criterion is restated to say so. No extra lane."},
{"label":"Upgrade the Mac to macOS 26","description":"KosmicKrisp is measured beside MoltenVK on the M1, with timings. Every M1 result file of milestones 0 and 1 was taken on macOS 14.7 and the tick-start question is open on it, so the benchmark and `cargo xtask measure --all` run again in a quiet window after the upgrade."},
{"label":"Record it as not runnable","description":"No runner job. The record is one line: KosmicKrisp needs macOS 26. The criterion is restated, and MoltenVK is the path with no comparison."}]},
{"header":"Windows GPU","question":"Milestone 2's Windows proofs need a GPU: the conformance suite on a discrete GPU after each GPU merge, the triangle on Windows, and the x86 figures for extract and remote upload. Which machine gives them?","multiSelect":false,"options":[
{"label":"Laptop, one desktop sitting (Recommended)","description":"The laptop's RTX 4070 runs the suite after each GPU merge and proves the triangle on Windows, through its own agent session. You run one sitting on the desktop at the measurement commit, `cargo xtask measure --all` under each system it boots, for the x86 desktop figures architecture section 20 asks for. Whatever of milestone 1's sessions is still owed runs in the same sitting."},
{"label":"Laptop only","description":"No desktop sitting for milestone 2. The laptop's figures, taken with nothing else running, stand as the x86 figures, and section 20's desktop is restated for this milestone. It has six performance cores."},
{"label":"Desktop only","description":"You run the suite, the triangle and the measurements on the desktop. A defect only a discrete GPU shows is then found at the measurement commit and not at the merge that made it."}]},
{"header":"Crash dumps","question":"Section 22.1 wants crash reporting, minidumps and symbol upload, decided by milestone 2. Nothing leaves your machines before milestone 5, and each operating system already writes its own report of a hardware fault. What is built now?","multiSelect":false,"options":[
{"label":"Text report now (Recommended)","description":"The panic hook moves into `lactose-app`, so every binary shares it, and writes its report to stderr and to a file. Hardware faults are read from the system's own crash reports. No minidump writer, monitor process or upload service is built until a build goes to a tester. One small lane, five runs."},
{"label":"Build minidumps now","description":"A minidump writer for panics and hardware faults, in a new crate that section 21 must admit, since the server cannot link `lactose-platform`. About nine runs, and still no upload until a service is chosen."}]},
{"header":"Asset watch","question":"Decisions D136 and D147 place the cooker's directory watch, hot reload without a restart, in milestone 2. No milestone 2 criterion needs it. It costs an `io` watch request on three platforms and SILO's watch, about sixteen runs, and its pack-retirement half is already milestone 3's. Where does it go?","multiSelect":false,"options":[
{"label":"Milestone 3 (Recommended)","description":"It is designed once, with streaming, residency and pack retirement. In milestone 2 a re-cooked shader is seen after the client restarts. The rows on the watch, the long dev session and the mount cost on Linux and Windows move with it."},
{"label":"Keep it in milestone 2","description":"Two more lanes, about sixteen runs, and a decision on unmounting taken before streaming designs residency."}]}
]}
```

```json
{"questions":[
{"header":"Xcode on M1","question":"The Mac has the command line tools only, so the Slang compiler cannot build a metallib there: a probe failed for want of Apple's `metal` compiler. The macOS CI runner can, but no branch reaches CI before its merge. Do you install Xcode on the Mac?","multiSelect":false,"options":[
{"label":"Install Xcode (Recommended)","description":"The newest Xcode that macOS 14.7 runs. Lanes and reviewers then cook and load metallibs before a merge, and the native Metal spike can test a binary archive built from a precompiled library. It takes several gigabytes of disk."},
{"label":"Runner only","description":"Nothing to install. A metallib is first cooked in CI after the merge that adds it, so a failure is found on main and fixed by a follow-up. The native Metal spike compiles its MSL at run time."}]},
{"header":"Test targets","question":"The deferred CI lever is one integration-test binary for each crate. It is measured on `lactose-ecs` alone: its 22 test binaries built as one took 5.2 s in place of 13.9 s. The Windows check job now takes about 5.5 of its 15 minutes. What is done in milestone 2?","multiSelect":false,"options":[
{"label":"New crates only (Recommended)","description":"Every crate milestone 2 adds is written with one integration-test target, and `tests/sour.rs` where it records, at no cost. The existing crates are converted only when a check job passes 8 minutes warm, by one lane of seven runs."},
{"label":"Convert the existing crates now","description":"One lane, about seven runs, before any milestone 2 lane starts, since it touches every crate's tests, the SOUR test list, the nightly's Miri list and `cargo xtask measure`. It is pushed alone."}]}
]}
```

One action is the owner's whatever the answers: the laptop's checkouts are refreshed once after
the line-ending merge (D148), before the laptop's first milestone 2 run.

## 6. Text for the coordinator

The numbers are allocated at application; `D<N>` follows D149. Exact OLD and NEW text is not
given: this plan is re-based on main when it is approved, and the row changes below are each row's
gate and the gist of its new text. The entries are written for the recommended answers. The
paragraph after them says what each other answer changes.

### Decision entry: the plan's rulings

```
**D<N>. The milestone 2 plan: how its criteria are read, which machine proves each, the order the render track is built in, and what is re-gated** (§4, §5.3, §12.1, §12.2, §12.3, §12.4, §13, §14, §16, §18, §19, §20, §22, `crate-tiers.toml`, `tools/xtask`, `docs/plans/milestone-2.md`)
- Why: section 22 gives milestone 2 seven proofs and names no workload, machine or order for
  them; none of the crates it delivers existed at `6f0508a`; seventeen verification rows gated
  the milestone, and D148 and D149 moved more to it. `docs/plans/milestone-2.md` is the map and
  gives each decision's reasons; these are the decisions.
  - *The triangle is the client binary's frame.* `lactose-app-client` runs a local world,
    extracts it each frame and draws instanced triangles at the extracted transforms through
    passes the render graph compiled and recorded, to a swapchain, on a hardware Vulkan driver on
    Windows and under MoltenVK on macOS. The proof is a readback of the last frame against a CPU
    reference with no validator message, in result file `triangle`. Rules out a triangle drawn by
    a test program beside the graph.
  - *Extract's 1 ms is the median of extract's pass on N frame workers, at 50,000 rows moved every
    tick among 100,000*, writing the previous and current transform to the device's mapped
    staging ring in render space and filling the presentation snapshot; the null backend's figure
    is recorded beside it. It is read on a quiet Apple M1 and on the x86 desktop. Rules out a
    criterion met on null alone or without the snapshot.
  - *Remote upload's 1 ms is the median frame of ten seconds paced at 144 Hz*: one batch sample
    of 5,000 slots, the conversion to render space and the write to the ring, with one writer
    pushing every slot at 30 Hz. P99 is recorded. Rules out a figure with no writer.
  - *The minimal renderer is the render slot table, the staging ring, its scatter into a
    persistent instance buffer in the cell-plus-offset format, triangles drawn by instance, the
    latched camera buffer and `RenderFeature`.* The GPU scene's meshes and materials, cluster
    submission, culling, deferred lighting, skinning and device-lost recovery are milestone 3's.
    Rules out a staging format that milestone 3 would rewrite.
  - *The presentation schedule at milestone 2 is interpolation, the camera rig's orbit and boom,
    the remote sample and upload, and event delivery.* Boom collision and pose evaluation are
    milestone 3's, the visual blend of a correction and the shadowing of the replicated copy
    milestone 4's, UI animation, the audio listener and markers milestone 5's.
  - *`rhi-api`'s facade follows the native Metal spike alone*, a throwaway program on the M1,
    never checked in. MoltenVK's open cost figures are taken through `rhi-vulkan` by
    `cargo xtask measure gpu-cost`, since a cost is reported or budgeted and changes no
    signature. Rules out a second throwaway Vulkan program. This replaces D139's order, the
    facade behind three spikes.
  - *Slang is one pinned release, which the cooker runs as `slangc` and refuses any other.*
    SPIR-V and MSL are cooked on every host with equal bytes; a metallib is cooked on macOS, by
    Apple's `metal` compiler. Rules out a Slang taken from a host's SDK, and FFI to Slang before
    process starts cost the cook something.
  - *One command runs the device tests: `cargo xtask gpu --driver <name>`.* A device test is an
    ignored test the command selects; under a named driver a missing device fails the run; each
    compares against a reference computed on the CPU. lavapipe runs them on every pull request
    on linux-x64. Rules out a checked-in image and a device test that passes with no device.
  - *No GPU runner.* Hardware runs are the M1's at every merge of a GPU crate, as an extra gate
    under MoltenVK and the validators, the Windows laptop's discrete GPU after each such push,
    and the desktop's at the measurement commit. Rules out a paid hosted runner and a
    self-hosted runner on a public repository.
  - *`lactose-app` drives the render side through `present-api` and `rhi-api`, and
    `lactose-app-client` wires the backends.* The server links `lactose-app` and is denied the
    render crates transitively. Rules out a feature of `lactose-app` that turns backend edges on.
  - *`lactose-rhi-vulkan` joins `[test_support]`*, so the graph and the renderer run on a device
    in their own tests. Five crates get a facade first: `rhi-api`, `render-graph`, `renderer`,
    `platform`, `present-api`.
  - *`rhi-api` is frozen or amended by a planner's decision at the exit*, from the native Metal
    spike and the conformance files.
  - *A crate added from milestone 2 on has one integration-test target*, and a second,
    `tests/sour.rs`, where it records. The existing crates are converted when a check job passes
    8 minutes warm.
  - *The physics phase's stall on render recording is recorded with no threshold* and sets
    `lactose-app`'s default reservation.
  - *Re-gated*: the directory watch, with its dev session and the mount cost on Linux and
    Windows, to milestone 3; the artist-surfaces decision to milestone 3, where a G-buffer pass
    and `rhi-metal` exist; the mesh and texture load against the 0.5 ms slice to milestone 3;
    device-lost recovery's test to milestone 3; the visual blend of a correction to milestone 4.
- Driven by: architecture section 22 and its exit step in `docs/plans/milestone-1.md`; the RHI
  spike's result (`docs/results/spike-rhi-apple-m1-2026-11-15.md`); the plan's probes of 16
  November 2026; the owner's ruling of that day that the plan be drafted ahead of milestone 1's
  exit, and the owner's answers of <date>.
- Replaced: D139's "`lactose-rhi-api`'s facade and `lactose-rhi-null` move to milestone 2,
  behind its MoltenVK, KosmicKrisp and native Metal spikes", in its order alone. D136's and
  D147's placing of the directory watch in milestone 2. D125's "Next lever, due now", for the
  existing crates: the Windows check job has run 5.2 to 5.7 of its 15 minutes since. Section
  20's "on a GPU runner nightly", which the next revision restates. The status line of
  `docs/proposals/artist-surfaces.md`.
```

### Decision entry: the Apple interim path

```
**D<N+1>. The Apple interim path until native Metal is MoltenVK; KosmicKrisp needs macOS 26 and is recorded where that runs** (§2 Graphics, §12.1, §22, §22.1, `lactose-rhi-vulkan`)
- Why: section 22.1 asks for the interim path by milestone 2, and section 22 for a MoltenVK and
  a KosmicKrisp spike behind it.
  - *MoltenVK carries section 12.1's list.* Under MoltenVK 1.4.2 on the M1 every item of the RHI
    spike ran with a GPU-side use read back exact, clean under the Khronos layer, Metal API
    validation and Metal shader validation
    (`docs/results/spike-rhi-apple-m1-2026-11-15.md`). Its four "no"s are capabilities `rhi-api`
    reports: no lazily allocated memory for an input attachment, a local read that restarts the
    Metal pass, a memoryless attachment lost across that restart, no indirect count.
  - *KosmicKrisp cannot be the path on the machines in reach.* It is built on Metal 4, which
    needs macOS 26 (Mesa's driver documentation). The development Mac runs macOS 14.7.4, where
    the KosmicKrisp driver of Vulkan SDK 1.4.363.1 enumerates no device, and a client below
    macOS 26 could not run it either.
  - *The interim path lasts one milestone.* `rhi-metal` lands in milestone 3 and carries the
    GPU-driven path, which MoltenVK cannot (D42, D43). Until then the Apple path draws milestone
    2's triangle and runs the conformance suite. The G-buffer in tile memory is not shown on
    Apple before `rhi-metal`.
  - *KosmicKrisp is recorded, not adopted*: `rhi-vulkan`'s conformance suite under it on a
    hosted macOS 26 runner if that runner's GPU offers Metal 4, capability answers only;
    otherwise one line, that it needs macOS 26. Section 22's "KosmicKrisp ... recorded" is read
    so.
  - Rules out: KosmicKrisp as the interim path; an operating system upgrade of the development
    Mac for a path that lasts one milestone, with every M1 result file of milestones 0 and 1
    taken again after it.
- Driven by: architecture section 22.1; the RHI spike; the plan's probe of 16 November 2026;
  the owner's answer of <date>.
- Replaced: sections 2 and 12.1, which leave the interim path open between the two; the next
  revision names it.
```

### Decision entry: crash reporting

```
**D<N+2>. Crash reporting until a build leaves the owner's machines is the panic hook's text report, shared by every binary through `lactose-app`; minidumps and symbol upload are decided at milestone 5** (§3, §5.1, §19, §22.1, `lactose-app`, `lactose-app-headless`, `lactose-app-client`)
- Why: section 22.1 asks for crash reporting, minidumps and symbol upload, by milestone 2, and
  milestone 2 adds a second binary and the first calls into a graphics driver.
  - *The hook is `lactose-app`'s.* It was the headless binary's
    (`crates/app-headless/src/main.rs`), and that binary's README and the verification row "The
    milestone 1 crash hook" gave its successor to `lactose-platform`, which `[server_denylist]`
    denies the server. `lactose-app` is what every binary links, and the hook is safe code.
  - *The report is text*: the thread, the panic and a forced backtrace, to stderr and to a file
    in a directory the binary names. A directory that cannot be written costs the file alone
    (`docs/verification.md`, section 3). A subprocess test holds the abort path.
  - *A hardware fault is read from the operating system's report*, on a machine set to keep
    one: Windows Error Reporting's local dumps, macOS's crash report, a core dump on Linux.
    The validation layers and these are what diagnose a fault inside a driver call on the
    owner's machines.
  - *No minidump writer, no monitor process and no symbol upload are built.* A writer that
    catches a hardware fault runs in a signal handler or in a second process: `unsafe` in a
    crate every binary links, which section 21 does not have. An upload needs a service and a
    symbol store, and nothing reads either before a build goes to a tester. Section 22.1's row
    moves to milestone 5.
  - Rules out: a hook in a crate the server cannot link; a spend on a crash service before
    there is a tester.
- Driven by: architecture section 22.1; `crate-tiers.toml`; the owner's answer of <date>.
- Replaced: sections 3 and 5.1's "crash dump", which the next revision states as the text
  report; the row's and the headless README's "`lactose-platform` ... whose hook replaces this
  one".
```

### If the owner answers otherwise

- Question 1, the upgrade: D<N+1>'s fourth bullet becomes a comparison on the M1, its "Rules
  out" loses the upgrade, and the quiet window after the upgrade re-writes milestones 0 and 1's
  M1 files. Not runnable: the bullet is its last clause alone, and `gpu-probe` asks nothing of a
  macOS 26 image.
- Question 2: "Laptop only" restates D<N>'s extract and upload bullets to "on the Windows laptop"
  and drops session C. "Desktop only" moves the laptop's runs to session C and drops the run
  after each push.
- Question 3, minidumps now: D<N+2> is rewritten by the planner as a proposal, since it adds a
  crate to section 21.
- Question 4, the watch stays: D<N>'s re-gate bullet loses its first clause, rows 138, 139 and
  142 keep their gates, and two lanes join queue 3: the `io` watch request, and SILO's watch
  with the decision on unmounting.
- Question 5, runner only: `slang`'s proof on the M1 stops at MSL, and the row on the `metal`
  compiler says so.
- Question 6, convert now: one lane precedes every other, and D<N>'s bullet says the existing
  crates were converted with it.

### Verification rows

Each change is its gate and the gist of its new text. Rows are cited at `6f0508a`.

| Row | Gate | Gist of the new text |
|---|---|---|
| 15, MoltenVK and KosmicKrisp | 2 | KosmicKrisp needs macOS 26 and Metal 4; the M1 runs 14.7.4 and the SDK's driver enumerates no device (D<N+1>). It is recorded on a hosted macOS 26 runner or as not runnable. MoltenVK's encode time and pipeline creation cost are result file `gpu-cost`, through `rhi-vulkan` |
| 16, Extract cost | 2 | The workload and machines of D<N>; result file `extract-50k` |
| 18, Native Metal | 2 | Result file `spike-metal`; the freeze decision at the exit |
| 19, Remote transform upload | 2 | The workload of D<N>; result file `remote-upload-5k` |
| 21, Physics phase stall | 2 | Result file `record-stall`, recorded with no threshold; it sets the default reservation |
| 34, MoltenVK multi-draw-indirect is a CPU loop | 2 | Closed by `gpu-cost`'s multi-draw figure and the reading of the pinned MoltenVK |
| 35, Slang's Metal target | 2 | Shown by `spike-metal`'s shaders, then held by the cooker's |
| 63, Windows efficiency-core steering | 2 | Lane `eco-qos`; measured on the laptop, a hybrid part, and on the desktop if it is one |
| 95, `lactose-input` is implemented | 2 | The camera rig's check is `lactose-presentation`'s test |
| 97, Artist-defined surfaces | 2 to 3 | Decided with deferred lighting on `rhi-metal`; milestone 2's cooker proves static specialisation and a link-time constant for both targets (D<N>) |
| 99, `lactose-predict` is implemented | 2 (after D149) | The presentation check is `lactose-presentation`'s test. The visual blend of a correction is milestone 4's |
| 102, Interpolation slot footprint | 2 | The loom model is lane `interp-loom`; the retry rate and the footprint are in `remote-upload-5k` |
| 133, The milestone 1 crash hook | 2 | The hook is `lactose-app`'s and writes a file too (D<N+2>); a subprocess test holds the abort path; minidumps are decided at milestone 5 |
| 135, SILO's load call | "3 (the baseline, a mesh pool and a texture), 2 (the shader kind)" | Milestone 2 cooks no mesh pool and no texture (D<N>) |
| 138, Mounted packs are never unmounted | 2 and 3 to 3 | The dev session waits for the watch loop, at milestone 3 (D<N>) |
| 139, Mount cost | 2 and 3 (after D149) to 3 | The Linux and Windows figure is taken with the watch loop's dev session, at milestone 3 |
| 142, The cooked pack set | 2 to 3 | The watch's half is milestone 3's |
| 149, The RHI spike ran under MoltenVK alone | 2 | Closed by lane `vulkan`: the eight design questions from the lavapipe run, the four hardware questions from the discrete GPU. Question 2 stays open on Vulkan if neither offers a lazily allocated memory type |
| 163, The keeper's timed wait on Windows | 1 and 2 (after D149) | The list is read by `frame-design`, and what the frame's pacing uses is measured on the laptop |
| The line-ending rule on a Windows host (D148) | 2 | Unchanged; closed by the pass |

Rows to add:

| Gate | Gist |
|---|---|
| 2 | Milestone 2's criteria are proven by result files and CI runs: `triangle`, `spike-metal`, `gpu-cost`, `rhi-conformance`, `extract-50k`, `remote-upload-5k`, and the first green `gpu (lavapipe)` and `shaders` runs (D<N>) |
| 2 | What lavapipe on the linux-x64 runner offers is not known: its Mesa version, the local read, a lazily allocated type, indirect count, a headless surface, GPU-assisted validation. `gpu-probe` records it before `vulkan` is briefed. A runner image change is a driver change |
| 2 | What the macos-arm64 runner offers is not known: its Xcode and `metal` compiler, a Metal device, and KosmicKrisp on a macOS 26 image. `gpu-probe` records it. A metallib's bytes depend on the Xcode that cooked it |
| 2 | The development Mac has no `metal` compiler: `slangc -target metallib` fails there. Open until Xcode is installed, or the row states that metallibs are the runner's alone |
| 2 | SPIR-V and MSL cooked by the pinned Slang are expected equal on linux-x64 and macos-arm64. The `shaders` job is the proof; a difference is a finding for the planner. MSL carries the source's path unless line directives are off |
| 2 | A window opened from an agent's shell on the M1, and from the laptop's agent session, is assumed. If either cannot present, that run is the owner's |
| none | Hardware runs of the device tests are made at merges on the M1 and after pushes on the laptop, since no GPU runner exists (D<N>). A defect that reaches main for want of one reopens the question |
| none | The existing crates keep one test binary for each file. Convert them when a check job passes 8 minutes warm (D<N>) |
| 3 | Device-lost recovery (architecture section 19) is tested when the GPU scene exists. `rhi-api` reports the loss as a value and null injects it from milestone 2 |
| 4 | The prediction world's copy of the local player shadowing the replicated one for extract and the slot table (section 9.4) is tested when `Client` holds both worlds |
| 5 | Minidumps and symbol upload (D<N+2>) |

### Other text

The directory watch's move changes one sentence in each of `crates/asset/README.md`,
`crates/io/README.md` and `tools/cook/README.md`, where "milestone 2" becomes "milestone 3".
`docs/proposals/artist-surfaces.md`'s status line and its two rows gated at 2 name milestone 3.
`crates/app-headless/README.md`'s sentence on the minidump follows D<N+2>, with the `crash-hook`
merge.

## 7. The exit

Milestone 2 exits when these have happened, in this order.

1. The measurement commit exists. The quiet window, the laptop's session and session C have run
   on it, and their result files are on main.
2. Every queue of section 3 has drained, so section 22's delivers are on main: `platform`,
   `rhi-api`, `rhi-null`, `rhi-vulkan`, `render-graph`, the minimal `renderer`, the Slang
   toolchain, `present-api`, and the presentation snapshot and schedule.
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and a nightly that holds the
   client's recording test has compared it equal on three platforms. The run ids are in the rows.
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
| Section 2, Graphics: "MoltenVK and KosmicKrisp spiked in milestone 2 as interim Apple paths" | The interim path is MoltenVK (D<N+1>) |
| Section 12.1: "with every claimed common feature verified in the milestone 2 spike", and its Apple path paragraph | The RHI spike, the native Metal spike and the conformance suite, by name; KosmicKrisp's requirement |
| Section 20: "RHI conformance runs under lavapipe on Linux CI and on a GPU runner nightly" | lavapipe on every pull request; hardware at merges and at the measurement commit (D<N>) |
| Section 20: "`rhi-api` stays unstable until the native Metal spike has run" | What the freeze decision ruled |
| Section 20's CI line and section 4's `xtask` line | The `gpu (lavapipe)` and `shaders` jobs; `cargo xtask gpu` |
| Section 22, milestone 2 | A paragraph under the table on how its criteria are read, as milestone 1 has: the triangle, the two workloads, the machines, KosmicKrisp |
| Section 22.1 | Crash reporting moves to 5; the Apple interim path is decided |
| Sections 3 and 5.1, "crash dump" | The text report, and that the hook is `lactose-app`'s (D<N+2>) |
| Section 5.3 | The snapshot's measured budget; what the presentation schedule runs before milestones 3 and 5 |
| Section 12.2 | Slang as a pinned release; a metallib cooked on macOS alone |
| Sections 12.4 and 19 | What the minimal renderer is; device-lost recovery tested at milestone 3 |
| Section 14 and D136, D147 | The directory watch at milestone 3, if the owner moves it |
| Section 20, tests | One integration-test target for a new crate |
| `CLAUDE.md` | The revision marker |

## 9. What depends on milestone 1's exit

This plan assumes milestone 1's criteria are met as written, with one exception a file on main
already shows: tick start on the M1 is 27.0 µs at P99 against 20
(`docs/results/whisk-apple-m1-2026-11-15.md`), which `keeper` fixes or D138 restates at 35 µs.
Nothing in milestone 2 reads the difference.

| Exit outcome | What it changes here |
|---|---|
| Criterion 1, the 100k tick on eight performance cores, is not proven: session B has not run. If it misses, the owner rules for another ECS lane or a restatement | An ECS lane edits `lactose-ecs` first; `present-impl` waits behind it. Section 4's arithmetic for extract assumes the ECS at the speed `tick-100k` measured on the M1 |
| Tick start: `keeper` runs after session A's files, or the criterion is restated | `keeper` edits `lactose-task`. `eco-qos` waits behind it, and `frame-design` reads its outcome for frame pacing |
| Criterion 9, edit-to-playing on Windows and Linux, is session B's. Over thirty seconds brings a build-time proposal | That proposal can change the workspace's test layout, and question 6 is asked again with it |
| Session A returns the desktop's CPU, GPU and second system | Section 4's desktop row, and what session C can run under the second system |
| Rows that wait on the desktop or on `keeper`: 64, 94, 122, 132, 148, and 77, 79, 80, 81 and 163's first half. The owner may re-gate any of them at exit step 6 | A row re-gated to 2 joins section 2's table. Session C can carry sessions A and B's commands |
| Row 166, the physics hash, waits on the first nightly at or after `10cf89e` | A difference between platforms is a finding in `lactose-physics-rapier` and reaches no milestone 2 lane |
| Exit step 7, the follow-up project review of the determinism and readiness seams | Its findings in `lactose-app`, `lactose-predict`, `lactose-ecs` or `lactose-net-host` are fixed or routed before `frame-design`, `present-impl`, `presentation` and `interp-loom` start |
| D148 and D149 are applied, and the exit pass | The decision numbers of section 6. Rows 99, 139 and 163 as section 2 reads them. The line-ending check as a gate of every merge |
| Main moves | Every line number here. The plan is re-based before it is approved |

Two pieces of work depend on nothing the exit can change: `spike-metal`, which touches no file of
the tree, and the coordinator's `gpu-probe`. The owner may release them before the exit.

## 10. What this plan could not verify

- **No GPU program was built for this plan.** Every fact about the RHI is the RHI spike's result
  file and its reviews. No crate was compiled: `ash`, winit and a Metal binding are assumed to
  build on the pinned toolchain and to pass `cargo deny`.
- **KosmicKrisp's requirement** is Mesa's driver documentation, read through a summarising fetch
  on 16 November 2026, beside one probe: with the loader pointed at the SDK's KosmicKrisp driver
  alone, `vulkaninfo --summary` on the M1 fails with "Failed to detect any valid GPUs in the
  current config". Whether a hosted macOS 26 runner exists under that label, and whether its GPU
  offers Metal 4, was not checked.
- **lavapipe on the runner** was not looked at: its Mesa version, the local read, a lazily
  allocated memory type, a headless surface and GPU-assisted validation are `gpu-probe`'s. Two
  searches found no statement that lavapipe supports the local read.
- **The macOS runner** was not looked at: its Xcode, its `metal` compiler and whether its virtual
  GPU runs Metal or MoltenVK tests.
- **Slang** was probed once: the SDK's `slangc`, version 2026.17-25-gc7954ebd1, compiled one
  shader with a statically specialised interface to SPIR-V and to MSL on the M1, and its metallib
  target failed with "failed to load downstream compiler 'metal'"; `xcrun -f metal` finds
  nothing. That SPIR-V and MSL are equal across hosts, that a link-time constant works for both
  targets, and which release to pin were not checked. Slang's licence was not read against
  `deny.toml`, which a subprocess does not consult.
- **Xcode.** That an Xcode with the `metal` compiler runs on macOS 14.7 is from memory, not
  checked, and so is its size.
- **The laptop.** Its GPU and its hybrid CPU are the coordinator's notes and the topology line of
  `docs/results/whisk-laptop-windows-2026-11-15.md`. Whether its agent session can open a window
  and present was not checked, nor whether the M1 can from an agent's shell.
- **The desktop.** Its CPU model, its GPU and its second system are in no document.
- **GPU runners.** That a hosted GPU runner is paid and that a self-hosted runner is unsafe on a
  public repository are GitHub's documentation as remembered, not re-read.
- **The test-target lever** is measured for `lactose-ecs` alone, in October, on a loaded M1. Its
  gain over the workspace is an estimate nobody has measured. The Windows check job's 5.2 to 5.7
  minutes is the workflow's comment, from three runs at or before `3264406`, not the latest runs.
- **Crash handling.** No minidump crate was read. Decision 5.9 rests on where such a writer must
  run, not on one crate's API. What each operating system writes for a fault by default was not
  tested on the three machines.
- **Whether `cargo xtask tiers` would admit a feature-gated backend edge from `lactose-app`** was
  not checked. Decision 5.11 does not rest on it.
- **The pending texts.** D148, D149 and the exit pass were read as drafts in the coordinator's
  scratch area on 16 November 2026. Their applied text may differ.
- **The run counts** are this plan's estimates by milestone 1's conventions. Milestone 1's lanes
  took two to five review rounds.
- **The 70 ns for each dirty row** is arithmetic. No extract exists to measure.
