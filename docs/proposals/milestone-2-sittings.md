# Proposal: the sittings that close milestone 2

Status: proposed, revised after its two reviews and again after the diff review of that revision,
for the owner. It answers the exit pass's audit of main at `ad1a99d` (26 November 2026): what stands
between main and milestone 2's exit (`docs/plans/milestone-2.md`, section 7), and in what order it
is done. It settles four things: the measurement commit, the order of the sittings and what each
asks of the owner, the Khronos layer's pin on Windows, and the stamp verification row 21's Windows
item waits for. The owner's five questions are near the end, in two calls, and after them what the
reviews changed. It amends no sentence of `docs/architecture.md`. Approved as recommended, it is
applied as one decision entry, the plan's and the verification rows' text below, and two lanes.

This proposal ran no measurement and no device. It read the tree, the audit, `reviewer-gpu`'s
reading for row 34, the two reviews with their probes and the diff review of the second version. It
ran two probes of its own: the Windows layer's digest, read on the development Mac out of LunarG's
installer, which was fetched and never run; and a detached power assertion, to see that it outlives
the shell that starts it. The rows below are built against main at `b0c5f44`.

## What stands between main and the exit

| What section 7 asks | State at `b0c5f44` |
|---|---|
| Step 1: a measurement commit, and the quiet window, the laptop's session and session C on it | No commit is named. No result file of milestone 2 is on main |
| Step 1: the desktop's first sitting | Not run. Its commands are written, at `a539d60` (D151, decision 5.19), and are revised with this proposal to ask for what the later sittings read from it |
| Step 3: `gpu (lavapipe)` and `shaders` green, no check job's median past ten minutes | Met. Lane `exit-pass-rows` wrote the run ids into the rows |
| Step 3: a nightly that holds the client's recording test | Not run. The last nightly, 45609400690 at `98d11cf`, is older than the test, which merged with `frame-impl` at `55ffdda` |
| Step 4: section 2's 26 rows Done or re-gated | Five were the pass's to write (35, 95, 99, 134, 170): `exit-pass-rows` merged at `b0c5f44`, and 170 takes this proposal's block. Three the laptop's (63, 109, 150). Six the first sitting's (64, 77, 80, 81, 164, 169). Six the measurement commit's files (15, 16, 19, 21, 34, 102). One the freeze decision's (18). Five re-gated already (97, 136, 139, 140, 143) |
| Criterion 1 on Windows | Cannot run. `tools/xtask/vulkan-layer.toml` pins no layer for Windows, and `triangle` and `cargo xtask gpu` both stop at `vulkan::host` |
| Row 21's Windows item | Cannot be measured. Both of its instants are inside `lactose-task` (D162's addendum of `measure-m2`, round two) |
| Lane `present-encoder` | In its second fix round. It edits the client's present call, which `triangle` runs through and `frame-pace` reads |

**What the audit did not read.** Six things, found while placing the sittings and in the reviews.

- *Section 2 lists 26 rows, and 85 rows of `docs/verification.md` are gated at milestone 2.* The 26
  are those gated at `a561b36`, and 21 of them still are. The other 64 were opened by the plan and
  its lanes. Exit step 4 names section 2's rows alone, and the audit read those. By their own text
  27 of the 64 owe a run on the laptop or the desktop: 176, 189, 190, 208 to 210, 228, 229, 234,
  236, 243, 246 and 247 the laptop's; 212, 214, 216, 218 to 220, 222, 224, 225, 238 to 241 and 244
  the desktop's. This proposal places those. The other 37 it read by their heads and status marks
  alone, and none against the tree.
- *No line of `lactose-rhi-vulkan` has run on a Windows driver.* Rows 210 and 236 say of Win32's
  surface "not yet run anywhere", and `cargo xtask gpu` stops on Windows for want of the pin. The
  first run there is the step of this plan most likely to find a defect of the engine, and such a
  defect's fix changes what every device measurement reads.
- *Nothing writes `rhi-conformance`.* Section 4 says `cargo xtask gpu --driver <name>` writes
  `rhi-conformance-<driver>-<machine>-<date>.md`, and criterion 2's proof and row 171 name the file.
  The command writes `target/gpu/<driver>-vulkan[-gpu-assisted].txt` (`tools/xtask/src/main.rs`,
  `run_vulkan`). Its lines hold the SDK's and the environment's paths as the machine has them, and
  under `--driver system` a second adapter's run overwrites the first's.
- *Session C as the plan has it is one command.* The rows owe the desktop more than `cargo xtask
  measure --all`: the device tests on a desktop's driver (rows 212, 214, 216, 218, 219, 222, 224 and
  244, two of them on a device whose present family is not its graphics family), the renderer's cost
  tests (rows 220 and 238) and the three barrier mutations (rows 225 and 239 to 241). The mutation
  script is in no checkout: lane `renderer-impl` left it outside the repository. Its verdict reads
  the gate's console output for a line the gate has not printed since `7424e7c`, so every mutation
  would read as caught. Row 244's probe is a build of a scratch copy, in no checkout either.
- *Nothing skips a window on a system that is not a client platform.* Section 4 says the
  measurements that need a window are skipped there and the file says so. `cargo xtask measure` has
  no such rule, and on Linux `lactose-platform` "builds over X11 and is not run" (its README,
  "Status").
- *Row 243 names a selection the tool does not make.* It has the laptop's GPUs "chosen by
  `VK_LOADER_DRIVERS_SELECT`". A run clears every `VK_LOADER_*` variable
  (`tools/xtask/src/vulkan.rs`, `CLEARED_PREFIXES`) and chooses the device by `--adapter`, whose
  name the device's name must contain (`crates/rhi-vulkan/README.md`, `VulkanDevice::new`).

## The measurement commit

**The decision.** The measurement commit is the first commit of main that holds the three lanes that
change a measured input after `measure-role`: `present-encoder`, `park-stamp` and `layer-windows`.
The coordinator records its hash in a verification row of its own, which this proposal adds, once
three things hold: `check` is green at it, the `measure` workflow is green at it, and the laptop's
first session has run the Windows pin on each of its GPUs at `layer-windows`' last merge. A failure
of any of the three is a fix round of the lane it names, and the commit is then that round's merge.

**A measured input** is anything a measurement builds, runs or computes a figure with. Among them:
the crates under `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and layer pins,
`shaders/`, `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`,
`.config/nextest.toml` and `.gitattributes`. Where the list is short the definition holds. The
second version's list ended at `.cargo/config.toml` and read as complete. It was not: every
measurement that is a test runs as `cargo nextest run` (`measure.rs`, `nextest_args`), under the
timeout and the kill `.config/nextest.toml` sets, and `.gitattributes` sets the bytes a Windows
checkout gives the shaders and the sources. The first version of this proposal said "a measured
crate" and held `tools/xtask` outside it. That was wrong. `tools/xtask` is `edit-to-playing`'s
stopwatch (`measure.rs`), computes `frame-pace`'s figures (`pace.rs`), cooks the shaders every
device measurement draws with, and sets every device run's environment (`vulkan.rs`).
`layer-windows` changes that environment on every host, so it is in the commit by the rule and not
by preference.

**How a row names it.** "The measurement commit", with its short hash beside it once it exists. A
file is the record at that commit. A file of a later commit is the record as `-at-<commit>`
(`docs/results/README.md`) only where the diff from the measurement commit changes no measured input
the file's measurement reads, and the row that cites the file gives the diff. Otherwise the file is
steering until the owner's ruling at section 7's step 7 names the record.

**What a figure taken before it is.** The rule is about section 4's measurements and their files of
record. A figure one of them gave before the commit is steering: it stays in the row it steered,
proves no criterion and stands for no file of record. The lanes' runs at `7e3b62a` and `16f45fc` in
rows 21 and 176 are such figures, and so is any measurement run at `b0c5f44`. A row that asks a
machine for a run and names no commit is another thing: the run closes it, and the row records the
commit it ran at. Three sittings so keep commits of their own: the desktop's first sitting at
`a539d60` (D151's addendum); the before-and-after runs of `eco-qos` at `a75596b` and its first
parent (row 63); and the laptop's first session at `layer-windows`' merge, for rows 109, 189, 208 to
210, 234 and 236 and the laptop's half of 190 and 247. None of those names a commit, and row 236
asks for "the merged head". The second version said a figure before the commit "closes no row",
which its own table of sittings contradicted. The laptop's first session writes no measurement file
but row 63's: its short device runs go to the target directory and are thrown away.

**After it.** A lane that merges later and changes a measured input runs the measurements it touches
and reports them as steering. Where it answers a finding of a sitting, the owner's ruling on the
finding says which file is the record (section 7, step 7).

**The nightly.** Exit step 3's nightly is the first at or after the measurement commit whose four
`sour` jobs pass: `sour (linux-x64)`, `sour (windows-x64)`, `sour (macos-arm64)`, and `sour
(architecture)`, which compares the three. Those jobs are what step 3 asks: the recording test equal
on three platforms, and each platform's release baseline equal to its dev one. Another job of that
run that fails is a finding in its own row and holds nothing of step 3. The first version said "the
first green nightly", which is stricter than the step: its review found a nightly that failed on
`gpu (lavapipe, GPU-assisted)` alone. `park-stamp` edits the executor and `present-encoder` edits
the package that holds the recording test, so a nightly before them compares a tree the exit does
not ship. The coordinator dispatches the nightly at the commit if the schedule did not run on it,
and the `measure` workflow with it, as section 9's step 6 did at the re-run's commit. The `measure`
workflow there is the first run of `record-stall` on Windows with `park-stamp`'s spans. Its figures
are a runner's and are discarded (`.github/workflows/measure.yml`).

| Option | What it rules out |
|---|---|
| A. The commit holds the three lanes and is recorded after the Windows pin's first run (recommended) | Sittings at the commit before the lanes merge and the laptop has been opened once: about two days |
| B. Main's head now. The lanes merge after it | One commit for the record. `present-encoder` then changes what `frame-pace` and `triangle` measured, `park-stamp` changes the executor under all eighteen measurements and `layer-windows` every device run's environment: each is run again as steering, and the files the criteria rest on are of a tree the exit does not ship |
| C. The commit holds `present-encoder` and `layer-windows` alone | Row 21's Windows item measured at the commit. It is the right commit if the owner re-gates that item, and this proposal's text then drops `park-stamp` from the list |
| D. Recorded before any Windows run, as the first version had it | A defect of the first Windows run found before the record is taken. Its fix moves the commit and the M1's hour is run again, or the owner rules that the M1's files stand. It is the third question's second option |

**What would change it.** `park-stamp` running past two days or failing its review: the commit is
taken without it, and its item is re-gated. The other two cannot be left out: `present-encoder`
changes the client the criteria run, and without `layer-windows` nothing validated runs on Windows.

## The sittings, in order

Five sittings and two things that need no machine.

| Order | Sitting | Machine and commit | What it needs from the owner | What it writes | Rows it closes or advances |
|---|---|---|---|---|---|
| Now | The desktop's first sitting | The desktop, under each system it boots, at `a539d60` | The owner at the desk for all of it. Mains power. Git, rustup, a C toolchain, nextest. Under Windows, as administrator: `LocalDumps` before the run and, after it, the Vulkan SDK 1.4.363.1 installed where the second question is answered as recommended, so session C needs no second setup | Milestone 1's twelve files for each system, the hash file `cargo xtask sour run --emit` writes, nextest's summary for `lactose-task`. The owner returns the CPU model, the systems, the GPU and `vulkaninfo`'s whole output; the `whisk` file's topology row says whether the processor is a hybrid one (row 80) | 64, 77, 80, 81, 164, 169; the desktop's half of 190; the desktop's queue families and memory types for 176 and 246; the facts decision 5.19 wants, which make session C's commands exact |
| Now | The lanes, and the rows not yet read | None | Nothing | `present-encoder`, `layer-windows` and `park-stamp` in parallel. One read-only pass over the 37 rows gated at milestone 2 that neither the audit nor this proposal read, so that what they owe a machine is in a commands file before the commit | None yet |
| 1 | The laptop's first session | The laptop, at `layer-windows`' merge; row 63's runs at `a75596b` and its first parent | Once, as administrator: the SDK's installer and `LocalDumps`. Then the laptop on mains, logged in, its screen unlocked, its screen and sleep set to Never for the session and put back after it, and the agent session opened in a shell that is not elevated, with the session's brief. The session is about three hours, an estimate. One hand in its first ten minutes: a window dragged by its title bar (row 209). The display's refresh rate read out once | Row 63's three `whisk` files, at their own commits. Nothing else of record: the gate's reports, plain and GPU-assisted, on each GPU; a short run of the four device measurements, thrown away; the loader's implicit layers, and each GPU's queue families and memory types; the window cases' output and the latency tool's lines; the three mutations' outcome on the RTX 4070, as labelled data | 63, 109, 189, 208, 209, 210, 234, 236; the laptop's half of 190 and 247; the machine's facts for 150, 176 and 246; the pin's row. Each row it closes records `layer-windows`' merge as its commit. The measurement commit is not recorded before it |
| 2 | The quiet window | The M1, at the measurement commit | The Mac on mains, awake, its screen unlocked, and unused for about an hour. One hand at the start: a window resized by its corner for a few seconds (row 209) | Eighteen measurements' files: milestone 1's twelve again, `extract-50k`, `remote-upload-5k` and `record-stall`, and `triangle`, `gpu-cost` and `frame-pace` under MoltenVK and under KosmicKrisp. The `rhi-conformance` files, under both drivers and under KosmicKrisp with GPU-assisted validation. The renderer's cost tests' figures, into their rows. The system daemon's CPU time before and after | The M1's half of 15, 16, 19, 21, 34, 102, 176, 192, 220, 228, 229 and 238; 171's files; 173's tick start read again; 209's M1 half |
| 3 | The laptop's session at the commit | The laptop, at the measurement commit | The laptop as before, its screen and sleep set to Never again and put back after, and the agent session opened in a shell that is not elevated. Nothing else | All eighteen measurements on the RTX 4070 as `laptop-windows`, its `triangle` file criterion 1's proof on Windows. `triangle`, `gpu-cost` and `frame-pace` on the integrated GPU as `laptop-windows-intel`. `rhi-conformance`, plain and GPU-assisted, on each GPU. `present-encoder`'s `cargo xtask gpu --client` on each GPU, as labelled data | 243; 150's files; the laptop's half of 21, 164's two items that `frame-pace` reads, 176, 228 and 229; labelled figures for 15, 16, 19 and 102 |
| 4 | Session C | The desktop, under each system it boots, at the measurement commit; row 63's runs at their own commits if its processor is a hybrid one | The owner at the desk for all of it. Mains power. Python 3 for the mutation script | All eighteen measurements under Windows; under a system that is not a client platform, the sixteen that need no window. `rhi-conformance`, plain and GPU-assisted, on each GPU. `cargo xtask gpu --client`. The renderer's cost tests' figures. The three mutations' outcome, each beside its control | The x86 desktop's half of 15, 16, 19 and 102; 176; 212, 214, 216, 218 to 220, 222, 224, 225, 238 to 241 and 244; 246 and 247; 63's desktop half if it has one |
| After | Files, rows and the exit's steps | None | The rulings of step 7 | The coordinator checks each machine's files in, scanned for paths as a public repository needs. The pass writes the rows. The first nightly at or after the commit whose `sour` jobs pass gives rows 170 and 232 their run id. Then section 7's steps 5 to 8 | Every row gated at milestone 2, Done or re-gated |

**Why this order.** The desktop's first sitting waits for nothing: its commit is fixed, its commands
are written and revised with this proposal, and what it returns of the desktop's GPU and second
system is what session C's commands need. The laptop's first session comes before the commit is
recorded because it is the first run of `lactose-rhi-vulkan` on a Windows driver and the first run
of the Windows pin. A defect found there is a fix of a measured input, and found before the commit
it costs no file of record. It waits for `layer-windows` alone. That lane starts once
`present-encoder` has merged, so it is the last of the three to start, and the session runs while
`park-stamp` finishes at most. It is on the commit's critical path: the commit waits for it, and for
a fix and a second run of the pin if it finds a defect. The quiet window is first of the three at
the commit because it is the cheapest and it reads the M1's half of every criterion a result file
proves: a criterion that misses there needs an engine lane, and the files of a laptop's session and
a desk sitting taken before that lane would be steering. The laptop is before the desktop because it
needs no owner. Session C is last because the owner's time at a desk is the scarcest thing in the
plan, and it then runs once, on a tree two machines have measured and with a mutation script that
has run on Windows once.

**The laptop is opened twice.** The owner is needed at the first session's start, as administrator
and then for about ten minutes, and before each session sets the laptop's screen and sleep to Never
and puts them back after it. The session at the commit asks for nothing else. The first session
holds, in this order. While the owner is still there: the checkouts' refresh (row 109), before
anything is built; the installed manifest's version, the installed layer's digest, the loader's
implicit layers and the devices' facts; and the latency tool, which is the one step that needs a
hand (row 209). Then, with nobody at it: the gate, plain and GPU-assisted, on each GPU, which fails
in its first minutes if the tool cannot use the pin; a short run of the four device measurements
under each GPU's machine name, which rehearses the measured environment and the files' names and is
thrown away; row 63's three runs at their own commits; the window cases (rows 208, 210, 234 and
236), fifteen on Windows; and the three mutations on the RTX 4070. Its session at the commit holds
the measurements and nothing the owner must attend. `frame-impl`'s by-hand form of criterion 1 is
dropped: `cargo xtask measure triangle` is that run once the pin exists (row 243). The first version
asked the laptop once. That put the first Windows run of the engine after the M1's hour, where a fix
of what it found would move the commit.

**Row 150's four hardware questions are not shown.** The spike's file leaves four to hardware
(`docs/results/spike-rhi-apple-m1-2026-11-15.md`, "Design questions for the lavapipe run"): an
ownership transfer's effect, memory that is not coherent, a heap indexed without the non-uniform
decoration, and a heap image in the wrong layout. The plan's table of what the spike left open says
of two of them: "Probed on the laptop's devices and under GPU-assisted validation, as labelled data
(decision 5.17)". No probe of any of the four exists. `ownership_transfer_keeps_the_contents`
performs the release and the acquire and checks the contents
(`crates/rhi-api/src/conformance/outputs.rs`). It never drops the transfer, so it cannot show what a
dropped one does, and where compute has no queue family of its own `lactose-rhi-vulkan` gives it a
second queue of the graphics family (`crates/rhi-vulkan/src/raw.rs`, `families`), across which there
is nothing to transfer. `unflushed_range_at_a_submit_is_refused` skips where `coherent_host_memory`
is set (`conformance/buffers.rs`), and `MemoryTypes::choose` takes a coherent host-visible type
wherever a device offers one, so the capability is set on every device that offers one. The cook
refuses an undecorated heap index (`tools/cook/README.md`, "The checks"), and null's rule alone
holds the wrong layout. The first version said the gate's two runs "answer the first two where the
device allows". That was wrong.

The recommendation is that row 150 and the plan's table say so: all four are recorded as not shown,
and "a run that shows nothing is recorded as that and discharges no rule" (decision 5.17). The
gate's runs on each GPU stay: they are criterion 2's evidence on those drivers. Each sitting records
what bears on the four and costs nothing: each device's queue families, so that a later reader knows
whether the ownership case crossed two families, and its memory types, so that a host-visible type
that is not coherent is known where one exists. The alternative is a lane that writes the four as
device probes: the transfer dropped across two families, a host-visible type forced non-coherent and
left unflushed, an undecorated heap index past the cook, and a heap image in the wrong layout. It is
about ten agent runs with `reviewer-gpu`, it edits `lactose-rhi-vulkan`, which is a measured input,
so the commit waits for it, and what it gives is labelled data on two drivers that proves no rule
either way. It is the owner's fourth question.

**The row of `frame-impl`'s runs on x86 (row 244).** It owes the desktop criterion 1 and "the
probe's paced runs under synchronisation validation", and lavapipe a run. The probe is "a probe
build of a scratch copy" (`crates/app-client/README.md`): no checkout holds it, so session C cannot
run it. No runner of lavapipe's presents to a window. Session C runs what a checkout holds: `cargo
xtask measure triangle`, which is criterion 1, and `present-encoder`'s `cargo xtask gpu --client`,
which starts the release client on a window under the gate's validators, as shipped and with the GPU
made to run behind the present thread. That is none of the probe's three faults: a present call that
sleeps 40 ms, every 13th making no submit, and every 97th draw torn (`crates/app-client/README.md`).
The probe's paced runs and lavapipe's half move to a row of their own, gated at milestone 3: the
first change after the exit that edits the client's present path checks the probe in or says why
not. The alternative is a lane now that adds the probe's three faults to the client behind a switch:
about six agent runs, which are the lane, `reviewer` and `reviewer-gpu`, a fix round,
`reviewer-diff` and `merger`, in a measured input, so the commit waits for it. It gives the
desktop's paced runs and not lavapipe's half. The re-gate is the recommendation, and it is the
owner's fifth question. The second version had the decision entry take it with no question. It is a
cut of an exit row's scope with a costed alternative that holds the commit, which is what the fourth
question is for row 150, so it is asked.

**A second GPU on one machine.** `LACTOSE_MEASURE_MACHINE` names the machine in a file's name
(`docs/results/README.md`). The laptop's RTX 4070 runs as `laptop-windows`, the name its files of 15
November carry, and the integrated GPU as `laptop-windows-intel`, each with `--adapter`. The desktop
runs as `desktop-windows` and `desktop-<system>`, in its first sitting as in session C: the first
sitting's commands did not name the machine, and its two systems' files of one day would have shared
a name. A run under the wrong name overwrites the other GPU's file of the same day (`measure.rs`,
`write`). The guard is in the commands. Each GPU's run is one command that sets the name itself,
carries the adapter's name written out, copies the files it wrote into the target directory and
counts them; after the second GPU's run the first one's files are compared with their copies; and
the `rhi-conformance` file's header names its adapter. The first session's short run rehearses all
of it. The second version set the name in one command and ran in another. That rests on a shell that
keeps its state between an agent's calls, which the laptop's session may not have, and a name that
is lost is silent: both GPUs' files take the machine's default name and the second run overwrites
the first. For the same reason no command rests on a changed directory: each laptop session detaches
the clone itself at its commit, where the second version made a worktree beside it.

**A system that is not a client platform.** Session C there names its measurements and leaves out
`triangle` and `frame-pace`. Nothing is built for it: the plan's sentence is amended to say so.

**What would change the order.** The owner preferring the laptop opened once: the first version's
order, with the risk named above. Or the owner preferring one day to three: the three sittings at
the commit then run without waiting for each other's reading, and a miss that needs an engine lane
costs the other two machines a second sitting. It is the owner's third question.

## The Khronos layer on Windows

D153 has the layer "pinned on every host". `vulkan-layer.toml` pins Linux by an archive the tool
fetches by digest and the Mac by the digest of the library the owner's installer leaves.

**The decision.** Windows takes the Mac's form. The pin is the SHA-256 of
`Bin\VkLayer_khronos_validation.dll` of the SDK the owner installs. The tool finds the SDK at
`VULKAN_SDK`, which LunarG's installer sets, or at `C:\VulkanSDK\<version>`. It reads the manifest
beside the library, `Bin\VkLayer_khronos_validation.json`, for the layer's name and version before
it takes the digest, so that another SDK at `VULKAN_SDK` is named as another version and not as a
wrong file. It takes the system's loader and the system's drivers, so `--driver system` is the only
driver there, and it sets no library path.

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
Installer Framework installer that carries ten 7z archives. The first holds
`Bin/VkLayer_khronos_validation.dll`, 21,867,448 bytes, and its manifest, which names
`VK_LAYER_KHRONOS_validation` at `1.4.363` with the library beside it. This proposal's probe fetched
the installer, checked its digest against the list, cut the archive out by its 7z signature and
header, and read the two files with the Mac's own `bsdtar`. Nothing of the installer was run.
`reviewer-gpu` repeated the derivation and got the same digest, found the file signed by LunarG, and
found it the installer's only copy. The lane derives it a third time by the same steps and writes
them into `tools/xtask/README.md`.

**What the installer does to the machine.** It needs an administrator. Beside the SDK's folder it
installs LunarG's Vulkan runtime for the whole machine, which is the loader every Vulkan program
there then finds; it sets `VULKAN_SDK` and the path for every user; and it registers the SDK's
explicit layers in the registry. The layer's library needs the Visual C++ runtime on the machine.
The owner's second question says what the install does.

**Which layers join a run.** Today a validated run adds its layers directory with
`VK_ADD_IMPLICIT_LAYER_PATH`, and a measured run sets `VK_LOADER_LAYERS_DISABLE=~implicit~`
(`tools/xtask/src/vulkan.rs`, `Run::environment` and `Run::measured`). On Windows both fail, as
`reviewer-gpu` read in the loader's source at the pinned SDK's tag. The first adds to the registry's
implicit layers, so a layer a driver or an overlay registered joins every validated run. The second
makes the loader warn once for each implicit layer it disabled, the device hears the warning as a
message, and a measured run that hears a message fails (`crates/app-client/tests/timing.rs`,
`Messages::assert_none`). The lane sets the exclusive `VK_IMPLICIT_LAYER_PATH` in place of both: the
tool's layers directory for a validated run, and its empty directory for a measured one. It does so
on every host, so that the form the laptop runs is one the M1 and CI have run. The device files'
Validation line, which names `~implicit~` today (`measure.rs`), follows. The loader ignores every
such variable in an elevated process, so the session's shell that is not elevated is what makes the
pin work, and each session checks it.

**How it is checked.** On the laptop, by the tool itself: `vulkan::sdk` refuses a manifest of
another version and then a library whose digest is not the pin's, so the first `cargo xtask gpu
--driver system` is the check. A digest that differs is a finding for the planner, and the session
re-pins nothing. A second pin is proposed only for a file whose version is 1.4.363.x and whose
Authenticode signature is LunarG's, and it is the owner's to approve.

| | The Mac can do it | Only the laptop can |
|---|---|---|
| The pin | Its form and parse; the digest out of the installer; the paths, the environment, the manifest's check and the implicit manifest's JSON for Windows as unit tests; clippy for `x86_64-pc-windows-msvc` | Run the installer, which needs an administrator. Show that the installed file is the archived one |
| The loader | Show that the exclusive path gives the M1's and CI's runs no message and no lost canary | Show that the system's loader, at the version the session records, takes the manifest from the exclusive path in a shell that is not elevated and loads nothing of the registry's with it |
| The run | The M1's `rhi-conformance` files | Hear each of the seven canaries on each of the laptop's GPUs, and no message in a measured run. Record the registry's implicit layers from the loader's own log (`VK_LOADER_DEBUG=layer`), since `vulkaninfo`'s list does not mark which layers are implicit |

| Option | What it rules out |
|---|---|
| A. Installed, as the Mac (recommended) | A run on a Windows machine with no administrator, and a machine whose system loader and environment the SDK has not touched. The owner installs the SDK once on the laptop and once on the desktop's Windows |
| B. The tool fetches the installer by digest and cuts the layer out | Nothing of the owner's, where the Visual C++ runtime is already there. The installer's layout is the Qt Installer Framework's, which is public, and the archive's own CRC checks each cut. It needs a 7z reader in the tool, a new dependency, and the loader stays whatever the GPU driver installed. About nine agent runs |
| C. No pin. Criterion 1 on Windows by hand, with whatever layer the machine has | Section 4's "its `triangle` file is criterion 1's proof on Windows", D153's sentence, and `cargo xtask gpu` on Windows at all: rows 210, 234 and 236 and the desktop's rows under Windows stay open |

**What the lane also carries.** Two things of the same crate, each too small for a lane.

- *A measurement `rhi-conformance` in `cargo xtask measure`.* It writes the file section 4 names,
  with section 4's four contents: the cases run, the capabilities reported, each canary's message,
  and the count of other messages by source. Its header is its own and not a timed measurement's,
  which would say "Validation off" of a validated run: the machine's rows, the adapter's name, the
  device as `DeviceInfo` reports it, and a Validation row with the layer's version and the
  validators' settings. No path of the machine is in it.
- *`renderer-impl`'s mutation script, checked in as `tools/xtask/barrier-mutations.py`*, beside
  `line-endings.sh`, so that the desktop needs one clone. The lane fixes its verdict. Today it looks
  in the gate's console output for "`<test>: passed`", which `run_vulkan` has not printed since
  `7424e7c`: only failed entries go to the console, so every mutation reads as caught, a mutation
  that was missed among them. The verdict is read from the gate's report under the target directory:
  `caught` only on the named test's `FAILED` line, `MISSED` on its `passed` line, and `NOT RUN` on
  neither. The lane shows it on the M1 with the row's own record: `between-dispatches` is missed
  under MoltenVK, whose serial compute encoder orders the dispatches, and caught under KosmicKrisp
  (row 239). For Windows it reads and writes its files as UTF-8 and makes each copy into a fresh
  directory.

**What would change it.** The laptop's installed file not being the archived one: then B, which cuts
the layer out of the archive the digest was read from. The system's loader not honouring the
exclusive path is not cured by B, under which the loader stays the driver's: it is a finding for the
planner, and its fix is in how the tool names its layers. The second version said B "can bring its
own loader", which the option does not do.

## Row 21's Windows item

D162 sends the `SetEvent` that `Pool::open_gate` makes for each frame worker to `record-stall`'s
Windows rows, as "the time from each `block_in_place`'s end to the record's next job on a worker
that was parked". `record-stall` is a test of `lactose-app-client` and reaches neither instant.
D162's addendum names two forms of a stamp it could read, two process-wide hooks or the same behind
a cargo feature, and leaves the choice as an interface decision.

**The decision.** Neither. The stamp is two `tracing` spans of `lactose-task`, at debug level,
target `whisk`.

- `open_gate`: around `Pool::open_gate` in `JobContext::block_in_place` (`crates/task/src/job.rs`),
  from the return of the closure to the gate's return.
- `park`: around each of the two waits a worker makes without a permit (`crates/task/src/gate.rs`),
  entered before the wait and exited when it returns, with a field that says which wait it is.
  - The idle protocol's, `sleep.park` in `gate::idle_park`, whatever the worker's look saw. A worker
    parks after a look that found nothing, and also after a look that found work with no permit to
    take it. The second is the worker the gate's opening is for. The first version's "entered after
    the look that found nothing" left it out.
  - A scope opener's on its latch, `opener.park(latch)` in `gate::wait_for_children`. An opener
    whose children other workers took parks there, and so does one the gate stopped beside its own
    children. It is outside the idle protocol, and `Pool::open_gate` wakes it with every other
    worker (`crates/task/src/pool.rs`). The second version spanned the idle protocol alone: such an
    opener's wake was not counted, and a next job it took would have read as taken by a worker that
    was awake. A replay's wait on a latch (`job.rs`) is not spanned: under a replay the log decides
    when each worker runs, and `record-stall` runs under none.

The first review's probe put spans at `open_gate` and at the idle protocol's wait: 53 of 53 loom
models and Miri's five `block_in_place` tests passed. No probe has run a span at the latch's wait,
and the lane's loom and Miri runs are the first.

**What it is and is not.** Three things the first version understated.

- *Both spans are in every build.* Architecture section 5.1 says "Finer levels compile out". Nothing
  builds that: the workspace's `tracing` sets no static maximum level. Until it is built, each span
  costs about 1 ns where nothing listens at debug (the crate's README). When it is built, it is set
  where `record-stall`'s build does not inherit it.
- *A listener's callbacks for `park` run inside a wait's protocol*, on a worker that is going to
  sleep or has just been woken, in the idle protocol after it has announced it is asleep. A listener
  that blocks there, or calls the executor, delays a wake sent to that worker. The crate's README
  says so under "Tracing", which today says "One `tracing` span per node run and one per range split
  off it" and nothing of a debug level.
- *A hook would be safe code too.* `gate::Hook`, the test-only hook the crate has, is an atomic flag
  and a mutex. The first version costed the hooks with an `unsafe` they do not need and with "always
  compiled", which the spans are as well.

What is left in the spans' favour is smaller and real. `record-stall` already reads this crate's
spans through a subscriber it installs (`crates/app-client/tests/stall.rs`), so no public item is
added, and no cargo feature, which unifies across the workspace's builds. Architecture section 21's
entry for `task` stands either way.

**What the file gains**, at each reservation:

- for each worker parked across an opening, in the idle protocol or on a latch, the time from the
  opening's start to that worker's `park` exit, with the count of each kind, the median, P99 and the
  worst. This is the item: what one `SetEvent` for each frame worker costs each of them;
- the opening's own length, which bounds it from above on the opener's side and is not the item;
- D162's own figure, for the openings after which a worker that was parked took the record's next
  job: that job's start from the opening's start and its start wait, with the count, the median and
  the worst, and no percentile under a hundred samples. In the review's probe on the M1 the opener
  took the next job itself in 1,859 to 1,924 of 2,000 openings, so this figure has a sample in under
  a tenth of openings;
- the count of openings after which a worker that was awake took it.

A full run makes 360 ticks at each reservation where it makes 60 today, so that the first row can
hold over 1,000 wakes on the laptop's six frame workers: up to four are parked across an opening at
a reservation of one and up to three at two. The six are its performance cores
(`docs/results/whisk-laptop-windows-2026-11-15.md`, "Detected topology"), and the test takes a frame
worker for each, up to eight (`crates/app-client/tests/stall.rs`). The 300 more ticks at each
reservation add seconds to a run: about four on the M1, by the diff review's arithmetic from the
short files' ticks. Every tick asserts that it saw an `open_gate` span, and each reservation that it
saw `park` spans, so a build that compiles debug spans out fails and writes no file with empty rows.
The rows are written on every host. On Windows they are the `SetEvent` item.

**What the lane owes for the spans' own cost.** On the M1, as steering, interleaved and three times
each: the record phase's figures and the time from an opening to the record's next job with nothing
listening at debug, with `open_gate` listened to, and with both. The three are within one another's
spread, or the README states the difference. The first version had the quiet window's WHISK file
judge the spans by "a row that moved beyond the spread". That is gone: `whisk` installs no listener
(`crates/task/benches/whisk.rs`), so it cannot see a listener's cost, and the quiet window's file is
read against D156 alone.

| Option | What it rules out |
|---|---|
| A. Two debug spans, lane `park-stamp` before the commit (recommended) | A measurement commit before the lane merges. A subscriber of `whisk`'s debug level that blocks in `park` |
| B. Re-gate the item to the change that revisits the keeper's wait | Knowing what opening the gate costs a record on Windows when `RECORD_RESERVATION` is set. D162 says the frame uses that `SetEvent`. No other run shows it: the client binary links no physics backend (`crates/app-client/Cargo.toml`), so `frame-pace` never opens the gate |
| C. Two public hooks, or the same behind a feature | A crate surface with no item added for a measurement. The same rows, the same rule for what a callback may do, about two agent runs more |

**What would change it.** The lane's paired runs showing that listening moves the record phase
beyond the spread: then the file says so beside its rows, or B. Or the owner refusing a change to
`lactose-task` before the commit: then B.

## Two points for the freeze decision

`reviewer-gpu`'s reading for row 34 found that the pinned MoltenVK 1.4.2 (`v1.4.2`, `db66022`)
encodes a multi-draw as a loop on the CPU, one Metal indirect draw for each record, inside
`vkQueueSubmit` under the defaults, and that KosmicKrisp loops inside recording. Row 34 then waits
on the quiet window's `gpu-cost` files alone. The reading raises two points. This proposal places
them and settles neither.

- *`maxDrawIndirectCount`.* `lactose-rhi-vulkan`'s `draw_indexed_indirect` never checks its count
  against the device's limit, and `lactose-rhi-api` carries no such limit. The fix the reading names
  is a field of `Capabilities` that every backend and null refuse above. That is a change to the
  surface the freeze fixes, so it belongs to the freeze decision's amendments (decision 5.13, row
  18), built by a lane the decision names. No driver in reach has a limit a frame meets, so the
  sittings do not wait for it. If it is built before the exit it merges after the measurement commit
  and changes a measured input, under section 3's rule.
- *D42's "emulated indirect-count still encodes the maximum draw count".* It fits KosmicKrisp. It
  does not fit the pinned MoltenVK, which reports `drawIndirectCount` false and records nothing for
  the count entry points. The correction of D42's sentence is a correction of record: it joins the
  conflicts row 18 already holds for the freeze decision. Its consequence, a culling path under
  MoltenVK where the count is not offered, is an input to milestone 3's plan and to no lane of
  milestone 2.

## The rows (exact text)

Each OLD is a run of whole lines of the named file at `b0c5f44` that occurs once in it, and NEW
replaces the run. Row 150's block is the whole row, as lane `exit-pass-rows` left it. `D16x` stands
for the decision's number, which the coordinator allocates. The rows name each other by `D16x` and
by their heads, never by a row number this proposal adds. Every block was applied in memory to the
two files of `b0c5f44`: each OLD was found once, at the lines its head states. If a merge before the
ruling moves a line, the blocks are built again from the merged files. `present-encoder`'s merge, as
its brief has it, inserts an addendum of D162 into the decisions log and replaces verification row
229, one line by one. It does not touch the plan, it moves no line of the verification file, and no
block here names row 229, so no block is affected and the next decision number stays the one after
D164. The plan's own record of the text it once appended to row 150, in its section "Verification
rows", still reads "probed on the laptop's devices": it records an edit that was made, and it stays.

The text is written for the recommended answers.

- Question 1 answered "Re-gate the item": `park-stamp`'s lane row, its verification row and its name
  in section 3's line and in the commit's row go, and row 21's and the plan's `record-stall` row's
  blocks are not applied. The planner writes the re-gate's sentence for row 21.
- Question 2 answered otherwise: the lane row and the pin's row go back to the planner.
- Question 3 answered "One laptop session": the two laptop bullets of section 4, the owner's
  actions, and the commit's third condition go back to the planner.
- Question 4 answered "A probe lane": the plan's two blocks for row 150, its section 2 entry and its
  table of what the spike left open, and row 150's block are not applied, and the planner writes the
  lane's row.
- Question 5 answered "A lane checks it in": row 244's block and the fault probe's row are not
  applied, the planner writes the lane's row, and the measurement commit holds that lane too.

~~~text
=== docs/plans/milestone-2.md line 114: section 2's entry for row 150: the four hardware questions are recorded as not shown
OLD:
| 150 | The RHI spike ran under MoltenVK alone | `vulkan`. **On the M1** (branch `vulkan`): device-only staging runs as a case under both drivers, and KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. lavapipe's run is the `gpu (lavapipe)` job's first, after the merge | The suite under lavapipe settles the spike's eight design questions where lavapipe can: it has one coherent memory type and one queue. The four questions the spike left to hardware are probed on the laptop's devices and recorded, and a run that shows nothing discharges no rule of null (decision 5.17). Question 2 needs a lazily allocated memory type, which lavapipe and the PC drivers surveyed do not offer: KosmicKrisp answers it on the M1 if it offers one, and `spike-metal` answers it for Metal |
NEW:
| 150 | The RHI spike ran under MoltenVK alone | `vulkan`. **On the M1** (branch `vulkan`): device-only staging runs as a case under both drivers, and KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. lavapipe's run is the `gpu (lavapipe)` job's first, after the merge | The suite under lavapipe settles the spike's eight design questions where lavapipe can: it has one coherent memory type and one queue. The four questions the spike left to hardware are recorded as not shown: no test makes any of the four misuses on a device (D16x), and a run that shows nothing discharges no rule of null (decision 5.17). The laptop's first session records each of its GPUs' queue families and memory types beside them. Question 2 needs a lazily allocated memory type, which lavapipe and the PC drivers surveyed do not offer: KosmicKrisp answers it on the M1 if it offers one, and `spike-metal` answers it for Metal |

=== docs/plans/milestone-2.md line 146: section 3's table of what the RHI spike left open: two questions no run probes
OLD:
| An ownership transfer's effect; a heap image in the wrong layout | Null's rules. Probed on the laptop's devices and under GPU-assisted validation, as labelled data (decision 5.17) |
NEW:
| An ownership transfer's effect; a heap image in the wrong layout | Null's rules. No test makes either misuse on a device, so no run on the laptop's devices can show an effect: both are recorded as not shown, which discharges no rule (decision 5.17, D16x) |

=== docs/plans/milestone-2.md line 185: the lane table: two lanes after its last row
OLD:
| `measure-role` | Two safe calls for measurement in `lactose-task` (D164): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |
NEW:
| `measure-role` | Two safe calls for measurement in `lactose-task` (D164): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |
| `layer-windows` | The Khronos layer pinned on Windows (D16x; D153's "pinned on every host"), in `tools/xtask`. `vulkan-layer.toml` gains `[host.windows-x86_64]` in the Mac's form: the SDK the owner installs, found at `VULKAN_SDK` or at `C:\VulkanSDK\<version>`, its `Bin\VkLayer_khronos_validation.dll` held to `library_sha256`, and its manifest `Bin\VkLayer_khronos_validation.json` read for the layer's name and version before the digest is taken, so that another SDK at `VULKAN_SDK` is named as that; the installer's `url` and `sha256` beside it, named in the error for an SDK that is not there and never fetched. The lane derives the digest on the Mac out of LunarG's installer, fetched by the SHA-256 LunarG publishes and never run, and writes the steps into `tools/xtask/README.md`; the proposal's reading, which its GPU review repeated, is `451ed11cbc799f858cbc0e843766d2990527c03f93f7a85f03bc1e67b605a435`. On Windows a run takes the system's loader and drivers, `--driver system` alone, and sets no library path. On every host a run sets the exclusive `VK_IMPLICIT_LAYER_PATH`, to the tool's layers directory for a validated run and to its empty directory for a measured one, in place of `VK_ADD_IMPLICIT_LAYER_PATH` and of `VK_LOADER_LAYERS_DISABLE=~implicit~`: a layer the Windows registry enables then joins no run, and the loader has nothing to warn of in a measured one. The device files' Validation line, the README and the unit tests follow. A measurement `rhi-conformance` in `cargo xtask measure`: the gate's Vulkan step under each driver `measure` takes, and under GPU-assisted validation where the host's gate has it, written as `rhi-conformance-<driver>[-gpu-assisted]-<machine>-<date>.md` with section 4's four contents under a header of its own: the machine's rows, the adapter's name, the device as `DeviceInfo` reports it, and a Validation row with the layer's version and the validators' settings, every path cut as a device file's is; `--all` runs it last and `--no-device` leaves it out. `renderer-impl`'s barrier-mutation script, checked in as `tools/xtask/barrier-mutations.py` beside `line-endings.sh`, with its verdict read from the gate's report under the target directory, `caught` only on the named test's `FAILED` line, `MISSED` on its `passed` line and `NOT RUN` on neither, since the gate has printed no passing line since `7424e7c`; its files read and written as UTF-8 and each copy made into a fresh directory, for Windows | Nothing | On the M1: the pin's parse and the Windows paths, environment and manifest check as unit tests; a unit test that no run sets `VK_ADD_IMPLICIT_LAYER_PATH` or `VK_LOADER_LAYERS_DISABLE`; clippy for `x86_64-pc-windows-msvc`; the digest derived by the lane and equal to the proposal's; under the exclusive path, `cargo xtask gpu` in both forms and `triangle`, `gpu-cost` and `frame-pace` under both drivers with no message, as steering, and `gpu (lavapipe)` green in CI; the three `rhi-conformance` files, with no path of the machine in them; the script's control, and `between-dispatches` read `MISSED` under MoltenVK and `caught` under KosmicKrisp, as the row of that barrier records the two drivers. The Windows run is the laptop's first session's, before the measurement commit is recorded, and a defect it finds is a fix round of this lane | 7 |
| `park-stamp` | Two `tracing` spans of `lactose-task` at debug level, target `whisk` (D16x, D162): `open_gate`, around `Pool::open_gate` in `JobContext::block_in_place`, from the return of the closure to the gate's return; and `park`, around each of the two waits a worker makes without a permit, entered before the wait and exited when it returns: `sleep.park` in `gate::idle_park`, whenever a worker parks in the idle protocol, whatever its look saw, nothing or work with no permit to take it; and `opener.park(latch)` in `gate::wait_for_children`, a scope's opener on its latch, which `Pool::open_gate` wakes with every other worker. A field of the span says which wait it is. A replay's wait on a latch is not spanned. The loom models still run the shipped functions. No public item, no `unsafe`, no hook and no cargo feature; architecture section 21's entry stands. The crate's README, "Tracing", names both spans and says that the node and range spans are the info level's, that both debug spans are in every build until section 5.1's compile-out is built, and that a listener's callbacks for `park` run on a worker that is going to sleep or has just been woken, in the idle protocol after it has announced it is asleep, so they must not block or call the executor. `record-stall`'s subscriber reads the two, and its file gains at each reservation: for each worker parked across an opening, in the idle protocol or on a latch, the time from the opening's start to that worker's `park` exit, with the count of each kind, the median, P99 and the worst; the opening's own length, which bounds it; for the openings after which a worker that was parked took the record's next job, that job's start from the opening's start and its start wait, with the count, the median and the worst; and the count of openings after which a worker that was awake took it. A full run makes 360 ticks at each reservation, so that the first row can hold over 1,000 wakes on the laptop's six frame workers. Every tick asserts that it saw an `open_gate` span, and each reservation that it saw `park` spans, so a build that compiles debug spans out fails. The rows are written on every host; on Windows they are the `SetEvent` item | `measure-role` merged | The rows in `record-stall`'s file on the M1. On the M1, as steering, what listening costs: the record phase's figures and the time from an opening to the record's next job with nothing listening at debug, with `open_gate` listened to and with both, interleaved and three times each, by a means that is the lane's; the three within one another's spread, or the difference stated in the README. `cargo xtask measure whisk` before and after. The loom models, Miri and the cross-target clippy runs for `lactose-task`, with the span at the latch's wait, which no probe has run. `tests/spans.rs` holds both spans' names, target and level, `park` at each of its two waits, and that neither is emitted at info. On Windows the `measure` workflow at the commit shows the test passing with the spans, and the rows are the laptop's session's at the commit | 7 |

=== docs/plans/milestone-2.md lines 285 to 287: section 3: the measurement commit, a measured input, and what a figure before it is
OLD:
- The measurement commit is the first commit of main that holds `frame-impl` and `measure-m2`. The
  quiet window, the laptop's session and session C run there (section 4). A lane that merges after
  it and edits a measured crate runs the measurements it touches and reports them as steering.
NEW:
- The measurement commit is the first commit of main that holds the three lanes that change a
  measured input after `measure-role`: `present-encoder`, `park-stamp` and `layer-windows` (D16x). A
  measured input is anything a measurement builds, runs or computes a figure with, among them the
  crates under `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and layer pins,
  `shaders/`, `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`,
  `.cargo/config.toml`, `.config/nextest.toml` and `.gitattributes`; where the list is short the
  definition holds. The coordinator records the commit's hash in the verification row that names it
  once `check` and the `measure` workflow are green at it and the laptop's first session has run the
  Windows pin on each of its GPUs at `layer-windows`' last merge; a failure of any of the three is a
  fix round of the lane it names, and the commit is then that round's merge. The quiet window, the
  laptop's session at the commit and session C check it out (section 4). A file is the record at it.
  A file of a later commit is the record as `-at-<commit>` only where the diff from the measurement
  commit changes no measured input the file's measurement reads, and its row gives the diff;
  otherwise the owner's ruling says which file is the record (section 7). A figure one of section
  4's measurements gave before it is steering: it stays in its row, proves no criterion and stands
  for no file of record. A row that asks a machine for a run and names no commit is closed by that
  run and records the commit it ran at: the laptop's first session closes such rows at
  `layer-windows`' merge. A lane that merges after it and changes a measured input runs the
  measurements it touches and reports them as steering.

=== docs/plans/milestone-2.md lines 308 to 312: section 4: the quiet window
OLD:
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s -d`, and `cargo xtask measure --all` runs once. It writes milestone 2's
  six files beside milestone 1's, `gpu-cost` under MoltenVK and under KosmicKrisp. The WHISK
  file's 60 Hz P99 is read against D156: under 30 µs, the planner proposes 35 µs again; the system
  daemon's CPU time is read before and after the run, as the remeasurement of 17 November 2026 did.
NEW:
- *The quiet window*, the coordinator's, at the measurement commit, the first sitting there (D16x).
  Every track stops, the machine is on mains power with low power mode off and its screen unlocked,
  and a detached `caffeinate -dims -t <seconds>` holds it awake for the window's length. The owner
  first resizes the latency tool's window by a corner, for row 209, and then leaves the machine.
  `cargo xtask measure --all` runs once. It writes milestone 2's six files beside milestone 1's,
  with `triangle`, `gpu-cost` and `frame-pace` under MoltenVK and under KosmicKrisp, and the
  `rhi-conformance` files. The renderer's cost tests run once after it, for rows 220 and 238. The
  WHISK file's 60 Hz P99 is read against D156: under 30 µs, the planner proposes 35 µs again; the
  system daemon's CPU time is read before and after the run, as the remeasurement of 17 November
  2026 did. Its files are read before the laptop's session at the commit starts.

=== docs/plans/milestone-2.md lines 313 to 315: section 4: the laptop's two sessions
OLD:
- *The laptop's session*, at the same commit, with nothing else running there: the same command,
  and `cargo xtask gpu --driver system`. Its `triangle` file is criterion 1's proof on Windows
  (the owner's answer 2).
NEW:
- *The laptop's first session*, at `layer-windows`' merge, before the measurement commit is recorded
  (D16x). The owner installs the pinned Vulkan SDK and sets `LocalDumps` (row 190) as administrator,
  and stays for its first minutes. The session runs in a shell that is not elevated: the checkouts'
  refresh (row 109), before anything is built; the SDK as installed, the loader's implicit layers,
  and each GPU's queue families and memory types; the latency tool under the owner's hand (row 209);
  `cargo xtask gpu --driver system`, plain and under GPU-assisted validation, on each GPU chosen by
  `--adapter`, which is the first run of `lactose-rhi-vulkan` on a Windows driver; a short run of
  the four device measurements, thrown away; `eco-qos`'s before-and-after runs at their own commits
  (row 63); the window cases (rows 208, 210, 234 and 236); and the barrier mutations once, the
  script's first run on Windows. A defect it finds is fixed before the commit is recorded.
- *The laptop's session at the commit*, after the quiet window's files are read, with nothing else
  running there and in a shell that is not elevated: `cargo xtask measure --all` on the RTX 4070,
  and `triangle`, `gpu-cost`, `frame-pace` and `rhi-conformance` again on the integrated GPU under a
  machine name of its own. `rhi-conformance` is the gate's run, plain and under GPU-assisted
  validation. Its `triangle` file on the RTX 4070 is criterion 1's proof on Windows (the owner's
  answer 2). The owner opens the session and is not needed after.

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
  under GPU-assisted validation; lane `present-encoder`'s `cargo xtask gpu --client`, for the row of
  `frame-impl`'s runs on the x86 desktop; the renderer's cost tests; the barrier mutations, each
  beside its control (rows 225 and 239 to 241); and `eco-qos`'s three `whisk` runs at their own
  commits if the first sitting shows a hybrid processor (row 63). Its `extract-50k` and
  `remote-upload-5k` files are the x86 desktop's figures. The command writes milestone 1's files
  again, which shows what milestone 2's lanes moved on x86. Under a system that is not a client
  platform the session names its measurements and leaves out `triangle` and `frame-pace`:
  `lactose-platform` has not been run there, and nothing skips them.

=== docs/plans/milestone-2.md lines 332 to 333: section 4: what each sitting needs of the owner
OLD:
Only the desktop's two sittings need the owner at a desk. The laptop's runs need the laptop awake
with its agent session open, and a run that opens a window needs a person logged in at it.
NEW:
Only the desktop's two sittings need the owner at a desk. The laptop's first session needs the owner
as administrator once and the owner's hand once in its first minutes. The quiet window needs the
owner's hand once at its start, and then the Mac unused. The laptop's session at the commit needs
the owner to open it. The laptop's runs need the laptop awake with its agent session open, and a run
that opens a window needs a person logged in at it.

=== docs/plans/milestone-2.md line 341: section 4, row `record-stall`: the gate's rows come from two spans, on every host
OLD:
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without, and on Windows the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is the `SetEvent` item of row 164's unmeasured list (D162) | Nothing: recorded. It sets `lactose-app`'s default reservation, `RECORD_RESERVATION`, which is one until the file is read (decision 5.16, D162) |
NEW:
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without, and, from `lactose-task`'s two debug spans, for each worker parked across an opening of the gate at a `block_in_place`'s end, in the idle protocol or on a scope's latch, the time from the opening's start to that worker's wake, with the opening's own length as its bound, and the time to the record's next job where a worker that was parked took it, each with its count, which on Windows is the `SetEvent` item of row 164's unmeasured list (D162, D16x) | Nothing: recorded. It sets `lactose-app`'s default reservation, `RECORD_RESERVATION`, which is one until the file is read (decision 5.16, D162) |

=== docs/plans/milestone-2.md lines 355 to 356: section 4: what writes `rhi-conformance`
OLD:
`cargo xtask gpu --driver <name>` writes `rhi-conformance-<driver>-<machine>-<date>.md`: the cases
run, the capabilities reported, each canary's message, and the count of other messages by source.
NEW:
`cargo xtask measure rhi-conformance` writes `rhi-conformance-<driver>-<machine>-<date>.md`, with
`-gpu-assisted` after the driver for that run: the cases run, the capabilities reported, each
canary's message, and the count of other messages by source, under a header that names the adapter,
the device as it reports itself and the validators the run was under (D16x). `cargo xtask gpu` keeps
its own report under the target directory.

=== docs/plans/milestone-2.md lines 1021 to 1024: section 5: the owner's actions, in order
OLD:
The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); the desktop's first sitting, once the M1's re-run is on main, with `LocalDumps`
set on the desktop; the laptop's checkouts refreshed once (D148) and `LocalDumps` set on the laptop,
before its first milestone 2 run; session C at the measurement commit.
NEW:
The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); the desktop's first sitting, once the M1's re-run is on main, with `LocalDumps`
set on the desktop and the pinned Vulkan SDK installed under its Windows; the laptop's first
session, once `layer-windows` has merged, with the pinned Vulkan SDK installed and `LocalDumps` set
as administrator, its checkouts refreshed (D148) and one window dragged in its first minutes; at the
measurement commit, one window resized at the quiet window's start and the Mac then left unused; the
laptop's session at the commit, opened; then session C (D16x).

=== docs/plans/milestone-2.md lines 1884 to 1887: section 7, step 3: which nightly
OLD:
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and a nightly that holds the client's
   recording test has compared it equal on three platforms, with each platform's release baseline
   equal to its dev one. The run ids are in the rows. No check job's median over its three most
   recent warm runs of main is past ten minutes, or the owner has ruled on the budget.
NEW:
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and the first nightly at or after the
   measurement commit whose four `sour` jobs pass, which holds the client's recording test, has
   compared it equal on three platforms, with each platform's release baseline equal to its dev one
   (D16x). Another job of that nightly that fails is a finding in its own row and holds nothing of
   this step. The run ids are in the rows. No check job's median over its three most recent warm
   runs of main is past ten minutes, or the owner has ruled on the budget.

=== docs/plans/milestone-2.md line 1888: section 7, step 4: every row gated at milestone 2
OLD:
4. The coordinator's pass has left every row of section 2 Done or re-gated.
NEW:
4. The coordinator's pass has left every row of `docs/verification.md` whose gate names milestone 2
   Done or re-gated, section 2's rows among them (D16x).


=== docs/verification.md line 21: verification row 21: the Windows item is two debug spans' (lane `park-stamp`)
OLD:
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153). Until the file is read the default is one frame worker: `lactose_app::RECORD_RESERVATION`, which `WorldConfig::shown()` gives the physics configuration (D162). The file is taken at a reservation of one and of two, and on Windows it also gives the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is what the `SetEvent` for each frame worker in `Pool::open_gate` costs a record (the keeper's unmeasured list, read by D162) | 2. **In part** (branch `measure-m2`, round one and its fix round; not the record): `stall::the_record_beside_the_physics_phase` of `lactose-app-client`'s `integration` target records `ClientRender` on null, each record one `run_job` timed from the job's start, its start wait from `run_job`'s call beside it, beside the composed tick timed inside its bounded future; the physics phase is `physics_api::sync`, `step` and `write_back`, from the first entry into any to the last exit, matched by name in the executor's node spans and `lactose-ecs`'s system spans. In the lane's fix-round `--all` run on the M1 (commit 7e3b62a, after fix rounds one and two), at load averages 4.32 5.02 6.16 before and 4.53 5.05 6.16 after, 60 ticks at each reservation: at one, the record's median and P99 beside the physics phase 7.7 and 49.5 µs, their start wait's 1.1 and 20.8 µs, the phase's median 2,502.7 µs; at two, 7.6 and 43.3 µs, the start wait's 1.1 and 19.6 µs, the phase's 2,737.1 µs; with no tick, 6.8 and 37.3 µs, the start wait's 0.8 and 12.3 µs. The Windows item is not measured: it needs a stamp where a frame-critical worker parks and wakes, inside `lactose-task`, whose hooks only its own unit tests compile, so a public surface of `lactose-task` first (D162 addendum at `measure-m2`, round two). `RECORD_RESERVATION` stays one until the quiet window's file is read |
NEW:
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153). Until the file is read the default is one frame worker: `lactose_app::RECORD_RESERVATION`, which `WorldConfig::shown()` gives the physics configuration (D162). The file is taken at a reservation of one and of two, and on Windows it also gives the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is what the `SetEvent` for each frame worker in `Pool::open_gate` costs a record (the keeper's unmeasured list, read by D162) | 2. **In part** (branch `measure-m2`, round one and its fix round; not the record): `stall::the_record_beside_the_physics_phase` of `lactose-app-client`'s `integration` target records `ClientRender` on null, each record one `run_job` timed from the job's start, its start wait from `run_job`'s call beside it, beside the composed tick timed inside its bounded future; the physics phase is `physics_api::sync`, `step` and `write_back`, from the first entry into any to the last exit, matched by name in the executor's node spans and `lactose-ecs`'s system spans. In the lane's fix-round `--all` run on the M1 (commit 7e3b62a, after fix rounds one and two), at load averages 4.32 5.02 6.16 before and 4.53 5.05 6.16 after, 60 ticks at each reservation: at one, the record's median and P99 beside the physics phase 7.7 and 49.5 µs, their start wait's 1.1 and 20.8 µs, the phase's median 2,502.7 µs; at two, 7.6 and 43.3 µs, the start wait's 1.1 and 19.6 µs, the phase's 2,737.1 µs; with no tick, 6.8 and 37.3 µs, the start wait's 0.8 and 12.3 µs. The Windows item is lane `park-stamp`'s (D16x): `lactose-task` emits two debug spans under target `whisk`, `open_gate` around `Pool::open_gate` at a `block_in_place`'s end and `park` around every wait a worker makes without a permit, the idle protocol's whatever the worker's look saw and a scope opener's on its latch, and the test's subscriber gives, for each worker parked across an opening, the time from the opening's start to that worker's wake, with its count and the opening's own length as its bound, and, where a worker that was parked took the record's next job, that job's start and start wait, with their count. The rows are written on every host, and the laptop's and the desktop's Windows files hold the item. No hook and no cargo feature is added, which D162's addendum at `measure-m2`, round two, named as the two forms. `RECORD_RESERVATION` stays one until the quiet window's file is read |

=== docs/verification.md line 150: verification row 150, the whole row: the laptop's four hardware questions are not shown
OLD:
| The RHI spike ran under MoltenVK alone (D139): its result file marks each "no" as MoltenVK's, against MoltenVK's published limits and the feature's Vulkan requirement, or as a design question. **The MoltenVK half is done:** `docs/results/spike-rhi-apple-m1-2026-11-15.md` shows the decision's ten items under MoltenVK 1.4.2; its four "no"s on the list and GPU-assisted validation's are MoltenVK's, and its design questions are the lavapipe run's to settle, or the desktop GPU's or KosmicKrisp's where it names them. Run the same list of architecture section 12.1 under lavapipe with `rhi-vulkan`'s first Linux job, as the conformant reference, and settle from it each design question the file gives it. Lane `vulkan` closes it (D153): the design questions lavapipe can answer, from its run; the four hardware questions probed on the laptop's devices and recorded, a run that shows nothing discharging no rule of the null backend; device-only staging as a case on the M1; question 2 answered under KosmicKrisp on the M1 if that driver offers a lazily allocated type, and open on Vulkan if not. **On the M1** (branch `vulkan`): a buffer in device-only memory filled through a staging copy and read back through a copy runs as a case of the suite under MoltenVK and KosmicKrisp (`device_memory_is_filled_through_a_staging_copy`, and `device_buffer_is_filled_through_a_staging_copy` among the device tests). KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. The lavapipe run is the `gpu (lavapipe)` job's first run, after the merge, and the laptop's four hardware questions are owed | 2. **The lavapipe half is done** (CI 45116598474 at `21631f5`, the `gpu (lavapipe)` job's first green run, job 137500764827; the run at `ae399a7`, 45108422263, failed): its `gpu-lavapipe` report, under the layer 1.4.363.1's core, synchronisation and thread-safety validation with every canary heard but the race across queues, skipped since compute shares the graphics queue, and no case failed, gives llvmpipe of Mesa 26.0.8 one queue (compute and present `Shared`), coherent host memory, no lazily allocated type (`memoryless_colour`, `memoryless_depth` and `memoryless_survives_local_read` false), multi-draw and indirect count. Of the spike's eight design questions: 1, the local-read barrier dropped, is not shown, since its case is a check the null backend alone makes and skips under lavapipe, so `rhi-null`'s `local` rule stays the only guard; 2 stays open on Vulkan, lavapipe offering no lazily allocated type; 3 is settled, memoryless being a request: lavapipe reports it not honoured (`memoryless_outcome_is_queryable_and_as_reported`), the G-buffer is lit in its pass in ordinary memory (`memoryless_g_buffer_is_lit_in_its_pass`), and a memoryless attachment stored or kept is refused; 4 is settled for the facade: a byte flipped, a truncation, a wrong checksum and a stale header each start the cache `Rejected` before the driver sees it, and garbage sealed behind lavapipe's own header makes the device with no message (`cache_that_fails_a_check_starts_empty`), whether lavapipe took or refused it not being in the report, so the seal and the header check are the guard; 5 is not this run's, which runs without GPU-assisted validation: the nightly's `gpu (lavapipe, GPU-assisted)` (45609400690 at `98d11cf`, job 139188694423) heard the address canary's read past a buffer and the heap canary's slot past the heap, and no case failed; 6 is not shown, compute having no queue of its own, so the pairing stays null's rule and the transfer's effect a hardware question; 7 is settled: indirect count is reported, and a count of one capped at two draws the first record alone, as the CPU reference does (`indirect_count_draws_where_reported`); 8 is not shown, lavapipe's host memory being coherent, where flush and invalidate are accepted (`flush_and_invalidate_are_accepted_where_memory_is_coherent`), so the non-coherent path is a hardware question |
NEW:
| The RHI spike ran under MoltenVK alone (D139): its result file marks each "no" as MoltenVK's, against MoltenVK's published limits and the feature's Vulkan requirement, or as a design question. **The MoltenVK half is done:** `docs/results/spike-rhi-apple-m1-2026-11-15.md` shows the decision's ten items under MoltenVK 1.4.2; its four "no"s on the list and GPU-assisted validation's are MoltenVK's, and its design questions are the lavapipe run's to settle, or the desktop GPU's or KosmicKrisp's where it names them. Run the same list of architecture section 12.1 under lavapipe with `rhi-vulkan`'s first Linux job, as the conformant reference, and settle from it each design question the file gives it. Lane `vulkan` closes it (D153): the design questions lavapipe can answer, from its run; the four hardware questions, which were to be probed on the laptop's devices, recorded as not shown (D16x), a run that shows nothing discharging no rule of the null backend; device-only staging as a case on the M1; question 2 answered under KosmicKrisp on the M1 if that driver offers a lazily allocated type, and open on Vulkan if not. **On the M1** (branch `vulkan`): a buffer in device-only memory filled through a staging copy and read back through a copy runs as a case of the suite under MoltenVK and KosmicKrisp (`device_memory_is_filled_through_a_staging_copy`, and `device_buffer_is_filled_through_a_staging_copy` among the device tests). KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. The lavapipe run is the `gpu (lavapipe)` job's first run, after the merge, and the laptop's four hardware questions are not shown (D16x): no test makes any of the four misuses on a device. The ownership case always performs the transfer, and where compute has no queue family of its own it crosses two queues of one family; the backend takes a coherent host-visible type wherever a device offers one, so the unflushed-range case skips; the cook refuses an undecorated heap index; and null's rule alone holds the wrong layout. The laptop's first session runs the gate, plain and GPU-assisted, on each of its GPUs as criterion 2's evidence there, and records each GPU's queue families and whether a host-visible type of it is not coherent. A run that shows nothing discharges no rule of null (decision 5.17) | 2. **The lavapipe half is done** (CI 45116598474 at `21631f5`, the `gpu (lavapipe)` job's first green run, job 137500764827; the run at `ae399a7`, 45108422263, failed): its `gpu-lavapipe` report, under the layer 1.4.363.1's core, synchronisation and thread-safety validation with every canary heard but the race across queues, skipped since compute shares the graphics queue, and no case failed, gives llvmpipe of Mesa 26.0.8 one queue (compute and present `Shared`), coherent host memory, no lazily allocated type (`memoryless_colour`, `memoryless_depth` and `memoryless_survives_local_read` false), multi-draw and indirect count. Of the spike's eight design questions: 1, the local-read barrier dropped, is not shown, since its case is a check the null backend alone makes and skips under lavapipe, so `rhi-null`'s `local` rule stays the only guard; 2 stays open on Vulkan, lavapipe offering no lazily allocated type; 3 is settled, memoryless being a request: lavapipe reports it not honoured (`memoryless_outcome_is_queryable_and_as_reported`), the G-buffer is lit in its pass in ordinary memory (`memoryless_g_buffer_is_lit_in_its_pass`), and a memoryless attachment stored or kept is refused; 4 is settled for the facade: a byte flipped, a truncation, a wrong checksum and a stale header each start the cache `Rejected` before the driver sees it, and garbage sealed behind lavapipe's own header makes the device with no message (`cache_that_fails_a_check_starts_empty`), whether lavapipe took or refused it not being in the report, so the seal and the header check are the guard; 5 is not this run's, which runs without GPU-assisted validation: the nightly's `gpu (lavapipe, GPU-assisted)` (45609400690 at `98d11cf`, job 139188694423) heard the address canary's read past a buffer and the heap canary's slot past the heap, and no case failed; 6 is not shown, compute having no queue of its own, so the pairing stays null's rule and the transfer's effect a hardware question; 7 is settled: indirect count is reported, and a count of one capped at two draws the first record alone, as the CPU reference does (`indirect_count_draws_where_reported`); 8 is not shown, lavapipe's host memory being coherent, where flush and invalidate are accepted (`flush_and_invalidate_are_accepted_where_memory_is_coherent`), so the non-coherent path is a hardware question |

=== docs/verification.md line 170: verification row 170: its nightly is not exit step 3's
OLD:
| SOUR hashes come from a dev build (project review 2, X5): a result that depends on debug assertions or overflow checks, such as a stale arena read that poisons only under `debug_assertions` (`crates/core/src/arena.rs`), could differ in release unseen. Lane `exit-tooling` (`1dd5dab`) adds `cargo xtask sour run --release` (nextest `--cargo-profile release`, output in `target/sour-release`); each nightly sour job runs one architecture-mode run with it and compares that baseline with the platform's dev baseline by `sour compare`. Shown on the Apple M1: the release baseline and `sour run --pr`'s dev baseline agree on all 18,446 hashes (load averages 14.72 22.97 15.18 to 13.05 19.79 15.02). The same lane records the bone scene (X4), `lactose_app::cost::bone_bodies_on_rapier_are_recorded` and its `::physics` stream: 20 characters with bone bodies on Rapier over 80 ticks, 0.229 s under nextest (load 1.93 5.73 5.28), among those hashes. **Done**: nightly run 44067232715 at `a539d60`, the first after the merge, held release equal to dev on linux-x64, windows-x64 and macos-arm64 (each platform's `sour compare` of its release baseline with its dev baseline), and its cross-platform job found linux-x64's hashes equal to windows-x64's and to macos-arm64's, the bone scene's streams among them | 2 |
NEW:
| SOUR hashes come from a dev build (project review 2, X5): a result that depends on debug assertions or overflow checks, such as a stale arena read that poisons only under `debug_assertions` (`crates/core/src/arena.rs`), could differ in release unseen. Lane `exit-tooling` (`1dd5dab`) adds `cargo xtask sour run --release` (nextest `--cargo-profile release`, output in `target/sour-release`); each nightly sour job runs one architecture-mode run with it and compares that baseline with the platform's dev baseline by `sour compare`. Shown on the Apple M1: the release baseline and `sour run --pr`'s dev baseline agree on all 18,446 hashes (load averages 14.72 22.97 15.18 to 13.05 19.79 15.02). The same lane records the bone scene (X4), `lactose_app::cost::bone_bodies_on_rapier_are_recorded` and its `::physics` stream: 20 characters with bone bodies on Rapier over 80 ticks, 0.229 s under nextest (load 1.93 5.73 5.28), among those hashes. **Done**: nightly run 44067232715 at `a539d60`, the first after the merge, held release equal to dev on linux-x64, windows-x64 and macos-arm64 (each platform's `sour compare` of its release baseline with its dev baseline), and its cross-platform job found linux-x64's hashes equal to windows-x64's and to macos-arm64's, the bone scene's streams among them | 2. **Done** for release against dev. The nightly of the plan's exit step 3 is a later one, the first at or after the measurement commit whose four `sour` jobs pass (D16x), and its run id is in the row of the client's recording test |

=== docs/verification.md line 232: verification row 232: the nightly exit step 3 waits for
OLD:
| `lactose-app-client` is both a shipped binary and a harness package (D162: criterion 3(c)'s recorded test is in the module `recorded` of its `integration` target, since that package alone may name the real render side in normal code). `cargo xtask features` then compares its boundary tree in full with its dev-dependencies included, against `lactose-app-headless`'s. Owed by `frame-impl`: the comparison passes with `lactose-rhi-null` and the test's other dev-dependencies in the tree. If a dev-dependency moves the boundary tree, that is a finding for the planner, and D162's placement of the test is what changes **Answered** (branch `frame-impl`, fix rounds 1 and 2). At round two `cargo xtask features` reported six findings, one cause: `bitflags` 2.13.2, in the boundary tree through `lactose-physics-rapier` (`parry3d-f64` builds it without `std`), was built with `std` for `lactose-app-client` and without it for the other roots, on aarch64-apple-darwin and x86_64-unknown-linux-gnu. No dev-dependency moved it, so D162's placement of the test did not: with normal edges alone (`cargo tree -e normal,features -i bitflags`) the window's dependencies ask `std`, `objc2-app-kit` under winit through `lactose-platform` and `objc2-quartz-core` under `raw-window-metal` through `lactose-rhi-vulkan` on macOS, and `rustix` on Linux, so any feature root that links a window moves the tree. `lactose-physics-rapier` names `bitflags` through the workspace, pinned `=2.13.2` with `default-features = false, features = ["std"]`, so every root builds it alike, and `crate-tiers.toml` lists it among the crate's direct dependencies with that reason. `std` keeps a parse error's text and implements `std::error::Error` for it, and changes no flag value, bit operation or iteration order. On the branch `cargo xtask features` and `cargo xtask boundary` both pass, and with them `lactose-xtask`'s `workspace::the_feature_check_passes_with_no_roots_and_reads_real_trees`; `boundary::tests::the_physics_backend_may_not_name_the_solvers_math`, which reads the real tier file, expects `bitflags` on the crate's list with the rest; `cargo xtask vendor` and `cargo deny check` accept the pin. A pin moves with `parry3d-f64`'s `bitflags` | 2 |
NEW:
| `lactose-app-client` is both a shipped binary and a harness package (D162: criterion 3(c)'s recorded test is in the module `recorded` of its `integration` target, since that package alone may name the real render side in normal code). `cargo xtask features` then compares its boundary tree in full with its dev-dependencies included, against `lactose-app-headless`'s. Owed by `frame-impl`: the comparison passes with `lactose-rhi-null` and the test's other dev-dependencies in the tree. If a dev-dependency moves the boundary tree, that is a finding for the planner, and D162's placement of the test is what changes **Answered** (branch `frame-impl`, fix rounds 1 and 2). At round two `cargo xtask features` reported six findings, one cause: `bitflags` 2.13.2, in the boundary tree through `lactose-physics-rapier` (`parry3d-f64` builds it without `std`), was built with `std` for `lactose-app-client` and without it for the other roots, on aarch64-apple-darwin and x86_64-unknown-linux-gnu. No dev-dependency moved it, so D162's placement of the test did not: with normal edges alone (`cargo tree -e normal,features -i bitflags`) the window's dependencies ask `std`, `objc2-app-kit` under winit through `lactose-platform` and `objc2-quartz-core` under `raw-window-metal` through `lactose-rhi-vulkan` on macOS, and `rustix` on Linux, so any feature root that links a window moves the tree. `lactose-physics-rapier` names `bitflags` through the workspace, pinned `=2.13.2` with `default-features = false, features = ["std"]`, so every root builds it alike, and `crate-tiers.toml` lists it among the crate's direct dependencies with that reason. `std` keeps a parse error's text and implements `std::error::Error` for it, and changes no flag value, bit operation or iteration order. On the branch `cargo xtask features` and `cargo xtask boundary` both pass, and with them `lactose-xtask`'s `workspace::the_feature_check_passes_with_no_roots_and_reads_real_trees`; `boundary::tests::the_physics_backend_may_not_name_the_solvers_math`, which reads the real tier file, expects `bitflags` on the crate's list with the rest; `cargo xtask vendor` and `cargo deny check` accept the pin. A pin moves with `parry3d-f64`'s `bitflags` | 2. **Owed, a nightly's**: the first nightly at or after the measurement commit whose `sour (linux-x64)`, `sour (windows-x64)`, `sour (macos-arm64)` and `sour (architecture)` jobs pass (D16x) compares the test's two streams equal on the three platforms, with each platform's release baseline equal to its dev one, and its run id is written here; another job of that run that fails is a finding in its own row. Nightly 45609400690 at `98d11cf`, the last before `ad1a99d`, is older than the test, which merged at `55ffdda` |

=== docs/verification.md line 243: verification row 243: `--adapter`, the pin's lane, and whose run it is
OLD:
| Criterion 1 on the Windows laptop (decision 5.1, D162): `lactose-app-client --fixed 600` on each of the laptop's GPUs, chosen by `VK_LOADER_DRIVERS_SELECT`, under the Khronos layer's core, synchronisation and thread-safety validation, each validator's canary run in a process of its own beside it. **Owed, the owner's**: the commands from a clean checkout are the coordinator's, from lane `frame-impl`, and `cargo xtask gpu` does not run there, since the pinned layer has no Windows build. It passes where every canary reports its own identifier and nothing else, and the run prints one `Outcome` with `counts`, `surely` with every entry above zero and `messages: 0`, and nothing at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live, and every pixel is as the reference rasterised from those placements says. The M1 gave `Counts { drawn: 1044, clear: 195304, margin: 260 }` under both of its drivers; the counts are the reference's, from the placements and the latched camera, so the same are expected. The run's two `info` lines give the device and its queue layout, which say whether the laptop's devices serve the present role from the graphics queue (row 176). `cargo xtask measure triangle` (branch `measure-m2`, round two) is the run on a host the layer is pinned for, and met the criterion on the M1 under both drivers with those counts; `tools/xtask/vulkan-layer.toml` pins no layer for Windows, so the laptop's file needs a pin for `windows-x86_64` first | 2 |
NEW:
| Criterion 1 on the Windows laptop (decision 5.1, D162): `lactose-app-client --fixed 600` on each of the laptop's GPUs, chosen by `--adapter`, which the device's name must contain, under the Khronos layer's core, synchronisation and thread-safety validation, each validator's canary run in a process of its own beside it. **Owed, the laptop's** (D16x): its first session shows the pin, once lane `layer-windows` has pinned the layer for Windows, and `cargo xtask measure triangle --driver system --adapter <name>` on each GPU in its session at the measurement commit is the run; lane `frame-impl`'s by-hand form is dropped. It passes where every canary reports its own identifier and nothing else, and the run prints one `Outcome` with `counts`, `surely` with every entry above zero and `messages: 0`, and nothing at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live, and every pixel is as the reference rasterised from those placements says. The M1 gave `Counts { drawn: 1044, clear: 195304, margin: 260 }` under both of its drivers; the counts are the reference's, from the placements and the latched camera, so the same are expected. The run's two `info` lines give the device and its queue layout, which say whether the laptop's devices serve the present role from the graphics queue (row 176). `cargo xtask measure triangle` (branch `measure-m2`, round two) is the run on a host the layer is pinned for, and met the criterion on the M1 under both drivers with those counts; `tools/xtask/vulkan-layer.toml` pins no layer for Windows at `b0c5f44`; lane `layer-windows` adds the pin (D16x), and the RTX 4070's `triangle` file at the measurement commit is the criterion's proof on Windows | 2 |

=== docs/verification.md line 244: verification row 244: what session C runs of it, and what it cannot
OLD:
| Lane `frame-impl`'s runs on the x86 desktop (D162, decision 5.1): criterion 1 and the probe's paced runs under synchronisation validation (row 230), on a device whose present family is not its graphics family, and on lavapipe, whose roles share one queue. **Owed to the desktop sitting**, as `measure-m2` is; nothing of this lane ran on x86 | 2 |
NEW:
| Lane `frame-impl`'s runs on the x86 desktop (D162, decision 5.1): criterion 1 and the probe's paced runs under synchronisation validation (row 230), on a device whose present family is not its graphics family, and on lavapipe, whose roles share one queue. **Owed to session C** (D16x), as `measure-m2` is: criterion 1 is `cargo xtask measure triangle` there, and the client's present under the gate's validators is lane `present-encoder`'s `cargo xtask gpu --client`, as shipped and lagged, on the desktop's GPU, with that device's queue families beside it. The probe's paced runs and lavapipe's half are not session C's: the probe is a build of a scratch copy that no checkout holds, and no runner of lavapipe's presents to a window. They are the row of the client's fault probe, gated at milestone 3. Nothing of this lane ran on x86 | 2 |

=== docs/verification.md: four rows appended at the end of the file, after the table's last row
NEW:
| The measurement commit of milestone 2 (D16x): the first commit of main that holds `present-encoder`, `park-stamp` and `layer-windows`, the three lanes open at `b0c5f44` that change a measured input, recorded once `check` and the `measure` workflow are green at it and the laptop's first session has run the Windows pin on each of its GPUs at `layer-windows`' last merge. A measured input is anything a measurement builds, runs or computes a figure with, among them the crates under `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and layer pins, `shaders/`, `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, `.config/nextest.toml` and `.gitattributes`; where the list is short the definition holds. The quiet window on the M1, the laptop's session at the commit and session C on the desktop each check it out, in that order, each read before the next. A file is the record at it. A file of a later commit is the record as `-at-<commit>` only where the diff from it changes no measured input the file's measurement reads, and the row that cites the file gives the diff; otherwise the owner's ruling at the plan's exit step 7 names the record. A figure a measurement gave before it is steering and stands for no file of record. A row that asks a machine for a run and names no commit is closed by that run and records the commit it ran at. Three sittings keep commits of their own: the desktop's first at `a539d60` (D151), `eco-qos`'s runs at `a75596b` and its first parent, and the laptop's first session at `layer-windows`' merge, for the rows it closes. To record here: the commit's hash; the commit and the date of the laptop's first session; for each sitting its date, its load and the files it wrote; the ids of the `measure` workflow's run and of the first nightly at or after the commit whose four `sour` jobs pass | 2. **Owed**: no such commit exists at `b0c5f44` |
| The Khronos layer on Windows is pinned as installed (D16x, D153; lane `layer-windows`): `library_sha256` is the SHA-256 of `Bin/VkLayer_khronos_validation.dll` as read on the development Mac, by the proposal and again by its GPU review, out of LunarG's installer `vulkansdk-windows-X64-1.4.363.1.exe`, which was fetched by the SHA-256 LunarG publishes for it and never run. No Windows host has checked it. What the laptop's first session shows, and nothing before it: that the installed manifest names the layer at `1.4.363` and the installed file is the archived one, which the tool checks in that order at its first run; that the system's loader, whose version the session records, takes the layer's manifest from the exclusive `VK_IMPLICIT_LAYER_PATH` in a shell that is not elevated and loads no layer of the registry's with it, the loader ignoring every such variable in an elevated process; that each of the seven canaries is heard on each of the laptop's GPUs; that a measured run under the empty path hears no message; and which implicit layers the registry holds, read from the loader's own log (`VK_LOADER_DEBUG=layer`), each with its enable and disable variables. A digest that differs is a finding, and the session re-pins nothing: a second pin is proposed only for a file whose version is 1.4.363.x and whose signature is LunarG's, and it is the owner's to approve. The desktop's Windows is the second host to check it | 2 |
| `lactose-task`'s two debug spans (D16x; lane `park-stamp`): `open_gate` and `park`, target `whisk`, which `record-stall` reads. `park` is around both waits a worker makes without a permit, the idle protocol's and a scope opener's on its latch, which an opening of the gate wakes too. Both are in every build: section 5.1's compile-out of finer levels is not built, and a span costs about 1 ns where nothing listens at debug. `park`'s callbacks run on a worker that is going to sleep or has just been woken, in the idle protocol after it has announced it is asleep: a listener that blocks there, or calls the executor, delays a wake sent to that worker. Owed by the lane, on the M1 as steering: the record phase's figures and the time from an opening to the record's next job with nothing listening at debug, with `open_gate` listened to and with both, interleaved; `cargo xtask measure whisk` before and after; the loom models, Miri and the cross-target clippy runs, with the span at the latch's wait, which no probe has run. Owed by the quiet window: the WHISK file at the measurement commit, read against D156 as the plan has it. `record-stall` fails where a build compiles debug spans out, so when section 5.1's compile-out is built it is set where `record-stall`'s build does not inherit it | 2 |
| The client's fault probe on x86 and on lavapipe (D162, D16x; from the row of lane `frame-impl`'s runs on the x86 desktop): the paced runs of a probe build, in which each present call sleeps 40 ms, every 13th makes no submit and every 97th draw is torn, ran on the M1 under both drivers and nowhere else. The probe is a build of a scratch copy that no checkout holds, so session C cannot run it, and no runner of lavapipe's presents to a window. Owed to the first change after milestone 2's exit that edits the client's present path, which checks the probe in or says why not, and to the first run of `lactose-platform` on Linux | 3 |
~~~

## Decision entry (exact text; number at merge)

The entry is written for the recommended answers, as the rows are. An answer that is not the
recommended one sends its bullet and its clause of "Rules out" back to the planner with the rows the
list above names: the spans' for question 1, the layer's for question 2, the order's for question 3,
the four hardware questions' for question 4 and the fault probe's for question 5.

**D16x. Milestone 2's sittings: the measurement commit holds every lane that changes a measured
input and is recorded after the Windows pin's first run, the Khronos layer is pinned on Windows as
installed, and the gate's wake is read from two debug spans** (§5.1, §20, §21, §22, `lactose-task`,
`tools/xtask`, `docs/plans/milestone-2.md`)
- Why: the exit pass's audit of main at `ad1a99d` found no measurement commit named, no result file
  of milestone 2 on main, the laptop unable to run criterion 1 for want of a pinned layer, and row
  21's Windows item unmeasurable for want of a stamp. The plan's section 4 named the sittings and
  not their order, and its session C was one command where the rows owe the desktop the device tests
  and the barrier mutations too.
  - *The measurement commit holds every lane that changes a measured input.* It is the first commit
    of main that holds `present-encoder`, `park-stamp` and `layer-windows`. A measured input is
    anything a measurement builds, runs or computes a figure with, among them the crates under
    `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and layer pins, `shaders/`,
    `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`,
    `.config/nextest.toml` and `.gitattributes`; where the list is short the definition holds.
    `tools/xtask` is one: it times `edit-to-playing`, computes `frame-pace`'s figures, cooks the
    shaders and sets every device run's environment. A file is the record at that commit. A file of
    a later commit is the record as `-at-<commit>` only where the diff changes no measured input its
    measurement reads; otherwise the owner's ruling names the record. A figure a measurement gave
    before the commit is steering: it stays in its row, proves no criterion and stands for no file
    of record. A row that asks a machine for a run and names no commit is closed by that run and
    records the commit it ran at. Three sittings keep commits of their own: the desktop's first, at
    `a539d60`; `eco-qos`'s runs, at `a75596b` and its first parent; and the laptop's first session,
    at `layer-windows`' merge.
  - *The commit is recorded after the Windows pin's first run.* No line of `lactose-rhi-vulkan` had
    run on a Windows driver. The laptop's first session, at `layer-windows`' merge, runs the gate on
    each of its GPUs, and with it everything the laptop owes at no commit. A defect it finds is
    fixed before any file of record is taken. The hash is recorded once that session has passed and
    `check` and the `measure` workflow are green at the commit.
  - *The sittings at the commit run in one order, and each is read before the next.* The M1's quiet
    window, the laptop's session and session C. A criterion that misses on the M1 needs an engine
    lane, and a sitting taken before that lane is steering. The owner's desk time comes last and
    once. The desktop's first sitting waits for nothing. The nightly of the exit is the first at or
    after the measurement commit whose four `sour` jobs pass; another job's failure in that run is a
    finding in its own row.
  - *Windows takes the layer as installed.* `vulkan-layer.toml` pins the SHA-256 of the SDK's
    `VkLayer_khronos_validation.dll`, read on the development Mac out of LunarG's installer, which
    is fetched by the digest LunarG publishes and never run. The owner installs the SDK. The tool
    checks the installed manifest's version and then the file's digest. A digest that differs is a
    finding: no session re-pins, and a second pin is the owner's to approve. The run takes the
    system's loader and drivers. This is D153's "pinned on every host" on the host it did not reach.
  - *A run names its implicit layers, on every host.* A run sets the exclusive
    `VK_IMPLICIT_LAYER_PATH`: the tool's layers directory where it validates and an empty directory
    where it measures. `VK_ADD_IMPLICIT_LAYER_PATH` and `VK_LOADER_LAYERS_DISABLE=~implicit~` go. On
    Windows the first let the registry's implicit layers into a validated run, and the second made
    the loader warn for each layer it disabled, which a measured run hears as a message and fails
    on.
  - *`rhi-conformance` is a measurement.* `cargo xtask measure rhi-conformance` runs the gate's
    Vulkan step and writes the file criterion 2 names, under a header of its own that names the
    adapter, the device and the validators, with no path of the machine in it. `cargo xtask gpu`
    keeps its report under the target directory.
  - *The gate's wake is read from two debug spans.* `lactose-task` emits `open_gate`, around
    `Pool::open_gate` at a `block_in_place`'s end, and `park`, around every wait a worker makes
    without a permit: the idle protocol's, whatever the worker's look saw, and a scope opener's on
    its latch, which an opening wakes too. Both are at debug level under target `whisk`.
    `record-stall`'s subscriber reads them and gives, for each worker parked across an opening, the
    time from the opening's start to its wake. 5.1's compile-out is not built. Until it is, the two
    spans are in every build, at about 1 ns each with no debug listener. When it is built, it is set
    where `record-stall`'s build does not inherit it. A listener's callbacks for `park` run on a
    worker that is going to sleep or has just been woken, and must not block or call the executor.
    Section 21's entry for `task` stands.
  - *The laptop's four hardware questions are not shown.* No test makes any of the four misuses on a
    device: the ownership case always performs the transfer, the backend takes coherent memory
    wherever a device offers it, the cook refuses an undecorated heap index, and null's rule alone
    holds the wrong layout. The gate's runs on the laptop are criterion 2's evidence there and
    answer none of the four. Each sitting records its devices' queue families and memory types.
    Null's rules and the cook's check stand (decision 5.17).
  - *The client's fault probe is re-gated.* `frame-impl`'s probe build is in no checkout. Session C
    runs criterion 1 and `cargo xtask gpu --client` for the row of that lane's runs on x86, and the
    probe's paced runs and lavapipe's half move to a row gated at milestone 3.
  - *The exit reads every row gated at milestone 2.* Section 2 lists the 26 gated at `a561b36`; 85
    are gated at `ad1a99d`. Step 4 reads them all.
  - Rules out: a measurement commit named before a lane that changes a measured input has merged, or
    before `lactose-rhi-vulkan` has run on a Windows driver; a file at another commit cited as the
    record where the diff changes what its measurement reads; a sitting at the measurement commit on
    the laptop or the desktop before the M1's files are read; a layer on Windows taken unchecked
    from whatever the machine has, and a pin changed by the session that found it wrong; a download
    the tool unpacks out of an installer; an implicit layer of the machine's in a validated or a
    measured run; a public hook in `lactose-task` for row 21's stamp, and a cargo feature for it; a
    probe program written for row 150's four questions; a lane for the client's fault probe before
    the exit; a window measurement run on a system `lactose-platform` has not been run on.
- Driven by: the exit pass's audit at `ad1a99d`; `docs/proposals/milestone-2-sittings.md`, its probe
  of LunarG's installer and its three reviews; `reviewer-gpu`'s reading for row 34; the owner's
  ruling.
- Replaced: in the plan's section 2, row 150's four hardware questions as "probed on the laptop's
  devices"; in its section 3, the measurement commit as "the first commit of main that holds
  `frame-impl` and `measure-m2`", "a measured crate", and two of the spike's open questions as
  "Probed on the laptop's devices"; in its section 4, `cargo xtask gpu --driver <name>` as what
  writes `rhi-conformance`, the laptop's session as one, session C as `cargo xtask measure --all`
  alone, the quiet window's `caffeinate -i -s -d`, and the window measurements "skipped" on a system
  that is not a client platform; in its section 7, step 3's "a nightly" and step 4's "every row of
  section 2"; in D162's addendum of `measure-m2`, round two, the two forms of a stamp it names, two
  process-wide hooks and the same behind a cargo feature; in `tools/xtask`'s runs,
  `VK_ADD_IMPLICIT_LAYER_PATH` and `VK_LOADER_LAYERS_DISABLE=~implicit~`.

## The owner's questions

As `AskUserQuestion` input, the recommended option first. A call takes four questions at most, so
the fifth is a second call's, asked right after the first. It is not folded into the fourth: row
150's hardware questions and the client's fault probe are independent, and one question whose
options are pairs of answers would hide that.

The first call:

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
          "description": "Lane `park-stamp`, about seven agent runs, before the measurement commit: two tracing spans at debug level, read by the subscriber `record-stall` already installs. No public item and no architecture change. Both spans are in every build until section 5.1's compile-out is built, at about a nanosecond each when nothing listens. A listener's callback for `park` runs on a worker that is going to sleep or has just been woken, so it must not block or call the executor, and the README says so. The lane runs beside `layer-windows`, which starts later, so it is not expected to hold the commit; if it runs past two days the commit is taken without it and the item is re-gated."
        },
        {
          "label": "Re-gate the item",
          "description": "No lane. The item moves to the change that revisits the keeper's wait, and the record's default reservation is set from the three machines' files without the Windows cost of opening the gate. The measurement commit waits for `present-encoder` and `layer-windows` alone."
        },
        {
          "label": "Two public hooks",
          "description": "The form D162's addendum names: two process-wide hooks as a public item of `lactose-task`, or the same behind a cargo feature, which unifies across the workspace's builds. The same rows, and the same rule for what a callback may do inside a wait. One more public item to keep, and about two agent runs more."
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
          "description": "You install LunarG's Vulkan SDK 1.4.363.1 once on each Windows, as administrator. The install also puts LunarG's Vulkan runtime on the whole machine, sets `VULKAN_SDK` and the path for every user, and registers the SDK's explicit layers. The tool refuses a layer whose SHA-256 is not the pin's, which was read on the Mac out of the same installer, never run. If the installed file differs, the run stops and reports it: the session re-pins nothing, and a second pin is yours to approve. Lane `layer-windows`, about seven agent runs."
        },
        {
          "label": "Tool fetches and unpacks",
          "description": "No install and no administrator, where the Visual C++ runtime is already on the machine. The tool downloads the 303 MB installer by digest and cuts the layer out with a 7z reader, a new dependency of the tool. The installer's layout is the Qt Installer Framework's, which is public, and the archive's CRC checks each cut. The loader stays whatever the GPU driver installed. About nine agent runs."
        },
        {
          "label": "No pin, by hand",
          "description": "No lane. The triangle on Windows is run by hand with whatever layer is installed, its result is not the `triangle` file the plan names, and `cargo xtask gpu` never runs on Windows: the laptop's and the desktop's device rows stay open or are re-gated."
        }
      ]
    },
    {
      "question": "The engine's Vulkan backend has never run on a Windows driver, and three sittings must read one measurement commit: the Mac's quiet hour, the laptop and your session C at the desktop. When is the laptop first opened, and in what order do the sittings run?",
      "header": "Order",
      "multiSelect": false,
      "options": [
        {
          "label": "Pin run first (Recommended)",
          "description": "The laptop is opened twice. First as soon as `layer-windows` merges, before the commit is named: you install the SDK as administrator and stay about ten minutes, and the session, about three hours, runs the Windows pin and everything the laptop owes at no commit. The commit waits for that session and, if it finds a defect, for the fix and a second run of the pin, with the laptop opened again. Then the Mac, the laptop and the desktop follow, each read before the next; the laptop's second session needs you only to start it. Before each laptop session you set its screen and sleep to Never, and back afterwards. A defect the first Windows run finds is fixed before any record is taken. About three days from the commit to the last file if nothing misses; a miss holds the later sittings for the lane it needs."
        },
        {
          "label": "One laptop session",
          "description": "The laptop is opened once, at the commit, after the Mac. You are at it once, as administrator and for about ten minutes, and set its screen and sleep once. The commit is named as soon as the lanes merge, about three hours sooner. The first Windows run of the backend then comes after the Mac's hour: a defect it finds moves the commit to its fix, and the Mac's hour is run again or you rule that its files stand. About three days if nothing misses."
        },
        {
          "label": "All in one day",
          "description": "The three sittings run at the commit without waiting for each other's reading, the laptop opened once as in the second option. A miss that needs an engine fix then makes the other machines' files steering, and you sit at the desktop a second time."
        }
      ]
    },
    {
      "question": "Row 150 leaves four questions of the RHI spike to the laptop's GPUs: a dropped ownership transfer, memory that is not coherent, a heap indexed without the non-uniform decoration, and a heap image in the wrong layout. The device tests make none of the four misuses on a device, so their runs can show no effect. What is done about the four?",
      "header": "Row 150",
      "multiSelect": false,
      "options": [
        {
          "label": "Record as not shown (Recommended)",
          "description": "No lane. The laptop runs the device tests, plain and GPU-assisted, on each GPU as criterion 2's evidence there, and records each GPU's queue families and memory types. Row 150 and the plan say all four are not shown, where the plan had them probed on the laptop. The null backend's rules and the cook's check stay the only guards, as decision 5.17 has it for a run that shows nothing."
        },
        {
          "label": "A probe lane",
          "description": "A lane writes the four as device probes: the transfer dropped across two queue families, a memory type forced non-coherent and left unflushed, an undecorated heap index past the cook, and a heap image in the wrong layout. About ten agent runs with the GPU reviewer. It edits `lactose-rhi-vulkan`, so the measurement commit waits for it. It gives labelled data on the laptop's two drivers and proves no rule either way."
        }
      ]
    }
  ]
}
~~~

The second call:

~~~json
{
  "questions": [
    {
      "question": "Row 244 owes the desktop the client's fault probe: paced runs, under synchronisation validation, of a build in which each present call sleeps 40 ms, every 13th makes no submit and every 97th draw is torn. The probe was a build of a scratch copy and is in no checkout, so session C cannot run it. What is done about it?",
      "header": "Fault probe",
      "multiSelect": false,
      "options": [
        {
          "label": "Re-gate the probe (Recommended)",
          "description": "No lane. Session C runs criterion 1 on the desktop and `cargo xtask gpu --client`: the release client on a window under the same validators, as shipped and with the GPU made to lag behind the present thread, which is none of the probe's three faults. The probe's paced runs on x86, and lavapipe's half, move to a new row gated at milestone 3: the first change after the exit that edits the client's present path checks the probe in or says why not. Until then the three faults have run on the M1's two drivers and nowhere else."
        },
        {
          "label": "A lane checks it in",
          "description": "A lane adds the probe's three faults to the client behind a switch, about six agent runs with the GPU reviewer. It edits `lactose-app-client`, which the device measurements run, so the measurement commit waits for it. Session C then makes the paced runs on the desktop's GPU, as labelled data. Lavapipe's half is re-gated either way: no runner of lavapipe's presents to a window."
        }
      ]
    }
  ]
}
~~~

One thing is not a question. The desktop's first sitting: the plan has two desktop sittings
(decision 5.19, D151's addendum), the first one's commands are written, and it is the one sitting
that can run today. Its one step that rests on an answer, the SDK's install, is marked so in its
commands and comes after its measurements.

## Cost and order

| Step | Who | Agent runs |
|---|---|---|
| This round's check | The coordinator, by diff | None |
| The decision, the plan's lines and the rows, applied | The coordinator | None |
| The desktop's first sitting | The owner | None |
| The rows gated at milestone 2 that nothing has read, 37 of them | One read-only agent | 1 |
| `present-encoder`'s fix round and merge | The lane, as briefed | Already counted |
| `layer-windows`: the pin, the exclusive path on every host, `rhi-conformance`, the script | A lane from main, `reviewer` and `reviewer-gpu`, a fix round and a second where the diff review asks for one, `reviewer-diff`, `merger` | 7 |
| `park-stamp`: the two spans, `record-stall`'s rows, the paired runs | A lane from main, `reviewer` and `reviewer-concurrency`, a fix round and a second where the diff review asks for one, `reviewer-diff`, `merger` | 7 |
| The laptop's first session | The laptop's own agent session | 1 |
| The nightly and the `measure` workflow at the commit | The coordinator, by dispatch | None |
| The quiet window | The coordinator, with every agent stopped | None |
| The laptop's session at the commit | The laptop's own agent session | 1 |
| Session C | The owner | None |
| The files checked in and the rows written | The coordinator's pass, or a lane as `exit-pass-rows` was | 2 |
| The freeze decision, the exit's read and the milestone 3 plan | As section 7 has them | Already counted |

About nineteen agent runs are still to spend. `park-stamp` and `layer-windows` run in parallel and
touch different crates: `crates/task` with `crates/app-client/tests/stall.rs`, and `tools/xtask`.
`present-encoder` is in its last round and edits `crates/app-client/src` and, for its client step,
`tools/xtask/src/main.rs`, so `layer-windows` starts from main once it has merged. The three lanes
each edit `docs/results/README.md`, and each later merge takes the earlier ones'.

## What the reviews changed

`reviewer` attacked the first version and found no blocking defect, four serious, ten minor and four
nits. `reviewer-gpu` read the Windows pin and row 150 and found no blocking defect, three serious,
four minor and two nits. All are accepted but one half of G5 and the number in M3.

| Finding | Answer |
|---|---|
| S1. The hooks were costed with what the spans also carry | Accepted. The first question's first option says that a listener's `park` callbacks run on a worker inside a wait, and that both spans are in every build. Its third option loses "always compiled" and the `unsafe` clause, which was false: `gate::Hook` is safe code. What is left against the hooks is a public item or a feature, and about two runs |
| S2. `park` "after the look that found nothing" misses the worker that parks beside work it has no permit for | Accepted. `park` is entered whenever `gate::idle_park` parks, whatever its look saw. Both arms of `gate.rs` reach `sleep.park`. Since the diff review it is at a scope opener's wait on its latch too (R8) |
| S3. `tools/xtask` is measured, and the rule missed the lock file, the toolchain, the shaders and the vendored crates | Accepted, and taken further. A measured input is defined. `layer-windows` is in the commit by the rule. `-at-<commit>` needs a diff that changes nothing its measurement reads. The gap the finding shows, a fix of the pin after the laptop's first run, is closed by running the pin before the commit is recorded |
| S4. `caffeinate -w $$ &` dies with the shell that started it | Accepted. A detached `caffeinate -dims -t <seconds>`, checked alive before the run and ended after it. A probe showed the form outlive its shell |
| M1. Session C cannot run row 244's probe | Accepted. Session C runs criterion 1 and `cargo xtask gpu --client`. The probe's paced runs and lavapipe's half move to a row gated at milestone 3, where the owner's fifth question is answered as recommended (R6) |
| M2. The laptop does not send back the memory types row 246 waits for | Accepted. The laptop's first session, the desktop's first sitting and session C return each device's memory types. They serve row 150 too |
| M3. The window cases' count is wrong | Accepted, with another number: fifteen on Windows. The first version's twelve was row 208's count before the swapchain's three. The review's fourteen is one short: `CASES` in `crates/platform/tests/window.rs` has sixteen entries and one is macOS's alone |
| M4. The parked-worker figure has no sample size, and the opening's length is a bound, not the item | Accepted. The item is each parked worker's wake, from the opening's start to its `park` exit, with its count. The opening's length is stated as a bound. D162's figure keeps its count, median and worst. A full run makes 360 ticks a reservation |
| M5. Only `open_gate` is asserted, and the README's "Tracing" is not amended | Accepted. Each reservation asserts `park` spans too. The lane amends "Tracing" |
| M6. "Finer levels compile out covers them" overclaims | Accepted, in the review's words: the compile-out is not built, the spans are in every build, and when it is built it is set where `record-stall` does not inherit it |
| M7. "The first green nightly" is stricter than step 3 | Accepted. The first nightly at or after the commit whose four `sour` jobs pass. Another job's failure is a finding in its own row |
| M8. Nothing checks what the spans cost the wake | Accepted. The lane owes paired runs with nothing listening, one span and both. "A row that moved beyond the spread" is gone from the quiet window |
| M9. Question 3's "about three days" has no condition | Accepted: "if nothing misses; a miss holds the later sittings for the lane it needs" |
| M10. Question 2 does not say what follows a digest that differs | Accepted. The run stops, the session re-pins nothing, and a second pin is the owner's |
| N1. Question 1's second option said "the M1's and the desktop's files" | Accepted: "the three machines' files" |
| N2. D16x ruled out every hook and feature in `lactose-task` | Accepted: "for row 21's stamp" |
| N3. Row 63 has a desktop half | Accepted. Session C adds `eco-qos`'s three runs if the first sitting shows a hybrid processor |
| N4. `typeperf` never stops | Accepted. It takes a sample count and ends by itself |
| G1. The mutation script's verdict is vacuous | Accepted. Lane `layer-windows` fixes it and checks it in as `tools/xtask/barrier-mutations.py`, with the verdict read from the gate's report, and shows `between-dispatches` missed under MoltenVK and caught under KosmicKrisp |
| G2. The gate's runs answer none of the four hardware questions | Accepted. Row 150, the plan's table, the decision entry and question 4 say all four are not shown. The paragraph below settles the two reviews' disagreement |
| G3. The registry's implicit layers join validated runs, and `~implicit~` fails every measured run on a machine that has one | Accepted, on every host: the exclusive `VK_IMPLICIT_LAYER_PATH` in place of both forms |
| G4. `vulkaninfo`'s list does not mark implicit layers | Accepted. The session reads the loader's own log, with each layer's enable and disable variables |
| G5. `rhi-conformance` under the measurements' header would misstate the run, and a second GPU's run overwrites the first's | Accepted for the header: its own, with the adapter, the device and a Validation row. Not accepted: a refusal to overwrite for this one measurement. `write` overwrites for every measurement, and a lane's second run of a day relies on it. A refusal for one measurement is a special case, and for all of them it changes every lane's re-run. The guard is each GPU's run as one command that sets the machine name itself, copies what it wrote and counts it (R4), and the adapter in the header |
| G6. A wrong SDK at `VULKAN_SDK` reads as a wrong digest, and the fallback re-pinned on the owner's word alone | Accepted. The manifest's version is checked first. No session re-pins. A second pin needs a 1.4.363.x file signed by LunarG and the owner's approval |
| G7. Question 2 left out what the install does to the machine, and overstated the other option's risk | Accepted. Both options are restated |
| G8. The manifest's path on Windows was not named, and "sets no library path" contradicted the tool | Accepted. `Bin\VkLayer_khronos_validation.json`. On Windows the run sets no library path, which the lane's unit test holds |
| G9. The script on Windows: encodings, a copy over an existing folder, and `python3` | Accepted. UTF-8 for the log and the report, a fresh directory for each copy, and `py -3` in the commands |

**The two reviews disagree on question 4, and this proposal follows `reviewer-gpu`.** `reviewer`
judged the question fair as worded and left row 150's substance to the GPU review. `reviewer-gpu`
read the two cases and the backend and found that the gate's runs answer none of the four. The tree
agrees with it, by the three files the section on row 150 cites: the ownership case never drops the
transfer, and the unflushed-range case skips wherever a device offers coherent memory. A question
that said "the device tests answer two of them" stated something false, so it was not fair, however
even its options were. The question now says that no test makes any of the four misuses, and its
options are to record that or to build the probes.

**The diff review of the second version.** `reviewer-diff` read the second version against the two
reviews and found no blocking and no serious defect, eight minor and six nits. It judged questions
1, 2 and 4 fair, the fourth once R1's block is added, and question 3 not fair until its recommended
option carries its costs. All fourteen are accepted.

| Finding | Answer |
|---|---|
| R1. The plan's section 2 still has row 150's four questions "probed on the laptop's devices" | Accepted. A block for the plan's line 114, which makes twenty. The plan's record of the text it once appended to row 150 stays: it records an edit that was made |
| R2. The desktop's first sitting is credited with work its commands do not ask for | Accepted, by revising the commands and keeping the claims. They now ask for `LocalDumps` before the run, `vulkaninfo`'s whole output, the processor's kinds of core from the `whisk` file's topology row and, after the measurements, the SDK's install where the second question is answered as recommended. They also name the machine for each system, which the review did not ask for. Session C's commands read the `LocalDumps` setting back |
| R3. "Closes no row" contradicts the rows the laptop's first session closes | Accepted, in both of the review's forms. The rule is about measurements and their files of record. A row that asks a machine for a run and names no commit is closed by that run and records its commit, and the laptop's first session is the third sitting with a commit of its own |
| R4. The laptop's commands rest on shell state an agent's calls may not keep | Accepted, and taken further. Every block is one command that sets what it needs: the machine name, the adapter's name written out, the commit. Each GPU's run copies the files it wrote into the target directory and counts them, and the first GPU's files are compared with their copies after the second's run, so an overwrite is seen and can be undone. A changed directory is shell state too, so the worktree beside the clone is gone and the clone itself is detached at the session's commit. Row 63's three runs each check out their commit in their own block and stop if the checkout missed. Where the session's shell is not PowerShell, a block is saved as a script and run as one |
| R5. Question 3's recommended option left out what it adds before the commit | Accepted. The option says the commit waits for the session, about three hours, and for a fix and a second run of the pin with the laptop opened again if it finds a defect, and that the owner sets the laptop's screen and sleep before each session and back after it. The second option says what it saves |
| R6. Row 244's re-gate was taken without a question | Accepted. It is the fifth question, in a second call |
| R7. The list of measured inputs left out nextest's configuration | Accepted. `.config/nextest.toml` and `.gitattributes` join the list, and the definition holds where the list is short |
| R8. A scope opener parked on its latch is woken by an opening and was not counted | Accepted, by the first of the review's two fixes. `park` is around both waits a worker makes without a permit, with a field that says which. The second fix, a figure that says it leaves such openers out, would keep a known wrong count: `Pool::open_gate` wakes every worker, and a next job taken by a woken opener would read as taken by a worker that was awake |
| n1. The first session cannot run "while the other two lanes finish" | Accepted: while `park-stamp` finishes at most, and on the commit's critical path |
| n2. Question 2's second option and the section disagree on the loader | Accepted. The loader stays the driver's under that option, and a loader that does not honour the exclusive path is not cured by it |
| n3. The quiet window's commands add a condition the plan does not have | Accepted: dropped. A fix of the pin is a round of `layer-windows` and moves its last merge |
| n4. Row 150's sentence for lane `vulkan` still says "probed" | Accepted: "which were to be probed on the laptop's devices, recorded as not shown" |
| n5. The caveat for 360 ticks overstates seconds | Accepted: seconds, in the proposal and in the quiet window's commands |
| n6. `park-stamp` holds the commit by nothing | Accepted. The first option says the lane is not expected to hold the commit, and what follows if it runs long |

## What this proposal could not verify

- That the file LunarG's installer leaves on a Windows disk is the file in its archive. The digest
  was read from the archive on the Mac, twice. The laptop's first session is the check, and the
  pin's row says what follows a difference.
- The Windows loader on the laptop. `reviewer-gpu` read the loader's source at the pinned SDK's tag:
  the exclusive path keeps the registry's implicit layers out, and an elevated process ignores every
  layer variable. Which loader the laptop has after the install, and which loader version first
  honours the exclusive variable, were not read. The session records the loader's version, and the
  canaries and the loader's log show whether the path was honoured.
- That a discrete GPU on a hybrid laptop presents to the panel with the driver's own implicit layer
  kept out of the run. The first version's `~implicit~` kept it out of measured runs as well. The
  first session's short `triangle` run is the check.
- Whether the Vulkan SDK is already on the laptop, and at which version, and whether the installer
  brings the Visual C++ runtime the layer's library imports. A layer that does not load fails the
  first gate run with no canary heard.
- The desktop: its CPU and whether it is a hybrid one, its GPU, its second system, and its devices'
  queue families and memory types. Rows 212 and 244 need a device whose present family is not its
  graphics family, and row 214 a family without the graphics bit. The first sitting's `vulkaninfo`
  output says whether the desktop has them, and a row it cannot run is the exit read's to re-gate.
- The laptop's second device. The plan expects an Intel integrated GPU and none was listed (section
  3's table of machines). The first session's device list answers, and with one device each session
  runs once.
- `present-encoder`'s `cargo xtask gpu --client` on a desktop GPU. The branch's head, `6c6bf19`,
  takes `--driver system` on any host and gives it KosmicKrisp's lag, set on the M1. Its lagged run
  fails where no call found a read pending, which a faster GPU may give. Such a failure on the
  laptop or the desktop is a finding about the lag, recorded as that.
- That the probe of row 244 cannot be rebuilt. Only the client's README was read for what it is. No
  patch of it was looked for.
- How long `cargo xtask measure --all` runs at eighteen measurements. "About an hour" on the M1 is
  an estimate from the lanes' runs on a loaded machine, and `record-stall` at 360 ticks a
  reservation adds seconds to it. The laptop's first session at about three hours is an estimate
  too: nothing of its length has been timed but row 63's runs.
- The cost of the two spans, with a listener and without. The crate's README gives a span about 1 ns
  with no subscriber, and the review's probe on a loaded M1 showed no ordered difference. The lane's
  paired runs are the first figures.
- The nightly the review cites as failing on one job alone. Its number was not checked against the
  run.
- The 37 rows gated at milestone 2 that neither the audit nor this proposal read. One of them may
  owe a machine something no commands file holds yet.
- A span at the latch's wait under loom and Miri. The first review's probe ran the other two places.
  Whether `record-stall` leaves an opener parked on its latch across an opening was not shown
  either: the file's count of each kind of wait answers it.
- The laptop's agent shell: which shell runs its commands, and whether it keeps a variable or a
  directory between two of them. The commands no longer rest on it.
- Whether the CI runner's Mesa registers an implicit layer of its own, which the exclusive path
  would drop from the `gpu (lavapipe)` jobs. The diff review says so from memory. Nothing chooses a
  device by it, and the lane's `gpu (lavapipe)` run is the check.
- Where `vulkaninfo` is on a Windows machine before the SDK is installed, and the SDK's name for its
  own copy. The commands say what to do where it is not found.
- Any figure. Every figure quoted is a row's, a lane's or a review's.
