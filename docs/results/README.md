# Measured results

Bare-metal measurements that CI cannot make, checked in as the architecture requires (section
20). Each measurement writes a short summary and, where it has structured figures, a
machine-readable file, named by measurement, machine and UTC date:

```text
<measurement>-<machine>-<date>.md
<measurement>-<machine>-<date>.json
```

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
--cargo-profile release --package <package> --test integration --run-ignored only --retries 0
--no-capture -E 'test(=<module>::<test>)'`; it prints each figure as a line `measure: <name>
<value>`, once, which is what the files carry, so it can also be run by hand with the same
command. `--short` sets `LACTOSE_MEASURE_SHORT`, under which each test cuts its repetitions; a
short run's files end in `-short` and say that their numbers mean nothing.
`--short` cuts nothing of `headless-120` and `edit-to-playing`, whose files it leaves
unmarked. CI runs `--all --short` by hand only (`.github/workflows/measure.yml`) to keep the
command working on linux-x64 and windows-x64; it keeps the edit-to-playing files, named for
the runner, and discards the rest.

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
| `edit-to-playing` | From saving a one-line change to the report line of `cargo run -p lactose-app-headless -- 1`, dev profile, warm build, median of five edits, at each of decision 5.9's two sites: the sketch game's system in `crates/app-headless/src/main.rs` and `start_executor` in `crates/app/src/runtime.rs`; the criterion reads the slower | `cargo` in a copy of the tree | Criterion 9: architecture section 22; the plan, section 4 and decision 5.9 |

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
