# Measured results

Bare-metal measurements that CI cannot make, checked in as the architecture requires (section
20). Each measurement writes a short summary and, where it has structured figures, a
machine-readable file, named by measurement, machine and UTC date:

```text
<measurement>-<machine>-<date>.md
<measurement>-<machine>-<date>.json
```

A second file of one measurement on one machine and UTC date keeps the first file's name and adds
a suffix after the date that says what it is: `-run2`, `-run3` and so on for a repeat at the same
commit, `-at-<commit>` for a run at another commit. A repeat of a run at another commit adds
`-run<n>` after its `-at-<commit>`, that file counting as the first. The tool writes every run
under the plain name, so a repeat is run with `--out` into a directory of its own and renamed
when it is checked in; its `Commit` and `Command` rows say what it ran. The row of
`docs/verification.md` that cites such a file says why it was taken.

## The milestone measurements

`cargo xtask measure --all` runs every measurement below and writes its files here;
`cargo xtask measure <name>…` runs the named ones. `--out <dir>` writes elsewhere inside the
repository. The machine is `LACTOSE_MEASURE_MACHINE` where it is set, else the CPU's model name
(`PROCESSOR_IDENTIFIER` on Windows) lower-cased with its other characters run together into
hyphens (the WHISK benchmark's rule, so `apple-m1`).

Every file opens with the header of `docs/plans/milestone-1.md`, section 4: the CPU and
operating system, the core topology the executor detected, the workers used, the commit (with
`-dirty` if a tracked file had changes), the build profile and the exact command, and the load
average before and after the run (on Windows, the processor's mean idle share over it, or why
it has none, for example a run shorter than Windows PowerShell's start and two samples; every
command but `edit-to-playing` builds what it runs before the window opens, so the window holds
the run and not its build). Times are printed and written, never asserted: a file states where
its threshold is, and the reader compares.

N, the frame workers of the plan's decision 5.1, follows its section 4: the performance cores
the platform reports or, on a part that reports no core types, its physical cores, at most
eight. `LACTOSE_MEASURE_WORKERS=<n>` sets it instead, and the file's header says so;
`spike-net`'s replays run on the spike's four workers whatever it says, as their header states.

A measurement that is a test is an ignored test of its package's one integration-test target,
`integration` (D150), selected by its module path and run in release by `cargo nextest run
--locked --cargo-profile release --package <package> --test integration --run-ignored only
--retries 0 --no-capture --no-tests=fail -E 'test(=<module>::<test>)'`; it prints each figure
as a line `measure: <name>
<value>`, once, which is what the files carry, so it can also be run by hand with the same
command. `--short` sets `LACTOSE_MEASURE_SHORT`, under which each test cuts its repetitions; a
short run's files end in `-short` and say that their numbers mean nothing.
`--short` cuts nothing of `headless-120` and `edit-to-playing`, whose files it leaves
unmarked. CI runs `--all --short --no-device` by hand only (`.github/workflows/measure.yml`) to
keep the command working on linux-x64 and windows-x64; it keeps the edit-to-playing files,
named for the runner, and discards the rest.

A measurement on a Vulkan device (`extract-50k`, `remote-upload-5k`) runs under the driver
`--driver` names (`moltenvk`, the default on macOS, `kosmickrisp`, `lavapipe`, or `system`, the
default elsewhere) on the adapter `--adapter` names or the loader's first, with no layer: every
implicit layer is disabled and no Metal validation is set. Its header adds the device's
identity as `vulkaninfo` reports it under the same environment: the device's name and vendor
and device ids, the driver's name, information and version, the device's and the instance's
API versions, the loader (the pinned SDK's, or the system's), the validation (off), the
presentation (none: the run makes no surface, so it has no present mode or refresh rate), the
memory type the ring's `MemoryClass::Upload` gets by `lactose-rhi-vulkan`'s rule, and the
variables set, paths written from the SDK's root or cut to a file name. A test that hears a
message from the device fails. `--no-device` leaves these out of `--all`; `cargo xtask gpu`
runs them short under its validators, figures discarded.

| Measurement | What | Run | Threshold |
|---|---|---|---|
| `tick-100k` | The median step of the 100k scene on one frame worker and on N (the performance cores, at most 8), S = (N·T_N − T₁)/(N − 1) and S + (T₁ − S)/8; the scene's parameters and the world hash after the last tick | `lactose-sim`, target `integration`, `cost::cost_of_a_tick_at_100k_rows` | Criterion 1: architecture section 22; `docs/plans/milestone-1.md`, section 4 and decision 5.1 |
| `physics-phase` | The sync, step and write-back spans of 100k rows with 2,000 awake bodies, a floor and 50,000 fixed boxes on Rapier, on N frame workers | `lactose-app`, target `integration`, `cost::cost_of_the_physics_phase_at_100k_rows` | Criterion 4: architecture section 22; the plan, section 4 |
| `composed-tick` | The `tick-100k` scene with the 2,000 bodies on Rapier beside it, on one frame worker and on N | `lactose-app`, target `integration`, `cost::composed::cost_of_the_composed_tick` | None: recorded beside criteria 1 and 4 (the plan, section 4) |
| `reload-100k` | The full diff of a 100k-row region reloaded in place, bodies on Rapier (D124): the reload tick's step and sync spans against a normal tick's | `lactose-app`, target `integration`, `cost::cost_of_a_100k_reload` | None: recorded (`docs/verification.md`) |
| `bone-bodies` | 200 characters with a Rapier body on each bone, against the same characters without | `lactose-app`, target `integration`, `cost::cost_of_bone_bodies_on_rapier` | None: recorded (the plan, section 4, "Bodies' cost per tick") |
| `prediction-slot` | The take and the restore of one prediction slot on the spike's thrown scene among 59 unclaimed bodies, so that five fixed colliders and 59 proxies make the 64 D139 admits, its bodies on Rapier, by part (world, physics, record; the criterion reads the world and physics snapshots, `snapshot_us`, and `restore_us`), and the slot's size. `restore_us` and `record_us` are each the median of 200 rounds; `snapshot_us` is the sum of the world and physics snapshots' medians, and `take_us` adds the record's; the `ring_bytes` times are one call each. The criterion is read at that sum, which bounds no single take | `lactose-app`, target `integration`, `spike::cost_of_a_prediction_slot` | Criterion 5: architecture section 22; the plan, section 4 |
| `rollback-seven-ticks` | A rollback that restores a slot and re-simulates seven ticks on the same scene | `lactose-app`, target `integration`, `spike::cost_of_a_seven_tick_rollback` | Criterion 7, recorded: architecture section 22; the plan, section 4 |
| `spike-net` | The netcode spike's four scenes, ten minutes of ticks each, under the clock script and on the server's clock: the correction rate, the largest absorbed difference per kind, rollback cost per tick, divergence and the rest of each replay's report, as `<scene>.<clock>.<key>`, and each replay's seconds from its first line of output to its report, `<scene>.<clock>.wall_s`. The claim on approach is not one of them. Each replay counts its server frames in which a loaded region is not `Simulated` or the spike's region is not loaded, `server_frozen_ticks`, and the most fixed colliders and proxies its prediction world held, `prediction_fixed_and_proxies`; the measurement fails, writing no file, unless every replay's first is 0, so that no rate counts a server-`Frozen` region's corrections (architecture section 9.4), and its second at most 64 (plan decision 5.5). The header states both. A short run replays the dynamic capsule alone, on each clock. Milestone 4's baseline is `spike-net-apple-m1-2026-11-16.md`, re-taken at `8941997`, not the file of 15 November, whose kinematic character's rollback figure under the clock script does not repeat (`docs/verification.md`, row 163) | `lactose-app`, target `integration`, the eight `spike::ten_minutes_of_` tests named for a scene and a clock, one `cargo nextest run` each, in turn | Criterion 7, recorded: architecture section 22; the plan, section 4 |
| `sim-skeleton` | The sim skeleton of 200 characters: (a) evaluation, (b) the stage with and without it, (c) bone bodies on the test backend | `lactose-anim`, target `integration`, `cost::the_sim_skeleton_budget` | Criterion 6, figure (b): architecture section 22; the plan, section 4 |
| `whisk` | Milestone 0's WHISK benchmark: `cargo bench -p lactose-task --bench whisk`, which writes its own `whisk-<machine>-<date>` pair, moved here | the benchmark | `crates/task/README.md` |
| `headless-120` | `cargo run -p lactose-app-headless -- 120`, dev profile, and the world hash it prints | the binary | None: its hash on x86-64 equals the M1's at the same commit (the plan, section 4) |
| `edit-to-playing` | From saving a one-line change to the report line of `cargo run -p lactose-app-headless -- 1`, dev profile, warm build, median of five edits, at each of decision 5.9's two sites: the sketch game's system in `game/game-sim/src/sketch.rs` (in the binary's own crate until lane `sketch` moved it, D152) and `start_executor` in `crates/app/src/runtime.rs`; the criterion reads the slower | `cargo` in a copy of the tree | Criterion 9: architecture section 22; the plan, section 4 and decision 5.9 |
| `extract-50k` | Extract's pass over 100,000 rows with a `Transform` and a mesh in four archetypes and sixteen regions, 50,000 moved by a kernel every tick and 10,000 presented with an `extracted` component (D157): one tick, then `Simulation::run_passes` with extract alone, the median of 60 passes after a full one, on N workers, writing the device's ring and the snapshot (`device.pass_us`, criterion 4's figure), the ring alone and the snapshot alone (their parts by difference, signed, so noise larger than a part shows below zero), and the first on one worker (`device.w1.dirty_row_ns`); the same on null (`null.*`). Each pass ends with an empty submit that signals its timeline value; a wait for a free region is outside the time and counted (`*.region_waits`) | `lactose-app-client`, target `integration`, `extract::cost_of_extract_at_50k_dirty_rows`, on a device | Criterion 4: the median on the device at most 1 ms (`docs/plans/milestone-2.md`, section 4); verification row 16 |
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer, a writer thread of class `Network` pushing every slot at 30 Hz; each frame, paced at 144 Hz by a timer for ten seconds, one job: `sample_batch` at two send intervals behind the newest state, the conversion to render space and the write to the device's ring through `lactose-presentation`'s `upload`, each part timed (`frame`, `batch`, `convert`, `write`: P50, P99, worst); then `prepare` and an empty submit. With the reads a write overlapped, so retried (each counted once), and their share of all reads, the buffer's bytes, and the frames started a period late | `lactose-app-client`, target `integration`, `upload::cost_of_the_remote_upload_of_5k`, on a device | Criterion 5: the median frame at most 1 ms, a P99 over 2 ms a finding (`docs/plans/milestone-2.md`, section 4); verification rows 19 and 102 |
| `record-stall` | The composed tick of `composed-tick` (100k rows, 2,000 awake bodies on Rapier) stepping on N workers beside the client binary's render side recording frame after frame on null, each record one `run_job` from its start to `draw`'s return, at a physics reservation of one frame worker and of two (the solver pool N less it): the record's P50, P99 and worst beside the physics phase (from the first entry into a physics node's span to the last exit), beside the rest of the tick, and with no tick, and the physics phase's and the tick's medians. 60 ticks at each after 10 of warm-up. The Windows `SetEvent` item (D162) is not yet measured | `lactose-app-client`, target `integration`, `stall::the_record_beside_the_physics_phase` | None: recorded; `RECORD_RESERVATION` is read from it (`docs/plans/milestone-2.md`, section 4 and decision 5.16; verification row 21) |

`edit-to-playing` never touches the developer's tree: it registers a detached worktree of
`HEAD` under `target/measure/edit-to-playing/tree`, edits and builds there with its own target
directory beside it, and removes the worktree however the run ends (a killed run's is removed
by the next). The uncommitted changes of the developer's tree are not in the copy, so the
command measures `HEAD`. The copy's target directory is kept for the next run's warm build;
its size, a dev build of the headless binary, is the file's `copy_target_gib` figure, and
deleting it costs the next run one cold build.

Run on a quiet machine: the numbers are wall-clock.

## Other files

| Benchmark | Command | Thresholds |
|---|---|---|
| Scalar precision (`scalar-precision-<machine>-<date>.md`) | The probe crate the file carries (manifest and source; the lock file is not carried, so pin the versions the file lists), built outside the workspace: `cargo build --release --manifest-path <probe>/Cargo.toml` from the repository root, then `<probe>/target/release/scalar-probe` | Architecture section 16; `docs/plans/milestone-1.md`, decision 5.4 |
| The spike's wake (`spike-wake-<machine>-<date>.md`) | Milestone 0's spike, as the file says | The file |
| The RHI spike (`spike-rhi-<machine>-<date>.md`) | A throwaway program outside the repository, never committed; the file names its hash, the SDK, MoltenVK and macOS versions, the environment and the commands | `docs/plans/milestone-1.md`, decision 5.10; D139 |
