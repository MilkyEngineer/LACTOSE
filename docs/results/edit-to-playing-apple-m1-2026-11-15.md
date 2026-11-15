# edit-to-playing, apple-m1, 2026-11-15

Criterion 9: from saving a one-line change to the headless binary's report line, both edits of decision 5.9. Threshold: architecture section 22; milestone 1 plan, section 4 and decision 5.9.

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 frame workers, the default executor's |
| Commit | `bef3455b4a56` |
| Profile | dev |
| Command | `cargo xtask measure --all`, which ran `cargo run --locked --target-dir ../target -p lactose-app-headless -- 1` |
| Load average | 3.79 7.12 5.87 before, 8.10 7.89 6.19 after |
| Local changes | not measured: the loop runs on `HEAD` as committed |
| Loop | in a detached worktree of `HEAD` at `measure/edit-to-playing/tree` under the target directory: one warm build before each site, then 5 one-line edits there, each timed from its save to the report line |
| Edit `sketch_system` | `crates/app-headless/src/main.rs`, after `counter.0 = counter.0.wrapping_add(1);` |
| Edit `app_function` | `crates/app/src/runtime.rs`, after `let mut builder = Executor::builder();` |
| Copy | the worktree is removed after the run; its target directory, `measure/edit-to-playing/target` under the workspace's, 2.2 GiB, is kept for the next run's warm build |

## Figures

| Figure | Value |
|---|---|
| `sketch_system.runs_s` | 1.58, 1.19, 1.51, 4.24, 1.16 |
| `sketch_system.median_s` | 1.51 |
| `app_function.runs_s` | 1.56, 1.54, 1.53, 2.25, 1.81 |
| `app_function.median_s` | 1.56 |
| `slower_median_s` | 1.56 |
| `copy_target_gib` | 2.2 |
