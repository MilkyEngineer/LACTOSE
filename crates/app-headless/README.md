# lactose-app-headless

The headless binary (architecture section 2): the executor, the IO pool and one server world
built from the game's registrations, run for a bounded number of ticks at tick rate, the
world hash reported, the world optionally saved, everything shut down in order. No network,
store, renderer or presentation.

| | |
|---|---|
| Tier | leaf |
| Determinism boundary | outside; declares nothing a world registers, so it carries no lint file (D152) |
| Unsafe | none, `#![forbid(unsafe_code)]` |

## What it holds

| Item | Contents |
|---|---|
| `main` | `crates/app/tests/headless.rs`, section 3: `Headless::start` with the executor's defaults and `WorldConfig::headless`, `Headless::run` for `TICKS` ticks, the report on stdout, `Headless::save` to `SLOT` if named, `Headless::shutdown` |
| The game | `game-sim`'s sketch game, registered as one crate, `game-sim`: a `Counter` component and a system that counts ticks, and a session applier that spawns player 1's claimed, anchored counter on the join the first frame carries. The binary holds the `CrateRegistration` and `Game` values, since a game crate may name neither `lactose-app` nor `lactose-persist`, with no setup and no migration function. The game replaces the sketch at milestone 5 |
| The cooked content | One region with an empty baseline and one asset, pinned against a store that has published everything, built in memory. The asset store replaces it at milestone 3 |
| The crash hook | `lactose-app`'s `install_crash_hook`, installed first in `main` (`crates/app/README.md`, "The crash hook"): a panic writes a text crash report (the thread, the panic, a forced backtrace) to stderr, then to a file in the crash directory (below), and calls `std::process::abort` |

## Usage

```text
lactose-app-headless [TICKS] [SLOT]
```

`TICKS` defaults to 600 (ten seconds at tick rate). With `SLOT`, the world is saved to that
file after the run, and its directory is created at start. Prints one line,
`<ticks> ticks in <frames> frames (<behind> behind), hash <hash>`, then `saved to <SLOT>`
when a save was asked for. Exits 1 with the error when the policy's response is `Stop` (a
startup or server failure). A failed save is logged and reported on stderr after the run
report, the app still shuts down in order (the policy's response to `Source::Save` is
`Log`), and the exit status is 2, so a script sees it.

| Variable | Effect |
|---|---|
| `LACTOSE_CRASH_DIR` | The crash directory: where a panic's report file goes, created when a panic needs it. Unset or empty, it is `crash` in the working directory. A run that does not panic creates nothing there |
| `LACTOSE_TEST_PANIC` | A test hook (`src/test_hook.rs`). Set and not empty, the run's first frame panics on the frame worker that runs the timer root, before any tick, and the crash hook reports and aborts. `stderr-held` panics while another thread is inside `eprintln!` and panics there a second later; `stderr-blocked` panics holding a lock another thread waits on inside `eprintln!`; on unix, `stderr-blocked-emfile` does the same with the descriptor table filled first, so that the hook cannot duplicate stderr at the panic (the test lowers the limit; above 65536 descriptors the fill panics instead); any other value panics plainly. Unset or empty, the run is unchanged |

## Rules a caller should know

- Panics abort, unconditionally (architecture section 5.1): the hook aborts in every
  profile, and the workspace's release profile sets `panic = "abort"`. Tests do not install
  the hook (D16); a test that expects an abort runs this binary in a subprocess, as
  `tests/crash.rs` does with `LACTOSE_TEST_PANIC`.
- The crash report is text, on stderr first and then in a file of its own in the crash
  directory, named by the binary, the UTC time and the process ID; one line on stderr after
  the report names the file, or says why there is none. A directory that cannot be created
  or written costs the file alone: the stderr report is written, and the process aborts.
  The hook is `lactose-app`'s, which every engine binary links; a hardware fault is the
  operating system's report, and minidumps are milestone 5's (D155).
- Traced from day one (architecture section 3): `main` installs `tracing-subscriber`'s
  `fmt` subscriber after the crash hook, writing every event at `info` and above as text to
  stderr, the assembly's at target `app` (every policy decision, every refusal) and the
  simulation's at `curd` among them; stdout holds only the report. No filter syntax, no
  colour, no `log` bridge.
- It is a feature root (`tools/xtask/ci-policy.toml`, `[feature_roots]`): its boundary
  feature tree must equal the harness packages', so it enables no feature on a boundary
  crate.
