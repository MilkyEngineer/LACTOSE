# edit-to-playing, laptop-windows, 2026-11-15

Criterion 9: from saving a one-line change to the headless binary's report line, both edits of decision 5.9. Threshold: architecture section 22; milestone 1 plan, section 4 and decision 5.9.

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 6 frame workers, the default executor's |
| Commit | `bef3455b4a56` |
| Profile | dev |
| Command | `cargo xtask measure --all`, which ran `cargo run --locked --target-dir ../target -p lactose-app-headless -- 1` |
| Processor idle | 45.9% over the run |
| Local changes | not measured: the loop runs on `HEAD` as committed |
| Loop | in a detached worktree of `HEAD` at `measure/edit-to-playing/tree` under the target directory: one warm build before each site, then 5 one-line edits there, each timed from its save to the report line |
| Edit `sketch_system` | `crates/app-headless/src/main.rs`, after `counter.0 = counter.0.wrapping_add(1);` |
| Edit `app_function` | `crates/app/src/runtime.rs`, after `let mut builder = Executor::builder();` |
| Copy | the worktree is removed after the run; its target directory, `measure/edit-to-playing/target` under the workspace's, 1.2 GiB, is kept for the next run's warm build |

## Figures

| Figure | Value |
|---|---|
| `sketch_system.runs_s` | 5.27, 2.48, 2.40, 2.65, 2.56 |
| `sketch_system.median_s` | 2.56 |
| `app_function.runs_s` | 5.48, 4.35, 3.51, 3.61, 3.62 |
| `app_function.median_s` | 3.62 |
| `slower_median_s` | 3.62 |
| `copy_target_gib` | 1.2 |
