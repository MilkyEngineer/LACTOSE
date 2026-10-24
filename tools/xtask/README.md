# lactose-xtask

The CI driver. Every rule that CI enforces and that is not a compiler, clippy or cargo-deny
diagnostic is a subcommand here, so it runs the same on a laptop as on a runner.

| | |
|---|---|
| Tier | leaf |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

Run it as `cargo xtask <command>` from anywhere inside the repository. The alias and every
cargo command the tool runs are `--locked`: a check never rewrites `Cargo.lock` and fails if
it is out of date. The one exception is the online step of `vendor`, which resolves the lock
of a manifest it writes under the target directory.

| Command | Enforces |
|---|---|
| `tiers` | `crate-tiers.toml` against the workspace (architecture section 3) |
| `boundary` | the file facts of the determinism boundary and the unsafe policy (sections 8.2, 8.3, 20, 21) |
| `features` | identical boundary feature trees across shipped binaries, and the same features for every package two roots share where one is a harness package (section 8.3) |
| `vendor` | each crate built from `vendor/` is its patch applied to its record, the patch is within the rule, the record is the crates.io archive's, and the advisory check covers the upstream versions (sections 8.2, 8.3) |
| `sour run`, `sour compare`, `sour self-test`, `sour matrix` | SOUR, the determinism harness (section 8.3) |
| `heavy` | nothing: it bounds how many heavy commands run at once on one machine (below) |

Findings are printed to standard output as `path: message`, one per line, and nothing else
is printed there. The exit status is 0 for none, 1 for findings and 2 when a check could not
run. `heavy` is not a check and is outside this contract, except that it too writes nothing
to standard output itself.

## Where the rules live

| File | Holds |
|---|---|
| `crate-tiers.toml` | Every dependency rule: tiers, test-support crates, the boundary list, the boundary's dependency allowlists, the server denylist, exceptions |
| `tools/xtask/ci-policy.toml` | What is not a dependency rule: the unsafe list of section 21, the crates that carry the boundary lint file without being in the boundary, the shipped binaries, the harness packages |
| `deny.toml` | Licences, sources, advisories and duplicate versions, enforced by `cargo deny check` |
| `crates/core/clippy.toml` | The boundary lints; every boundary crate carries an identical copy |

A new shipped binary goes into `[feature_roots]`, and a package with simulation tests into
`[harness]`, both in `ci-policy.toml`. Nothing in the code names a crate of the workspace
except where a rule itself does: `lactose-app-server` (the server denylist), the
`lactose-app-` prefix and `lactose-sim` (the two guards below).

## `tiers`

The header comments of `crate-tiers.toml` are the rules; `src/tiers.rs` implements them as
written. A crate the file lists but that does not exist yet is fine. A workspace crate the
file does not list, a path dependency that is not a workspace member, and an `[exceptions]`
entry that names anything but backend crates are findings. A dependency whose name is in the
file is held to the rules wherever it is fetched from. A feature in `[dev_only_features]`, or
a feature of its crate that turns it on, may be turned on by a dev-dependency edge alone: a
normal or build edge enabling it, a `[features]` value forwarding to it (`name/feature` or
`name?/feature`, under the dependency's rename if it has one), the crate's `default` turning it
on, and a listed feature the crate does not declare are findings.

## `boundary`

- Each boundary crate, and each crate of `[lint_file_carriers]`, has a `clippy.toml`
  byte-identical to `crates/core/clippy.toml`, and that file still bans a required minimum
  and every route to a thread outside an executor (`THREAD_ROUTES` in `src/boundary.rs`; a
  unit test holds the list to the reference's entries that say a boundary crate creates no
  thread or cite section 16, so none can be dropped quietly behind `allow-invalid`).
- Each other crate of `[boundary_internal] any` compiles into every boundary crate and has a
  `clippy.toml` whose entries under the paths that reach files, the environment, a
  subprocess or the network, or create a thread through the standard library (`OUTSIDE_WORLD` in `src/boundary.rs`), are
  exactly those of `crates/core/clippy.toml`, compared entry by entry as parsed TOML. A
  thread the executor does not run is outside its determinism and chaos model wherever the
  code that starts it lives. The reference's float, executor, solver-pool and rayon entries are not
  required of it. A unit test holds `OUTSIDE_WORLD` to the
  reference's entries that cite section 8.2, so an entry added there is not missed.
- Nothing switches the boundary lints off. In a boundary crate, a crate of
  `[lint_file_carriers]` or a crate of `[boundary_internal] any`, `allow`, `expect` or
  `warn` of `clippy::disallowed_methods`, `clippy::disallowed_types`, their old names
  `clippy::disallowed_method` and `clippy::disallowed_type`, `clippy::all`, `clippy::style`
  or `warnings` is a finding as an inner attribute anywhere or as an outer attribute on a
  `mod`, and `allow` or `warn` of them is a finding on any item, since only an `expect`
  fails once it is no longer needed. Comments inside an attribute are ignored.
  `#[expect(..., reason = "...")]` on one item that is not a module is the sanctioned form.
  Every `.rs` file of the package is read, and every file a `#[path]` in them names (also
  inside `cfg_attr`), resolved against the including file's directory, wherever it lives; a
  named file the check cannot read is a finding. Also findings: a `.clippy.toml` in the
  crate directory or above it, those lints weakened in `[lints]` or `[workspace.lints]`, and
  `CLIPPY_CONF_DIR` or an `-A` flag for those lints in a workflow, script or cargo
  configuration file.
- A crate on the unsafe list has a `SAFETY.md`. Every other crate has
  `#![forbid(unsafe_code)]` before the first item of every crate root: library, binaries,
  integration tests, benches, examples and `build.rs`.
- Every crate has a `README.md`.
- A boundary crate names only what the allowlists in `crate-tiers.toml` let it name, and
  every third-party crate it ends up compiling through normal and build edges is vetted
  there. Architecture section 8.2 says "directly or at any depth" and a boundary crate's
  tests run under SOUR, so a dev-dependency on a workspace crate is held to the same list,
  with two additions for tests only: a `[test_support]` crate and `[boundary_internal] dev`.
- Every workspace crate a boundary crate reaches through normal and build edges, at any
  depth, through crates outside the boundary, is a boundary crate or one
  `[boundary_internal]` lets that crate name; the finding gives the chain. A boundary crate
  on the way is not passed through: its own reach is checked as its own, so `task`, which
  `ecs` alone may name, reaches every crate that depends on `ecs` without a finding. The
  same holds for what each workspace crate outside the boundary that its tests name reaches,
  against what a test may name. Cargo never compiles a dependency's dev-dependencies, so the
  crate's own dev edges are the only ones followed.
- The x86-64-v3 pin is the only `rustflags` setting in any cargo configuration file, and
  no workflow, script or cargo configuration names `RUSTFLAGS` or sets a target CPU or
  target feature (`target_cpu` and `target-cpu` are the same to rustc and to this check).
  Whole-line `#` comments are skipped; a trailing comment is scanned.
- No cargo configuration file has an `include` key, and no `.cargo` directory or cargo
  configuration file is a symbolic link: cargo would read through either what no check
  reads. `vendor` refuses both through the same function.

The scanner reads source text. It does not expand macros, so an attribute a macro writes is
not seen. It resolves a `#[path]` inside an inline `mod` block as if it stood outside it,
where rustc adds the block's name to the directory; no such attribute exists today. A plain
`mod name;` in a file reached only through `#[path]` from outside the package is not
followed.

## `features`

For each root package and each platform CI builds for, `cargo tree --target <triple>` gives
the resolved features of every package. The check keeps the boundary crates and everything
below them, tells packages compiled for the host (build-dependencies, proc macros and what
they pull in) from packages compiled for the target, and requires the results to agree.
The roots are `[feature_roots]` and `[harness]`; a harness package's tree includes its
dev-dependencies. The shipped binaries' trees must be identical: the same packages with the
same features. A harness package that is not a shipped binary is compared with every other
root on the package names both trees hold, each of which must have the same versions and
features (a version split is a finding); it may hold packages another root lacks (itself,
its own dev-dependencies) and lack packages another holds, since two harness packages
always differ that way. `--package <name>` (repeatable) compares the named packages
instead. A workspace package named `lactose-app-*` that is not in `[feature_roots]` is a
finding.

## `vendor`

- Every `[patch.crates-io]` entry of the root manifest names `vendor/<name>-<version>`; no
  other `[patch]` table exists, and no cargo configuration file sets `[patch]`, `[source]`
  or `paths`. No cargo configuration file has an `include` key, and no `.cargo` directory or
  cargo configuration file is a symbolic link or tracked by git beneath a directory the
  file walk skips (`target/`, a nested checkout, the root's `.scratchpad/`). No config alias
  passes `--config` or `-Z`, no workflow or script passes `--config` at all, and none of
  them sets `CARGO_HOME`. Every resolved package with no source that is not a member lives at
  `vendor/<name>-<version>/` and is named by such an entry; every directory under `vendor/`
  is such a package, holds that name and version, and the name resolves at no other version
  with every feature on. Nothing else lives directly under `vendor/` but those directories,
  their patches and `README.md`.
- Each directory holds the `.cargo-checksum.json` that `cargo vendor --versioned-dirs`
  writes, `.gitattributes` marks `vendor/**` `-text`, and `vendor/<name>-<version>.patch`
  beside it is a unified diff of at most fifty changed lines, comments included, and no
  added line over 200 characters, that never names `Cargo.toml`, `Cargo.toml.orig`,
  `Cargo.lock` or `.cargo_vcs_info.json`, and marks every file it changes with a `LACTOSE
  patch:` comment. It never touches a file that holds `unsafe` before or after it, and
  never creates a build script (`build.rs` or the manifest's `package.build`), though it
  may change one. Its added lines hold none of `unsafe`, `no_mangle`, `export_name`,
  `link_section`, `link_name`, `naked`, `asm`, `global_asm`, `naked_asm`, `include`,
  `include_str` or `include_bytes` as a whole identifier, comments included, and each
  hunk's added lines, read joined as written and with comments blanked, hold no `path`
  attribute and no out-of-line `mod <name>;`. The files and the counts are git's
  (`git apply --numstat -z`), and the check's own reading must agree with git's file by
  file.
- Reversing the patch on a copy under `target/vendor/` gives exactly the files and sha256
  hashes the record holds: nothing changed, added or missing. Applying it forward to that
  tree gives the copy byte for byte. Each hunk must apply where it says, in both
  directions: git placing one at an offset or with fuzz is a finding.
- `vendor/README.md` has a section per directory and none for a directory that is gone.
- Online, over a manifest under `target/vendor/upstream/` that pins each upstream version as
  the root `[workspace.dependencies]` entry does: `cargo vendor` re-derives each record and
  it must equal the committed one, and `cargo deny check advisories` reports nothing;
  `cargo-deny` skips a path source, so the workspace run cannot see these. This step
  resolves the shadow's lock without `--locked`; `--offline` passes through to both.

The token scan reads the patch's added lines only, and a word in a comment counts, so a
notice does not name them. It cannot see unsafe code reached through a macro the crate
already has, or through a build script's output; review covers those.

## SOUR

SOUR runs every simulation test once per run configuration and requires the world hashes to
agree. `cargo xtask sour matrix` prints the modes; `src/harness/mod.rs` holds them as data.

| Mode | Configurations compared against the baseline | Pull request |
|---|---|---|
| workers | 1 worker (the baseline has 4) | yes |
| repeat | the baseline again | nightly |
| layout | 1, 7 and 64 rows and 16 KiB, each with seeded permutation | 7 rows; the rest nightly |
| chaos | three seeds | nightly |
| architecture | the baseline of another machine | nightly |

- `sour run --pr | --nightly | --mode <name>…` launches `cargo nextest run` over the packages
  of `[harness]` once per configuration and reports, per test, the first tick whose hash
  differs. It also reports a test whose stated configuration is not the one it was launched
  with, and a chaos test that left no execution log. `--emit <file>` writes the baseline
  hashes.
- `sour compare <reference> <file>` compares two emitted files. Architecture mode is each
  machine emitting and one job comparing. A missing file, or a reference with no hashes, fails.
- `sour self-test` proves the runner through nextest on the stand-in simulations in
  `src/harness/stand_in.rs`: a sound one must pass every mode, and one flawed stand-in per
  mode must be reported by that mode at the tick its flaw starts. The nested build uses its
  own target directory, `target/xtask-self-test`, so it never replaces the running binary.

While `[harness]` is empty, `sour run` and `sour compare` say so and exit 0. Once
`lactose-sim` exists in the workspace an empty `[harness]` is a finding.

Runs write to `target/sour/<mode>/<variant>/`: `hashes.tsv`, and `logs/<test>.execlog` for
the execution logs of a chaos run, which are deleted if the run agreed with its reference.

### What a simulation test does

`src/harness/protocol.rs` is the contract, and its module documentation is the schema: six
`LACTOSE_SOUR_*` environment variables (schema version 1) and a tab-separated record file
(format version 2). A test:

1. calls `Session::from_env()` and builds its world and executor from `Session::config()`;
2. opens `Session::recorder(name, &applied)`, where `applied` is the configuration the
   builders report they were built with, not a copy of the session's;
3. calls `Recorder::record(tick, hash)` at every sync point it hashes;
4. under a chaos seed, calls `Session::write_execution_log(name, bytes)` on every run.

Outside the runner the session is the production configuration and records nothing, so the
same test runs under a plain `cargo nextest run`.

The protocol module depends only on `std` and on nothing else in this crate. It is written
to move, unchanged, into a foundation crate outside the boundary that the harness test crate
and the tests of `sim` and the game crates depend on; a leaf crate such as this one cannot
be depended on by any of them. Until that crate exists the module lives here.

## Heavy commands

Several agent worktrees share one machine, each with its own target directory, so cargo's
locks never make one wait for another. `cargo xtask heavy [--slots <n>] -- <program>
[<arg>…]` runs the program only while it holds one of `n` machine-wide slots (default 2):

```sh
cargo xtask heavy -- cargo nextest run --workspace --no-tests=pass
```

- A slot is an exclusive lock (`File::try_lock`: `flock` on Unix, `LockFileEx` on Windows)
  on `slot-<i>.lock` in `lactose-heavy/` under the temporary directory. The slots are tried
  in order; while all are held, `heavy` says so once on standard error, naming the holders'
  process IDs where their owner files can be read, and tries again every 200 ms. There is no queue: whichever caller tries first
  after a slot frees takes it, so a script that runs wrapped commands back to back tends to
  win the gap between them over a waiter that polls. When it takes a slot, `heavy` prints
  `heavy: slot <i> after <n>s` to standard error and writes its token, `<pid>-<nanoseconds>`,
  to `slot-<i>.owner`.
- To see who holds the slots: `lsof +D "$TMPDIR/lactose-heavy"` names the processes with a
  `slot-<i>.lock` open. A hung program holds its slot until it is stopped.
- The program inherits the standard streams and the working directory, and `heavy` exits
  with its status, or on Unix with 128 plus the signal that killed it. A command line
  `heavy` cannot parse, or a program it cannot start, exits 2, the same as a program that
  exits 2.
- The lock belongs to the `heavy` process, not to the program, and the operating system
  releases it when that process ends however it ends, so a killed agent never leaves a stale
  slot. The program never inherits the lock's handle, so killing `heavy` alone releases the
  slot while the program lives on; stop the whole process group instead.
- The program runs with `LACTOSE_HEAVY_SLOT=<i>:<token>`. A `heavy` that finds it set, the
  slot locked and the token in `slot-<i>.owner` runs its program at once without taking a
  second slot, so a wrapped script may call `heavy` again without deadlocking. Any other
  value, such as one left to a program whose `heavy` died, is reported and ignored, and
  that `heavy` takes a slot like any caller, except in the instant after another caller
  takes that slot and before it writes its token. A holder clears its owner file when it
  lets the slot go, which closes that instant for a program that outlives a `heavy` that
  ended normally; a `heavy` killed by a signal leaves its token, so on that route the
  instant remains.
- `--slots` is a per-machine convention, not a setting: the bound holds only while every
  caller passes the same count. The default is what the agent definitions use; a caller
  with a different count breaks the bound for everyone. Likewise every caller must see the
  same temporary directory (`TMPDIR` on Unix, `TMP` or `TEMP` on Windows); on macOS it is
  per user.
- The directory, the file names, the default count and the variable are a protocol between
  every worktree on the machine, each at its own commit, so none of them changes in one tree
  alone; a change needs a cut-over the coordinator announces.
- The files are never deleted: deleting a held lock file lets a second holder in. Taking a
  slot refreshes the lock file's times, since macOS deletes files in the temporary directory
  untouched for three days.

Wrap: the first build of a fresh worktree or copy, even of one crate; workspace clippy and
the cross-target clippy runs; workspace nextest and workspace doc tests; `cargo xtask sour
run` in any mode; Miri and loom runs; and any loop of repeated test runs, `--stress-count`
included. Run one wrapped command at a time per target directory, since cargo would run a
second one only after the first while both hold a slot; chain several in one `sh -c`. Never
wrap a command that waits on another. Do not wrap: `cargo fmt`, `cargo xtask tiers`,
`boundary`, `features` and `vendor`, `cargo deny`, or a single nextest run over one crate
whose build is done. CI never uses `heavy`: its runners are single-tenant.
