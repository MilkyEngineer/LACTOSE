# Milestone 2 plan: Pixels

Status: proposed, not approved. Written at main `6f0508a`, architecture revision 3.3, on 16 November
2026, ahead of milestone 1's exit by the owner's ruling of that day. Revisions 2 and 3 answer its
two rounds of review, each a general one and a GPU one, and revision 4 folds in the owner's answers
of 16 November 2026 and the closing reviews' six findings. This text is the re-base that milestone
1's exit step 8 asks for, at main `a561b36` on 17 November 2026. It takes the exit's fix round, D150
and D151, the follow-up project review's findings, and the owner's change of the upgrade to macOS
27.2.1. The two rounds of review of the re-base found two serious and twelve minor defects, and they
are answered in this text. The coordinator applies it when the owner approves it. This plan maps
each milestone 2 criterion of architecture section 22, and every `docs/verification.md` row that
gates milestone 2, to a lane, the coordinator or the owner. It orders the lanes and takes the
decisions that order needs, the two of section 22.1 that are due by milestone 2 among them. It
proposes four `docs/decisions-log.md` entries, an addendum to D151 and the verification row text
that goes with them (section 6). The first entry is architecture revision 3.4, which is a few
sentences and lands before the first lane: what a world registers is held to section 8.2 wherever it
is declared, with two readings and the record that milestone 1's exit asked for. Section 6 gives its
exact text. The plan's own rulings reach the architecture in revision 3.5, a separate pass, and
section 8 lists what it must settle. Section 9 says what stands between milestone 1's exit and the
first lane. Section 10 says what this plan could not verify.

Conventions:

- A verification row is cited by its line number in `docs/verification.md` at `a561b36` and by its
  opening words, since line numbers move. Rows 1 to 168 kept their numbers from `0810f9f`; row 169
  is D151's and row 170 the release baseline's.
- Finding ids X1 to X5 and M1 to M5 are those of the follow-up project review of 16 November 2026 at
  `4ffcd62`, milestone 1's exit step 7. S1 is the review's of lane `exit-tooling`. N4 and N6 are the
  exit's criteria review's, at `8941997`. Each is glossed where it is first used.
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
- "The discrete GPU" is the Windows laptop's. "The desktop" is the owner's x86 desktop. "The M1" in
  milestone 2 is the development Mac on macOS 27.2.1: the owner upgrades it on 17 November 2026,
  after milestone 1's criteria were proven and reviewed on macOS 14.7.4, and before milestone 2's
  first lane (section 9).
- A probe is a command run on the development Mac on 16 November 2026, under macOS 14.7.4 and
  outside the tree: this plan's, or one of its reviews'. Section 10 lists them. Nothing in this plan
  was run on macOS 27.2.1.

## 1. The criteria

Section 22 gives milestone 2 seven proofs. At `a561b36` none of the eight crates it delivers
exists, nor the client binary; `crate-tiers.toml` already names each of them. The RHI spike has
run under MoltenVK (`docs/results/spike-rhi-apple-m1-2026-11-15.md`).

| # | Criterion | What proves it | Machine | File or run | Closed by |
|---|---|---|---|---|---|
| 1 | A triangle through the graph on Windows and macOS | The client binary in its fixed-frame mode: a fixed clock, a scripted input stream, a local world whose rows move, extract each frame, and instanced triangles at the extracted transforms, drawn by a `render-graph` pass after the scatter pass into an offscreen target, which the present thread draws or blits to the swapchain image. The run dumps what it drew the last frame with. A CPU reference is rasterised from the dump and compared with the offscreen target's readback, each validator is shown on, and none reports anything else (decision 5.1) | Windows: the discrete GPU. macOS: the M1 under MoltenVK (decision 5.7) | `triangle` | `frame-impl`, `measure-m2` |
| 2 | MoltenVK, KosmicKrisp and native Metal spikes recorded, and `rhi-api` frozen or amended on their findings | Three records and a decision. MoltenVK: the RHI spike's file; the one shape it still owes, where the drawable blocks, shown by `swapchain`; and its open cost figures through `rhi-vulkan` (decision 5.6). Native Metal: a throwaway program on the M1. KosmicKrisp: `rhi-vulkan`'s conformance suite, the swapchain's cases and the cost figures under it on the M1, beside MoltenVK's (decision 5.7). The planner's exit decision that records the freeze (decision 5.13) | The M1, on macOS 27.2.1 | `spike-rhi` (on main), `spike-metal`, `rhi-conformance` and `gpu-cost` under both drivers; `docs/plans/rhi-freeze-decision.md` | `spike-metal`, `vulkan`, `swapchain`, `measure-m2`, the exit |
| 3 | Overlap with exclusive extract | Tests of the client's frame, by ordering and never by timing. (a) No stage of a tick starts before extract's pass has resolved, and no frame reads the snapshot buffer or the ring region extract is writing. (b) Frame N's record and tick N+1's stages are in flight at once: each is held at a barrier until the other has started, so the test hangs if they run in turn. (c) A test that records through the harness session steps one world with the render side attached, on `rhi-null`, and without it, and SOUR finds the hashes equal in every mode and in the nightly's release build | The M1 at the merge; the three CI platforms for (c) | The nightly's run id, in the row | `frame-design`, `frame-impl` |
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

Milestone 1's exit sent its x86 half here too (D151). The desktop's figures for milestone 1's
criteria 1, 4, 6 and 9 and for milestone 0's carried criteria are read in milestone 2: row 169,
decision 5.19. The eight-core tick the desktop gives for criterion 1 is of the tick alone. The
composed tick, the same scene with the physics phase, takes 5,772.5 µs on the M1's four frame
workers and projects to 4,433.1 µs on eight (`docs/results/composed-tick-apple-m1-2026-11-15.md`),
and no criterion reads it (the exit review's N6). Neither figure is a frame budget. What milestone 2
measures of a frame beside physics is `record-stall`.

One outcome is not section 22's and binds the milestone all the same: every check job of the
pull-request workflow stays inside ten of its fifteen minutes, by the median of its three most
recent warm runs of main (decision 5.15).

## 2. The verification rows

The 26 rows whose gate names milestone 2 at `a561b36` each have an owner below, or are re-gated with
a reason. "Pass" is the coordinator's verification pass. "The first sitting" is the desktop's, for
milestone 1's sessions A and B (decision 5.19). Section 6 gives each row's new text.

| Row | Opening words | Owner | What closes it |
|---|---|---|---|
| 15 | MoltenVK and KosmicKrisp | `vulkan`, `swapchain`, `measure-m2` | Both drivers on the M1 under macOS 27.2.1. The conformance suite under each gives the capability answers: indirect-draw behaviour, memoryless attachments and local reads. `gpu-cost` under each gives the encode time for ten thousand draws and the pipeline creation cost |
| 16 | Extract cost | `present-impl`, `measure-m2` | `extract-50k`: the pass's time, and the cost per dirty row on one worker |
| 18 | Native Metal | `spike-metal`, then the freeze decision | The spike's result file: argument buffers, residency, memoryless attachments, framebuffer fetch, indirect draws. `rhi-api` freezes at `vulkan`'s merge, and the exit decision records what is frozen and what was amended. **The spike is done** (`docs/results/spike-metal-apple-m1-2026-11-17.md`): every item yes; the RHI spike's fourteen points kept, point 4 with the rule that a memoryless attachment is never stored; samplers, aliased transients and the drawable added; indirect command buffers kept out of `rhi-api`; three conflicts with section 12.1, D45 and D43 named for the freeze decision |
| 19 | Remote transform upload | `presentation`, `measure-m2` | `remote-upload-5k` |
| 21 | Physics phase stall | `frame-impl`, `measure-m2` | `record-stall`, recorded |
| 34 | MoltenVK multi-draw-indirect is a CPU loop | `measure-m2` | `gpu-cost`: one multi-draw of N commands against N draws under MoltenVK, beside a reading of the pinned MoltenVK |
| 35 | Slang's Metal target supports argument buffers, function constants and framebuffer fetch | `spike-metal`, `slang` | The spike's shaders are MSL the Slang compiler emitted and use all three. The cooker's are then held by a pipeline state made from each: on the M1 from the cooked MSL, compiled through Metal at run time, and on the runner from the metallib. **The spike's half is done** (`docs/results/spike-metal-apple-m1-2026-11-17.md`): Slang's MSL, with a 1024-texture argument buffer indexed non-uniformly, two function constants and a framebuffer-fetch input, compiled through Metal as MSL 3.1 with no error (Metal reports no warnings at run time; offline `metal` warns twice on the frame's vertex functions) and made into thirteen pipelines that read back exact |
| 63 | Windows efficiency-core steering | `eco-qos` | The benchmark's rows with two background zstd tasks, before and after, on the laptop, which is a hybrid part |
| 64 | `lld-link` on Windows | Session B, in the first sitting | The desktop's edit-to-playing file under each system it boots, at most 30 s (D151) |
| 77 | Wake latency against the 20 µs P99 criterion | Session A, in the first sitting | The desktop's `whisk` file: the wake from an idle pool under 35 µs at P99, with the makespan and the steal beside it. The laptop missed the first two (D151) |
| 80 | The Linux and Windows core-topology parsers | Session A, in the first sitting | The topology the desktop's files print (D151) |
| 81 | Tick-start latency against the P99 criterion | Session A, in the first sitting, for the x86 half | The desktop's `whisk` file: under 1 ms at P99 on Windows (D144), and under 35 µs on its other system if that is Linux (D138's addendum). The M1's half is done: `keeper` merged at `0ec2d90`, and the file there gives 24.4 µs against 35 |
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
| 164 | The keeper's timed wait on Windows | `frame-design`; session A, in the first sitting | Its second half asks that the unmeasured list be read where frame pacing on Windows is designed. `frame-design` reads it. An item its design uses becomes a measurement it names for `measure-m2`, and the rest is re-gated with its reason. Its first half, the desktop's Windows `whisk` file, is session A's, at the sitting's commit (D151, decision 5.19) |
| 169 | The x86 half of milestones 0 and 1 | Sessions A and B, in the first sitting; the planner on a miss | Every criterion sessions A and B read passes on the desktop, the labelled laptop's misses first, and the desktop's recorded hashes equal the nightly's at the sitting's commit, by `cargo xtask sour compare`. Or the planner brings a remedy and the owner rules on it when the files are in, without waiting for this plan's exit |
| 170 | SOUR hashes come from a dev build | Pass | The first nightly after `a561b36` in which each platform's release baseline equals its dev baseline, and the bone scene's two streams are equal across the three platforms (X5). A difference is a finding in the crate it names |

## 3. The lanes

Lane `test-binaries` has merged as D150 (`72be20d`, fixed by `d7a0088`): each package's integration
tests are one binary, and its rule binds every crate milestone 2 adds (decision 5.15). Architecture
revision 3.4 is applied before `present-api` and `sketch` are briefed: the boundary check merged at
`a561b36` refuses that facade as revision 4 wrote it (decision 5.18). No lane that builds or
measures on the Mac starts before it is on macOS 27.2.1 and the coordinator's re-check is done
(section 9). Each row below is the size of a brief.

**Queue 1, the RHI.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `spike-metal` | The native Metal spike: a throwaway program on the M1 against Metal itself, never checked in, as the milestone 1 plan's decision 5.10 ruled for the RHI spike. Its question: what must `rhi-api` expose for section 12.1's list to run on Metal with no call Vulkan or a null backend cannot honour? It walks: a bindless heap as an argument buffer indexed past slot 0 with a non-uniform index, in a shader form the Slang compiler emits and Metal accepts; residency declared per encoder, for the heap and for a buffer a shader reaches only through its `gpuAddress`; a memoryless G-buffer read in its pass by framebuffer fetch, and a memoryless depth attachment kept across that read; two transients that alias one untracked heap with the fence between them; an indexed indirect draw and an indirect command buffer; a pipeline from MSL the Slang compiler emitted (the SDK's copy, its version in the file), specialised by a function constant; a binary archive saved and reloaded in a second process with fail-on-miss, run without shader validation; command buffers encoded on two threads and enqueued in a fixed order; a shared event waited on before it is signalled; a sampler; a drawable from a layer in no window, with `nextDrawable`, a render into its texture and `presentDrawable`, three in flight, submitted on a second command queue that waits on an event the first signals, for the call's shape alone; a readback against a CPU reference. It is built with macOS 14 as its deployment target, and any call newer than macOS 14 is named as such in the file | The Mac on macOS 27.2.1 and the coordinator's re-check done (section 9). `reviewer-gpu` reviews it alone | Result file `spike-metal`. Each item yes or no with the state the caller carried. Metal API validation and Metal shader validation each shown on by a misuse it reports, a heap slot and a buffer dereference without residency among them. Whether the fence, dropped with the GPU made to lag, showed. Whether a binary archive works from a library compiled from source. The heap's shader form it chose, a fixed-size array in a parameter block or `DescriptorHandle`. The device comes from `MTLCopyAllDevices`, and the run fails when there is none. Each of the RHI spike's fourteen points marked kept, changed or Vulkan's alone. Rows 18 and 35 | 7 |
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
| `renderer` | Architect: the facade of the minimal `lactose-renderer` of decision 5.4 and no more: the render slot table by `StableKey`, which declares no system and takes despawns and a restore's reconciliation from extract's sink (decision 5.18), the staging ring's writer, which offers no read, the scatter, the instanced triangle pass, the latched camera buffer, `RenderFeature` | The `graph` and `present-api` facades merged | A facade. `reviewer` and `reviewer-gpu` both read it. Any math crate it takes leaves the shipped binaries' boundary feature trees identical (`cargo xtask features`) | 7 |
| `frame-design` | Architect: the client's frame in `lactose-app`, and what `lactose-app-client` wires. The frame's order: the ticks, `Simulation::run_passes`, the snapshot's publication, then the next ticks beside the presentation schedule, graph compile, record, latch and submit. What the frame renders into: two offscreen targets or more, of which the present thread holds the newest finished one and draws or blits it to the swapchain image on the present queue (decision 5.6). Who paces it against the present thread. The traits of `present-api` and `rhi-api` through which `app` drives the render side without naming a backend (decision 5.11). The physics reservation for render recording. Where criterion 3's recording test lives. Frame pacing on Windows, read against the keeper's unmeasured list (row 164), with any measurement it needs named for `measure-m2`. The mover of the client's local world, a system of `game-sim` that the binary and criterion 3's recorded test both link (decision 5.18) | The `platform`, `graph`, `renderer` and `present-api` facades, `server-closure` and `sketch` merged | A facade and decision text that amends D131's surface. `lactose-app`'s normal-dependency closure reaches no `[server_denylist]` crate, by the rule `server-closure` adds. `reviewer` and `reviewer-gpu` both read it | 7 |
| `graph-impl` | Implements the graph: order, culling, transient lifetimes, aliasing, transitions, queues, the capped compile cache, record on frame workers with one span for each pass, and one command pool for each recording job and frame in flight | `graph` and `rhi-null` merged; `vulkan` merged for its device tests | On null: a build that drops a derived transition fails a test, by null's state rule; an unreachable pass is culled and a pass whose only output is a readback or an import is kept; each input of the cache key, changed alone, forces a recompile; two transients alias, the second starts undefined, and a read before its first write is refused. On devices, by class of hazard (decision 5.17): a build that drops the barrier on a tracked attachment or buffer yields a synchronisation-validation message the gate counts; a hazard between the host and the GPU yields a wrong readback with the GPU made to lag; aliasing, reads through the heap, indirect and index buffers and ownership transfers rest on null's rules. A command pool is reset only after its frame's timeline value, held with the GPU made to lag (`VUID-vkResetCommandPool-commandPool-00040`). Where a device has one queue the roles share it: null runs the graph under both capability sets, the M1's four families run the several-queue path, and lavapipe's one queue never does. `reviewer-gpu` and `reviewer-concurrency` read beside the first review | 11 |
| `renderer-impl` | Implements the minimal renderer | `renderer`, `graph-impl`, `vulkan` and `slang` merged | Slots are allocated on the CPU, freed from the structural log's despawns and reconciled after a restore, both handed to the slot table by extract's sink, shown with a prediction world's rollback. A batch names each slot once: the batch's builder resolves duplicates on the CPU in batch order and asserts it, a test on null holds it, a build without the resolution fails that test, and a device test puts many duplicates in different workgroups of a large batch. With the GPU made to lag, the ring never overwrites a region a frame in flight reads, and frame N's readback shows N's camera after N+1's has been written. An instance and the camera far from the origin in one cell render the pixels they render at the origin. A device test steps a world, extracts, renders and compares the readback with the CPU reference. Steering: the ring's write cost for 50k rows. Both specialist reviewers read beside the first review | 9 |
| `frame-impl` | The frame in `lactose-app` and the binary `lactose-app-client`: a local world, the cooked pack set mounted by the binary through `lactose_asset::pack_set`, since `lactose-app` takes no dependency on `lactose-asset` before milestone 3 (D136), the pipeline cache file, resize, and the fixed-frame mode of decision 5.1 | `frame-design`, `present-impl`, `presentation`, `renderer-impl`, `swapchain` and `crash-hook` merged | Criterion 3's three tests; the recording test is in the module `recorded` of its package's `integration` target, and that package is a `[harness]` package (D150). Criterion 1's run on the M1 and on the laptop, and each of its four mutations fails it. `lactose-app-client` joins `[feature_roots]`, and its boundary feature tree equals `lactose-app-headless`'s. The mover is written in `game-sim`, a crossing the brief names, and the recording test links it there: no copy of it is in test code. The binary declares nothing a world registers and is no carrier (decision 5.18). `lactose-app`'s closure still reaches no `[server_denylist]` crate. Both specialist reviewers read beside the first review, and a third fix round is expected | 11 |

**Queue 3, shaders, presentation and tools.**

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `slang` | The Slang toolchain in `tools/cook` [RICOTTA]: the shader kind, cooked by the pinned Slang release to SPIR-V and MSL on every host and to a metallib on macOS (decision 5.8). The pin, a tracked file with the version and each host's archive checksum, and one command that installs it on a host. The cooked format `rhi-api` names, with the heap's shader form. Two checks on what the cook emitted: every access through the heap carries the non-uniform decoration in the SPIR-V, and every fragment output has one location on both targets. Slang's warnings are errors. On macOS the cook runs `metal` itself on the MSL it emitted, under a stable file name, as MSL 3.1 with macOS 14 as the deployment target. The `shaders` job. One engine shader is written against an interface specialised statically and a link-time constant, for both targets | `rhi-api` merged | Cooking twice writes the same SPIR-V and MSL, whatever the checkout's path; whether two cooks write the same metallib is recorded and nothing rests on it. Each check has its mutations for the heap form `spike-metal` chose, and the cook refuses each: the marker removed; for a fixed array, the marker inside a helper that is not inlined; for `DescriptorHandle`, `NonUniformResourceIndex` in place of `nonuniform`, which Slang only warns of; a shader whose one output is `SV_Target1`. Every cooked MSL is compiled on the M1 through Metal at run time, as MSL 3.1, and a pipeline state made from it with its function constants, by a Swift script under `tools/xtask`, run with the command line tools' `swift` by `cargo xtask gpu`, which this lane adds with that step alone: no `unsafe` outside section 21. It takes its device from `MTLCopyAllDevices` and fails when there is none. The cook's `metal` step runs on the M1 too, with the Xcode the owner installs after the upgrade. Criterion 7, with each metallib also loaded on the oldest macOS image the hosted runners offer, if it has a Metal device. A pipeline from the cooked SPIR-V passes the suite's pipeline case, once `vulkan` runs it. The shader kind's load against the 0.5 ms slice (row 136). The Slang behaviours the GPU reviews found on the SDK's copy are run again on the pin. `reviewer-gpu` reads beside the first review, and a third fix round is expected. Pushed alone | 10 |
| `present-api` | Architect: the facade of `lactose-present-api`: the mesh, light, decal and camera rig components, VFX and audio request events, UI view-model resources, the debug-draw buffer, and the presentation snapshot, triple-buffered. Extract is its pass system, as the gather is `net-api`'s (D121), and writes transforms through a sink the renderer's ring implements. The sink also takes the structural log's despawns since extract's cursor, and the signal for a full pass after a restore or a load, so the renderer's slot table needs no system of its own. The crate declares components and a `#[system]` outside the boundary, so this change lists it under `[lint_file_carriers]` and gives it a byte-identical copy of `crates/core/clippy.toml`, three tables with the CPUID entries (decision 5.18). The listing is also a line of `tools/xtask/src/policy.rs`'s test of the carriers' list, a crossing its brief names. It names no RHI, platform or `net-host` type, so it stays what `[boundary_internal]` admits | Revision 3.4 applied (section 9) | A facade and decision text. `cargo xtask boundary` passes on it, and reports its components and its system with the crate taken off the carriers' list. Its README states its boundary status: outside, `[boundary_internal]`, a lint-file carrier. The consumer rows of `crates/anim/README.md`, `crates/physics-api/README.md` and `crates/predict/README.md` agree | 6 |
| `present-impl` | The snapshot and extract | `present-api` merged. No other lane edits `lactose-ecs` while it does | By counts: the rows dirty in this tick or the one before are written and no other; a row that stopped moving has its previous transform set; a teleport carries the no-interpolate flag; the pass after a restore or a load is full. A reader that interpolates two snapshots never reads the third while extract writes it, shown by a loom model or under Miri, whichever the facade's design admits. A region's world hash is the same before the pass and after it, and with the pass run twice: a pass writes no hashed column (revision 3.4, section 8.1). `reviewer-concurrency` reads beside the first review. Steering: extract's time at 50k dirty rows on null | 8 |
| `presentation` | `lactose-presentation`: the schedule per frame on the snapshot, interpolation between two snapshots, the camera rig's orbit and boom, the batch sample of the interpolation buffer and the remote upload through the ring, presentation events delivered once (decision 5.5). It declares nothing a world registers: its schedule runs on the snapshot (decision 5.18) | `present-impl` and the `renderer` facade merged | Rows 95 and 99. Each remote slot is sampled and written once a frame. A delivered event is not delivered again after a rollback. `cargo xtask boundary` passes with the crate outside `[lint_file_carriers]`. Steering: the upload's time for 5k slots. `reviewer-concurrency` reads beside the first review | 8 |
| `measure-m2` | `cargo xtask measure` gains `extract-50k`, `remote-upload-5k`, `record-stall`, `gpu-cost` and `triangle`, their tests, which are ignored tests of `lactose-app-client`'s `integration` target that the command selects by module path (D150), and the header of section 4 for a file taken on a device | `frame-impl` merged | One command writes every file on the M1. The measurements that need no device run short in the `measure` workflow, their numbers discarded. Times are printed, never asserted. `reviewer-gpu` reads beside the first review | 8 |
| `sketch` | The sketch game moves out of the headless binary into `game-sim`, a `[boundary]` crate that `crate-tiers.toml` already names: its component, its system and its session applier, with `register_types` and `register_systems`. `game-sim` depends on `lactose-core`, `lactose-ecs`, `lactose-reflect` and `lactose-sim` and needs no allowlist entry; it carries a byte-identical `clippy.toml` and a README. The binary keeps the `CrateRegistration` and `Game` values, whose types a game crate may not name, passes `migrations: None` in place of its empty migration function, and leaves `[lint_file_carriers]`, with its `clippy.toml` and the `#[expect]` on `Args::parse` (decision 5.18). `crates/app/tests/headless.rs` links `game-sim` in place of its copy of the sketch. `cargo xtask measure edit-to-playing` follows the system to its new file, with `docs/results/README.md`'s row. The carriers' list is also a line of `tools/xtask/src/policy.rs`'s test | Revision 3.4 applied. No lane editing `tools/xtask/src` or `lactose-app` in flight | The binary's 120-tick hash and the recorded stream `lactose_app::headless::a_recorded_headless_run`, with its `::physics` stream, are unchanged. `cargo xtask boundary`, `tiers` and `features` pass with `game-sim` in the workspace and the binary off the carriers. With the system patched to read the clock, clippy refuses `game-sim` and, the lint set aside, the recorded test fails under SOUR's repeat mode: the copy it replaces passed both. The patch reads a bit of `subsec_micros()`, since the clock has microsecond resolution on macOS and a nanosecond bit never changes. The report says that the session applier runs in no recorded test: the test spawns through its own applier. The edit-to-playing figure at the new site, as steering. Pushed alone | 5 |
| `server-closure` | `cargo xtask tiers` applies `[server_denylist]` to `lactose-app`'s normal-dependency closure whether or not `lactose-app-server` exists. Today `server_denylist` in `tools/xtask/src/tiers.rs` returns when that package is absent, and it arrives at milestone 4 | No lane editing `tools/xtask/src` in flight | A fixture in which `lactose-app` reaches a denied crate fails with no server package, and passes with the edge removed. The tree at its base passes. `crate-tiers.toml`'s comment on `[server_denylist]` and `tools/xtask/README.md`'s sentence on the server denylist state the new rule, in the same change | 5 |
| `interp-loom` | A loom model of `lactose-net-host`'s sequence lock (row 102), behind an optional feature as `lactose-input-host`'s is; `LOOM_MODELS` in `tools/xtask/src/features.rs` gains the crate | No lane editing `tools/xtask/src` in flight | The model passes, and fails without each ordering the crate states. The nightly loom job runs it. `tools/xtask/README.md`'s sentence on the loom rule names the crate, in the same change | 6 |
| `crash-hook` | The crash hook moves from the headless binary into `lactose-app`, shared by every engine binary, and writes its report to a file as well as to stderr (decision 5.9). RICOTTA keeps its own hook: it does not link `lactose-app`. Its brief names one crossing: `crates/persist/tests/diff.rs`, whose release build warns three times in each nightly's release SOUR step, of two imports and a function that only code under `cfg(debug_assertions)` uses | `sketch` merged, since both edit the headless binary | A subprocess test: a panic in the headless binary writes the report to both and aborts, and a directory that cannot be written costs only the file. This closes the row's "checked by inspection only". Its report records, for the M1 and the laptop, whether the system keeps a crash report; the desktop's setting is recorded in its first sitting. A release build of `lactose-persist`'s tests prints no warning | 5 |
| `eco-qos` | Windows efficiency-core steering (row 63): `ThreadClass::Background` sets the power-throttling thread setting, in `crates/core/src/thread_class.rs`. Whether `ThreadClass::Io` follows is reported and not decided | Nothing: `keeper` merged at `0ec2d90` | On the laptop, the WHISK benchmark's rows with two background zstd tasks, before and after. Miri for `lactose-core` | 5 |

**The coordinator's passes.**

- After the upgrade and before the first lane, with no lane (section 9): the Vulkan SDK, the
  validation layer, the SDK's Slang and Xcode's `metal` re-checked on macOS 27.2.1; `vulkaninfo`
  under MoltenVK and under KosmicKrisp; the RHI spike's program run again; row 62's Rosetta
  re-check; then the quiet window's `cargo xtask measure --all`, which writes the WHISK file too,
  and the `measure` workflow dispatched at that commit before the desktop's first sitting.
- `gpu-probe`, pushed alone: a manually dispatched job that prints what each runner offers. On
  linux-x64, the Mesa version and lavapipe's `vulkaninfo`: the local read, its memory types and
  queues, indirect count, a headless surface, and whether the pinned validation layer and
  GPU-assisted validation load. On macos-arm64, the Xcode version, whether `xcrun metal`
  exists, whether a Metal device is present, and the same for the oldest macOS image the
  hosted runners offer. Beside it, the laptop's `vulkaninfo` for each of its devices, and the
  desktop's from its first sitting: the queue families that have the graphics bit, and which
  of them can present. Its output goes
  into the rows section 6 adds. The io_uring probe of the nightly is the precedent.
- The verification pass, after the `vulkan` merge and again at the exit: section 2's Done marks and
  section 6's rows. Row 170 closes sooner, at the first nightly that shows each platform's release
  baseline equal to its dev one.
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
| 3, shaders, presentation and tools | `present-api`, `slang`, `present-impl`, `presentation`, `measure-m2`; beside them `sketch`, `server-closure`, `interp-loom`, `crash-hook` and `eco-qos` | 66 |

The 23 lanes are 175 runs. Document work beside them is about 23: this plan's two rounds of review
and their answers (6), its re-base, which carries revision 3.4, with that diff's review and its
answer (3), revision 3.5 (3), the freeze decision (3) and the exit (8, as milestone 1 counted its
own: two criteria reviewers, and the milestone 3 plan with its reviews and answers). The whole is
about 200 runs at two fix rounds a lane, and about 240 if a third of the lanes take four or five.
The split of the check jobs, if the owner takes it at the re-base (section 5), is one more lane of
five, `ci-split`, merged before the first crate lane is pushed. D151 adds none: the sittings are the
owner's, and their rows close in the coordinator's pass. A miss on the desktop adds the planner's
remedy, about four runs for a proposal with its review, its answer and its application, and seven
more if the remedy is a lane.

The longest chain is `spike-metal`, `rhi-api`, `slang`, `vulkan`, `renderer-impl`, `frame-impl`,
`measure-m2`: 63 runs, and about 65 where `graph-impl`'s device tests wait on `vulkan`. `slang` is
on it because `vulkan`'s output cases run cooked shaders: it takes the first slot that frees once
`rhi-api` has merged. `present-api` needs revision 3.4 alone and starts before it. The first slot
that frees goes to the chain's head. `sketch`, `server-closure`, `interp-loom`, `crash-hook` and
`eco-qos` need nothing in their queue and fill a slot no head can take. `sketch` goes first of them;
it and `server-closure` merge before `frame-design` starts, and `crash-hook` before `frame-impl`.

Five facades come first: `rhi-api`, `graph`, `renderer`, `platform` and `present-api`, and
`frame-design` amends a sixth, `lactose-app`'s. `lactose-rhi-null` and `lactose-rhi-vulkan`
implement `rhi-api`. `lactose-presentation`'s one consumer is the client's frame, which
`frame-design` covers. `tools/cook` is a leaf.

Eight rules bind whatever the coordinator reorders:

- One lane at a time edits each of `lactose-ecs`, `lactose-app`, `tools/xtask/src` and `tools/cook`.
  Edits to `crate-tiers.toml` and `ci-policy.toml` are ordered by merge. In `tools/xtask/src` the
  order is `ci-split` if the owner takes it, then `sketch`, `present-api`, `server-closure`,
  `interp-loom`, `slang`, `vulkan`, `measure-m2`. `present-api`'s edit there is one line of a test,
  the carriers' list in `policy.rs`: it may be written while `sketch` is in flight, and its merge
  takes main's list.
- A backend's lane may amend `lactose-rhi-api` in the change that needs it, called out in its merge
  (section 20, decision 5.13). While `rhi-null`, `vulkan` and `graph-impl` are in flight together,
  the amendment merges first and the others take main.
- A push that changes the cache key, which is `Cargo.lock`, any `Cargo.toml`, `.cargo/config.toml`
  or `rust-toolchain.toml`, is pushed alone, and nothing follows it until its Windows check
  completes (D125). A push that changes a workflow is pushed alone too. That is every lane that
  creates a crate or adds a dependency: `rhi-api`, `rhi-null`, `vulkan`, `swapchain`, `platform`,
  `platform-impl`, `graph`, `renderer`, `frame-design`, `graph-impl`, `renderer-impl`, `frame-impl`,
  `slang`, `present-api`, `presentation`, `sketch` and `interp-loom`, and `present-impl`, `eco-qos`
  or `measure-m2` if they touch a manifest; and `gpu-probe`. So at least eighteen of the milestone's
  pushes each hold the push slot for one Windows check: 7:20 to 9:38 over the six runs since D150's
  merge, so under three hours across the milestone.
- After a push that changes a GPU crate, the next such push waits for a green `gpu (lavapipe)`
  job too.
- A crate that milestone 2 adds follows D150. Its integration tests are one binary, `integration`,
  whose root declares every other file of `tests/` as a module. A file of recording tests is that
  binary's module `recorded`, included by `#[path]`. A test file that needs a process of its own is
  listed under `[test_targets]` `separate` with its reason. A measured test or a device test is an
  ignored test, selected by `--test integration` and its module path. A nightly Miri filter names a
  module the binary's root declares (decision 5.15). A file that is not text is named `binary` in
  `.gitattributes` by the change that adds the first of its kind (D148).
- What a world registers is held to every rule of section 8.2 and declared in a boundary crate or a
  lint-file carrier, or in test code alone: a `#[system]`, a pass system included, a `#[component]`
  or `#[resource]` that is not `derived`, an applier, a hand-written `impl Reflect`, a
  registration's setup and migration functions. A system that writes a hashed column is declared in
  a library, and runs on rows in a test that links that library and records through the session; a
  copy of it in test code does not count (revision 3.4, sections 8.1 and 8.3). Of milestone 2's
  crates `lactose-present-api` is a carrier, the mover is `game-sim`'s, no other crate declares any,
  and no binary is a carrier once `sketch` has merged (decision 5.18).
- The measurement commit is the first commit of main that holds `frame-impl` and `measure-m2`. The
  quiet window, the laptop's session and session C run there (section 4). A lane that merges after
  it and edits a measured crate runs the measurements it touches and reports them as steering.
- A device test compares against a reference the test computes on the CPU, at a margin it
  justifies. No image is checked in (decision 5.10).

## 4. Machines, workloads and result files

| Machine | What it can show | What it cannot |
|---|---|---|
| The development Mac: Apple M1, four performance and four efficiency cores, on macOS 27.2.1 from milestone 2's first lane, with the Vulkan SDK's MoltenVK and KosmicKrisp, its validation layer, and the Xcode current for macOS 27.2.1, each re-checked after the upgrade (section 9) | MoltenVK and KosmicKrisp under the Khronos layer, Metal API validation and Metal shader validation. A window and a present, and a headless surface. The native Metal spike. Cooked MSL compiled through Metal at run time, and a metallib built with Xcode's `metal`. A buffer in device-only memory. A hazard between the host and the GPU, with the GPU made to lag. Timing in a quiet window | macOS 14, the oldest client the cook targets: nothing in reach runs it once the Mac is upgraded. A missing barrier between two GPU uses, by pixels: Metal orders tracked resources itself. GPU-assisted validation under MoltenVK, which cannot compile it; under KosmicKrisp it is not known |
| The Windows laptop: Intel i7-12650H, six performance and four efficiency cores, an RTX 4070 laptop GPU and, it is expected, an Intel integrated one, Windows 11, driven through its own agent session | Vulkan on a discrete GPU and the Win32 swapchain. The triangle on Windows. GPU-assisted validation. The cost of a write-combined ring. The power-throttling setting on a hybrid part. Frame pacing on Windows | Section 20's x86 desktop figures, which are the desktop's (the owner's answer 2). Memory that is not coherent, or lazily allocated: no PC driver surveyed offers either. Its figures are labelled |
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until its first sitting, which returns them (decision 5.19) | The x86 desktop figures section 20 asks for, and the x86 half of milestones 0 and 1 (D151). A second discrete GPU | Anything without the owner at it |
| linux-x64 runner: no GPU | lavapipe, a CPU rasteriser: the conformance suite and the device tests under the pinned validation layer on every pull request, and under GPU-assisted validation nightly | Timing. Tile memory. A vendor's driver. A window. Two queues or an ownership transfer: Mesa's source gives lavapipe one queue and one coherent memory type |
| windows-x64 runner: no GPU, no Vulkan driver | The build, the lints and the tests on null | Any Vulkan call. The Win32 path runs on the laptop alone |
| macos-arm64 runner: Xcode, a virtual GPU | Cooking metallibs (criterion 7). Its oldest image is the one place a metallib can be loaded on an older macOS | Timing. Whether it runs Metal or MoltenVK tests, and what its oldest image offers, is `gpu-probe`'s to find |

**Five kinds of sitting produce every file a criterion rests on.**

- *At each merge of a GPU crate*, on the M1: the extra gate, under MoltenVK and the three
  validators. A run that presents holds the display awake with `caffeinate -d`: under MoltenVK a
  frame into a sleeping display may hold the present thread's submit for seconds (decision 5.6).
  Then, after the push, the laptop's run on its devices.
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s -d`, and `cargo xtask measure --all` runs once. It writes milestone 2's
  five files beside milestone 1's, `gpu-cost` under MoltenVK and under KosmicKrisp. The WHISK
  file's 60 Hz P99 is read against D156: under 30 µs, the planner proposes 35 µs again; the system
  daemon's CPU time is read before and after the run, as the remeasurement of 17 November 2026 did.
- *The laptop's session*, at the same commit, with nothing else running there: the same command,
  and `cargo xtask gpu --driver system`. Its `triangle` file is criterion 1's proof on Windows
  (the owner's answer 2).
- *The desktop's first sitting*, the owner's, as soon as the M1's re-run after the upgrade is on
  main and before the first crate lane merges (decision 5.19, section 9). It is milestone 1's
  sessions A and B, at one commit, the re-run's, under each system the desktop boots. Three
  commands: `cargo xtask measure --all`, which writes the WHISK file and every file of milestone 1;
  `cargo nextest run --locked -p lactose-task`; and `cargo xtask sour run --mode architecture --emit
  <file>`, whose file the coordinator compares with the linux-x64 file of the nightly at that commit
  by `cargo xtask sour compare`. The owner sends back the files, the CPU model, the systems the
  desktop boots and its GPU, with `vulkaninfo`'s output where a Vulkan loader is installed. The x86
  figures are read against the criteria's budgets, beside the M1's files of the same commit. Row 169
  holds what the sitting settles, the labelled laptop's misses first.
- *Session C*, the owner's, at the measurement commit, on the desktop under each system it boots:
  `cargo xtask measure --all`. Its `extract-50k` and `remote-upload-5k` files are the x86 desktop's
  figures. The command writes milestone 1's files again, which shows what milestone 2's lanes moved
  on x86. Under a system that is not a client platform, the measurements that need a window are
  skipped and the file says so.

Only the desktop's two sittings need the owner at a desk. The laptop's runs need the laptop awake
with its agent session open, and a run that opens a window needs a person logged in at it.

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

The other remedy was to switch off MoltenVK's synchronous submits. It is not taken. It is a setting
of the interim driver alone, it moves the wait onto MoltenVK's own dispatch queue and leaves the
frame and the present on one queue, and the RHI spike's file is of MoltenVK's defaults. The present
queue serves the other backends too: `vkQueuePresentKHR` takes a queue the caller must serialise, so
a FIFO present that blocks, which section 5.1 already expects, holds its queue on any driver.
`swapchain` shows the rule on null with the present queue held, and under MoltenVK with the drawable
made to block. On native Metal `nextDrawable` is a call the engine places, so `spike-metal` walks
it, headless, for its shape. KosmicKrisp is a conformant Vulkan driver, so it is the same surface;
it is measured through `rhi-vulkan` once the Mac is on macOS 27.2.1 (decision 5.7). The native Metal
spike is the one that can change a signature (D43): residency, argument buffers, heaps, indirect
command buffers and the binary archive. So the facade waits for it alone, and the cost figures are
taken through `rhi-vulkan`, on the path that ships, under each driver with one command. This rules
out a second throwaway Vulkan program, a frame worker that submits into a swapchain image, a present
that holds the frame's queue, and a frame that waits on the present thread through its one target.
What would change it: `spike-metal` reporting that an item cannot be decided without a MoltenVK
cost. That cost is then measured with the RHI spike's program first. Or a PC driver whose one
graphics queue a blocking present holds, shown on the laptop: the present alone then moves to a
queue of another family that can present, after the blit on the graphics queue.

**5.7 The Apple interim path is MoltenVK, and KosmicKrisp is measured beside it on the same Mac.**
Section 22.1 asks for the path by milestone 2. Every item of the RHI spike ran under MoltenVK 1.4.2
with a GPU-side use read back exact. KosmicKrisp is built on Metal 4 and needs macOS 26 or later
(Mesa's driver documentation); on macOS 14.7.4 the SDK's driver enumerated no device. The owner has
ruled that the Mac is upgraded at milestone 1's exit, and on 17 November 2026 named its system:
macOS 27.2.1. So from milestone 2's first lane both drivers are expected on one machine. Nothing has
run on that system, and the coordinator's re-check says whether the SDK's MoltenVK and its
KosmicKrisp do (section 9). The path is still MoltenVK, for three reasons that do not rest on what
KosmicKrisp could not do. The decision is needed before the first lane, and the only evidence in
hand is MoltenVK's: the spike, the facade's fourteen points and null's rules come from it, and
nothing has run under KosmicKrisp. MoltenVK runs on macOS 14 and 15, where KosmicKrisp cannot, and
macOS 14 is the oldest client the cook targets (decision 5.8). And the path lasts one milestone:
`rhi-metal` lands in milestone 3. So the M1's gate runs under MoltenVK, and KosmicKrisp is no gate.
It is recorded three times through `rhi-vulkan`, by one command each and with no lane of its own:
the conformance suite at `vulkan`'s merge, the swapchain's cases at `swapchain`'s, and `gpu-cost` in
the quiet window, beside MoltenVK's figures. A case that fails under it is recorded and fails no
merge. Two costs of the path are accepted for one milestone. The G-buffer in tile memory is not
shown on Apple before `rhi-metal`, since MoltenVK's local read restarts the Metal pass. GPU-assisted
validation does not run under MoltenVK, so it runs under lavapipe and on the laptop. One shape the
path showed is adopted for every backend and outlasts it: the present thread's submits and its queue
(decision 5.6). This rules out a gate under a driver nothing has run on, a second gate driver at
every merge, and a KosmicKrisp record without timings. What would change it: KosmicKrisp passing
every output case of the suite on the M1 by decision 5.10's rule, and offering one thing MoltenVK
does not: a memoryless attachment read in its pass and kept across a local read, GPU-assisted
validation that runs, or a drawable taken outside a submit. The planner then brings the owner a
switch of the gate's driver, as an addendum to D154, with MoltenVK run beside it for macOS 14. A
KosmicKrisp that does not enumerate the M1 on macOS 27.2.1 is recorded as that, and the question
returns to the owner.

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
  that Metal refuses. The heap is a fixed-size array in a parameter block, the form `spike-metal`
  chose (`docs/results/spike-metal-apple-m1-2026-11-17.md`): its slot is one number on both
  targets, where a `DescriptorHandle` is an `MTLResourceID` on Metal and an index into a heap
  Slang declares itself on SPIR-V, so the value a caller writes would differ by backend. The cook
  refuses `DescriptorHandle`: a SPIR-V module that declares the `RuntimeDescriptorArray`
  capability, which Slang's own heap needs and the fixed array does not; Slang places that heap at
  set 1 or 2 by what else the module declares. The cooked format pins the heap's set, names the
  form, and each heap array's type and capacity. The marker is a helper that switches on the
  target, and it emits SPIR-V with no decoration unless it is inlined. So the cook reads the
  SPIR-V, where every access through the heap must be decorated, and treats Slang's warnings as
  errors.
- *Outputs.* An output struct whose one field is `SV_Target1` gets location 0 in SPIR-V and
  colour 1 in MSL. The cook requires each fragment output to have one location on both targets.
- *The local read.* Slang turns an input attachment into a fragment input at the colour index
  of the same number, and can emit nothing else, so the cook has no check for it. The facade's
  local read is that identity, and `rhi-api` and null hold it at run time.

SPIR-V and MSL are cooked on every host and their bytes are equal across hosts, which the `shaders`
job compares. The cook turns line directives off, since the MSL carries the source's path when the
compiler is given an absolute one. Cooked MSL is compiled on the M1 through Metal at run time, as
MSL 3.1 named in its compile options since their default is the system's newest, and a pipeline
state made from it, by a Swift script that `cargo xtask gpu` runs with the command line tools'
`swift`: that needs no Xcode, and no `unsafe` outside section 21, which exempts `tools/cook` for
Slang and compression alone. A metallib needs Apple's `metal` compiler, so it is cooked on macOS: on
the runner, and on the M1 once the owner has installed the Xcode current for macOS 27.2.1 (answer
5), so a wrong flag is found on the branch. Slang's own call of that compiler passes
`-std=metal3.1`, no deployment target and an input file whose name changes at every run (the GPU
review's stand-in compiler). So the cook runs `metal` itself on the MSL it emitted, under a stable
name, as MSL 3.1 with macOS 14 as the deployment target. MSL 3.1 needs macOS 14, and that makes
macOS 14 the oldest client the cook targets. Once the Mac is upgraded no machine in reach runs macOS
14. That a metallib cooked for it loads there is then shown only on a hosted runner's oldest image,
if that has a Metal device, and that the engine runs there is not shown at all. Equal bytes are
claimed for SPIR-V and MSL and not for a metallib: whether two cooks agree is recorded. One metallib
the runner cooked is loaded on the M1. This rules out a Slang taken from whichever SDK a host has, a
check that reads the source alone, a check that cannot fail, and cooked bytes that depend on a
checkout's path. What would change it: process starts dominating the cook at milestone 3's 48
pipelines, or the cooker needing reflection the command does not give. The FFI is taken then.

**5.9 Crash reporting is the panic hook's text report, in `lactose-app`, until a build leaves the
owner's machines.** Section 22.1 asks for minidumps and symbol upload by milestone 2. The hook is in
the headless binary today, and its README and row 134 give it to `lactose-platform`, a crate
`[server_denylist]` denies the server. `lactose-app` is what every engine binary links, and the hook
is safe code. RICOTTA has a hook of its own and keeps it: a tool does not link the assembly. The
report goes to stderr and to a file. A hardware fault, which a driver call makes possible from this
milestone on, is read from the operating system's own report. macOS writes one by default. Windows
keeps a local dump only once Windows Error Reporting's `LocalDumps` key is set, which is off by
default and an administrator's change (Microsoft's documentation), so the owner sets it once on the
laptop and the desktop; `crash-hook`'s report records the M1's and the laptop's setting, and the
desktop's is recorded in its first sitting. A minidump writer that catches such a fault runs in a
signal handler, which is `unsafe` that section 21 admits in no crate every binary links, or in a
second process, which is a monitor to build and ship. An upload needs a service and a symbol store,
and nothing reads either before a build goes to a tester. So neither is built, and the row of
section 22.1 moves to milestone 5. This rules out a hook the server cannot link. What would change
it: a fault on the owner's machines that the system's report does not explain, or a build given to a
tester sooner.

**5.10 One command runs the device tests by one rule, lavapipe runs them on every pull request,
and no GPU runner is bought.** `cargo xtask gpu --driver <name>` sets the loader's environment
for its child as the RHI spike's run header set it by hand, cooks the suite's shaders, runs the
conformance suite and every device test, and writes a result file. The drivers are `lavapipe`,
`moltenvk`, `kosmickrisp` and `system`; under `system` the device is selected by name and
recorded, since a laptop has two. A device test is an ignored test that the command selects, as a
measurement is, so a job with no driver does not run it, and under a named driver a missing
device fails the run. The rule a run passes by:

- *What is counted.* Every message from four sources: the Vulkan debug messenger; Metal API
  validation's and Metal shader validation's lines on stderr; the driver's own error and warning
  lines, MoltenVK's and, under KosmicKrisp, Mesa's log, recorded in KosmicKrisp's files beside
  MoltenVK's.
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

**5.15 Every crate milestone 2 adds follows D150, and the CI budget is read as a median over every
check job, which stands at nine minutes of ten on Windows.** D125 called one integration-test binary
for each crate the next lever. Lane `test-binaries` carried it out as milestone 1's work and merged
as D150 (`72be20d`, fixed by `d7a0088`): 179 integration-test targets became 46. Its rule binds
every crate milestone 2 adds. A package with more than one integration test file builds them as one
target, `integration`, whose root declares each other file of `tests/` as a module. A file whose
name is a codename is declared under a name for what it holds, so a file of recording tests is the
module `recorded`, included by `#[path]`. A test file that shares process state stays a target of
its own, listed with its reason under `[test_targets]` `separate`, and `cargo xtask boundary`
reports a second target outside that list. A nightly Miri filter is `<module>::`, of a module the
root declares. A measured test and a device test are ignored tests of `integration`, selected by
`--test integration` and the module path, as `cargo xtask measure` selects milestone 1's. That binds
`measure-m2`, `interp-loom`, `frame-impl`'s `[harness]` package and each new crate.

The conversion did not buy the room it was made for. Before it, three runs of one code gave the
Windows check 11:39, 11:22 and 9:30 (runs 43781979172, 43795852109 and 43799365983). The four runs
of main after it, each a full cache match, gave 9:08, 9:04, 9:38 and 7:20 (43827738169, 43855576989,
43868030471 and 43872419194), and D150's addendum reads the gain as no larger than the runs' own
spread. Two more full matches followed, at `d7f3e3b` and `a561b36` (43886836183 and 43896909697):
9:30 and 9:13 on Windows. So at `a561b36` the medians of the three most recent runs are 9:13 on
Windows, 7:28 on macOS and 4:26 on Ubuntu, and five of the six Windows runs since the conversion are
over nine minutes (the review of this re-base, from the job logs). The workflow's own signal is ten
of fifteen minutes, and milestone 2 adds nine crates and a binary.

The budget is read as revision 3 of this plan set it. One run reads either side of ten by chance, so
the figure is the median, for each check job, of the three most recent runs of main that were full
cache matches. It is read from the runs main makes anyway. No run is made again for it: the
workflow's concurrency group cancels a run in progress when another run of main starts, and a first
run cancelled after a cache-key change leaves a partial cache entry (D125). The first run after a
push that changes the key is not a full match and is not counted, so through a stretch of such
pushes the figure lags until three counted runs exist. While every median is ten minutes or less,
nothing more is done. When one passes ten, the owner is asked question 6 before the next lane that
adds a crate is pushed.

That rule holds every crate push for the owner's answer and a five-run lane, and its figure lags
exactly while crates are being pushed, when a Windows run on a new cache key is nearest the timeout.
With the Windows median at 9:13, the re-base puts the question to the owner now, as its question 4,
with the trigger as one of the answers. This plan recommends the split now: lane `ci-split`, merged
before the first crate lane is pushed.

Two costs outside the check jobs are recorded and not budgeted here. Since `a561b36` each nightly
SOUR job also builds the tests in release and takes a release baseline (X5). The review of that lane
estimates five to fifteen minutes more a platform, fifteen to thirty-five of the job's sixty, with
the release build possibly cold each night. And the repository's Actions cache stood at 10.74 GB at
that merge, over GitHub's 10 GB, by the same review: entries are evicted, and a job whose entry went
runs cold. Milestone 2 adds two jobs with caches, `gpu (lavapipe)` and `shaders`, and a split adds
one for each job it divides. The lane that adds the first of them records each job's cache size and
says which jobs save one (section 6, rows to add).

This rules out pushing crate lanes onto a job two crates from its timeout, a trigger pulled by a
runner's noise, and hours of main held for re-runs. What would change it: the owner's answer to
question 4.

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

**5.18 What a world registers is held to section 8.2 wherever it is declared: `lactose-present-api`
carries the boundary lint file, the sketch game moves into `game-sim`, which both binaries and the
recorded tests link, and `lactose-presentation` and `lactose-renderer` declare nothing.** Since
`a561b36`, `cargo xtask boundary` reports, in the shipped sources of every crate that is neither a
boundary crate nor listed under `[lint_file_carriers]`, each `#[system]`, each `#[component]` or
`#[resource]` that is not `derived`, each implementation of `SessionApplier`, `InputApplier` or
`StagedState`, and each hand-written `impl Reflect`. The follow-up project review asked for it (X1):
the headless binary's sketch game declared a component, a system and an applier outside every
boundary lint. The check cannot tell a pass system from a tick-stage one: a pass system is declared
as any exclusive system is, and only its registration differs. Revision 4's first lane put the
presentation components and extract's pass system in `lactose-present-api`, which is
`[boundary_internal]` and neither a boundary crate nor a carrier. The review of the check built that
facade as a probe, and the check refused it (S1 of that review). Two remedies exist. The rule could
be narrowed to admit a pass system of an outside crate that writes no hashed column; no tool can
check that of an exclusive system, which takes the whole world. Or the crate carries the boundary
lint file. The second is taken. A crate whose types a world stores, and whose system reads that
world between ticks, is code section 8.2 is for; `lactose-reflect-macros` is already both
`[boundary_internal]` and a carrier; and the boundary file holds every ban of the internal one.

The review of this re-base found two holes in the rule as the re-base first wrote it, and both are
closed here. A system declared in a binary cannot run in a recorded test: a test cannot link a
binary, so it runs a copy, and a copy is the mechanism X1 found. That is the state of the headless
binary's `sketch::count` at `a561b36`, whose only recorded run is of a copy in
`crates/app/tests/headless.rs`, and the re-base's default would have repeated it for the client's
mover (S1). And a lint file is not section 8.2: float `%`, global mutable state and the RNG's keying
are held by review, inside the boundary alone as the text stood, so a carrier's system could break
them and comply (S2).

- What a world registers is held to every rule of section 8.2, those that review holds included,
  whichever side its crate is on. Section 3 says so too.
- `lactose-present-api` joins `[lint_file_carriers]` in the `present-api` change, with a
  byte-identical copy of `crates/core/clippy.toml` as it stands at the lane's base: its three
  tables, the CPUID entries among them. The review built such a facade as a carrier: `cargo xtask
  boundary` passed, and reported its four components and its system with the crate off the list.
- The sketch game moves into `game-sim`, by lane `sketch`. `game-sim` is a `[boundary]` crate and a
  `[game]` crate in `crate-tiers.toml` already, and section 3 names it as the inside half of the
  game. It takes the sketch's component, its system and its session applier. Both binaries link it,
  and `crates/app/tests/headless.rs` links it in place of its copy. Section 22 gives `game-sim` to
  milestone 5: the crate arrives now, the game then.
- The headless binary leaves the carriers in the same change. A game crate may name neither
  `lactose-app` nor `lactose-persist`, so the `CrateRegistration` and `Game` values stay in the
  binary; they are not what the rule lists. Its migration function is empty, and `migrations: None`
  builds the same empty set: the review of this answer showed the binary off the list with `cargo
  xtask boundary` passing and the 120-tick hash unchanged. So no binary is a carrier at milestone 2,
  and the carriers are the two proc-macro crates and `lactose-present-api`. Keeping the binary a
  carrier for a function that does nothing was the other choice. Where a game crate's registration
  value and a real migration are declared is a question for milestone 5 (section 6, rows to add).
- The sketch's session applier moves with it and runs in no recorded test:
  `crates/app/tests/headless.rs` spawns through an applier of its own. Section 8.3's sentence is of
  systems, so nothing is broken, and the applier is under the boundary's lints in `game-sim`; a row
  carries its recorded run. Routing the test's spawn through the shipped applier in this lane would
  risk the recorded stream for no criterion.
- The mover of the client's local world (decision 5.1) is a system of `game-sim`. `frame-design`
  specifies it and `frame-impl` writes it. Criterion 3's recorded test links it there.
  `lactose-app-client` declares nothing a world registers, passes no setup and no migration
  function, and is no carrier, so its window, DPI and file code is under no float or IO ban.
- `lactose-presentation` is not a carrier. Section 8.1 puts presentation outside the boundary. Its
  schedule runs on the snapshot, not on a world. And the boundary file bans the float methods a
  camera rig and an interpolation are written with, `f32::clamp`, `min`, `max` and `sin` among them.
  The crate declares no `#[system]`, no component or resource that is not `derived`, and no applier,
  and the check holds that from its first commit.
- `lactose-renderer` declares no system. Its slot table is freed from the structural log and
  reconciled after a restore (section 12.4), and outside the boundary the log is reached through the
  world alone. So extract's sink hands the slot table the despawns since extract's cursor and the
  signal for a full pass, beside the transforms, and the `present-api` facade says so (M4).
- Of the eight crates section 22 has milestone 2 deliver, `lactose-present-api` alone carries a lint
  file. The other seven are outside the boundary and carry none, as `lactose-app` and
  `lactose-net-host` carry none. `game-sim`, the ninth crate, is inside.
- A pass system leaves the world hash as it found it: it writes no hashed column and nothing a
  tick's systems read. `present-impl` shows it for extract, and criterion 3's recorded test shows it
  under every mode.
- The check sees neither a `CrateRegistration`'s setup and migration functions nor which stage a
  system is registered in (M3). The rule names both, review holds them, and a row records the limit
  with the extension that would lift it: nothing outside a carrier registers either at `a561b36`,
  and nothing is planned to before milestone 5.

Revision 3.4 states the rule in sections 3, 8.1 and 8.3, with what the tool does not see (section
6). This rules out a facade the first gate refuses, a recorded test that runs a copy of what ships,
a carrier that keeps the lints and breaks the rules, and the boundary's float bans on crates that
section 8.1 puts outside. What would change it: the owner keeping `game-sim` for milestone 5. The
sketch and the mover then go into a library target of each binary's package, each package a carrier
and a `[harness]` package for its own recorded test, and the client's window code takes the bans
with local exceptions. Or the `present-api` facade or `frame-design` giving `lactose-presentation` a
system that runs on a world: the crate then joins the carriers in that change, called out at its
merge.

**5.19 The desktop reads one commit at a sitting, and milestone 1's x86 half is read first, before
the first crate lane merges.** This is the plan's recommendation on the re-base's questions 1 and 2,
and it stands only as the owner answers. D151 moved milestone 1's sessions A and B into milestone
2's one desktop sitting, each at the commit milestone 1's plan gives it. Those are three commits
besides session C's: main's head for session A, `bef3455` for session B, and `9c1e005` for the
Windows `whisk` file (D144). The follow-up project review found that session B's comparison of
recorded hashes has no reference at `bef3455` (M1): no nightly ran there, the headless stream was
renamed after it, and the one digest of the spike's hashes at that commit is computed by a script
outside the tree. A `whisk` file from a commit before `b7cedcf` is also judged on Linux against 20
µs, which D138 has since restated.

So a sitting reads one commit, with one set of commands. `cargo xtask measure --all` writes every
file of sessions A and B, the WHISK file among them, and any commit from `0ec2d90` on holds
`timer-windows` and `keeper`, which is all D144's separate commit was for. The recorded hashes are
compared by the tool that compares them every night. The desktop runs `cargo xtask sour run --mode
architecture --emit`, and the coordinator compares its file with the linux-x64 file of the nightly
at that commit by `cargo xtask sour compare`, dispatching the nightly there if the schedule did not
run on it. No digest is used, and `docs/plans/milestone-1.md`'s sentence that a result file compares
the hashes is corrected (section 6). The x86 figures are read against the criteria's budgets, beside
the M1's files of the same commit.

Milestone 1's half is the first thing the desktop does, not the last. The labelled laptop missed
four budgets on x86, and the desktop has measured nothing: its CPU, its GPU and its second system
are in no document. A miss found now gets its remedy beside milestone 2's first lanes, before
`present-impl` takes `lactose-ecs`. A miss found at the measurement commit gets it after the
milestone's last lane. The first sitting also gives `gpu-probe` the desktop's queue families before
`swapchain` is designed, and proves the desktop's toolchain on commands the laptop has already run.
Its commit is the one the M1's re-run after the upgrade is taken at (section 9), so both machines'
files are of one commit. Session C stays at the measurement commit, where `cargo xtask measure
--all` writes milestone 1's files again and shows what milestone 2's lanes moved.

This costs the owner a second sitting, of three commands under each system. It rules out three old
commits built under each system, a reference dug out of artifacts that expire on 13 February 2027,
and an x86 miss of milestone 1 found when milestone 2 is over. What would change it: the owner's
answers. With one sitting, sessions A and B run beside session C at the measurement commit, by the
same commands. With each session at its own commit, D151 stands as written, and the coordinator
first checks in a hash file taken on the M1 at `bef3455` as session B's reference.

**For the owner.** Four questions are open at the re-base and come first below. The rest were
answered on 16 and 17 November 2026, through the coordinator's questions, and are written into this
plan.

**Asked at the re-base.** The coordinator asks these four; the planner does not. This plan is
written for the first option of each, and the list after the block says what another answer changes.

```json
{"questions":[
{"header":"One commit","question":"Milestone 2 plan, re-base, question 1 of 4. Milestone 1's two desktop sessions (A: the scheduler benchmark; B: `cargo xtask measure --all` and the recorded hashes) moved into milestone 2 by D151, each at the commit milestone 1's plan gave it. That is three old commits: main's head of that day, `bef3455` and `9c1e005`. No nightly ran at `bef3455`, so session B's hash comparison has no reference file there. At which commits does the desktop run them?","multiSelect":false,"options":[
{"label":"One commit a sitting (Recommended)","description":"You check out one commit and run three commands under each system the desktop boots. `cargo xtask measure --all` writes every file of both sessions, and the recorded hashes are compared with that commit's nightly by `cargo xtask sour compare`. The x86 figures are of current code, beside the Mac's files of the same commit, not of `bef3455`."},
{"label":"Each at its own commit","description":"D151 as written: three checkouts and three builds under each system. The coordinator first checks in a hash file taken on the Mac at `bef3455` as the reference. The figures are like for like with the laptop's run of 15 November."}]},
{"header":"Sitting when","question":"Question 2 of 4. The desktop has measured nothing yet. The laptop's labelled run missed four milestone 1 budgets on x86: the 100k tick, the physics phase, the wake from an idle pool and the makespan. The plan had one desktop sitting, at milestone 2's measurement commit, after its last lane. When do milestone 1's sessions run?","multiSelect":false,"options":[
{"label":"Now, then session C (Recommended)","description":"Two sittings. The first is as soon as the Mac's re-run after the upgrade is in: sessions A and B, three commands under each system. A miss gets its remedy beside milestone 2's first lanes, and the plan learns the desktop's CPU, GPU and second system before the swapchain is designed. Session C stays at the measurement commit and writes milestone 1's files again at no cost."},
{"label":"One sitting, at the end","description":"As D151 has it. An x86 miss of a milestone 1 criterion is found after milestone 2's last lane, and its remedy follows the milestone. The desktop's CPU, GPU and second system stay unknown until then unless you send them ahead."}]},
{"header":"Old hashes","question":"Question 3 of 4. The nightly of 15 November (run 43703297881, at `c50b688`) showed the netcode spike's 7,200 recorded hash lines equal on linux-x64, windows-x64 and macos-arm64, which is the cross-platform half of milestone 1's criterion 7. Its hash files are run artifacts, and GitHub deletes them on 13 February 2027. Every later nightly repeats the comparison at its own commit. Are the files kept?","multiSelect":false,"options":[
{"label":"Let them lapse (Recommended)","description":"Nothing is done. The verification rows keep the run's id and its counts, and the newest nightly is the live proof. With one commit a sitting, the desktop is compared with a current nightly and never needs the old files."},
{"label":"Check one in","description":"The coordinator downloads the linux-x64 file, 10,334 lines of text, into `docs/results` before it expires, as the record of what milestone 1's exit was proven on. The three platforms' files are byte-identical, so one stands for all."}]},
{"header":"CI budget","question":"Question 4 of 4. Since each package's tests became one binary (D150) the Windows check job has taken 7:20 to 9:38 of its 15 minutes over six runs, and the median of the last three is 9:13, against the workflow's own signal of 10: the conversion gained less than the runs' spread. Milestone 2 adds nine crates and a binary, and the median is read only from runs with a full cache match, so it lags while crates are being pushed. The plan asks you about the budget once a median passes ten, and holds every push that adds a crate until you answer and a five-run lane merges. Do you rule now?","multiSelect":false,"options":[
{"label":"Split the check jobs now (Recommended)","description":"One small lane, five runs, before the first crate lane is pushed: each check job becomes two, by package group, with a check in `cargo xtask` that every package is in exactly one group. The lane also says which jobs save a cache, since the repository's cache is already over GitHub's 10 GB. Each run then uses twice the runners, and no push waits on the budget during the milestone."},
{"label":"Split at the trigger","description":"You rule now, and nothing is done until a median passes ten. The coordinator then briefs the same lane without asking again, and crate pushes wait for the lane alone."},
{"label":"Ask me at the trigger","description":"As the plan had it. The question comes back when a median passes ten, and crate pushes wait for your answer and the lane."},
{"label":"Raise the budget to 20 minutes","description":"No lane. Architecture section 20's fifteen minutes is restated, every job's timeout follows, and a cold run after a cache-key change takes longer still."}]}
]}
```

- *Question 1, "Each at its own commit".* Decision 5.19's first half falls and D151 stands as
  written. The coordinator checks in a hash file taken on the M1 at `bef3455` by `cargo xtask sour
  run --mode architecture --emit`, as session B's reference, before the sitting. A `whisk` verdict
  on Linux from a commit before `b7cedcf` is read against 35 µs by hand.
- *Question 2, "One sitting, at the end".* Sessions A and B run beside session C at the measurement
  commit, by the same commands. Section 9's remedy for a miss follows the milestone's last lane, and
  the owner is asked to send the desktop's CPU, GPU and second system ahead.
- *Question 3, "Check one in".* The coordinator downloads run 43703297881's linux-x64 hash file into
  `docs/results`, and row 148 cites it.
- *Question 4.* "Split at the trigger" and "Ask me at the trigger" take `ci-split` out of the first
  week: decision 5.15's rule runs as written, with or without a second question. "Raise the budget"
  restates section 20 in revision 3.5 and adds no lane.

**Answered.**

| Question | Answer | What it settles |
|---|---|---|
| 1, KosmicKrisp | "Upgrade the Mac to macOS 26", which was not the recommended option | KosmicKrisp is measured beside MoltenVK on the M1, with timings (decision 5.7). The WHISK benchmark and `cargo xtask measure --all` are run again on the M1 after the upgrade (section 9) |
| 2, Windows GPU | "Laptop, one desktop sitting" | As recommended (section 4) |
| 3, Crash dumps | "Text report now" | As recommended (decision 5.9) |
| 4, Asset watch | "Milestone 3" | As recommended (decision 5.14) |
| 5, Xcode, put again after answer 1 | "Install Xcode", after the upgrade | The Xcode current for the system the Mac is then on, macOS 27.2.1. The cook's `metal` step runs on the M1 before a merge (decision 5.8) |
| Added by the coordinator after answer 1: when is the Mac upgraded? | "At milestone 1's exit": after the exit is recorded and before milestone 2's first lane | Milestone 1 closes on the system its figures were taken on, and every milestone 2 figure on the M1 is of one system (section 9) |
| Added by the coordinator after revision 4: a milestone 1 figure that crosses its budget in the re-run after the upgrade | "A finding; M1 stays closed" | The crossing is recorded in the re-run's result file and in a row gated at milestone 2, the planner brings a remedy, and milestone 2's lanes go ahead (section 9) |
| The owner, on 17 November 2026, through the coordinator | The upgrade is to macOS 27.2.1, not 26, on 17 November, after exit step 5 | Every statement of this plan about the M1's system reads macOS 27.2.1. The questions below are kept as they were asked and say macOS 26, and KosmicKrisp's own requirement is macOS 26 or later. A fix that changes measured code after the upgrade is measured on macOS 27.2.1, and its file says so (section 9) |
| 6, CI budget | Put at the re-base, as its question 4 | Decision 5.15 |

The questions follow as the record of what was asked: they say macOS 26, and the third quotes the
wake at 31.6 µs, which the file at `0ec2d90` has since put at 31.0. The first block is as this plan
put it, each question asked with "Milestone 2 plan, question N of 5." before it. The second is the
coordinator's second call, as asked: the Xcode question put again after answer 1 (as first written
it offered Xcode 16.2, the newest for macOS 14), and the added question. The third is the question
the coordinator added after revision 4.

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
{"header":"Xcode on M1","question":"Milestone 2 plan, question 5 of 5. The Mac has the command line tools only. The cooker's Metal shader source can be compiled and run there through Metal at run time, but building a metallib needs Apple's `metal` compiler from Xcode; the Slang compiler fails without it. The macOS CI runner has it, but no branch reaches CI before its merge. Since the Mac is moving to macOS 26 (your answer to question 1), it would be the Xcode current for that system, not 16.2. Do you install Xcode on the Mac?","multiSelect":false,"options":[
{"label":"Install Xcode (Recommended)","description":"Installed after the macOS upgrade. Lanes and reviewers then build and load metallibs before a merge, so a wrong language version or compiler flag is found on the branch, and the native Metal spike can test a binary archive built from a precompiled library. It takes several gigabytes of disk. The runner's Xcode stays the one whose metallibs count."},
{"label":"Runner only","description":"Nothing to install. A metallib is first built in CI after the merge that adds it, so a failure there is found on main and fixed by a follow-up, and GPU pushes wait meanwhile. The native Metal spike compiles its shader source at run time and reports whether a binary archive works that way."}]},
{"header":"macOS 26 when","question":"The Mac goes to macOS 26 for the KosmicKrisp spike (your answer to question 1). Every M1 result file so far was taken on macOS 14.7, so the M1 benchmark and `cargo xtask measure --all` run again in a quiet window after the upgrade. Milestone 1's remaining measurement sessions (A and B) are on the x86 desktop. When do you upgrade?","multiSelect":false,"options":[
{"label":"At milestone 1's exit (Recommended)","description":"After milestone 1's exit is recorded and before milestone 2's first lane. Milestone 1 closes on the OS its figures were taken on, and every milestone 2 figure on the M1 is on macOS 26. The quiet-window re-run comes straight after the upgrade."},
{"label":"Now","description":"Before milestone 1 exits. Any M1 figure milestone 1 still needs is taken on macOS 26, and the earlier M1 files are re-run now. A lane building on the Mac while it upgrades stops and is resumed."},
{"label":"When KosmicKrisp is due","description":"Milestone 2's early lanes run on macOS 14.7, and the upgrade comes once `rhi-vulkan` exists and the KosmicKrisp spike is due. Milestone 2's M1 figures then span two OS versions."}]}
]}
```

```json
{"questions":[
{"header":"Re-run miss","question":"After the Mac moves to macOS 26 at milestone 1's exit, the M1 benchmark and `cargo xtask measure --all` are re-run in a quiet window. Two milestone 1 figures have thin margins on the M1: the prediction slot at 90.0 µs against its 100 µs budget, and the wake from an idle pool at 31.6 µs against 35 µs. If the re-run on macOS 26 puts one of them over its budget, what happens?","multiSelect":false,"options":[
{"label":"A finding; M1 stays closed (Recommended)","description":"Milestone 1's exit stands on the macOS 14.7 figures it was proven on. The crossing is recorded in the re-run's result file and in a verification row gated at milestone 2, and the planner brings you a remedy (a fix lane or a restated budget). Milestone 2's lanes go ahead meanwhile."},
{"label":"Reopen the criterion","description":"The criterion is re-proven on macOS 26 before milestone 2's first lane: a fix lane runs first and milestone 2 waits for it. Milestone 1's exit is restated as met on macOS 26."}]}
]}
```

Question 6 is asked only if the owner answers the re-base's question 4 with "Ask me at the trigger",
and then when a check job's median over its three most recent warm runs of main passes ten minutes
(decision 5.15). The coordinator writes the job or jobs, the medians and the spread where the angle
brackets stand.

```json
{"questions":[
{"header":"CI budget","question":"The <job or jobs> check job takes <median> minutes of its 15, the median of its three most recent warm runs of main, which spread from <low> to <high>, past the workflow's own signal of 10. When each package's tests became one binary (D150) the Windows job took 7:20 to 9:38 over six runs, against 9:30 to 11:39 before. What is done before the next lane that adds a crate is pushed?","multiSelect":false,"options":[
{"label":"Split the check jobs (Recommended)","description":"Each check job becomes two, by package group, with a check in `cargo xtask` that every package is in exactly one group, so a new crate cannot go untested. One small lane, five runs. Each run then uses twice the runners, and each job has room for the milestone's crates."},
{"label":"Raise the budget to 20 minutes","description":"No lane. Architecture section 20's fifteen minutes is restated, every job's timeout follows, and a cold run after a cache-key change takes longer still."}]}
]}
```

The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); the desktop's first sitting, once the M1's re-run is on main, with `LocalDumps`
set on the desktop; the laptop's checkouts refreshed once (D148) and `LocalDumps` set on the laptop,
before its first milestone 2 run; session C at the measurement commit.

## 6. Text for the coordinator (exact)

Every OLD below occurs exactly once in its file at `a561b36`, and an OLD or a NEW may span lines.
Revision 3.4's entry is D152, and the plan's three entries are D153 to D155; if another decision is
applied first, each moves up by one, here and wherever sections 1 to 9 cite them. D150's addendum
cites this plan's question 6 and decision 5.15, and both keep their numbers and their rule, so that
addendum needs no change. The text is applied in three steps.

- *Before `present-api` and `sketch` are briefed:* revision 3.4, that is the architecture
  amendments, `CLAUDE.md`'s two lines, the review history's entries and D152, with milestone 1's
  exit record. `CLAUDE.md` is the owner's file: its two lines are proposed here and applied only on
  the owner's approval of this plan. The marker is one unit. The amendment to the architecture's
  status, the one to section 8.1's "At revision 3.3" and `CLAUDE.md`'s marker line are applied
  together or not at all, so the document never names a revision the marker does not. If the owner
  declines `CLAUDE.md`'s lines, those three are left out, D152 loses "architecture revision 3.4"
  from its title and its bullet on the marker, the review history's entry opens "Amendments after
  milestone 1's exit (D152)", the exit record says "went to D152" where it says "went to
  architecture revision 3.4 (D152)", and every other amendment applies under revision 3.3.
- *At the owner's approval:* D153 to D155, the verification rows and the other text.
- *As the owner answers the re-base's questions:* D151's addendum and the two amendments marked as
  following it, for the first option of questions 1 and 2; lane `ci-split`, for the first option of
  question 4.

### Architecture revision 3.4

Nine amendments to `docs/architecture.md`, two lines of `CLAUDE.md` and two entries of the decisions
log's review history. The marker moves because `CLAUDE.md` gains a rule that CI enforces, and a copy
of that file without the rule must be detectable as stale. Sections 3, 8.1 and 8.3 state the rule as
`cargo xtask boundary` enforces it at `a561b36`, pass systems included and a hand-written `impl
Reflect` reported, with what the tool does not see and review holds. The sentence on recorded tests
is the follow-up project review's X2, stated of systems: the review's own words, "a test that steps
a world records through the session", would be contradicted by some thirty test files that step a
world to assert one behaviour and record nothing. The review of this re-base found the first text
short in two places, and this text answers it: a system that writes hashed state is declared in a
library a recording test links, and what a world registers is held to all of section 8.2, not to its
lints alone.

**The status.** `docs/architecture.md`, line 7 at `a561b36`. OLD:

~~~
Status: design baseline, revision 3.3, after three adversarial reviews and an editorial pass;
revision 3.3 settles what the whole-project review of milestone 1 found against this document
and writes in the rulings of `docs/plans/milestone-1.md`: how milestone 1's criteria are read,
what moves to later milestones, and the workflow in force. Every section is a decision,
~~~

NEW:

~~~
Status: design baseline, revision 3.4, after three adversarial reviews and an editorial pass;
revision 3.3 settles what the whole-project review of milestone 1 found against this document
and writes in the rulings of `docs/plans/milestone-1.md`: how milestone 1's criteria are read,
what moves to later milestones, and the workflow in force; revision 3.4 follows milestone 1's
exit and states where what a world registers may be declared (D152). Every section is a decision,
~~~

**Section 3, the boundary principle.** `docs/architecture.md`, line 89 at `a561b36`. OLD:

~~~
**The determinism boundary is a crate boundary.** A crate is inside or outside. Crates that
would straddle it are split: `net` (outside) and `predict` (inside), `game-sim` and
`game-present`, `nav` and `nav-query`. Section 8.1 lists the inside crates.
~~~

NEW:

~~~
**The determinism boundary is a crate boundary.** A crate is inside or outside. Crates that
would straddle it are split: `net` (outside) and `predict` (inside), `game-sim` and
`game-present`, `nav` and `nav-query`. Section 8.1 lists the inside crates. What a world
registers, stores and runs is held to section 8.2 wherever it is declared: a crate outside that
must declare some of it carries the boundary lint file, and a system that writes hashed state
is declared in a library, never in a binary (sections 8.1 and 8.3, D152).
~~~

**Section 8.1, the list's revision.** `docs/architecture.md`, line 546 at `a561b36`. OLD:

~~~
Inside: the `[boundary]` list in `crate-tiers.toml`, which is authoritative. At revision 3.3
~~~

NEW:

~~~
Inside: the `[boundary]` list in `crate-tiers.toml`, which is authoritative. At revision 3.4
~~~

**Section 8.1, what a world registers.** `docs/architecture.md`, line 554 at `a561b36`. OLD:

~~~
Outside: everything else, including `net` (transport, RTT, interpolation buffer),
presentation, pose evaluation, VFX, audio, UI, asset IO timing and input sampling.
~~~

NEW:

~~~
Outside: everything else, including `net` (transport, RTT, interpolation buffer),
presentation, pose evaluation, VFX, audio, UI, asset IO timing and input sampling.

What a world registers, stores and runs is held to every rule of section 8.2, those that review
holds included, whichever side its crate is on. That is every `#[system]`, a pass system
included, every `#[component]` or `#[resource]` that is not `derived`, every implementation of
`SessionApplier`, `InputApplier` or `StagedState`, every hand-written `impl Reflect`, and the
setup and migration functions of a `CrateRegistration`. Each is declared in a boundary crate,
in a crate outside it that carries the boundary lint file (section 8.3), or in test code alone.
Carrying the file brings no crate inside. The dependency allowlist does not bind a carrier, so
review holds what its declared code calls; its other code is outside, and where that code must
reach the outside world it takes a local exception with its reason. A pass system runs once a
frame with the whole world, whether or not a tick ran, so it leaves the world hash as it found
it: it writes no hashed column and nothing a tick's systems read. No tool checks that; the lane
that adds a pass system proves it with a recorded test (D152).
~~~

**Section 8.3, the carriers and the check.** `docs/architecture.md`, line 725 at `a561b36`. OLD:

~~~
behind a path source. The two proc-macro crates carry the boundary `clippy.toml` as well, since the
code they emit is compiled inside boundary crates, and `lactose-app-headless` carries it for its
sketch game; `[lint_file_carriers]` in `tools/xtask/ci-policy.toml` lists the three.
~~~

NEW:

~~~
behind a path source. The two proc-macro crates carry the boundary `clippy.toml` as well, since the
code they emit is compiled inside boundary crates. So does each crate outside the boundary that
declares what a world registers (section 8.1): `lactose-present-api` for the presentation
components and extract's pass system, and `lactose-app-headless` for its sketch game until that
moves into `game-sim` at milestone 2, after which no binary is a carrier (D152).
`[lint_file_carriers]` in `tools/xtask/ci-policy.toml` lists the carriers, and a crate joins it in
the change that adds its first such declaration. `cargo xtask boundary` reports, in the shipped
sources of any other crate outside the boundary, a `#[system]`, a `#[component]` or `#[resource]`
that is not `derived`, an `impl` of `SessionApplier`, `InputApplier` or `StagedState`, and a
hand-written `impl Reflect`, which declares a component or a resource that no attribute shows.
Shipped sources are the crate's module tree without what only tests, benches and examples compile:
its `cfg(test)` items, `tests/`, `benches/`, `examples/`, the build script, and items behind a
feature that `[dev_only_features]` lists. Attributes and traits are matched by the last segment of
their path as written, so one brought in under another name, written by a macro the crate's source
does not spell out, or inside a `cfg_attr`, is not seen. `Component` and `Resource` implementations
are not matched, since the renderer's GPU traits share the names. A setup or a migration function
is not matched, nor is a tick-stage system told from a pass system. Review holds what the check
does not see.
~~~

**Section 8.3, recorded tests.** `docs/architecture.md`, line 731 at `a561b36`. OLD:

~~~
`cargo xtask sour run` holds the tests that record hashes to the list beside it.
~~~

NEW:

~~~
`cargo xtask sour run` holds the tests that record hashes to the list beside it.
A test runs under the modes above only when it takes its configuration from the harness session and
records its hashes through it, whichever package holds it. So every system that writes a hashed
column is declared in a library, and runs on rows in at least one test that links that library and
records through the session; that test's package is listed under `[harness]`. A copy of a system in
test code is not that system, and a test can run a binary's system only as a copy. A test that
steps a world and records nothing runs once, under one configuration, and proves no determinism. No
tool reports a system that no recorded test runs: the review of the lane that adds the system does
(D152).
~~~

**Section 22, milestone 0's row.** `docs/architecture.md`, line 1393 at `a561b36`. The row is one
line; only this part of it changes. OLD:

~~~
with and without two background zstd threads, on a developer M-series Mac and an x86 desktop
~~~

NEW:

~~~
with and without two background zstd tasks, on a developer M-series Mac and an x86 desktop
~~~

**Section 22, the exit and milestone 0's load.** `docs/architecture.md`, line 1400 at `a561b36`.
OLD:

~~~
Milestone 1's criteria are read as follows (D139). Each timing criterion has its own
~~~

NEW:

~~~
Milestone 1 exited on 16 November 2026, with milestone 0's carried criteria, on the Apple M1's
files under macOS 14.7.4 (`docs/plans/milestone-1.md`, section 7); the x86 half of both is read
on the desktop in milestone 2 (D151). Milestone 0's two zstd loads are tasks of the background
class, as the engine runs background CPU work (section 5.1). On the M1 they run on the
efficiency cores and barely reach the frame workers. On a hybrid part under Windows the
background class sets thread priority alone, and steering there is an open verification item.
On a part with no efficiency cores they are two low-priority threads beside the frame workers,
the harder case. The desktop's first file says which part it is (D152).

Milestone 1's criteria are read as follows (D139). Each timing criterion has its own
~~~

**Section 22, the tick is not a frame.** `docs/architecture.md`, line 1405 at `a561b36`. OLD:

~~~
workers do not lengthen it, and milestone 1's is the M1's on four (D151). The tick excludes
the physics phase, which is measured on its own scene, and the world hash, which SOUR pays at
every step and a shipped binary only when it asks. The prediction-world snapshot is the world
~~~

NEW:

~~~
workers do not lengthen it, and milestone 1's is the M1's on four (D151). The tick excludes the
physics phase, which is measured on its own scene, and the world hash, which SOUR pays at every
step and a shipped binary only when it asks. The composed tick, the same scene with the 2,000
bodies beside it, is read by no criterion: it takes 5,772.5 µs on the M1's four frame workers
and projects to 4,433.1 µs on eight (`docs/results/composed-tick-apple-m1-2026-11-15.md`). The
eight-core figure the desktop gives for the tick is therefore of the tick alone, and no budget
for a frame that steps physics (D152). The prediction-world snapshot is the world
~~~

**The marker.** `CLAUDE.md`, line 3 at `a561b36`. OLD:

~~~
Revision marker: architecture 3.3. At task start run `git show HEAD:CLAUDE.md | head -3` and
~~~

NEW:

~~~
Revision marker: architecture 3.4. At task start run `git show HEAD:CLAUDE.md | head -3` and
~~~

**The rule.** `CLAUDE.md`, line 54 at `a561b36`. OLD:

~~~
- Systems use the `#[system]` macro of architecture section 6.1 and declare the worlds they run in.
~~~

NEW:

~~~
- Systems use the `#[system]` macro of architecture section 6.1 and declare the worlds they run in.
- What a world registers is held to section 8.2's rules wherever it is declared, and is declared in
  a `[boundary]` crate or a crate of `[lint_file_carriers]` in `tools/xtask/ci-policy.toml`, or in
  tests alone: a `#[system]`, a `#[component]` or `#[resource]` that is not `derived`, an applier, a
  hand-written `impl Reflect`, a registration's setup and migration functions. A system that writes
  hashed state is declared in a library and runs in a test that links it and records under SOUR
  (architecture sections 8.1 and 8.3).
~~~

**The review history.** `docs/decisions-log.md`, line 6535 at `a561b36`. OLD:

~~~
- **Revision 3.3** (the commit that carries D140) settled what that review found against the
  architecture and wrote in the plan's rulings: the server links no RHI backend, and
  `rhi-null` moves to milestone 2; facade first with each backend a lane of its own replaces
  vertical slices; a prediction world's region modes; where the NaN assertion fires; how
  milestone 1's criteria are read; and copy-on-write snapshots are not built. Its review
  (not checked in) found nothing blocking, 4 serious and 9 minor findings, all answered
  before the commit.
~~~

NEW:

~~~
- **Revision 3.3** (the commit that carries D140) settled what that review found against the
  architecture and wrote in the plan's rulings: the server links no RHI backend, and
  `rhi-null` moves to milestone 2; facade first with each backend a lane of its own replaces
  vertical slices; a prediction world's region modes; where the NaN assertion fires; how
  milestone 1's criteria are read; and copy-on-write snapshots are not built. Its review
  (not checked in) found nothing blocking, 4 serious and 9 minor findings, all answered
  before the commit.
- **Follow-up project review** at `4ffcd62` (16 November 2026, not checked in), milestone 1's exit
  step 7: every finding minor or a note once verified. Lanes `exit-docs` and `exit-tooling` fixed
  what was a lane's, and D152 and `docs/plans/milestone-2.md` took the rest.
- **Revision 3.4** (the commit that carries D152) followed milestone 1's exit: what a world
  registers is held to section 8.2 wherever it is declared, in a boundary crate or a crate that
  carries the boundary lint file; a system that writes hashed state is declared in a library and
  runs in a recorded test; and section 22 records the exit and two readings of its criteria. Its two
  reviews (not checked in) found nothing blocking, 2 serious and 12 minor findings, all answered in
  the text applied.
~~~

### Decision entry: revision 3.4

```
**D152. What a world registers is held to section 8.2 wherever it is declared, in a boundary crate or a crate that carries the lint file; a system that writes hashed state is declared in a library and runs in a recorded test; architecture revision 3.4** (§3, §8.1, §8.3, §22, `CLAUDE.md`, `tools/xtask`, `tools/xtask/ci-policy.toml`, `game-sim`, `lactose-app-headless`, `lactose-present-api`, `lactose-presentation`, `lactose-renderer`, `lactose-app-client`, `docs/plans/milestone-1.md`, `docs/plans/milestone-2.md`)
- Why: section 3 makes the determinism boundary a crate boundary, and nothing tied what a world
  registers to the boundary's crates. `lactose-app-headless`'s sketch game declared a persistent
  replicated component, a tick-stage system and a session applier outside every boundary lint; the
  follow-up project review patched the system to read the clock, every check passed, and three runs
  printed three hashes (X1). Lane `exit-tooling` built the check (`87c2c77`, `7094406`, merged at
  `a561b36`). This entry is the rule that check enforces, what review holds beyond it, and what
  milestone 2's crates do about it.
  - *What a world registers is held to every rule of section 8.2, those that review holds included,
    whichever side its crate is on.* That is a `#[system]`, a pass system included, a `#[component]`
    or `#[resource]` that is not `derived`, an implementation of `SessionApplier`, `InputApplier` or
    `StagedState`, a hand-written `impl Reflect`, and the setup and migration functions of a
    `CrateRegistration`. Each is declared in a `[boundary]` crate, in a crate listed under
    `[lint_file_carriers]`, which carries a byte-identical copy of the boundary `clippy.toml`, or in
    test code alone. A lint file is not section 8.2: float `%`, global mutable state and the RNG's
    keying are held by review, and the review of this rule's first text put a float `%` into the
    carrier's system and passed every gate. Rules out a carrier that keeps the lints and breaks the
    rules, and "a pass system of an outside crate writes no hashed column" as the bar, which the
    project review proposed and no tool can check of an exclusive system.
  - *What the tool enforces, and what it does not see.* `cargo xtask boundary` reports the first
    four kinds in the shipped sources of a crate that is neither a boundary crate nor a carrier. It
    matches attributes and traits by the last segment of their path as written, so one brought in
    under another name, written by a macro the crate's source does not spell out, or inside a
    `cfg_attr`, is not seen. It does not see a setup or a migration function, nor which stage a
    system is registered in. Review holds those, and a verification row carries the extension that
    would lift the first.
  - *A carrier stays outside the boundary.* The dependency allowlist does not bind it, so review
    holds what its declared code calls. Its other code is outside, and where that code must reach
    the outside world it takes a local `#[expect]` with its reason. Rules out moving a binary or a
    foundation crate into `[boundary]` to declare a component.
  - *A system that writes a hashed column is declared in a library, and runs on rows in a test that
    links that library and records through the session (X2).* A test comes under SOUR's modes only
    by taking its configuration from the harness session and recording through it, whichever package
    holds it. A copy of a system in test code is not that system, and a test can run a binary's
    system only as a copy: that is how the patched sketch passed. The project review asked for "a
    test that steps a world records through the session"; some thirty test files at `a561b36`
    step a world and record nothing, each asserting one behaviour, so the rule is stated of systems,
    as the review's X4 applied it to the bone systems. No tool reports a system that no recorded
    test runs: review does. Rules out a package-level check, which would need an exemption list and
    add no mode.
  - *The sketch game moves into `game-sim`, and no binary is a carrier.* The rule is false of one
    system at `a561b36`: `sketch::count` is declared in the headless binary, and
    `lactose_app::headless::a_recorded_headless_run` runs a copy. The milestone 2 plan's lane
    `sketch` moves the sketch's component, system and session applier into `game-sim`, a
    `[boundary]` crate that `crate-tiers.toml` already names, which both binaries and the recorded
    tests link. The mover of the client's local world is written there too. The binaries keep the
    `CrateRegistration` and `Game` values, whose types a game crate may not name and which the rule
    does not list. The headless binary's migration function is empty, so by choice it passes
    `migrations: None` and leaves the carriers in that lane, where it could have stayed one for a
    function that does nothing; the client binary is never one. The sketch's session applier runs in
    no recorded test after the move, since the test spawns through an applier of its own: the
    sentence above is of systems, and a verification row carries the applier's recorded run. Rules
    out a library target in each binary's package, each a carrier and a `[harness]` package, and the
    boundary's bans on a client's window code.
  - *Milestone 2's other crates.* `lactose-present-api` declares the presentation components and
    extract's pass system, and joins the carriers with its facade: the merged check refused that
    facade as the milestone 2 plan first wrote it (the review of `exit-tooling`, S1).
    `lactose-presentation` does not join: section 8.1 puts presentation outside, its schedule runs
    on the snapshot, and the boundary file bans the float methods it is written with.
    `lactose-renderer` declares no system: extract's sink hands its slot table the structural log's
    despawns and a restore's reconciliation. Rules out the boundary's float bans on presentation and
    render code.
  - *A pass system leaves the world hash as it found it.* It runs once a frame with the whole world,
    whether or not a tick ran, so it writes no hashed column and nothing a tick's systems read. No
    tool checks that; the lane that adds a pass system proves it with a recorded test, extract's
    first. Rules out a hash that depends on how many frames were drawn.
  - *Section 22 says what milestone 0's zstd load is, and that the eight-core tick is no frame
    budget (the exit review's N4 and N6).* The two loads are tasks of the background class. On the
    M1 they run on the efficiency cores; on a hybrid part under Windows that steering is an open
    verification item; on a part with no efficiency cores they are two low-priority threads beside
    the frame workers, the harder case, and the desktop's first file says which part it is. The
    composed tick with the physics phase takes 5,772.5 µs on the M1's four frame workers and
    projects to 4,433.1 µs on eight, and no criterion reads it.
  - *Milestone 1's exit is recorded* in `docs/plans/milestone-1.md`, section 7, and in one sentence
    of section 22: its steps 1 to 7 were complete at `a561b36` on 16 November 2026 UTC, on the Apple
    M1 under macOS 14.7.4.
  - *The marker moves to architecture 3.4*, since `CLAUDE.md` gains the rule. The milestone 2 plan's
    own rulings are revision 3.5's.
- Driven by: the follow-up project review at `4ffcd62` (X1, X2); the review of lane `exit-tooling`
  (S1), whose probe showed the merged check refusing the milestone 2 plan's `present-api` facade;
  the exit's criteria review at `8941997` (N4, N6); the review of the milestone 2 plan's re-base,
  whose probes showed that a test cannot link a binary's system and that a float `%` in a carrier
  passes every gate; `docs/plans/milestone-2.md`, decision 5.18.
- Replaced: section 8.3's "`[lint_file_carriers]` in `tools/xtask/ci-policy.toml` lists the three";
  section 22's "two background zstd threads", and its placing of `game-sim` wholly in milestone 5;
  the first site of milestone 1's edit-to-playing measurement (`docs/plans/milestone-1.md`, decision
  5.9), which follows the sketch's system into `game-sim`.
```

### Milestone 1's exit, recorded

No document on main says that milestone 1 has exited. The record goes where the exit's steps are,
`docs/plans/milestone-1.md`, section 7, with the plan's status line, and revision 3.4's sentence in
section 22 points to it. The date is the merge commit's in UTC, as result files are dated; at the
developer's clock, ten hours ahead, it is 17 November. The third amendment is the follow-up project
review's M1: no result file compares hashes.

**The status line.** `docs/plans/milestone-1.md`, line 3 at `a561b36`. OLD:

~~~
Status: approved by the owner on 14 November 2026 (D139). Written at main `3264406`, architecture revision 3.2.
~~~

NEW:

~~~
Status: approved by the owner on 14 November 2026 (D139); exited on 16 November 2026 (section 7).
Written at main `3264406`, architecture revision 3.2.
~~~

**Section 7, the record.** `docs/plans/milestone-1.md`, line 818 at `a561b36`. OLD:

~~~
8. The planner writes the milestone 2 plan.
~~~

NEW:

~~~
8. The planner writes the milestone 2 plan.

**The exit, as it happened.** Steps 1 to 7 were complete at main `a561b36` on 16 November 2026 UTC.
Step 1: the quiet windows at `bef3455` and `c50b688`, with the `whisk` file re-taken at `0ec2d90`
and the `spike-net` file at `8941997`. Step 2: D146. Step 3: every queue drained, `lactose-rhi-api`
and `lactose-rhi-null` having moved to milestone 2 (D139, D140). Step 4: every row gated at 1 is
Done. Step 5: the criteria review read `8941997` and found no criterion not met. Criteria 2, 3 and 8
are met; 5 and 7 are met as restated (D143, D139); 1 is met as restated by D139 and D151, and 9 by
decision 5.9, D149 and D151; 4 and 6 are met on the M1, their x86 half moved by D151. Of milestone
0's carried criteria, the makespan, the wake and the steal are met on the M1, and tick start as
restated at 35 µs (D138); Windows' tick start, held to 1 ms (D144), is the desktop's. So step 6 had
nothing to rule on. Step 7: the follow-up project review read `4ffcd62`. Its findings and the
criteria review's were fixed by lanes `exit-docs` (`d7f3e3b`) and `exit-tooling` (`a561b36`), or
went to architecture revision 3.4 (D152) and to the milestone 2 plan. Step 8 is
`docs/plans/milestone-2.md`, re-based on `a561b36`. Every M1 file the exit rests on was taken under
macOS 14.7.4, and both reviews ran there; the Mac moves to macOS 27.2.1 after it. What the exit
leaves open is the x86 half: the verification row "The x86 half of milestones 0 and 1 (D151)".
~~~

**Section 4, session B's hashes.** `docs/plans/milestone-1.md`, line 276 at `a561b36`. OLD:

~~~
- *Session B*, the owner's, at the same commit. On the x86 desktop, on each operating system it
  boots: `cargo xtask measure --all`, and the spike's recording tests, whose hashes the result
  file compares with the nightly's. The owner commits the files it writes, or sends them back. Its
  run of the 100k tick on eight frame workers is criterion 1's proof.
~~~

NEW:

~~~
- *Session B*, the owner's, at the same commit. On the x86 desktop, on each operating system it
  boots: `cargo xtask measure --all`, and the recording tests' hashes, written by
  `cargo xtask sour run --mode architecture --emit` and compared with a reference file by
  `cargo xtask sour compare`. No result file compares hashes; `docs/plans/milestone-2.md`,
  decision 5.19, names the reference. The owner commits the files it writes, or sends them
  back. Its run of the 100k tick on eight frame workers is criterion 1's proof.
~~~

### Decision entry: the plan's rulings

```
**D153. The milestone 2 plan: how its criteria are read, which machine proves each, the order the render track is built in, and what is re-gated** (§4, §5.1, §5.3, §12.1, §12.2, §12.3, §12.4, §13, §14, §16, §18, §19, §20, §22, `crate-tiers.toml`, `tools/xtask`, `docs/plans/milestone-2.md`)
- Why: section 22 gives milestone 2 seven proofs and names no workload, machine or order for
  them; none of the crates it delivers existed at `a561b36`; twenty-six verification rows gated
  the milestone. `docs/plans/milestone-2.md` is the map and gives each decision's reasons; these are
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
  - *A crate added from milestone 2 on follows D150, and no check job's median over its three most
    recent warm runs of main passes ten of its fifteen minutes.* The Windows check's stood at 9:13
    at the plan's re-base. Rules out crate lanes pushed onto a job at its timeout, a budget read
    from one run, and runs made again to read it.
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
  ruling of that day that the plan be drafted ahead of milestone 1's exit; the owner's
  answers of 16 and 17 November 2026; and the plan's re-base at `a561b36`, which took D150,
  D151 and D152.
- Replaced: D139's "`lactose-rhi-api`'s facade and `lactose-rhi-null` move to milestone 2,
  behind its MoltenVK, KosmicKrisp and native Metal spikes", in its order alone. D136's and
  D147's placing of the directory watch in milestone 2. Section 20's "on a GPU runner nightly".
  Section 22's placing of the scatter and the cell-plus-offset format wholly in milestone 3,
  and of the client `app` wholly in milestone 4: the client binary arrives at milestone 2 with
  a local world. The status line of `docs/proposals/artist-surfaces.md`. Revision 3.5
  restates each. Section 22's placing of `game-sim` is D152's to replace.
```

### Decision entry: the Apple interim path

```
**D154. The Apple interim path until native Metal is MoltenVK; KosmicKrisp is measured beside it on the development Mac, which moves to macOS 27.2.1** (§2 Graphics, §5.1, §12.1, §20, §22, §22.1, `lactose-rhi-vulkan`, `docs/results`)
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
    so every such command is the present thread's, on a queue of its own (D153).
  - *KosmicKrisp is measured, not adopted.* It is built on Metal 4, which needs macOS 26 or later
    (Mesa's driver documentation); on macOS 14.7.4 its driver in Vulkan SDK 1.4.363.1 enumerated no
    device. The owner moves the development Mac to macOS 27.2.1 on 17 November 2026, after milestone
    1's criteria were proven and reviewed on macOS 14.7.4 and before milestone 2's first lane, so
    milestone 1 closes on the system its figures were taken on and every milestone 2 figure on the
    M1 is of one system. `rhi-vulkan`'s conformance suite, the swapchain's cases and `cargo xtask
    measure gpu-cost` then run under KosmicKrisp beside MoltenVK, on one machine, with timings.
    Section 22's "KosmicKrisp ... recorded" is those files. A case that fails under it is recorded
    and fails no merge.
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
    owner; milestone 1 is not reopened. A fix that changes measured code after the upgrade is
    measured on macOS 27.2.1, and its file says so. After it no machine in reach runs macOS 14.
  - Rules out: a gate under a driver nothing has run on; a second gate driver at every merge; a
    KosmicKrisp record taken on a hosted runner's virtual GPU, without timings.
- Driven by: architecture section 22.1; the RHI spike; the plan's probe of 16 November 2026 and
  the GPU review's reading of MoltenVK 1.4.2's source; the owner's answers: on 16 November 2026,
  that the Mac is upgraded at milestone 1's exit; on 17 November, that its system is macOS 27.2.1.
- Replaced: sections 2 and 12.1, which leave the interim path open between the two; revision
  3.5 names it.
```

### Decision entry: crash reporting

```
**D155. Crash reporting until a build leaves the owner's machines is the panic hook's text report, shared by every engine binary through `lactose-app`; minidumps and symbol upload are decided at milestone 5** (§3, §5.1, §19, §21, §22.1, `lactose-app`, `lactose-app-headless`, `lactose-app-client`)
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
  moves to milestone 5; sections 3 and 5.1's "crash dump", which revision 3.5 states as
  the text report; the row's and the headless README's "`lactose-platform` ... whose hook
  replaces this one".
```

### Addendum to D151, for the first option of questions 1 and 2

Appended to D151 in `docs/decisions-log.md`, after its `- Replaced:` bullet, when the owner has
answered. With "One sitting, at the end" its second and third sentences give the measurement commit
and one sitting, and its title loses its second half. With "Each at its own commit" there is no
addendum.

```
- Addendum (the milestone 2 plan's re-base, 2026-11-17): *the desktop reads one commit at a sitting,
  and milestone 1's half is read first.* The owner's answers at the re-base. Sessions A and B run in
  a sitting of their own, as soon as the M1's re-run after its upgrade is on main and before
  milestone 2's first crate lane merges. They run at that re-run's commit, not at the three commits
  milestone 1's plan and D144 gave them: `cargo xtask measure --all` writes every file of both, the
  WHISK file among them, and any commit from `0ec2d90` on holds `timer-windows` and `keeper`. The
  desktop's recorded hashes are an emitted file, compared with the linux-x64 file of the nightly at
  that commit by `cargo xtask sour compare`; no digest is used. Session C stays at milestone 2's
  measurement commit, where the same command writes milestone 1's files again. A miss is brought to
  the owner when the first sitting's files are in. Rules out three old commits built under each
  system the desktop boots, and an x86 miss of milestone 1 found when milestone 2 is over. Replaced:
  this entry's "milestone 2's one desktop sitting, each at the commit milestone 1's plan gives it",
  and D144's commit for the desktop's Windows `whisk` file.
```

**Following the addendum: the milestone 1 plan, section 4.** `docs/plans/milestone-1.md`, line 282
at `a561b36`. OLD:

~~~
16 November 2026. Both run in milestone 2's one desktop sitting, each at the commit given above.
~~~

NEW:

~~~
16 November 2026. Both run in milestone 2, in the desktop's first sitting, at that sitting's one
commit (D151's addendum; `docs/plans/milestone-2.md`, decision 5.19).
~~~

### Lane `ci-split`, for the first option of question 4

A small lane, five runs, first in `tools/xtask/src`'s order and pushed alone as a workflow change.
Each check job of `.github/workflows/ci.yml` becomes two, by package group. The groups are a table
of `tools/xtask/ci-policy.toml`, and `cargo xtask boundary` reports a workspace package that is in
no group or in two, so a new crate cannot go untested. The lane records each job's cache size, since
the repository's cache is over GitHub's 10 GB, and says which jobs save one. Its proof: both halves
green on the three platforms; each half's time against ten minutes, in the workflow's header; a
fixture package in no group fails the check. D153's bullet on the budget then gains "each check job
is two, by package group". With "Split at the trigger" the same lane runs when a median passes ten.
With "Raise the budget" there is no lane, and revision 3.5 restates section 20.

### Verification rows

Each change is anchored on the row's opening words, as `docs/plans/milestone-1.md` gave its own. An
appended text begins with its space. Rows 64, 77, 80 and 81 need no change: D151 wrote them. Row 170
is open as the merge wrote it. Row 169's change follows D151's addendum.

~~~
--- section 1, the row "MoltenVK and KosmicKrisp": append to its last cell ---
 Both drivers run on the M1 once it is on macOS 27.2.1 (D154): KosmicKrisp needs Metal 4, so macOS 26 or later, and on macOS 14.7.4 the SDK's driver enumerated no device. Lanes `vulkan` and `swapchain` record the capability answers under each, from the conformance suite and the swapchain's cases, and result file `gpu-cost` the encode time and the pipeline creation cost, through `rhi-vulkan`

--- section 1, the row "Extract cost": append to its second cell ---
; the workload and machines of D153, result file `extract-50k`

--- section 1, the row "Native Metal": append to its second cell ---
; result file `spike-metal`, and the planner's freeze decision at milestone 2's exit (D153). Its gate cell then records the spike as done in part (`docs/results/spike-metal-apple-m1-2026-11-17.md`), the freeze decision owed

--- section 1, the row "Remote transform upload": append to its second cell ---
; the workload of D153, result file `remote-upload-5k`

--- section 1, the row "Physics phase stall": append to its second cell ---
; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153)

--- section 2, the row "MoltenVK multi-draw-indirect is a CPU loop.": append to its text ---
 Closed by result file `gpu-cost`'s multi-draw figure and a reading of the pinned MoltenVK (D153)

--- section 2, the row "Slang's Metal target supports argument buffers, function constants and framebuffer fetch.": append to its text ---
 Shown first by the native Metal spike's shaders, which are MSL the Slang compiler emitted, then held by a pipeline state made from each cooked shader: on the M1 from the cooked MSL, on the macos-arm64 runner from the metallib (D153). Its gate cell then records the spike's half as done (`docs/results/spike-metal-apple-m1-2026-11-17.md`), the cooked shaders owed

--- section 4, the row "Local x86-64 runs on Apple Silicon": append to its text; its gate stays `| none |` ---
 The Mac moves to macOS 27.2.1 on 17 November 2026 (D154): the re-check is one boundary crate's tests, built for `x86_64-apple-darwin` and run under Rosetta there, in the coordinator's pass after the upgrade

--- section 4, the row "Windows efficiency-core steering": append to its text ---
 Lane `eco-qos` (D153): measured on the laptop, a hybrid part, by the WHISK benchmark's rows with the two background zstd tasks, before and after, and on the desktop if it is one

--- section 4, the row "`lactose-input` is implemented (D120 addendum)": append to its text ---
 The camera rig's check is `lactose-presentation`'s test (D153)

--- section 4, the row "Artist-defined surfaces behind fixed pipelines": append to its text, and replace its gate `| 2 |` with `| 3 |` ---
 Re-gated to milestone 3 (D153): decided with deferred lighting on `rhi-metal`, where a G-buffer pass exists; milestone 2's cooker proves static specialisation and a link-time constant for both targets

--- section 4, the row "`lactose-predict` is implemented (D122 addendum)": append to its text ---
 The presentation check is `lactose-presentation`'s test (D153); the visual blend of a correction is milestone 4's

--- section 4, the row "Interpolation slot footprint": append to its text ---
 The loom model is lane `interp-loom`; the reader's retry rate under a 30 Hz writer and the footprint are in result file `remote-upload-5k` (D153)

--- section 4, the row "The line-ending rule on a Windows host (D148)": append to its text ---
 Its CI half is met: run 43795852109 at `8869d2c` read 889 files and refused none on the three hosts, and the Windows cache key's last hash equals the others'. The owner's refresh of the laptop's checkouts remains

--- section 4, the row "The milestone 1 crash hook": append to its text ---
 The hook moves into `lactose-app` and writes a file too (D155, lane `crash-hook`); a subprocess test holds the abort path; RICOTTA keeps its own hook; minidumps are decided at milestone 5

--- section 4, the row "SILO's load call against section 5.1's 0.5 ms slice (D136)": append to its text, and replace its gate `| 3 (the baseline), 2 (mesh and texture) |` with `| 3 (the baseline, a mesh pool and a texture), 2 (the shader kind) |` ---
 Milestone 2 cooks no mesh pool and no texture: it measures the shader kind's load against the slice (D153)

--- section 4, the row "Mounted packs are never unmounted and published versions never unloaded (D136)": append to its text, and replace its gate `| 2 and 3 |` with `| 3 |` ---
 The dev session waits for the watch loop, at milestone 3 (D153)

--- section 4, the row "Mount cost (D136)": append to its text, and replace its gate `| 2 and 3 |` with `| 3 |` ---
 The Linux and Windows figure is taken with the watch loop's dev session, at milestone 3 (D153)

--- section 4, the row "The cooked pack set of a fresh process (D136, D147)": append to its text, and replace its gate `| 2 |` with `| 3 |` ---
 The directory watch, the row's open half, is milestone 3's (D153)

--- section 4, the row "The RHI spike ran under MoltenVK alone (D139)": append to its text ---
 Lane `vulkan` closes it (D153): the design questions lavapipe can answer, from its run; the four hardware questions probed on the laptop's devices and recorded, a run that shows nothing discharging no rule of the null backend; device-only staging as a case on the M1; question 2 answered under KosmicKrisp on the M1 if that driver offers a lazily allocated type, and open on Vulkan if not

--- section 4, the row "The keeper's timed wait on Windows (D144)": append to its text ---
 The unmeasured list is read by the architect of the client's frame, lane `frame-design` (D153): an item the design uses becomes a measurement it names, and the rest is re-gated with its reason

--- section 4, the row "The x86 half of milestones 0 and 1 (D151)": append to its text, with D151's addendum ---
 By D151's addendum the desktop reads one commit at a sitting. Sessions A and B run first, at the commit of the M1's re-run on macOS 27.2.1, by `cargo xtask measure --all`, `cargo nextest run --locked -p lactose-task` and `cargo xtask sour run --mode architecture --emit`, whose file is compared with the linux-x64 file of the nightly at that commit by `cargo xtask sour compare`. The eight-core tick it gives is of the tick alone and is no frame budget (D152)

--- section 4, add rows ---
| Milestone 2's criteria are proven by result files and CI runs: `triangle`, `spike-metal`, `gpu-cost`, `rhi-conformance`, `extract-50k`, `remote-upload-5k`, and the first green `gpu (lavapipe)` and `shaders` runs (D153) | 2 |
| The development Mac moves to macOS 27.2.1 on 17 November 2026, between milestone 1's exit and milestone 2's first lane (D154). Every M1 result file of milestones 0 and 1 was taken on macOS 14.7.4: `cargo xtask measure --all`, which writes the WHISK file too, is run again in a quiet window straight after the upgrade, at main's head, and read against `whisk-apple-m1-2026-11-16` (`0ec2d90`), `spike-net-apple-m1-2026-11-16` (`8941997`) and the files of 15 November for the rest. The margins are thin for the prediction slot, 90.0 µs against 100, and the wake from an idle pool, 31.0 µs against 35. A figure that crosses a milestone 1 criterion there is a finding for the owner, recorded in the re-run's file and here, and milestone 1 is not reopened (the owner's ruling of 16 November 2026). From the upgrade on, a fix that changes measured code is measured on macOS 27.2.1, and its file says so | 2 |
| The RHI spike ran on macOS 14.7.4, under MoltenVK 1.4.2 of Vulkan SDK 1.4.363.1. Its program is run again on macOS 27.2.1, by its result file's commands, before `rhi-api` is briefed: its ten items and its five misuses read as they did, or the difference goes to the planner. Nothing was run on macOS 27.2.1 for the plan. If that SDK does not run there and is replaced, the reading of MoltenVK's source behind the present thread's queue (D153) is repeated for the new version first | 2 |
| KosmicKrisp on the M1 (D154): that it enumerates the device on macOS 27.2.1, the output cases it passes, whether it offers a lazily allocated type and keeps a memoryless attachment across a local read, whether GPU-assisted validation runs under it, and whether it takes the drawable inside a submit. Recorded at `vulkan`'s and `swapchain`'s merges and in `gpu-cost`. Passing every output case and offering one of those brings a switch of the gate's driver to the owner | 2 |
| Which queue families of the laptop's devices, and of the desktop's from its first sitting, have the graphics bit and can present is `gpu-probe`'s to record. Where only the graphics queue's does, the present thread shares that queue under its lock (D153), and a blocking present there is a finding for the planner | 2 |
| The pull-request check jobs against ten of their fifteen minutes, by the median of each job's three most recent full-match runs of main, read from the runs main makes anyway (D153). At `a561b36` the medians are 9:13 on Windows, 7:28 on macOS and 4:26 on Ubuntu, over runs 43872419194, 43886836183 and 43896909697: the Windows check took 7:20, 9:30 and 9:13 there, and 9:08, 9:04 and 9:38 in the three runs before, the first since D150, whose addendum holds those. When a median passes ten, or sooner if the owner so rules, the check jobs are split by package group or the budget is restated | 2 |
| The repository's Actions cache stood at 10.74 GB at `a561b36`, over GitHub's 10 GB, by the review of lane `exit-tooling`; entries are evicted, and a job whose entry went runs cold. What a cold Windows check takes is not measured. Milestone 2 adds the `gpu (lavapipe)` and `shaders` jobs, and a split of the check jobs one cache for each job it divides. The lane that adds the first of them records each job's cache size and says which jobs save one. The nightly SOUR job's release baseline (X5) is estimated at five to fifteen minutes more a platform, of its sixty | 2 |
| What lavapipe on the linux-x64 runner offers is not known: its Mesa version, the local read, indirect count, a headless surface, GPU-assisted validation. Mesa's source at an older commit gives it one coherent memory type and one queue, so the graph's several-queue path runs on null and on the M1 and never in CI. `gpu-probe` records it before `vulkan` is briefed. A runner image change is a driver change | 2 |
| What the macos-arm64 runner offers is not known: its Xcode and `metal` compiler, a Metal device, and the same on its oldest macOS image. `gpu-probe` records it. A metallib's bytes depend on the Xcode that cooked it | 2 |
| The validation layer is pinned by SDK version and checksum on every host (D153): Ubuntu 24.04's package is 1.3.275, and the swapchain's semaphore checks need 1.4.313 | 2 |
| Under MoltenVK 1.4.2 the drawable is taken in the submit of a command that uses the swapchain image, with the queue held, by a reading of its source. The present thread has a queue of its own and the frame two targets or more (D153). `swapchain` shows that on null, and under MoltenVK only if a minimised window or a sleeping display makes the drawable block for a second: which does, if either, is not known. After MoltenVK's "could not be acquired" the image returns `VK_ERROR_OUT_OF_POOL_MEMORY` until the swapchain is rebuilt, by the same reading; the case shows the rebuild. Section 5.1's present-thread sentence is restated in revision 3.5 | 2 |
| The development Mac gets the Xcode current for macOS 27.2.1 after the upgrade (the owner's answer 5); until then `slangc -target metallib` fails there. Whether that Xcode's `metal` compiler is a component to install apart, and whether it still compiles MSL 3.1 for a macOS 14 deployment target, are not known: the coordinator's re-check after the upgrade records both | 2 |
| SPIR-V and MSL cooked by the pinned Slang are expected equal on linux-x64 and macos-arm64. The `shaders` job is the proof; a difference is a finding for the planner. The heap's two forms and their markers, the outputs' locations and the input attachment's index were found on the SDK's 2026.17-25 build and are run again on the pin | 2 |
| The metallib's language: Slang's own call of `metal` passes `-std=metal3.1`, no deployment target and a changing file name, so the cook calls `metal` itself, as MSL 3.1 for macOS 14, the oldest client it targets. Whether two cooks write equal metallib bytes is recorded. Once the Mac is upgraded nothing in reach runs macOS 14: a load on a hosted runner's oldest image, if it has a Metal device, is the one check left, and that the engine runs on macOS 14 is unshown | 2 |
| A window opened from an agent's shell on the M1, and from the laptop's agent session, is assumed. If either cannot present, that run is the owner's | 2 |
| Windows keeps no local crash dump until `LocalDumps` is set, by an administrator (D155): the owner sets it on the laptop and the desktop, and `crash-hook`'s report records each machine | 2 |
| A pass system leaves the world hash as it found it (D152): no test holds that at `a561b36`. Lane `present-impl` adds it for extract, and criterion 3's recorded test runs it under every mode. The gather's test is owed by the first lane that edits `lactose-net-api`, at milestone 4 at the latest | 2 (extract), 4 (the gather) |
| The headless binary's sketch system, `sketch::count`, is declared in a binary, so the one recorded test of it, `lactose_app::headless::a_recorded_headless_run`, runs a copy in `crates/app/tests/headless.rs`: section 8.3's rule that a system that writes a hashed column runs in a recorded test that links it is false of that system at `a561b36` (D152). Lane `sketch` moves the sketch game's component, system and session applier into `game-sim`, which the binary and the test both link; the test then runs the shipped system, with the binary's 120-tick hash and the recorded stream unchanged, and the binary leaves `[lint_file_carriers]`. The session applier still runs in no recorded test, since the test spawns through an applier of its own | 2 |
| Hardware runs of the device tests are made at merges on the M1 and after pushes on the laptop, since no GPU runner exists (D153). A defect that reaches main for want of one reopens the question | none |
| No machine in reach is known to have host-visible memory that is not coherent, or untracked aliasing; a lazily allocated type on Vulkan is KosmicKrisp's to show on the M1. The null backend's rules are the guard, each held by a mutation (D153). A machine that has one reopens the row | none |
| Device-lost recovery (architecture section 19) is tested when the GPU scene exists. `rhi-api` reports the loss as a value and null injects it from milestone 2 | 3 |
| The device tests of milestone 2 are Vulkan's alone: `lactose-rhi-metal` joins `[test_support]` with its lane, so that fences between aliases and residency get a device test | 3 |
| The prediction world's copy of the local player shadowing the replicated one for extract and the slot table (section 9.4) is tested when `Client` holds both worlds | 4 |
| `cargo xtask boundary`'s check of what a world registers compares names as written (D152): a declaring attribute or an applier trait brought in under another name, written by another crate's macro or derive, or inside a `cfg_attr`, is not seen, and `Component` and `Resource` implementations are not matched. It does not see a `CrateRegistration`'s setup and migration functions, nor which stage a system is registered in, so a migration outside a carrier and a tick-stage system inside one are review's to find. Reporting, outside a boundary crate or a carrier, a `CrateRegistration` literal whose `setup` or `migrations` is not `None`, and a `Migration` literal, would lift the first, and is due before `game-sim` holds the game; a binary's registration with both `None` is not reported. A client-side `impl StagedState` in `lactose-net` will be reported at milestone 4: decide with that crate's facade whether it is a carrier or the implementation sits in `lactose-net-api` | 4 |
| A game crate's registration (D152): `CrateRegistration` and `Game` are `lactose-app`'s and `Migrations` is `lactose-persist`'s, and `crate-tiers.toml` lets a `[game]` crate name neither an assembly nor a backend crate, while `crates/app/src/game.rs` has a game crate expose its `CrateRegistration` as a `pub static`. From milestone 2 the binaries hold the registration values with `setup` and `migrations` both `None`, and no binary is a carrier. Decide where a game crate's registration value and its migration functions are declared before `game-sim` holds the game: a migration function needs a boundary crate or a carrier, and its types are outside the boundary | 5 |
| No tool reports a system that writes a hashed column and runs in no recorded test (D152), and section 8.3's sentence does not reach an applier. The follow-up project review counted units for the anim systems alone (X4, closed by the bone scene). The sketch's session applier in `game-sim` runs only in the headless binary's own run, which no SOUR mode covers. Count every system's units across the nightly's recorded scenes once, and record the shipped session applier's joins, before `game-sim` adds systems | 5 |
| Minidumps and symbol upload (D155) | 5 |
~~~

### Other text

The directory watch's move changes one sentence in each of `crates/asset/README.md`,
`crates/io/README.md` and `tools/cook/README.md`, where "milestone 2" becomes "milestone 3".
`docs/proposals/artist-surfaces.md`'s status line and its two rows gated at 2 name milestone 3.
`crates/app-headless/README.md`'s sentence on the minidump follows D155, with the `crash-hook`
merge. `crates/app/README.md` says "`app-client` and `app-server` at milestone 4" in three places:
the client binary arrives at milestone 2 with a local world, and those sentences change with
`frame-design`. `crates/sim/src/schedule.rs` says of `Pass::Extract` that `presentation` copies the
dirty rows: the system is `lactose-present-api`'s, and `present-impl`'s brief names that comment as
a crossing. Lane `sketch` changes `crates/app-headless/README.md`, the list and the comment of
`[lint_file_carriers]` in `tools/xtask/ci-policy.toml`, `docs/results/README.md`'s `edit-to-playing`
row and `crates/app/src/game.rs`'s sentence on where the sketch is written.

Three small things are taken or left as follows. `docs/results/README.md`'s command omits two
arguments `cargo xtask measure` passes, and `crates/task/README.md` calls the benchmark's load
threads (N4); both are corrected below. No digest method is written into `tools/xtask/README.md`:
decision 5.19 drops the digest for `cargo xtask sour compare`. D143's citation of an old path is
history and stays.

**The measured test's command.** `docs/results/README.md`, line 35 at `a561b36`. OLD:

~~~
`integration` (D150), selected by its module path and run in release by `cargo nextest run
--cargo-profile release --package <package> --test integration --run-ignored only --retries 0
--no-capture -E 'test(=<module>::<test>)'`; it prints each figure as a line `measure: <name>
~~~

NEW:

~~~
`integration` (D150), selected by its module path and run in release by `cargo nextest run
--locked --cargo-profile release --package <package> --test integration --run-ignored only
--retries 0 --no-capture --no-tests=fail -E 'test(=<module>::<test>)'`; it prints each figure
as a line `measure: <name>
~~~

**The benchmark's load.** `crates/task/README.md`, line 244 at `a561b36`. OLD:

~~~
log-uniformly from 10 µs to 1 ms, and each with and without two background threads running
zstd compression:
~~~

NEW:

~~~
log-uniformly from 10 µs to 1 ms, and each with and without two tasks of the background class
compressing with zstd:
~~~

## 7. The exit

Milestone 2 exits when these have happened, in this order.

1. The measurement commit exists. The quiet window, the laptop's session and session C have run on
   it, and their result files are on main. Milestone 1's sessions A and B have run in the desktop's
   first sitting, and their files are on main too (D151, decision 5.19).
2. Every queue of section 3 has drained, so section 22's delivers are on main: `platform`,
   `rhi-api`, `rhi-null`, `rhi-vulkan`, `render-graph`, the minimal `renderer`, the Slang
   toolchain, `present-api`, and the presentation snapshot and schedule.
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and a nightly that holds the client's
   recording test has compared it equal on three platforms, with each platform's release baseline
   equal to its dev one. The run ids are in the rows. No check job's median over its three most
   recent warm runs of main is past ten minutes, or the owner has ruled on the budget.
4. The coordinator's pass has left every row of section 2 Done or re-gated.
5. The planner has written the freeze decision (decision 5.13), and the owner has approved it.
6. A `reviewer` and a `reviewer-gpu` read each of the seven criteria against its test and its
   result file. They report one verdict for each: met; met as restated, naming the decision; or
   not met. They re-run what they can on the M1 and state what they could not. They read
   row 169, "The x86 half of milestones 0 and 1", against sessions A and B's files the same
   way, if the first sitting has not already closed it (D151).
7. The owner rules on every "not met": another lane, or a restatement recorded in the decisions
   log. Milestone 2 does not exit on an unrecorded miss. Whether a project review reads the
   render seams at that commit, as one read milestone 1's, is asked then.
8. The planner writes the milestone 3 plan.

## 8. For revision 3.5

Revision 3.5 is the planner's pass after this plan is approved. It must settle these. Revision 3.4
came first, with the re-base, and is not repeated here: sections 8.1 and 8.3, section 22's exit
sentence and its two readings, and the marker (D152, section 6).

| Sentence | What 3.5 decides |
|---|---|
| Section 2, Graphics: "MoltenVK and KosmicKrisp spiked in milestone 2 as interim Apple paths" | The interim path is MoltenVK, with KosmicKrisp measured beside it on the M1 (D154) |
| Section 12.1: "with every claimed common feature verified in the milestone 2 spike", and its Apple path paragraph | The RHI spike, the native Metal spike and the conformance suite, by name; KosmicKrisp's requirement |
| Section 5.1: "A dedicated present thread calls present so a blocking `nextDrawable` or FIFO present never stalls the event pump or a worker" | The present thread also submits every command that reads or writes a swapchain image, on a queue of its own where the device has one that can draw and present, from one of two or more offscreen targets (D153) |
| Section 20: "`rhi-api` stays unstable until the native Metal spike has run" | The spike has run and the Vulkan backend has passed the suite: what the exit decision recorded as frozen, and what `rhi-metal` may amend |
| Section 20: "RHI conformance runs under lavapipe on Linux CI and on a GPU runner nightly" | lavapipe on every pull request; hardware at merges and at the measurement commit (D153) |
| Section 20's CI line and section 4's `xtask` line | The `gpu (lavapipe)` and `shaders` jobs; `cargo xtask gpu` and its rule |
| Section 22, milestones 2, 3 and 4 | A paragraph under the table on how milestone 2's criteria are read, as milestone 1 has: the triangle, the two workloads, the machines, KosmicKrisp. The scatter and the cell-plus-offset row format arrive in milestone 2; the GPU scene stays milestone 3's. The client binary arrives in milestone 2 with a local world; the networked client stays milestone 4's. `game-sim` arrives in milestone 2 holding the sketch game, and the game stays milestone 5's (D152) |
| Section 22.1 | Crash reporting moves to 5; the Apple interim path is decided |
| Sections 3 and 5.1, "crash dump" | The text report, and that the hook is `lactose-app`'s (D155) |
| Section 4 | The denylist binds `lactose-app`'s closure, with or without the server binary. `crate-tiers.toml`'s comment changes with lane `server-closure` |
| Section 5.3 | The snapshot's measured budget; what the presentation schedule runs before milestones 3 and 5 |
| Sections 2 and 12.2 | Slang as a pinned release; what the cook checks in what it emitted; a metallib cooked on macOS alone, as MSL 3.1; macOS 14 as the oldest client the cook targets, which no machine in reach runs once the Mac is upgraded |
| Sections 12.4 and 19 | What the minimal renderer is; device-lost recovery tested at milestone 3 |
| Section 14 and D136, D147 | The directory watch at milestone 3 |
| Section 20, the CI budget and results | How the budget is read, the median of each check job's three most recent warm runs of main; its figure, if the owner raises it (question 6). The M1's figures are of macOS 27.2.1 from milestone 2 on, and `cargo xtask measure --all` is run again there. The nightly's release baseline (X5). The desktop's two sittings, if the owner takes them (D151's addendum) |
| Section 12.1: "residency calls where the backend needs them" and "rather than relying on automatic hazard tracking"; D45's premise; D43 on indirect command buffers | The native Metal spike's three conflicts (`docs/results/spike-metal-apple-m1-2026-11-17.md`, "Against the architecture"), each amended or kept by the freeze decision at the exit (D153) |
| `CLAUDE.md` | The revision marker |

## 9. Between milestone 1's exit and the first lane

Milestone 1 exited on the M1 under macOS 14.7.4. Its exit steps 1 to 7 were complete at main
`a561b36` on 16 November 2026 UTC, and this re-base is step 8; section 6 writes the record into
`docs/plans/milestone-1.md`, section 7. No criterion was "not met". Criteria 1, 4, 6 and 9 and
milestone 0's makespan, wake and steal are met on the M1 alone, and their x86 half is row 169's.
Tick start is met on the M1 as restated: D138's addendum holds it to 35 µs at P99 on macOS and
Linux (on macOS 50 µs since D156, after milestone 1's exit), `keeper` merged at `0ec2d90`, and the
M1's file there gives 24.4 µs (`docs/results/whisk-apple-m1-2026-11-16.md`). Windows' tick
start, held to 1 ms (D144), is met nowhere yet: it is the desktop's (D151). So `eco-qos` and
`frame-design` wait on `keeper` no longer.

**The steps**, in this order (the owner's answers 1 and 5, the added ones, and decision 5.19):

1. Revision 3.4 is applied (section 6): the architecture's sentences, `CLAUDE.md`'s two lines if the
   owner approves them, D152 and the exit's record. It is documents alone and waits for nothing.
   `present-api` and `sketch` are briefed after it.
2. The owner upgrades the Mac to macOS 27.2.1, on 17 November 2026. Milestone 1's criteria were
   proven and reviewed on macOS 14.7.4, and this re-base was written there. From the upgrade on, a
   fix that changes measured code is measured on macOS 27.2.1, and its file says so; the files of
   macOS 14.7.4 stay milestone 1's proof.
3. The owner installs the Xcode current for macOS 27.2.1, and its Metal toolchain if that Xcode
   installs it as a component of its own.
4. The coordinator re-checks what the upgrade can break, with no lane: the Vulkan SDK's loader,
   MoltenVK and the validation layer; `vulkaninfo` under MoltenVK, and under KosmicKrisp, which must
   now enumerate the M1; the SDK's `slangc`; `xcrun metal`, and that it compiles MSL 3.1 for a macOS
   14 deployment target; main's build and its gates. It runs the RHI spike's program again by its
   result file's commands: the ten items and the five misuses read as they did on macOS 14.7.4, or
   the difference goes to the planner before `rhi-api` is briefed. If the SDK has to be replaced to
   run there, `reviewer-gpu` repeats decision 5.6's reading of MoltenVK's source for the new version
   first. It re-checks row 62 (M5): one boundary crate's tests, built for `x86_64-apple-darwin`, are
   run under Rosetta, and the row records whether the x86-64-v3 build still faults. If it no longer
   does, whether a local x86-64 run steers anything is a question for the planner; it proves nothing
   either way.
5. The coordinator's quiet window: `cargo xtask measure --all` at main's head, which writes the
   WHISK file too, its files checked in beside those of macOS 14.7.4. The files it is read against
   are `whisk-apple-m1-2026-11-16` (`0ec2d90`), `spike-net-apple-m1-2026-11-16` (`8941997`) and
   those of 15 November for the rest. A figure that moved is read against the lanes merged since the
   old file's commit, and where that does not explain it, that one measurement is run again at the
   old commit. A figure that crosses a milestone 1 criterion is a finding for the owner, recorded in
   the re-run's file and in a row gated at milestone 2; the planner brings a remedy and milestone
   2's lanes go ahead. Milestone 1 is not reopened, by the owner's ruling.
6. The coordinator dispatches the `measure` workflow at that commit. It has run once, at `bef3455`,
   and it is what shows the command working on windows-x64 and linux-x64 before the owner sits. It
   dispatches the nightly there too if the schedule did not run on that commit, for the hash file
   the desktop is compared with.
7. The desktop's first sitting, at that commit (decision 5.19): the coordinator gives the owner its
   three commands. A miss goes to the planner when the files are in. With "One sitting, at the end"
   this step moves to the measurement commit, and step 6 with it.

The first lane that builds on the Mac waits for step 4, and no lane waits for steps 5 to 7: a quiet
window stops the tracks for its run and no longer. If KosmicKrisp does not enumerate the M1 on macOS
27.2.1, the record is that line, decision 5.7 stands with no comparison, and the owner is told.

| What the exit left open | What it changes here |
|---|---|
| The re-run after the upgrade moves a figure across a milestone 1 criterion. The margins are thin for the prediction slot, 90.0 µs against 100 (`prediction-slot-apple-m1-2026-11-15`, at `c50b688`), and the wake from an idle pool, 31.0 µs against 35 (`whisk-apple-m1-2026-11-16`, at `0ec2d90`) | The owner has ruled: a finding, and milestone 1 stays closed. The planner brings a restatement, or a lane in `lactose-task`, `lactose-predict` or `lactose-physics-rapier` that runs beside milestone 2's and edits none of its crates |
| Criterion 1 on eight performance cores is the desktop's (D151), and the laptop's 4,689.9 µs on six is part of the finding. If the desktop misses, the planner brings a remedy and the owner rules: another ECS lane, or a restatement | With the first sitting the miss is known before `present-impl` takes `lactose-ecs`; with one sitting, at the measurement commit. An ECS lane edits `lactose-ecs` under the one-lane rule. Section 4's arithmetic for extract assumes the ECS at the speed `tick-100k` measured on the M1 under macOS 14.7.4. The eight-core figure is of the tick alone and is no frame budget (N6) |
| Criterion 9, edit-to-playing on Windows and Linux, is the desktop's (D151). Over thirty seconds brings a build-time proposal | That proposal can change the workspace's build layout, and decision 5.15's figure is read again with it |
| The desktop's CPU, its GPU and its second system are unknown until its first sitting | Section 4's desktop row, what session C can run under the second system, and `gpu-probe`'s record of its queue families |
| The re-base's four questions are open (section 5) | Decision 5.19, D151's addendum, lane `ci-split`, and whether a hash file is checked in |
| D151 closed rows 94, 123, 133 and 149 on the M1. `keeper` closed row 79 and the M1's half of row 81. Nightly 43849444834 at `d7a0088` closed row 167, the physics hash. The desktop's part of rows 64, 77, 80, 81 and 164, and row 169, are gated at 2 | They are in section 2's table |
| Row 170, the release baseline (X5), is open until a nightly shows release equal to dev on three platforms | The coordinator's pass closes it. A difference is a finding in the crate it names, and reaches a milestone 2 lane only if that crate is `lactose-ecs` or `lactose-app` |
| Exit step 7, the follow-up project review. Its lane findings are fixed by `exit-docs` and `exit-tooling`; X1 and X2 are revision 3.4; M1 and M5 are in this re-base | `present-api` and `sketch` wait for revision 3.4. `frame-design`, `present-impl`, `presentation` and `interp-loom` are released: no finding is open in `lactose-app`, `lactose-predict`, `lactose-ecs` or `lactose-net-host`. Their briefs carry rule 8 of section 3 |
| Another decision is applied before this plan's | The decision numbers of section 6, each up by one |
| Main moves | Every line number here, and each OLD of section 6, which is checked against `a561b36` |

One piece of work depends on none of this: the coordinator's `gpu-probe`, which reads the runners
and the laptop. `spike-metal` touches no file of the tree, but it waits for macOS 27.2.1, since the
facade is written from it.

## 10. What this plan could not verify

Checked against documentation for revision 2, on 16 November 2026, through summarising fetches:

- **Xcode.** Apple's support page lists Xcode 16.2 as the newest that runs on macOS 14, and Xcode
  26 as running on macOS 15.6 to 26. Nothing was looked up for macOS 27.2.1: which Xcode is
  current there, its size, whether its `metal` compiler is a component to install apart, and
  whether it still compiles MSL 3.1 for a macOS 14 deployment target, were not checked.
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
  That it enumerates the M1 on macOS 27.2.1, passes any case, offers a lazily allocated type,
  runs GPU-assisted validation or takes the drawable outside a submit is unknown until the
  upgrade.
- **macOS 27.2.1.** The RHI spike, both GPU reviews' probes and every figure of milestones 0 and 1
  are of macOS 14.7.4. Nothing was run on macOS 27.2.1 and nothing about it was looked up. It is
  past the release Mesa's documentation names for KosmicKrisp, and past any system the installed SDK
  was read against. That Vulkan SDK 1.4.363.1's loader, MoltenVK 1.4.2 and KosmicKrisp run there as
  they did, that Rosetta still faults on the x86-64-v3 baseline, and that the M1's figures hold, is
  unknown until the coordinator's re-check and the quiet window.
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
- **The desktop.** Its CPU model, its GPU and its second system are in no document. Whether it has
  the toolchain `cargo xtask measure` needs under each system was not asked. That `cargo xtask sour
  run --mode architecture --emit` runs there outside CI was not tried: it is the nightly's own step.
  The `measure` workflow has run once, at `bef3455`.
- **The boundary check on milestone 2's crates.** Nothing here was built by the plan. The first
  review of this re-base built a `present-api` facade as a carrier, with four components and
  extract's pass system: `cargo xtask boundary` passed, and reported five findings with the crate
  off the list. The second built the sketch's move into `game-sim` on `a561b36`: the 120-tick hash
  and the recorded streams were identical under three configurations, `boundary`, `tiers`,
  `features` and clippy passed, the binary came off the carriers with `migrations: None`, and the
  clock-patched system made the recorded test differ. It ran no `cargo xtask sour run` and nothing
  on Windows or Linux. Which of `lactose-present-api`'s components are `derived` is the facade's to
  say, and so is where the camera rig's orbit state lives; that `lactose-presentation` declares
  nothing a world registers rests on it.
- **Pass systems.** That the gather leaves the world hash as it found it is read from
  `crates/sim/README.md` and `crates/net-api`'s description, not run, and no test holds it (section
  6, rows to add).
- **Systems and recorded tests.** Fifty-five test files step a world at `a561b36` and about twenty
  of them name a session or record, by this plan's text search; the review's wider search finds 93
  files that step something and 31 that name no session or record. Either way some thirty step a
  world and record nothing, which is all D152 says. That every system that writes a hashed column
  runs on rows in a recorded test was not checked beyond the follow-up review's X4 and the sketch's
  system.
- **The exit's date.** 16 November 2026 is `a561b36`'s commit date in UTC. The commit's own clock,
  ten hours ahead, reads 17 November.
- **The CI budget.** The check jobs' figures up to `4ffcd62` are D150's addendum and the workflow's
  header on main; those at `d7f3e3b` and `a561b36` are the review's of this re-base, from the job
  logs. The plan itself looked at no run. The Actions cache's size, 10,736,001,967 bytes in 87
  entries by that review, and the cost of the nightly's release baseline, an estimate of lane
  `exit-tooling`'s review, are taken as given. What a crate adds to the Windows check since D150 is
  not measured, nor what a cold Windows check takes, nor each half's time after a split. That a run
  made again joins the workflow's concurrency group is GitHub's documented behaviour, not tried.
- **Crash handling.** No minidump crate was read. Decision 5.9 rests on where such a writer must
  run, not on one crate's API. What macOS and Linux write for a fault by default was not tested
  on the machines.
- **The run counts** are this plan's estimates by milestone 1's conventions. Milestone 1's lanes
  took two to five review rounds.
- **The 70 ns for each dirty row** is arithmetic. No extract exists to measure.
