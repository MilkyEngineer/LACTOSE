# Proposal: the sittings that close milestone 2

Status: proposed, for its review and then the owner. It answers the exit pass's audit of main at
`ad1a99d` (26 November 2026): what stands between main and milestone 2's exit
(`docs/plans/milestone-2.md`, section 7), and in what order it is done. It settles four things: the
measurement commit, the order of the sittings and what each asks of the owner, the Khronos layer's
pin on Windows, and the stamp verification row 21's Windows item waits for. The owner's four
questions are at the end. It amends no sentence of `docs/architecture.md`. Approved as
recommended, it is applied as one decision entry, the plan's and the verification rows' text
below, and two small lanes.

This proposal ran no measurement and no device. It read the tree, the audit and `reviewer-gpu`'s
reading for row 34, and ran one probe: the Windows layer's digest, read on the development Mac out
of LunarG's installer, which was fetched and never run.

## What stands between main and the exit

| What section 7 asks | State at `ad1a99d` |
|---|---|
| Step 1: a measurement commit, and the quiet window, the laptop's session and session C on it | No commit is named. No result file of milestone 2 is on main |
| Step 1: the desktop's first sitting | Not run. Its commands are written, at `a539d60` (D151, decision 5.19) |
| Step 3: `gpu (lavapipe)` and `shaders` green, no check job's median past ten minutes | Met. Lane `exit-pass-rows` writes the run ids into the rows |
| Step 3: a nightly that holds the client's recording test | Not run. The last nightly, 45609400690 at `98d11cf`, is older than the test, which merged with `frame-impl` at `55ffdda` |
| Step 4: section 2's 26 rows Done or re-gated | Five the pass can write now (35, 95, 99, 134, 170). Three the laptop's (63, 109, 150). Six the first sitting's (64, 77, 80, 81, 164, 169). Six the measurement commit's files (15, 16, 19, 21, 34, 102). One the freeze decision's (18). Five re-gated already (97, 136, 139, 140, 143) |
| Criterion 1 on Windows | Cannot run. `tools/xtask/vulkan-layer.toml` pins no layer for Windows, and `triangle` and `cargo xtask gpu` both stop at `vulkan::host` |
| Row 21's Windows item | Cannot be measured. Both of its instants are inside `lactose-task` (D162's addendum of `measure-m2`, round two) |
| Lane `present-encoder` | In its fix round. It edits the client's present call, which `triangle` runs through and `frame-pace` reads |

**What the audit did not read.** Five things, found while placing the sittings.

- *Section 2 lists 26 rows, and 85 rows of `docs/verification.md` are gated at milestone 2.* The
  26 are those gated at `a561b36`, and 21 of them still are. The other 64 were opened by the plan
  and its lanes. Exit step 4 names section 2's rows alone, and the audit read those. By their own
  text 27 of the 64 owe a run on the laptop or the desktop: 176, 189, 190, 208 to 210, 228, 229,
  234, 236, 243, 246 and 247 the laptop's; 212, 214, 216, 218 to 220, 222, 224, 225, 238 to 241
  and 244 the desktop's. This proposal places those. The other 37 it read by their heads and
  status marks alone, and none against the tree.
- *Nothing writes `rhi-conformance`.* Section 4 says `cargo xtask gpu --driver <name>` writes
  `rhi-conformance-<driver>-<machine>-<date>.md`, and criterion 2's proof and row 171 name the
  file. The command writes `target/gpu/<driver>-vulkan[-gpu-assisted].txt`
  (`tools/xtask/src/main.rs`, `run_vulkan`). Its lines hold the SDK's and the environment's paths
  as the machine has them, and under `--driver system` a second adapter's run overwrites the
  first's.
- *Session C as the plan has it is one command.* The rows owe the desktop more than
  `cargo xtask measure --all`: the device tests on a desktop's driver (rows 212, 214, 216, 218,
  219, 222, 224 and 244, two of them on a device whose present family is not its graphics
  family), the renderer's cost tests (rows 220 and 238) and the three barrier mutations (rows 225
  and 239 to 241). The mutation script is in no checkout: lane `renderer-impl` left it outside
  the repository.
- *Nothing skips a window on a system that is not a client platform.* Section 4 says the
  measurements that need a window are skipped there and the file says so. `cargo xtask measure` has
  no such rule, and on Linux `lactose-platform` "builds over X11 and is not run" (its README,
  "Status").
- *Row 243 names a selection the tool does not make.* It has the laptop's GPUs "chosen by
  `VK_LOADER_DRIVERS_SELECT`". A run clears every `VK_LOADER_*` variable
  (`tools/xtask/src/vulkan.rs`, `CLEARED_PREFIXES`) and chooses the device by `--adapter`, whose
  name the device's name must contain (`crates/rhi-vulkan/README.md`, `VulkanDevice::new`).

## The measurement commit

**The decision.** The measurement commit is the first commit of main that holds every lane that
edits a measured crate. At `ad1a99d` four are in: `frame-impl`, `measure-m2`, `snapshot-order` and
`measure-role`. Two are owed: `present-encoder`, and `park-stamp` below. The coordinator records
its hash in a verification row of its own, which this proposal adds, once the last of them has
merged and `check` and the `measure` workflow are green at that commit. A failure there is a fix
round of the lane it names, and the commit is that round's merge.

A measured crate is one that a measurement of `cargo xtask measure` builds into what it times.
`tools/xtask` is not one. Lane `layer-windows` below edits `tools/xtask` alone and changes no
existing measurement's code, so it may merge after the commit. The recommendation is that the
commit waits for it all the same, since it runs beside the other two and every file then cites one
hash.

**How a row names it.** "The measurement commit", with its short hash beside it once it exists.
A file is the record at that commit alone. A file taken at a later commit is the record only as
`-at-<commit>` (`docs/results/README.md`), where the row that cites it gives the diff from the
measurement commit and why no figure in the file reads it. That is the form a fix round of
`layer-windows` takes after the laptop's first run.

**What a figure taken before it is.** Steering. It stays in the row it steered, proves no
criterion and closes no row. The lanes' runs at `7e3b62a` and `16f45fc` in rows 21 and 176 are
such figures, and so is anything run at `ad1a99d`. Two sittings have commits of their own and are
not steering: the desktop's first sitting at `a539d60` (D151's addendum), and the laptop's
before-and-after runs of `eco-qos` at `a75596b` and its first parent (row 63).

**After it.** Section 3's rule stands: a lane that merges later and edits a measured crate runs the
measurements it touches and reports them as steering. Where such a lane answers a finding of a
sitting, the owner's ruling on the finding says which file is the record (section 7, step 7).

**The nightly.** Exit step 3's nightly is the first green one at or after the measurement commit.
`park-stamp` edits the executor and `present-encoder` edits the package that holds the recording
test, so a nightly before them compares release with dev on three platforms for a tree the exit
does not ship. The coordinator dispatches the nightly at the commit if the schedule did not run on
it, and the `measure` workflow with it, as section 9's step 6 did at the re-run's commit. The
`measure` workflow there is the first run of `record-stall` on Windows with `park-stamp`'s spans.
It shows the test passing there before the laptop is asked. Its figures are a runner's and are
discarded (`.github/workflows/measure.yml`).

| Option | What it rules out |
|---|---|
| A. The commit waits for every lane that edits a measured crate, and for `layer-windows` (recommended) | Sittings before those lanes merge: about two days |
| B. Main's head now. The lanes merge after it | One commit for the record. `present-encoder` then changes what `frame-pace` and `triangle` measured, and `park-stamp` changes the executor under all eighteen measurements: each is run again as steering, and the files the criteria rest on are of a tree the exit does not ship |
| C. The commit waits for `present-encoder` alone | Row 21's Windows item measured at the commit. It is the right commit if the owner re-gates that item, and this proposal's text then drops `park-stamp` from the list |

**What would change it.** A lane of the two that runs past two days or fails its review: the
commit is taken without it, and its item is re-gated or run at its own commit by the rule above.

## The sittings, in order

Four sittings and two things that need no machine. Each machine's owner-facing needs are asked
once.

| Order | Sitting | Machine and commit | What it needs from the owner | What it writes | Rows it closes or advances |
|---|---|---|---|---|---|
| Now | The desktop's first sitting | The desktop, under each system it boots, at `a539d60` | The owner at the desk for all of it. Mains power. Git, rustup, a C toolchain, nextest. Under Windows, as administrator: `LocalDumps`, and the Vulkan SDK 1.4.363.1 installed, so session C needs no second setup | Milestone 1's twelve files for each system, the hash file `cargo xtask sour run --emit` writes, nextest's summary for `lactose-task`. The owner returns the CPU model, the systems, the GPU and `vulkaninfo`'s output | 64, 77, 80, 81, 164, 169; the desktop's half of 190; the desktop's queue families for 176; the facts decision 5.19 wants, which make session C's commands exact |
| Now | The lanes, and the rows not yet read | None | Nothing | `present-encoder`, `layer-windows` and `park-stamp` in parallel. One read-only pass over the 37 rows gated at milestone 2 that neither the audit nor this proposal read, so that what they owe a machine is in a commands file before the commit | None yet |
| 1 | The quiet window | The M1, at the measurement commit | The Mac on mains, awake, its screen unlocked, and unused for about an hour. One hand at the start: a window resized by its corner for a few seconds (row 209) | Eighteen measurements' files: milestone 1's twelve again, `extract-50k`, `remote-upload-5k` and `record-stall`, and `triangle`, `gpu-cost` and `frame-pace` under MoltenVK and under KosmicKrisp. The `rhi-conformance` files, under both drivers and under KosmicKrisp with GPU-assisted validation. The renderer's cost tests' figures, into their rows. The system daemon's CPU time before and after | The M1's half of 15, 16, 19, 21, 34, 102, 176, 192, 220, 228, 229 and 238; 171's files; 173's tick start read again; 209's M1 half |
| 2 | The laptop's session | The laptop, at the measurement commit; row 63's runs at `a75596b` and its first parent | Once, as administrator: the SDK's installer and `LocalDumps`. Then the laptop on mains, logged in, its screen unlocked and set not to turn off, and the agent session opened in a shell that is not elevated, with the session's brief. One hand in the session's first minutes: a window dragged by its title bar (row 209). The display's refresh rate read out once | Row 63's three `whisk` files. All eighteen measurements on the RTX 4070 as `laptop-windows`, its `triangle` file criterion 1's proof on Windows. `triangle`, `gpu-cost` and `frame-pace` on the integrated GPU as `laptop-windows-intel`. `rhi-conformance`, plain and GPU-assisted, on each GPU. The window cases' output and the latency tool's lines | 63, 109, 150's hardware half, 189, 208, 209, 210, 234, 236, 243; the laptop's half of 21, 164's two items that `frame-pace` reads, 176, 190, 228, 229, 246 and 247; labelled figures for 15, 16, 19 and 102 |
| 3 | Session C | The desktop, under each system it boots, at the measurement commit | The owner at the desk for all of it. Mains power. Python 3 for the mutation script | All eighteen measurements under Windows; under a system that is not a client platform, the sixteen that need no window. `rhi-conformance`, plain and GPU-assisted, on each GPU. The renderer's cost tests' figures. The three mutations' outcome, each beside its control | The x86 desktop's half of 15, 16, 19 and 102; 176; 212, 214, 216, 218 to 220, 222, 224, 225, 238 to 241 and 244; 246 and 247 |
| After | Files, rows and the exit's steps | None | The rulings of step 7 | The coordinator checks each machine's files in, scanned for paths as a public repository needs. The pass writes the rows. The first green nightly at or after the commit gives rows 170 and 232 their run id. Then section 7's steps 5 to 8 | Every row gated at milestone 2, Done or re-gated |

**Why this order.** The first sitting waits for nothing: its commit is fixed, its commands are
written, and what it returns of the desktop's GPU and second system is what session C's commands
need. The quiet window is first of the three at the commit because it is the cheapest and it reads
the M1's half of every criterion a result file proves: a criterion that misses there needs an
engine lane, and the files of a laptop's session and a desk sitting taken before that lane would
be steering. The laptop is second because its session is the first run of the Windows pin, and a
defect of the tool found there is fixed before the owner sits at the desktop, whose Windows needs
the same pin. Session C is last because the owner's time at a desk is the scarcest thing in the
plan, and it then runs once, on a tree two machines have measured.

**The laptop is asked once.** Its session holds, in this order. While the owner is still there:
the checkouts' refresh (row 109), before anything is built; the installed layer's digest and the
devices' list; and the latency tool, which is the one step that needs a hand (row 209). Then,
with nobody at it: at the measurement commit the pin's first run on each GPU, which fails in its
first minutes if the tool cannot use the pin, and the measurements; row 63's three runs at their
own commits; and the window cases (rows 208, 210, 234 and 236). `frame-impl`'s by-hand form of
criterion 1 is dropped: `cargo xtask measure triangle` is that run once the pin exists (row 243).

**Row 150's four hardware questions.** The spike's file leaves four to hardware
(`docs/results/spike-rhi-apple-m1-2026-11-15.md`, "Design questions for the lavapipe run"): an
ownership transfer's effect, memory that is not coherent, a heap indexed without the non-uniform
decoration, and a heap image in the wrong layout. The plan's table of what the spike left open
says of two of them: "Probed on the laptop's devices and under GPU-assisted validation, as
labelled data (decision 5.17)". The probes are the gate's two runs on each GPU,
`cargo xtask gpu --driver system`, plain and with `--gpu-assisted`. They answer the first two
where the device allows: `ownership_transfer_keeps_the_contents` and the canary of a race across
queues run where two roles have two queue families, and the capabilities line gives
`coherent_host_memory`. No case in the tree makes either heap misuse on a device: the cook refuses
the first (`tools/cook/README.md`, "The checks") and null's rule holds the second. Those two are
recorded as not shown, and a run that shows nothing discharges no rule of null (decision 5.17).
The alternative is a lane that writes the two misuses as device probes, about seven agent runs
with `reviewer-gpu`. It is the owner's fourth question.

**A second GPU on one machine.** `LACTOSE_MEASURE_MACHINE` names the machine in a file's name
(`docs/results/README.md`). The laptop's RTX 4070 runs as `laptop-windows`, the name its files of
15 November carry, and the integrated GPU as `laptop-windows-intel`, each with `--adapter`. The
desktop runs as `desktop-windows` and `desktop-<system>`. No code is needed.

**A system that is not a client platform.** Session C there names its measurements and leaves
out `triangle` and `frame-pace`. Nothing is built for it: the plan's sentence is amended to say
so.

**What would change the order.** The owner preferring one day to three: the three sittings at the
commit then run without waiting for each other's reading, and a miss that needs an engine lane
costs the other two machines a second sitting. It is the owner's third question.

## The Khronos layer on Windows

D153 has the layer "pinned on every host". `vulkan-layer.toml` pins Linux by an archive the tool
fetches by digest and the Mac by the digest of the library the owner's installer leaves.

**The decision.** Windows takes the Mac's form. The pin is the SHA-256 of
`Bin\VkLayer_khronos_validation.dll` of the SDK the owner installs. The tool finds the SDK at
`VULKAN_SDK`, which LunarG's installer sets, or at `C:\VulkanSDK\<version>`. It takes the system's
loader and the system's drivers, so `--driver system` is the only driver there.

~~~toml
# LunarG's installer, which an owner runs as administrator: the SDK at `VULKAN_SDK`, or at
# `C:\VulkanSDK\<version>` where the installer puts it. Nothing is fetched. `url` and `sha256`
# name the installer, for the error that says the SDK is not there.
[host.windows-x86_64]
url = "https://sdk.lunarg.com/sdk/download/1.4.363.1/windows/vulkansdk-windows-X64-1.4.363.1.exe"
sha256 = "c02932bd9a968b22d404536108adaacac908c081cd6b0113f5bfefc523dd0a6a"
library_sha256 = "451ed11cbc799f858cbc0e843766d2990527c03f93f7a85f03bc1e67b605a435"
~~~

**How the digest is obtained with no Windows host in reach.** LunarG publishes each SDK file's
SHA-256 in one list, and the list's digest for the Linux archive is the one the pin already holds.
The Windows installer for 1.4.363.1 is 303,066,832 bytes with the SHA-256 above. It is a Qt
installer that carries ten 7z archives. The first holds `Bin/VkLayer_khronos_validation.dll`,
21,867,448 bytes, and its manifest, which names `VK_LAYER_KHRONOS_validation` at `1.4.363` with
the library beside it. This proposal's probe fetched the installer, checked its digest against the
list, cut the archive out by its 7z signature and header, and read the two files with the Mac's
own `bsdtar`. Nothing of the installer was run. The lane derives the digest again by the same
steps and writes them into `tools/xtask/README.md`.

**How it is checked.** On the laptop, by the tool itself: `vulkan::sdk` refuses a library whose
digest is not the pin's, so the first `cargo xtask gpu --driver system` is the check. A digest
that differs is a finding for the planner. The fallback is the Mac's own history: the pin takes
the installed file's digest on the owner's word.

| | The Mac can do it | Only the laptop can |
|---|---|---|
| The pin | Its form and parse; the digest out of the installer; the paths, the environment and the implicit manifest's JSON for Windows as unit tests; clippy for `x86_64-pc-windows-msvc` | Run the installer, which needs an administrator. Show that the installed file is the archived one |
| The loader | Nothing | Show that the system's loader takes the manifest from `VK_ADD_IMPLICIT_LAYER_PATH` in a shell that is not elevated. The loader's documentation says it ignores such variables in an elevated process |
| The run | The M1's `rhi-conformance` files | Hear each of the seven canaries on each of the laptop's GPUs. Record which implicit layers the laptop's registry adds, from `vulkaninfo`'s list |

| Option | What it rules out |
|---|---|
| A. Installed, as the Mac (recommended) | A run on a Windows machine with no administrator. The owner installs the SDK once on the laptop and once on the desktop's Windows |
| B. The tool fetches the installer by digest and cuts the layer out | Nothing of the owner's. It rests on the installer's internal layout, which LunarG does not document, and on Windows' own `tar` reading 7z, which nothing here has shown. About eight agent runs |
| C. No pin. Criterion 1 on Windows by hand, with whatever layer the machine has | Section 4's "its `triangle` file is criterion 1's proof on Windows", D153's sentence, and `cargo xtask gpu` on Windows at all: rows 210, 234 and 236 and the desktop's rows under Windows stay open |

**What the lane also carries.** Two things of the same crate, each too small for a lane. A
measurement `rhi-conformance` in `cargo xtask measure`, which writes the file section 4 names with
the measurements' header and no path of the machine in it. And `renderer-impl`'s mutation script,
checked in, so that the desktop needs one clone.

**What would change it.** The laptop's installed file not being the archived one twice over, or the
loader refusing the manifest outside an elevated shell: then B, or the owner's word for the digest.

## Row 21's Windows item

D162 sends the `SetEvent` that `Pool::open_gate` makes for each frame worker to `record-stall`'s
Windows rows, as "the time from each `block_in_place`'s end to the record's next job on a worker
that was parked". `record-stall` is a test of `lactose-app-client` and reaches neither instant.
D162's addendum names two forms of a stamp it could read, two process-wide hooks or the same
behind a cargo feature, and leaves the choice as an interface decision.

**The decision.** Neither. The stamp is two `tracing` spans of `lactose-task`, at debug level,
target `whisk`.

- `open_gate`: from the return of a `block_in_place`'s closure to the return of `Pool::open_gate`
  (`crates/task/src/job.rs`, `JobContext::block_in_place`).
- `park`: a worker's wait in the idle protocol, entered after the look that found nothing and
  exited when the wait returns (`crates/task/src/pool.rs`, `idle`; `gate::idle_park`).

This is the smallest surface that works, for four reasons the tree gives.

- `record-stall` already reads the physics phase from this crate's spans, by a subscriber it
  installs (`crates/app-client/tests/stall.rs`; D162's addendum, "`record-stall` reads the physics
  phase from the spans"). The two new spans arrive at the same subscriber.
- No public item is added and no `unsafe`. Architecture section 21's entry for `task` stands.
- Architecture section 5.1 already has it: "`tracing` spans are emitted per system and per stolen
  range, never per chunk job. Finer levels compile out." A debug span is a finer level. The info
  spans a profiler reads stay as they are, one for each node and each range.
- A hook is new shared state that every build carries, and it needs a contract for what a
  callback may do inside the idle protocol, which `gate.rs` and `sleep.rs` model under loom. A
  span runs the subscriber's code on the worker too, but through a path the crate already takes
  at every node.

The file gains, at each reservation: for each opening of the gate, the time from its start to the
start of the record's next job on a worker that was parked across it, that job's start wait beside
it, the opening's own length, and the count of openings after which the next record ran on a
worker that was awake. Every tick asserts that it saw an opening. A build that compiles debug
spans out then fails; it does not write a file with no rows. The rows are written on every host.
On Windows they are the `SetEvent` item.

| Option | What it rules out |
|---|---|
| A. Two debug spans, lane `park-stamp` before the commit (recommended) | A measurement commit before the lane merges. A shipping build that sets a static maximum level of info without deciding what `record-stall` then does |
| B. Re-gate the item to the change that revisits the keeper's wait | Knowing what opening the gate costs a record on Windows when `RECORD_RESERVATION` is set. D162 says the frame uses that `SetEvent`. No other run shows it: the client binary links no physics backend (`crates/app-client/Cargo.toml`), so `frame-pace` never opens the gate |
| C. Two public hooks, or the same behind a feature | Section 21's entry standing as it is, if a hook's storage needs `unsafe`; and a small crate surface. A feature unifies across the workspace's builds, as D162's addendum says |

**What would change it.** `cargo xtask measure whisk` before and after the lane on the M1, with
the 60 Hz row or the wake row moved beyond the two before runs' spread: then B. Or the owner
refusing a change to `lactose-task` before the commit: then B.

## Two points for the freeze decision

`reviewer-gpu`'s reading for row 34 found that the pinned MoltenVK 1.4.2 (`v1.4.2`, `db66022`)
encodes a multi-draw as a loop on the CPU, one Metal indirect draw for each record, inside
`vkQueueSubmit` under the defaults, and that KosmicKrisp loops inside recording. Row 34 then waits
on the quiet window's `gpu-cost` files alone. The reading raises two points. This proposal places
them and settles neither.

- *`maxDrawIndirectCount`.* `lactose-rhi-vulkan`'s `draw_indexed_indirect` never checks its count
  against the device's limit, and `lactose-rhi-api` carries no such limit. The fix the reading
  names is a field of `Capabilities` that every backend and null refuse above. That is a change to
  the surface the freeze fixes, so it belongs to the freeze decision's amendments (decision 5.13,
  row 18), built by a lane the decision names. No driver in reach has a limit a frame meets, so
  the sittings do not wait for it. If it is built before the exit it merges after the measurement
  commit under section 3's rule.
- *D42's "emulated indirect-count still encodes the maximum draw count".* It fits KosmicKrisp. It
  does not fit the pinned MoltenVK, which reports `drawIndirectCount` false and records nothing for
  the count entry points. The correction of D42's sentence is a correction of record: it joins the
  conflicts row 18 already holds for the freeze decision. Its consequence, a culling path under
  MoltenVK where the count is not offered, is an input to milestone 3's plan and to no lane of
  milestone 2.

## The rows (exact text)

Each OLD is a run of whole lines of the named file at `ad1a99d` that occurs once in it, and NEW
replaces the run. `D16x` stands for the decision's number, which the coordinator allocates. The
rows name each other by `D16x` and by their heads, never by a row number this proposal adds. The
text is written for the recommended answers. With question 1 answered "Re-gate the item",
`park-stamp`'s lane row, its verification row and its name in section 3's line go, and row 21's
and the plan's `record-stall` row's blocks are not applied: the planner writes the re-gate's
sentence for row 21. With question 2 answered otherwise, the lane row and the pin's row go back
to the planner.

**The dependency.** Lane `exit-pass-rows` edits verification rows 34, 35, 95, 134, 150, 171 and
177 from the same base. This proposal changes none of those lines but row 150, and of row 150 only
the clause on the laptop, which the lane leaves. That block is a fragment and is applied to the row
as the lane's merge leaves it. Rows 170 and 232, the nightly's, and section 3's line on the
measurement commit are this proposal's.

~~~text
=== docs/plans/milestone-2.md line 185: the lane table: two lanes after its last row
OLD:
| `measure-role` | Two safe calls for measurement in `lactose-task` (D164): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |
NEW:
| `measure-role` | Two safe calls for measurement in `lactose-task` (D164): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |
| `layer-windows` | The Khronos layer pinned on Windows (D16x; D153's "pinned on every host"), in `tools/xtask` alone. `vulkan-layer.toml` gains `[host.windows-x86_64]` in the Mac's form: the SDK the owner installs, found at `VULKAN_SDK` or at `C:\VulkanSDK\<version>`, its `Bin\VkLayer_khronos_validation.dll` held to `library_sha256`, with the installer's `url` and `sha256` beside it, named in the error for an SDK that is not there and never fetched. The lane reads the digest on the Mac out of LunarG's installer, fetched by the SHA-256 LunarG publishes and never run, and writes the steps into `tools/xtask/README.md`; the proposal's own reading is `451ed11cbc799f858cbc0e843766d2990527c03f93f7a85f03bc1e67b605a435`. On Windows a run takes the system's loader and drivers, `--driver system` alone, sets no library path and writes the layer's implicit manifest as on the other hosts. A measurement `rhi-conformance` in `cargo xtask measure`: the gate's Vulkan step under each driver `measure` takes, and under GPU-assisted validation where the host's gate has it, written as `rhi-conformance-<driver>[-gpu-assisted]-<machine>-<date>.md` under the measurements' header, with every path cut as a device file's is; `--all` runs it last and `--no-device` leaves it out. No existing measurement's code changes. `renderer-impl`'s barrier-mutation script, checked in beside `line-endings.sh` and made to read and write its files as UTF-8 on every host, since its first run on Windows is session C's | Nothing | On the M1: the pin's parse and the Windows paths, environment and manifest as unit tests; clippy for `x86_64-pc-windows-msvc`; the digest derived by the lane and equal to the proposal's; the three `rhi-conformance` files, with no path of the machine in them; the script's control run under KosmicKrisp. The Windows run is the laptop's session's, and a defect it finds is a fix round of this lane | 6 |
| `park-stamp` | Two `tracing` spans of `lactose-task` at debug level, target `whisk` (D16x, D162): `open_gate`, from the return of a `block_in_place`'s closure to the return of `Pool::open_gate`, and `park`, a worker's wait in the idle protocol, entered after the look that found nothing and exited when the wait returns. Where each span sits is the lane's; the loom models still run the shipped functions. No public item, no `unsafe`, no hook and no cargo feature: the crate's README names both under "Tracing", and architecture section 21's entry stands. `record-stall`'s subscriber reads them, and its file gains at each reservation, for each opening: the time from its start to the start of the record's next job on a worker that was parked across it, that job's start wait, the opening's own length, and the count of openings after which the next record ran on a worker that was awake. Every tick asserts that it saw an opening, so a build that compiles debug spans out fails. The rows are written on every host; on Windows they are the `SetEvent` item | `measure-role` merged | The rows in `record-stall`'s file on the M1. `cargo xtask measure whisk` before and after, as steering: the 60 Hz row and the wake row within the two before runs' spread. The loom models, Miri and the cross-target clippy runs for `lactose-task`. `tests/spans.rs` holds both spans' names, target and level, and that neither is emitted at info. On Windows the `measure` workflow at the commit shows the test passing with the spans, and the rows are the laptop's session's | 6 |

=== docs/plans/milestone-2.md lines 285 to 287: section 3: the measurement commit, and what a figure before it is
OLD:
- The measurement commit is the first commit of main that holds `frame-impl` and `measure-m2`. The
  quiet window, the laptop's session and session C run there (section 4). A lane that merges after
  it and edits a measured crate runs the measurements it touches and reports them as steering.
NEW:
- The measurement commit is the first commit of main that holds every lane that edits a measured
  crate: `frame-impl`, `measure-m2`, `snapshot-order`, `measure-role`, `present-encoder` and
  `park-stamp` (D16x). A measured crate is one a measurement builds into what it times, and
  `tools/xtask` is not one. The coordinator records the commit's hash in the verification row that
  names it, once `check` and the `measure` workflow are green at it; a failure there is a fix
  round of the lane it names, and the commit is that round's merge. The quiet window, the laptop's
  session and session C check it out (section 4). A file is the record at it, or at a later commit
  as `-at-<commit>` where its row gives the diff and why no figure of the file reads it. A figure
  taken before it is steering: it stays in its row, proves no criterion and closes no row. A lane
  that merges after it and edits a measured crate runs the measurements it touches and reports
  them as steering; where it answers a sitting's finding, the owner's ruling says which file is
  the record (section 7).

=== docs/plans/milestone-2.md lines 308 to 312: section 4: the quiet window
OLD:
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s -d`, and `cargo xtask measure --all` runs once. It writes milestone 2's
  six files beside milestone 1's, `gpu-cost` under MoltenVK and under KosmicKrisp. The WHISK
  file's 60 Hz P99 is read against D156: under 30 µs, the planner proposes 35 µs again; the system
  daemon's CPU time is read before and after the run, as the remeasurement of 17 November 2026 did.
NEW:
- *The quiet window*, the coordinator's, at the measurement commit, the first of the three
  sittings there (D16x). Every track stops, the machine is on mains power with low power mode off
  and its screen unlocked, and it holds `caffeinate -i -s -d`. The owner first resizes the latency
  tool's window by a corner, for row 209, and then leaves the machine. `cargo xtask measure --all`
  runs once. It writes milestone 2's six files beside milestone 1's, with `triangle`, `gpu-cost`
  and `frame-pace` under MoltenVK and under KosmicKrisp, and the `rhi-conformance` files. The
  renderer's cost tests run once after it, for rows 220 and 238. The WHISK file's 60 Hz P99 is
  read against D156: under 30 µs, the planner proposes 35 µs again; the system daemon's CPU time
  is read before and after the run, as the remeasurement of 17 November 2026 did. Its files are
  read before the laptop's session starts.

=== docs/plans/milestone-2.md lines 313 to 315: section 4: the laptop's one session
OLD:
- *The laptop's session*, at the same commit, with nothing else running there: the same command,
  and `cargo xtask gpu --driver system`. Its `triangle` file is criterion 1's proof on Windows
  (the owner's answer 2).
NEW:
- *The laptop's session*, once, at the same commit, after the quiet window's files are read, with
  nothing else running there and in a shell that is not elevated: `cargo xtask measure --all` on
  the RTX 4070, and `triangle`, `gpu-cost`, `frame-pace` and `rhi-conformance` again on the
  integrated GPU under a machine name of its own, each GPU chosen by `--adapter`.
  `rhi-conformance` is the gate's run, plain and under GPU-assisted validation, and on each GPU
  it is row 150's probe. Its `triangle` file on the RTX 4070 is criterion 1's proof on Windows
  (the owner's answer 2). The owner installs the pinned Vulkan SDK first, as administrator
  (D16x). The same session holds what the laptop owes at commits of its own or at none: the
  checkouts' refresh (row 109), before anything is built; the latency tool under the owner's hand
  (row 209), while the owner is still there; `eco-qos`'s before-and-after runs (row 63);
  `LocalDumps` (row 190); and the window cases (rows 208, 210, 234 and 236).

=== docs/plans/milestone-2.md lines 326 to 330: section 4: session C
OLD:
- *Session C*, the owner's, at the measurement commit, on the desktop under each system it boots:
  `cargo xtask measure --all`. Its `extract-50k` and `remote-upload-5k` files are the x86 desktop's
  figures. The command writes milestone 1's files again, which shows what milestone 2's lanes moved
  on x86. Under a system that is not a client platform, the measurements that need a window are
  skipped and the file says so.
NEW:
- *Session C*, the owner's, at the measurement commit, last, on the desktop under each system it
  boots (D16x): `cargo xtask measure --all`, whose `rhi-conformance` is the gate's run, plain and
  under GPU-assisted validation; the renderer's cost tests; and the barrier mutations, each beside
  its control (rows 225 and 239 to 241). Its `extract-50k` and `remote-upload-5k` files are the
  x86 desktop's figures. The command writes milestone 1's files again, which shows what milestone
  2's lanes moved on x86. Under a system that is not a client platform the session names its
  measurements and leaves out `triangle` and `frame-pace`: `lactose-platform` has not been run
  there, and nothing skips them.

=== docs/plans/milestone-2.md lines 332 to 333: section 4: what each sitting needs of the owner
OLD:
Only the desktop's two sittings need the owner at a desk. The laptop's runs need the laptop awake
with its agent session open, and a run that opens a window needs a person logged in at it.
NEW:
Only the desktop's two sittings need the owner at a desk. The quiet window needs the owner's hand
once at its start, and then the Mac unused. The laptop's session needs the owner as administrator
once and the owner's hand once in its first minutes, and then the laptop awake with its agent
session open and a person logged in at it. The laptop's runs after a merge need the laptop awake
with its agent session open, and a run that opens a window needs a person logged in at it.

=== docs/plans/milestone-2.md line 341: section 4, row `record-stall`: the gate's rows come from two spans, on every host
OLD:
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without, and on Windows the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is the `SetEvent` item of row 164's unmeasured list (D162) | Nothing: recorded. It sets `lactose-app`'s default reservation, `RECORD_RESERVATION`, which is one until the file is read (decision 5.16, D162) |
NEW:
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without, and, from `lactose-task`'s two debug spans, the time from each opening of the gate at a `block_in_place`'s end to the record's next job on a worker that was parked across it, with that job's start wait and the opening's own length, which on Windows is the `SetEvent` item of row 164's unmeasured list (D162, D16x) | Nothing: recorded. It sets `lactose-app`'s default reservation, `RECORD_RESERVATION`, which is one until the file is read (decision 5.16, D162) |

=== docs/plans/milestone-2.md lines 355 to 356: section 4: what writes `rhi-conformance`
OLD:
`cargo xtask gpu --driver <name>` writes `rhi-conformance-<driver>-<machine>-<date>.md`: the cases
run, the capabilities reported, each canary's message, and the count of other messages by source.
NEW:
`cargo xtask measure rhi-conformance` writes `rhi-conformance-<driver>-<machine>-<date>.md`, with
`-gpu-assisted` after the driver for that run: the cases run, the capabilities reported, each
canary's message, and the count of other messages by source (D16x). `cargo xtask gpu` keeps its
own report under the target directory.

=== docs/plans/milestone-2.md lines 1021 to 1024: section 5: the owner's actions, in order
OLD:
The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); the desktop's first sitting, once the M1's re-run is on main, with `LocalDumps`
set on the desktop; the laptop's checkouts refreshed once (D148) and `LocalDumps` set on the laptop,
before its first milestone 2 run; session C at the measurement commit.
NEW:
The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); the desktop's first sitting, once the M1's re-run is on main, with `LocalDumps`
set on the desktop and the pinned Vulkan SDK installed under its Windows; at the measurement
commit, one window resized at the quiet window's start and the Mac then left unused; then the
laptop's one session, with the pinned Vulkan SDK installed and `LocalDumps` set as administrator,
its checkouts refreshed (D148) and one window dragged in the session's first minutes; then
session C (D16x).

=== docs/plans/milestone-2.md lines 1884 to 1887: section 7, step 3: which nightly
OLD:
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and a nightly that holds the client's
   recording test has compared it equal on three platforms, with each platform's release baseline
   equal to its dev one. The run ids are in the rows. No check job's median over its three most
   recent warm runs of main is past ten minutes, or the owner has ruled on the budget.
NEW:
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and the first green nightly at or
   after the measurement commit, which holds the client's recording test, has compared it equal on
   three platforms, with each platform's release baseline equal to its dev one (D16x). The run ids
   are in the rows. No check job's median over its three most recent warm runs of main is past ten
   minutes, or the owner has ruled on the budget.

=== docs/plans/milestone-2.md line 1888: section 7, step 4: every row gated at milestone 2
OLD:
4. The coordinator's pass has left every row of section 2 Done or re-gated.
NEW:
4. The coordinator's pass has left every row of `docs/verification.md` whose gate names milestone
   2 Done or re-gated, section 2's rows among them (D16x).


=== docs/verification.md line 21: verification row 21: the Windows item is two debug spans' (lane `park-stamp`)
OLD:
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153). Until the file is read the default is one frame worker: `lactose_app::RECORD_RESERVATION`, which `WorldConfig::shown()` gives the physics configuration (D162). The file is taken at a reservation of one and of two, and on Windows it also gives the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is what the `SetEvent` for each frame worker in `Pool::open_gate` costs a record (the keeper's unmeasured list, read by D162) | 2. **In part** (branch `measure-m2`, round one and its fix round; not the record): `stall::the_record_beside_the_physics_phase` of `lactose-app-client`'s `integration` target records `ClientRender` on null, each record one `run_job` timed from the job's start, its start wait from `run_job`'s call beside it, beside the composed tick timed inside its bounded future; the physics phase is `physics_api::sync`, `step` and `write_back`, from the first entry into any to the last exit, matched by name in the executor's node spans and `lactose-ecs`'s system spans. In the lane's fix-round `--all` run on the M1 (commit 7e3b62a, after fix rounds one and two), at load averages 4.32 5.02 6.16 before and 4.53 5.05 6.16 after, 60 ticks at each reservation: at one, the record's median and P99 beside the physics phase 7.7 and 49.5 µs, their start wait's 1.1 and 20.8 µs, the phase's median 2,502.7 µs; at two, 7.6 and 43.3 µs, the start wait's 1.1 and 19.6 µs, the phase's 2,737.1 µs; with no tick, 6.8 and 37.3 µs, the start wait's 0.8 and 12.3 µs. The Windows item is not measured: it needs a stamp where a frame-critical worker parks and wakes, inside `lactose-task`, whose hooks only its own unit tests compile, so a public surface of `lactose-task` first (D162 addendum at `measure-m2`, round two). `RECORD_RESERVATION` stays one until the quiet window's file is read |
NEW:
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153). Until the file is read the default is one frame worker: `lactose_app::RECORD_RESERVATION`, which `WorldConfig::shown()` gives the physics configuration (D162). The file is taken at a reservation of one and of two, and on Windows it also gives the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is what the `SetEvent` for each frame worker in `Pool::open_gate` costs a record (the keeper's unmeasured list, read by D162) | 2. **In part** (branch `measure-m2`, round one and its fix round; not the record): `stall::the_record_beside_the_physics_phase` of `lactose-app-client`'s `integration` target records `ClientRender` on null, each record one `run_job` timed from the job's start, its start wait from `run_job`'s call beside it, beside the composed tick timed inside its bounded future; the physics phase is `physics_api::sync`, `step` and `write_back`, from the first entry into any to the last exit, matched by name in the executor's node spans and `lactose-ecs`'s system spans. In the lane's fix-round `--all` run on the M1 (commit 7e3b62a, after fix rounds one and two), at load averages 4.32 5.02 6.16 before and 4.53 5.05 6.16 after, 60 ticks at each reservation: at one, the record's median and P99 beside the physics phase 7.7 and 49.5 µs, their start wait's 1.1 and 20.8 µs, the phase's median 2,502.7 µs; at two, 7.6 and 43.3 µs, the start wait's 1.1 and 19.6 µs, the phase's 2,737.1 µs; with no tick, 6.8 and 37.3 µs, the start wait's 0.8 and 12.3 µs. The Windows item is lane `park-stamp`'s (D16x): `lactose-task` emits two debug spans under target `whisk`, `open_gate` from the return of a `block_in_place`'s closure to the return of `Pool::open_gate` and `park` for a worker's wait in the idle protocol, and the test's subscriber gives for each opening the time to the record's next job on a worker that was parked across it, that job's start wait, the opening's own length and the count of openings after which the next record ran on a worker that was awake. The rows are written on every host, and the laptop's and the desktop's Windows files hold the item. No hook and no cargo feature is added, which D162's addendum at `measure-m2`, round two, named as the two forms. `RECORD_RESERVATION` stays one until the quiet window's file is read |

=== docs/verification.md line 150: verification row 150, a fragment of its first cell: the laptop's half. Applied to the row as lane `exit-pass-rows`' merge leaves it, where the fragment still occurs once
OLD (fragment):
and the laptop's four hardware questions are owed
NEW (fragment):
and the laptop's four hardware questions are its session's (D16x): the gate's plain and GPU-assisted runs on each of its GPUs, which answer the ownership transfer's effect where a device gives two roles two queue families, and non-coherent memory by the capabilities each device reports; no case makes a heap indexed without the decoration, or a heap image in the wrong layout, on a device, so those two are recorded as not shown, which discharges no rule of null

=== docs/verification.md line 170: verification row 170: its nightly is not exit step 3's
OLD:
| SOUR hashes come from a dev build (project review 2, X5): a result that depends on debug assertions or overflow checks, such as a stale arena read that poisons only under `debug_assertions` (`crates/core/src/arena.rs`), could differ in release unseen. Lane `exit-tooling` (`1dd5dab`) adds `cargo xtask sour run --release` (nextest `--cargo-profile release`, output in `target/sour-release`); each nightly sour job runs one architecture-mode run with it and compares that baseline with the platform's dev baseline by `sour compare`. Shown on the Apple M1: the release baseline and `sour run --pr`'s dev baseline agree on all 18,446 hashes (load averages 14.72 22.97 15.18 to 13.05 19.79 15.02). The same lane records the bone scene (X4), `lactose_app::cost::bone_bodies_on_rapier_are_recorded` and its `::physics` stream: 20 characters with bone bodies on Rapier over 80 ticks, 0.229 s under nextest (load 1.93 5.73 5.28), among those hashes. **Done**: nightly run 44067232715 at `a539d60`, the first after the merge, held release equal to dev on linux-x64, windows-x64 and macos-arm64 (each platform's `sour compare` of its release baseline with its dev baseline), and its cross-platform job found linux-x64's hashes equal to windows-x64's and to macos-arm64's, the bone scene's streams among them | 2 |
NEW:
| SOUR hashes come from a dev build (project review 2, X5): a result that depends on debug assertions or overflow checks, such as a stale arena read that poisons only under `debug_assertions` (`crates/core/src/arena.rs`), could differ in release unseen. Lane `exit-tooling` (`1dd5dab`) adds `cargo xtask sour run --release` (nextest `--cargo-profile release`, output in `target/sour-release`); each nightly sour job runs one architecture-mode run with it and compares that baseline with the platform's dev baseline by `sour compare`. Shown on the Apple M1: the release baseline and `sour run --pr`'s dev baseline agree on all 18,446 hashes (load averages 14.72 22.97 15.18 to 13.05 19.79 15.02). The same lane records the bone scene (X4), `lactose_app::cost::bone_bodies_on_rapier_are_recorded` and its `::physics` stream: 20 characters with bone bodies on Rapier over 80 ticks, 0.229 s under nextest (load 1.93 5.73 5.28), among those hashes. **Done**: nightly run 44067232715 at `a539d60`, the first after the merge, held release equal to dev on linux-x64, windows-x64 and macos-arm64 (each platform's `sour compare` of its release baseline with its dev baseline), and its cross-platform job found linux-x64's hashes equal to windows-x64's and to macos-arm64's, the bone scene's streams among them | 2. **Done** for release against dev. The nightly of the plan's exit step 3 is a later one, the first green nightly at or after the measurement commit (D16x), and its run id is in the row of the client's recording test |

=== docs/verification.md line 232: verification row 232: the nightly exit step 3 waits for
OLD:
| `lactose-app-client` is both a shipped binary and a harness package (D162: criterion 3(c)'s recorded test is in the module `recorded` of its `integration` target, since that package alone may name the real render side in normal code). `cargo xtask features` then compares its boundary tree in full with its dev-dependencies included, against `lactose-app-headless`'s. Owed by `frame-impl`: the comparison passes with `lactose-rhi-null` and the test's other dev-dependencies in the tree. If a dev-dependency moves the boundary tree, that is a finding for the planner, and D162's placement of the test is what changes **Answered** (branch `frame-impl`, fix rounds 1 and 2). At round two `cargo xtask features` reported six findings, one cause: `bitflags` 2.13.2, in the boundary tree through `lactose-physics-rapier` (`parry3d-f64` builds it without `std`), was built with `std` for `lactose-app-client` and without it for the other roots, on aarch64-apple-darwin and x86_64-unknown-linux-gnu. No dev-dependency moved it, so D162's placement of the test did not: with normal edges alone (`cargo tree -e normal,features -i bitflags`) the window's dependencies ask `std`, `objc2-app-kit` under winit through `lactose-platform` and `objc2-quartz-core` under `raw-window-metal` through `lactose-rhi-vulkan` on macOS, and `rustix` on Linux, so any feature root that links a window moves the tree. `lactose-physics-rapier` names `bitflags` through the workspace, pinned `=2.13.2` with `default-features = false, features = ["std"]`, so every root builds it alike, and `crate-tiers.toml` lists it among the crate's direct dependencies with that reason. `std` keeps a parse error's text and implements `std::error::Error` for it, and changes no flag value, bit operation or iteration order. On the branch `cargo xtask features` and `cargo xtask boundary` both pass, and with them `lactose-xtask`'s `workspace::the_feature_check_passes_with_no_roots_and_reads_real_trees`; `boundary::tests::the_physics_backend_may_not_name_the_solvers_math`, which reads the real tier file, expects `bitflags` on the crate's list with the rest; `cargo xtask vendor` and `cargo deny check` accept the pin. A pin moves with `parry3d-f64`'s `bitflags` | 2 |
NEW:
| `lactose-app-client` is both a shipped binary and a harness package (D162: criterion 3(c)'s recorded test is in the module `recorded` of its `integration` target, since that package alone may name the real render side in normal code). `cargo xtask features` then compares its boundary tree in full with its dev-dependencies included, against `lactose-app-headless`'s. Owed by `frame-impl`: the comparison passes with `lactose-rhi-null` and the test's other dev-dependencies in the tree. If a dev-dependency moves the boundary tree, that is a finding for the planner, and D162's placement of the test is what changes **Answered** (branch `frame-impl`, fix rounds 1 and 2). At round two `cargo xtask features` reported six findings, one cause: `bitflags` 2.13.2, in the boundary tree through `lactose-physics-rapier` (`parry3d-f64` builds it without `std`), was built with `std` for `lactose-app-client` and without it for the other roots, on aarch64-apple-darwin and x86_64-unknown-linux-gnu. No dev-dependency moved it, so D162's placement of the test did not: with normal edges alone (`cargo tree -e normal,features -i bitflags`) the window's dependencies ask `std`, `objc2-app-kit` under winit through `lactose-platform` and `objc2-quartz-core` under `raw-window-metal` through `lactose-rhi-vulkan` on macOS, and `rustix` on Linux, so any feature root that links a window moves the tree. `lactose-physics-rapier` names `bitflags` through the workspace, pinned `=2.13.2` with `default-features = false, features = ["std"]`, so every root builds it alike, and `crate-tiers.toml` lists it among the crate's direct dependencies with that reason. `std` keeps a parse error's text and implements `std::error::Error` for it, and changes no flag value, bit operation or iteration order. On the branch `cargo xtask features` and `cargo xtask boundary` both pass, and with them `lactose-xtask`'s `workspace::the_feature_check_passes_with_no_roots_and_reads_real_trees`; `boundary::tests::the_physics_backend_may_not_name_the_solvers_math`, which reads the real tier file, expects `bitflags` on the crate's list with the rest; `cargo xtask vendor` and `cargo deny check` accept the pin. A pin moves with `parry3d-f64`'s `bitflags` | 2. **Owed, a nightly's**: the first green nightly at or after the measurement commit (D16x) compares the test's two streams equal on linux-x64, windows-x64 and macos-arm64, with each platform's release baseline equal to its dev one, and its run id is written here. Nightly 45609400690 at `98d11cf`, the last before `ad1a99d`, is older than the test, which merged at `55ffdda` |

=== docs/verification.md line 243: verification row 243: `--adapter`, the pin's lane, and whose run it is
OLD:
| Criterion 1 on the Windows laptop (decision 5.1, D162): `lactose-app-client --fixed 600` on each of the laptop's GPUs, chosen by `VK_LOADER_DRIVERS_SELECT`, under the Khronos layer's core, synchronisation and thread-safety validation, each validator's canary run in a process of its own beside it. **Owed, the owner's**: the commands from a clean checkout are the coordinator's, from lane `frame-impl`, and `cargo xtask gpu` does not run there, since the pinned layer has no Windows build. It passes where every canary reports its own identifier and nothing else, and the run prints one `Outcome` with `counts`, `surely` with every entry above zero and `messages: 0`, and nothing at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live, and every pixel is as the reference rasterised from those placements says. The M1 gave `Counts { drawn: 1044, clear: 195304, margin: 260 }` under both of its drivers; the counts are the reference's, from the placements and the latched camera, so the same are expected. The run's two `info` lines give the device and its queue layout, which say whether the laptop's devices serve the present role from the graphics queue (row 176). `cargo xtask measure triangle` (branch `measure-m2`, round two) is the run on a host the layer is pinned for, and met the criterion on the M1 under both drivers with those counts; `tools/xtask/vulkan-layer.toml` pins no layer for Windows, so the laptop's file needs a pin for `windows-x86_64` first | 2 |
NEW:
| Criterion 1 on the Windows laptop (decision 5.1, D162): `lactose-app-client --fixed 600` on each of the laptop's GPUs, chosen by `--adapter`, which the device's name must contain, under the Khronos layer's core, synchronisation and thread-safety validation, each validator's canary run in a process of its own beside it. **Owed, the laptop's session's** (D16x): `cargo xtask measure triangle --driver system --adapter <name>` on each GPU, once lane `layer-windows` has pinned the layer for Windows; lane `frame-impl`'s by-hand form is dropped. It passes where every canary reports its own identifier and nothing else, and the run prints one `Outcome` with `counts`, `surely` with every entry above zero and `messages: 0`, and nothing at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live, and every pixel is as the reference rasterised from those placements says. The M1 gave `Counts { drawn: 1044, clear: 195304, margin: 260 }` under both of its drivers; the counts are the reference's, from the placements and the latched camera, so the same are expected. The run's two `info` lines give the device and its queue layout, which say whether the laptop's devices serve the present role from the graphics queue (row 176). `cargo xtask measure triangle` (branch `measure-m2`, round two) is the run on a host the layer is pinned for, and met the criterion on the M1 under both drivers with those counts; `tools/xtask/vulkan-layer.toml` pins no layer for Windows at `ad1a99d`; lane `layer-windows` adds the pin (D16x), and the RTX 4070's `triangle` file at the measurement commit is the criterion's proof on Windows | 2 |

=== docs/verification.md: three rows appended at the end of the file, after the table's last row
NEW:
| The measurement commit of milestone 2 (D16x): the first commit of main that holds every lane that edits a measured crate, `present-encoder` and `park-stamp` the last of them, recorded once `check` and the `measure` workflow are green at it. The quiet window on the M1, the laptop's session and session C on the desktop each check it out, in that order, each read before the next. A file is the record at it, or at a later commit as `-at-<commit>` where the row that cites it gives the diff and why no figure of the file reads it; a figure taken before it is steering. The desktop's first sitting is at `a539d60` (D151) and `eco-qos`'s runs on the laptop are at `a75596b` and its first parent. To record here: the commit's hash; for each sitting its date, its load and the files it wrote; the ids of the `measure` workflow's run and of the first green nightly at or after the commit | 2. **Owed**: no such commit exists at `ad1a99d` |
| The Khronos layer on Windows is pinned as installed (D16x, D153; lane `layer-windows`): `library_sha256` is the SHA-256 of `Bin/VkLayer_khronos_validation.dll` as read on the development Mac out of LunarG's installer `vulkansdk-windows-X64-1.4.363.1.exe`, which was fetched by the SHA-256 LunarG publishes for it and never run. No Windows host has checked it. Four things the laptop's session shows, and nothing before it: that the file the installer leaves is the archived one, which the tool's own digest check answers at its first run; that the system's loader takes the layer's manifest from `VK_ADD_IMPLICIT_LAYER_PATH` in a shell that is not elevated, the loader's documentation saying it ignores such variables in an elevated process; that each of the seven canaries is heard on each of the laptop's GPUs; and which implicit layers the laptop's registry adds to a validated run, read from `vulkaninfo`'s list and recorded. A digest that differs is a finding for the planner, and the pin then takes the installed file's digest on the owner's word, as the Mac's was taken. The desktop's Windows is the second host to check it | 2 |
| `lactose-task`'s two debug spans (D16x; lane `park-stamp`): `open_gate` and `park`, target `whisk`, which `record-stall` reads. Owed by the lane: `cargo xtask measure whisk` before and after on the M1, the 60 Hz row and the wake row within the two before runs' spread, as steering; the loom models, Miri and the cross-target clippy runs. Owed by the quiet window: the WHISK file at the measurement commit, the first quiet figure of an executor that holds the spans, read against D156 as the plan has it. A row that moved beyond the spread sends the spans back to the planner, and the item is then re-gated to the change that revisits the keeper's wait. `record-stall` fails where a build compiles debug spans out, so a static maximum level for a shipping build is a decision about this measurement, to be taken when such a level is first set | 2 |
~~~

## Decision entry (exact text; number at merge)

**D16x. Milestone 2's sittings: the measurement commit holds every lane that edits a measured
crate, the Khronos layer is pinned on Windows as installed, and the gate's wake is read from two
debug spans** (§5.1, §20, §21, §22, `lactose-task`, `tools/xtask`, `docs/plans/milestone-2.md`)
- Why: the exit pass's audit of main at `ad1a99d` found no measurement commit named, no result
  file of milestone 2 on main, the laptop unable to run criterion 1 for want of a pinned layer,
  and row 21's Windows item unmeasurable for want of a stamp. The plan's section 4 named the
  sittings and not their order, and its session C was one command where the rows owe the desktop
  the device tests and the barrier mutations too.
  - *The measurement commit holds every lane that edits a measured crate.* It is the first commit
    of main that holds `frame-impl`, `measure-m2`, `snapshot-order`, `measure-role`,
    `present-encoder` and `park-stamp`, and its hash is recorded in a verification row once
    `check` and the `measure` workflow are green at it. A measured crate is one a measurement
    builds into what it times, and `tools/xtask` is not one. A file is the record at that
    commit, or at a later one as `-at-<commit>` where its row gives the diff and why no figure
    of the file reads it. A figure taken before it is steering: it stays in its row, proves no
    criterion and closes no row. The desktop's first sitting, at `a539d60`, and `eco-qos`'s runs
    on the laptop, at `a75596b` and its first parent, keep their own commits.
  - *The sittings run in one order, and each is read before the next.* The desktop's first
    sitting, which waits for nothing; then at the measurement commit the M1's quiet window, the
    laptop's one session and session C. A criterion that misses on the M1 needs an engine lane,
    and a sitting taken before that lane is steering. The laptop's session is the first run of
    the Windows pin, before the desktop's Windows needs it. The owner's desk time comes last and
    once. The nightly of the exit is the first green one at or after the measurement commit.
  - *Windows takes the layer as installed.* `vulkan-layer.toml` pins the SHA-256 of the SDK's
    `VkLayer_khronos_validation.dll`, read on the development Mac out of LunarG's installer,
    which is fetched by the digest LunarG publishes and never run. The owner installs the SDK,
    and the tool's own digest check on the laptop shows that the installed file is the archived
    one. The run takes the system's loader and drivers. This is D153's "pinned on every host"
    on the host it did not reach.
  - *`rhi-conformance` is a measurement.* `cargo xtask measure rhi-conformance` runs the gate's
    Vulkan step and writes the file criterion 2 names, under the measurements' header and with no
    path of the machine in it. `cargo xtask gpu` keeps its report under the target directory.
  - *The gate's wake is read from two debug spans.* `lactose-task` emits `open_gate`, from the
    return of a `block_in_place`'s closure to the return of `Pool::open_gate`, and `park`, a
    worker's wait in the idle protocol, at debug level under target `whisk`. `record-stall`'s
    subscriber reads them and gives, for each opening, the time to the record's next job on a
    worker that was parked across it. Section 5.1's "Finer levels compile out" covers them,
    section 21's entry for `task` stands, and the test fails where they are compiled out.
  - *The laptop's four hardware questions are the gate's two runs on each GPU.* The plain run and
    the GPU-assisted run answer the ownership transfer and non-coherent memory where the device
    allows. No case makes either heap misuse on a device, so those two are recorded as not
    shown, and null's rule and the cook's check stand (decision 5.17).
  - *The exit reads every row gated at milestone 2.* Section 2 lists the 26 gated at `a561b36`;
    85 are gated at `ad1a99d`. Step 4 reads them all.
  - Rules out: a measurement commit named before a lane that edits a measured crate has merged;
    a file at another commit cited as the record with no diff given; a sitting at the
    measurement commit on the laptop or the desktop before the M1's files are read; a layer on
    Windows taken unchecked from whatever the machine has; a download the tool unpacks by an
    installer's undocumented layout; a public hook in `lactose-task` for a measurement, and a
    cargo feature for one; a probe program written for row 150's two heap questions; a window
    measurement run on a system `lactose-platform` has not been run on.
- Driven by: the exit pass's audit at `ad1a99d`; `docs/proposals/milestone-2-sittings.md` and its
  probe of LunarG's installer; `reviewer-gpu`'s reading for row 34; the owner's ruling.
- Replaced: in the plan's section 3, the measurement commit as "the first commit of main that
  holds `frame-impl` and `measure-m2`"; in its section 4, `cargo xtask gpu --driver <name>` as
  what writes `rhi-conformance`, session C as `cargo xtask measure --all` alone, and the window
  measurements "skipped" on a system that is not a client platform; in its section 7, step 4's
  "every row of section 2"; in D162's addendum of `measure-m2`, round two, the two forms of a
  stamp it names, two process-wide hooks and the same behind a cargo feature.

## The owner's questions

As `AskUserQuestion` input, the recommended option first.

~~~json
{
  "questions": [
    {
      "question": "Row 21's Windows item is what opening the physics gate costs the render record: one SetEvent for each frame worker after every physics phase. It cannot be measured today, because both instants are inside `lactose-task`. How is it stamped?",
      "header": "Row 21",
      "multiSelect": false,
      "options": [
        {
          "label": "Two debug spans (Recommended)",
          "description": "Lane `park-stamp`, about six agent runs, before the measurement commit: two tracing spans at debug level, which `record-stall` reads through the subscriber it already has. No public function, no `unsafe`, no architecture change. The measurement commit waits for it, about a day more."
        },
        {
          "label": "Re-gate the item",
          "description": "No lane. The item moves to the change that revisits the keeper's wait, and the record's default reservation is set from the M1's and the desktop's files without the Windows cost. The measurement commit waits for `present-encoder` alone."
        },
        {
          "label": "Two public hooks",
          "description": "The form D162's addendum names: two process-wide hooks in `lactose-task`, always compiled, with a contract for what a hook may do inside the idle protocol. About eight agent runs and more surface, for the same rows."
        }
      ]
    },
    {
      "question": "Criterion 1 on Windows needs the Khronos validation layer pinned there, and no Windows machine is reachable from the Mac. How is the layer pinned on the laptop and the desktop's Windows?",
      "header": "Windows pin",
      "multiSelect": false,
      "options": [
        {
          "label": "Installed SDK (Recommended)",
          "description": "You install LunarG's Vulkan SDK 1.4.363.1 once on each Windows, as administrator. The tool refuses a layer whose SHA-256 is not the pin's, and the pin was read on the Mac out of the same installer, never run. Lane `layer-windows`, about six agent runs."
        },
        {
          "label": "Tool fetches and unpacks",
          "description": "No install and no administrator. The tool downloads the 303 MB installer by digest and cuts the layer out. It rests on the installer's internal layout and on Windows' own tar reading 7z, neither shown yet. About eight agent runs."
        },
        {
          "label": "No pin, by hand",
          "description": "No lane. The triangle on Windows is run by hand with whatever layer is installed, its result is not the `triangle` file the plan names, and `cargo xtask gpu` never runs on Windows: the laptop's and the desktop's device rows stay open or are re-gated."
        }
      ]
    },
    {
      "question": "Three sittings read the measurement commit: the Mac's quiet window (about an hour, the Mac unused), the laptop's session (an install as administrator, then its agent session) and your session C at the desktop. In what order?",
      "header": "Order",
      "multiSelect": false,
      "options": [
        {
          "label": "One after another (Recommended)",
          "description": "The Mac, then the laptop, then the desktop, each read before the next. A criterion that misses on the Mac, or a defect of the Windows pin found on the laptop, costs no later sitting. About three days from the commit to the last file."
        },
        {
          "label": "All in one day",
          "description": "The three run without waiting for each other's reading. A miss that needs an engine fix then makes the other two machines' files steering, and you sit at the desktop a second time."
        }
      ]
    },
    {
      "question": "Row 150 leaves four questions of the RHI spike to the laptop's GPUs. The device tests answer two of them. No test makes the other two misuses on a device: a heap indexed without the non-uniform decoration, and a heap image in the wrong layout. What does the laptop run for those two?",
      "header": "Row 150",
      "multiSelect": false,
      "options": [
        {
          "label": "The gate's runs (Recommended)",
          "description": "No lane. The laptop runs `cargo xtask gpu`, plain and GPU-assisted, on each GPU. The two heap questions are recorded as not shown, and the null backend's rule and the cook's check stay the only guards, as decision 5.17 already has it."
        },
        {
          "label": "A probe lane",
          "description": "A lane writes the two misuses as device probes, one from a shader the cook refuses, about seven agent runs with the GPU reviewer, and the laptop runs them. It gives labelled data on two drivers and proves no rule either way if they show nothing."
        }
      ]
    }
  ]
}
~~~

The desktop's first sitting is not a question. The plan has two desktop sittings (decision 5.19,
D151's addendum), the first one's commands are written, and it is the one sitting that can run
today.

## Cost and order

| Step | Who | Agent runs |
|---|---|---|
| This proposal's review, its answer and the answer's review | `reviewer`, `planner`, `reviewer-diff` | 3 |
| The decision, the plan's lines and the rows, applied after `exit-pass-rows` merges | The coordinator | None |
| The desktop's first sitting | The owner | None |
| The rows gated at milestone 2 that nothing has read, 37 of them | One read-only agent | 1 |
| `present-encoder`'s fix round and merge | The lane, as briefed | Already counted |
| `layer-windows`: the pin, `rhi-conformance`, the script | A lane from main, `reviewer` and `reviewer-gpu`, `reviewer-diff`, `merger` | 6 |
| `park-stamp`: the two spans, `record-stall`'s rows | A lane from main, `reviewer` and `reviewer-concurrency`, `reviewer-diff`, `merger` | 6 |
| The nightly and the `measure` workflow at the commit | The coordinator, by dispatch | None |
| The quiet window | The coordinator, with every agent stopped | None |
| The laptop's session | The laptop's own agent session | 1 |
| Session C | The owner | None |
| The files checked in and the rows written | The coordinator's pass, or a lane as `exit-pass-rows` is | 2 |
| The freeze decision, the exit's read and the milestone 3 plan | As section 7 has them | Already counted |

About nineteen agent runs are new. The three lanes run in parallel and touch different crates:
`crates/app-client/src`, `tools/xtask`, and `crates/task` with `crates/app-client/tests/stall.rs`.
`park-stamp` and `layer-windows` both add to `docs/results/README.md`, and the second to merge
takes the first's.

## What this proposal could not verify

- That the file LunarG's installer leaves on a Windows disk is the file in its archive. The digest
  was read from the archive on the Mac. The laptop's first run is the check, and the pin's row
  says what follows a difference.
- The Windows loader's behaviour: that it takes an implicit layer's manifest from
  `VK_ADD_IMPLICIT_LAYER_PATH`, that it ignores the variable in an elevated process, and which
  implicit layers the laptop's registry adds to a run. The second is from the loader's
  documentation as remembered, not from a page read for this proposal. So is the installer
  setting `VULKAN_SDK` and installing to `C:\VulkanSDK\<version>`.
- Whether the Vulkan SDK is already on the laptop, and at which version.
- The desktop: its CPU, its GPU, its second system, and its devices' queue families. Rows 212
  and 244 need a device whose present family is not its graphics family, and row 214 a family
  without the graphics bit. The first sitting's `vulkaninfo` output says whether the desktop has
  them, and a row it cannot run is the exit read's to re-gate.
- The laptop's second device. The plan expects an Intel integrated GPU and none was listed
  (section 3's table of machines). `vulkaninfo`'s list at the session's start answers, and with
  one device the session runs once.
- How long `cargo xtask measure --all` runs at eighteen measurements. "About an hour" on the M1 is
  an estimate from the lanes' runs on a loaded machine.
- That Windows' own `tar` reads a 7z archive. Option B of the pin rests on it, and nothing here
  ran on Windows.
- The cost of the two spans. The crate's README gives a span about 1 ns with no subscriber. The
  lane measures `whisk` before and after, and the quiet window's file is the first quiet figure.
- `present-encoder`'s second gate step, `cargo xtask gpu --client`, which its fix round adds. The
  commands files name it as that lane's and its form is checked at the merge.
- The 37 rows gated at milestone 2 that neither the audit nor this proposal read. One of them may
  owe a machine something no commands file holds yet.
- Any figure. Every figure quoted is a row's or a lane's.
