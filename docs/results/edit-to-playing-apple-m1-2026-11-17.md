# edit-to-playing, apple-m1, 2026-11-17

Criterion 9: from saving a one-line change to the headless binary's report line, both edits of decision 5.9. Threshold: architecture section 22; milestone 1 plan, section 4 and decision 5.9.

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 frame workers, the default executor's |
| Commit | `a539d60461a8` |
| Profile | dev |
| Command | `cargo xtask measure --all`, which ran `cargo run --locked --target-dir ../target -p lactose-app-headless -- 1` |
| Load average | 2.98 3.95 3.67 before, 11.97 6.17 4.49 after |
| Local changes | not measured: the loop runs on `HEAD` as committed |
| Loop | in a detached worktree of `HEAD` at `measure/edit-to-playing/tree` under the target directory: one warm build before each site, then 5 one-line edits there, each timed from its save to the report line |
| Edit `sketch_system` | `crates/app-headless/src/main.rs`, after `counter.0 = counter.0.wrapping_add(1);` |
| Edit `app_function` | `crates/app/src/runtime.rs`, after `let mut builder = Executor::builder();` |
| Copy | the worktree is removed after the run; its target directory, `measure/edit-to-playing/target` under the workspace's, 2.5 GiB, is kept for the next run's warm build |

## Figures

| Figure | Value |
|---|---|
| `sketch_system.runs_s` | 1.72, 1.65, 1.73, 1.87, 1.65 |
| `sketch_system.median_s` | 1.72 |
| `app_function.runs_s` | 1.73, 3.68, 2.27, 3.66, 2.26 |
| `app_function.median_s` | 2.27 |
| `slower_median_s` | 2.27 |
| `copy_target_gib` | 2.5 |
