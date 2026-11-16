# edit-to-playing, github-runner-linux-x64, 2026-11-15

Criterion 9: from saving a one-line change to the headless binary's report line, both edits of decision 5.9. Threshold: architecture section 22; milestone 1 plan, section 4 and decision 5.9.

| | |
|---|---|
| CPU | Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz (linux Ubuntu 26.04.1 LTS, x86_64) |
| Topology | 1 performance + 0 efficiency cores, as the executor detected them |
| Workers | 1 frame workers, the default executor's |
| Commit | `bef3455b4a56` |
| Profile | dev |
| Command | `cargo xtask measure --all --short --out target/measure/results`, which ran `cargo run --locked --target-dir ../target -p lactose-app-headless -- 1` |
| Load average | 4.86, 3.06, 1.54 before, 4.76, 3.91, 2.11 after |
| Local changes | not measured: the loop runs on `HEAD` as committed |
| Loop | in a detached worktree of `HEAD` at `measure/edit-to-playing/tree` under the target directory: one warm build before each site, then 5 one-line edits there, each timed from its save to the report line |
| Edit `sketch_system` | `crates/app-headless/src/main.rs`, after `counter.0 = counter.0.wrapping_add(1);` |
| Edit `app_function` | `crates/app/src/runtime.rs`, after `let mut builder = Executor::builder();` |
| Copy | the worktree is removed after the run; its target directory, `measure/edit-to-playing/target` under the workspace's, 0.9 GiB, is kept for the next run's warm build |

## Figures

| Figure | Value |
|---|---|
| `sketch_system.runs_s` | 0.92, 0.93, 0.93, 0.94, 0.92 |
| `sketch_system.median_s` | 0.93 |
| `app_function.runs_s` | 3.19, 3.22, 3.24, 3.21, 3.21 |
| `app_function.median_s` | 3.21 |
| `slower_median_s` | 3.21 |
| `copy_target_gib` | 0.9 |
