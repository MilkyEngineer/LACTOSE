# lactose-xtask

The CI driver. Every rule that CI enforces and that is not a compiler, clippy or cargo-deny
diagnostic is a subcommand here, so it runs the same on a laptop as on a runner, except the
line-ending rule, which is a script beside it (below).

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
| `boundary` | the file facts of the determinism boundary and the unsafe policy, the nightly Miri job's split of a package, one test binary per package, and one check group per package, run by the workflow's check job (sections 8.2, 8.3, 20, 21; D150) |
| `group` | nothing: it prints the packages of a CI check job's group as cargo arguments, once the groups divide the workspace (under `boundary`) |
| `features` | identical boundary feature trees across shipped binaries, and the same features for every package two roots share where one is a harness package (section 8.3) |
| `vendor` | each crate built from `vendor/` is its patch applied to its record, the patch is within the rule, the record is the crates.io archive's, and the advisory check covers the upstream versions (sections 8.2, 8.3) |
| `sour run`, `sour compare`, `sour self-test`, `sour matrix` | SOUR, the determinism harness (section 8.3) |
| `heavy` | nothing: it bounds how many heavy commands run at once on one machine (below) |
| `measure` | nothing: it runs the milestone measurements and writes their result files (below) |
| `slang` | nothing: it installs the pinned Slang release under the target directory and prints its `slangc` (below) |
| `gpu [--driver <name>] [--adapter <name>] [--only <metal\|vulkan>] [--gpu-assisted] [--cooked <dir>]` | the GPU run's rule of decision 5.10 of the milestone 2 plan: the Metal step on macOS and the Vulkan step (below); with no `--driver`, the M1's gate, under MoltenVK and KosmicKrisp |

Findings are printed to standard output as `path: message`, one per line, and nothing else
is printed there. The exit status is 0 for none, 1 for findings and 2 when a check could not
run. `heavy`, `measure` and `gpu` are not checks and are outside this contract, except that
they too write nothing to standard output themselves. `slang` prints the path of `slangc`
there, and nothing else. Nor is `group`, which prints one line of cargo
arguments there, and nothing when it exits with 2.

## Where the rules live

| File | Holds |
|---|---|
| `crate-tiers.toml` | Every dependency rule: tiers, test-support crates, the boundary list, the boundary's dependency allowlists, the server denylist, exceptions |
| `tools/xtask/ci-policy.toml` | What is not a dependency rule: the unsafe list of section 21, the crates that carry the boundary lint file without being in the boundary, the shipped binaries, the harness packages, the tests that record hashes, the packages the nightly Miri job splits, the test targets that stay binaries of their own and the package groups of CI's check jobs |
| `deny.toml` | Licences, sources, advisories and duplicate versions, enforced by `cargo deny check` |
| `crates/core/clippy.toml` | The boundary lints; every boundary crate carries an identical copy |
| `.gitattributes` | The line-ending rule and its exceptions, which `line-endings.sh` checks (below) |

A new shipped binary goes into `[feature_roots]`, a package with simulation tests into
`[harness]` `packages`, each simulation test into `[harness]` `tests` and a test target
that must be a binary of its own into `[test_targets]`, and every new package into the group
of `[check_groups]` that the table's rule names, all in `ci-policy.toml`, in the commit that
adds them. Nothing in the code names a crate of the workspace except where a rule itself does:
`lactose-app-server` (the server denylist, which binds the `[assembly]` crates too, whether
or not the server exists), the `lactose-app-` prefix and `lactose-sim` (the two guards below),
and `lactose-task`, `lactose-input-host` and `lactose-net-host` (the loom rule).

## `tiers`

The header comments of `crate-tiers.toml` are the rules; `src/tiers.rs` implements them as
written. A crate the file lists but that does not exist yet is fine. A workspace crate the
file does not list, a path dependency that is not a workspace member, and an `[exceptions]`
entry that names anything but backend crates are findings. A dependency whose name is in the
file is held to the rules wherever it is fetched from. A feature in `[dev_only_features]`, or
a feature of its crate that turns it on, may be turned on by a dev-dependency edge alone: a
normal or build edge enabling it, a `[features]` value forwarding to it (`name/feature` or
`name?/feature`, under the dependency's rename if it has one), the crate's `default` turning it
on, and a listed feature the crate does not declare are findings. A `[server_denylist]` crate
that an `[assembly]` crate (today `lactose-app`) or `lactose-app-server` reaches through normal
or build edges, at any depth, is a finding naming the chain, whether or not the server exists;
a dev edge is not followed, and an assembly crate's chain that the server's own finding
already shows after its first step is not reported twice.

## `boundary`

- Each boundary crate, and each crate of `[lint_file_carriers]`, has a `clippy.toml`
  byte-identical to `crates/core/clippy.toml`. Every entry of that file is pinned in
  `src/boundary.rs`: it is a route to a thread outside an executor (`THREAD_ROUTES`; a unit
  test holds the list to the reference's entries that say a boundary crate creates no thread
  or cite section 16, so none can be dropped quietly behind `allow-invalid`), one of the few
  entries under `rayon::`, `rapier3d_f64::`, `parry3d_f64::` or `std::thread::` that start
  no thread (`NOT_THREAD_ROUTES`), or in `REQUIRED_LINT_PATHS`, the rest in the reference's
  order. Each is pinned with its table: `PINNED_TYPES` lists the entries that belong under
  `disallowed-types`, `PINNED_MACROS` the two of `disallowed-macros` (run-time CPU feature
  detection, `std::arch::is_x86_feature_detected` and `std::arch::is_aarch64_feature_detected`,
  section 8.2), every other one belongs under `disallowed-methods` (CPUID read directly,
  `core::arch::x86_64::__cpuid`, `__cpuid_count` and `__get_cpuid_max`, which returns
  CPUID's registers for the leaf it is given, among them, with `allow-invalid` for
  aarch64), and clippy applies a path
  only in its own table. An entry the file drops or moves to another table, and an entry it
  adds that none of the three lists holds, are findings, so the reference's banned set equals
  the union of the three, each entry in its table. `tests/boundary_lints.rs` runs clippy with
  the reference over a probe that calls each route D119 bans through the solver's math crates
  (simba, num-traits, num-complex, glam, glamx, approx), one line per route, and requires
  every line refused: each of those entries carries `allow-invalid`, which would otherwise
  hide a path that stopped resolving. The same probe calls the feature detection macro of the
  target it is built for (the x86 one by its root path, `std::is_x86_feature_detected!`) and
  requires it refused, and on x86-64 calls CPUID by its `core` path, its `std::arch`
  re-export and a renamed import, and `__get_cpuid_max`, each refused. Rapier's `RevoluteJoint::angle` and parry's `VectorExt::angle` are
  pinned but not in that probe, which does not compile Rapier.
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
  `warn` of `clippy::disallowed_methods`, `clippy::disallowed_types`,
  `clippy::disallowed_macros`, the first two's old names
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
- `[unsafe_crates]` equals the table of architecture section 21 (found by its heading,
  `## <n>. Unsafe policy`): each backticked name in a row's first cell is a crate, its last
  path segment the role, so `tools/cook` is `lactose-cook`. A missing document, heading or
  table, or a row that names no crate, is a finding, not a pass.
- A crate on the unsafe list has a `SAFETY.md`. Every other crate has
  `#![forbid(unsafe_code)]` before the first item of every crate root: library, binaries,
  integration tests, benches, examples and `build.rs`.
- Every crate has a `README.md`.
- A boundary crate names only what the allowlists in `crate-tiers.toml` let it name, and
  every third-party crate it ends up compiling through normal and build edges is vetted
  there. Architecture section 8.2 says "directly or at any depth" and a boundary crate's
  tests run under SOUR, so a dev-dependency on a workspace crate is held to the same list,
  with two additions for tests only: a `[test_support]` crate and `[boundary_internal] dev`.
  Each name on a boundary crate's `normal` or `dev` list is one its manifest names; a crate
  it reaches only through others is vetted on `transitive`, so a list cannot grow
  permissions the manifest does not use.
- Every workspace crate a boundary crate reaches through normal and build edges, at any
  depth, through crates outside the boundary, is a boundary crate or one
  `[boundary_internal]` lets that crate name; the finding gives the chain. A boundary crate
  on the way is not passed through: its own reach is checked as its own, so `task`, which
  `ecs` alone may name, reaches every crate that depends on `ecs` without a finding. The
  same holds for what each workspace crate outside the boundary that its tests name reaches,
  against what a test may name. Cargo never compiles a dependency's dev-dependencies, so the
  crate's own dev edges are the only ones followed.
- The x86-64-v3 pin, as `rustflags` and as `rustdocflags` with the same flags (cargo passes
  `rustflags` to rustc only, so doctests need their own), is the only flag setting in any
  cargo configuration file, and no workflow, script or cargo configuration names
  `RUSTFLAGS` or `RUSTDOCFLAGS` or sets a target CPU or target feature (`target_cpu` and
  `target-cpu` are the same to rustc and to this check). Whole-line `#` comments are
  skipped; a trailing comment is scanned.
- The packages whose sources include the harness protocol (`src/harness/protocol.rs`) by
  `#[path]` are exactly `[harness]` `packages`: a package that records and is not listed
  is never launched, and one listed that does not record proves nothing.
- D99: outside the boundary, `impl SimulationAsset` and a `Loader` for a kind the
  simulation reads appear only in code compiled for tests, so a kind is declared, and
  decoded, where section 8.2 holds its loader. The kinds are the implementing types of the
  shipped `impl SimulationAsset`s of the boundary crates. A shipped `impl Loader` outside
  the boundary is a finding when its `type Asset` names a kind by its last path segment
  (`lactose_anim::Clip` is `Clip`, and `<lactose_anim::Clip as T>::Name` is `Clip`), names
  a type parameter of the impl (`T`, `Vec<T>`, `L::Asset`, `<L as Loader>::Asset`), or is
  not in its body as written.
  Shipped code is read from the root of each library, proc-macro and binary target,
  wherever it lives, through every out-of-line `mod` (by `#[path]`, by the file layout, and
  inside inline modules, each of which adds its name, or the directory its own `#[path]`
  names, to the directory) and every `include!` of a string literal, placing each one's own
  modules where rustc does. An item under a `cfg`
  that holds only for `test` or a `[dev_only_features]` feature (or one that turns it on)
  is skipped, as are `tests/`, `benches/`, `examples/` and the build script. A file reached
  by a shipped route after a test-only one is read again as shipped, and a file under
  `src/` that no route reaches is read as shipped, unless its own inner `cfg` says
  otherwise. The scan reads tokens and compares names as written: an impl a macro writes is
  not seen, nor an `include!` whose path is not a literal (`concat!(env!(...))`), nor a
  trait or kind brought in under another name (`use ... as`, a type alias, a projection
  such as `<Self as Decodes>::Kind`); a type outside
  the boundary that shares a kind's name is reported; and an `impl ... for` header in a
  `macro_rules!` body is read as if it were an impl.
- What a world registers and runs is written under the boundary lint file (project review
  2, X1). A crate that is neither a boundary crate nor one of `[lint_file_carriers]` ships
  no `#[system]`, no `#[component]` or `#[resource]` whose arguments lack `derived`, and no
  `impl` of `SessionApplier`, `InputApplier` or `StagedState`, the traits through which the
  input sync point writes into the world, and no hand-written `impl Reflect`, which declares
  a component or a resource no attribute shows. Pass systems are `#[system]`s like any
  other. Shipped code is read as for D99, and attributes and traits are compared by their
  last path segment as written: one brought in under another name (`use ... as`), in
  code produced by a macro the crate's source does not spell out (another crate's macro or
  derive), or inside a `cfg_attr`, is not seen. `lactose-present-api` is a carrier for the presentation components and extract's pass system, and no binary is one: the headless binary's sketch game is `game-sim`'s (D152).
- No cargo configuration file has an `include` key, and no `.cargo` directory or cargo
  configuration file is a symbolic link: cargo would read through either what no check
  reads. `vendor` refuses both through the same function.
- The nightly Miri job (`src/miri.rs`): for each package of `[miri_split]`, the entries of the
  `miri` job's matrix in `.github/workflows/nightly.yml` that name it run, together, each test
  of its library, of each integration test target `cargo metadata` lists and of its doc tests
  exactly once: one entry runs a target unfiltered, or exactly two divide it, one naming
  libtest filters after `--` and the other skipping each of them with `--skip`, which runs
  each test once whatever the tests are called. A finding names each target no entry runs, one
  entry runs only part of, or several entries run otherwise, with the entries. The check reads
  one shape of the job, today's, and refuses any other rather than reading it: a matrix of
  `include` alone (a key beside it folds the entries into its combinations), no `if` or
  `defaults` on the job and no `defaults` on the workflow, and one item of the job's one
  `steps` list that uses `matrix.args`,
  `run: cargo +$MIRI_TOOLCHAIN miri test --locked ${{ matrix.args }}`, with only `env` beside
  it, so that the words after an entry's `--` are libtest's and nothing follows them. The
  job's other steps are not read. The reader takes the YAML the job is written in (block
  mappings and sequences, plain, quoted and block scalars, LF or CRLF line breaks, space
  indentation) and refuses at its line what it does not read on the way to and inside the
  entries: another line break (a lone CR, NEL, LS or PS), a tab indent, a document marker or
  directive, a line of a mapping on the way that is not a plain or quoted `key:` (an explicit
  `? key`, or a list at the indent of its key, so that entries or steps written at the indent
  of `include:` or `steps:` are refused, as the real file never writes them), a key it follows
  (`jobs`, `miri`, `strategy`, `matrix`, `include`, `steps`) twice in its mapping, and a key
  twice in one entry. Every entry names exactly one workspace member with a plain `-p NAME` or
  `--package NAME`; `--workspace`, no `-p`, a second `-p`, `-p=`, `-pNAME`, `NAME@VERSION` and
  a glob are refused. Its `args` are split as the run step's bash splits them; an unquoted
  glob character, which bash would expand against the checkout, a line break, which ends the
  command, and other shell syntax are refused. Of cargo's options it reads `-p`, `--lib`,
  `--test` and `--doc`, and refuses any other; after the first `--`, filters and
  `--skip FILTER`, and it refuses a `--` with nothing after it, an empty filter, `--skip=`, a
  second `--` and every other libtest option (`--exact` would change what a filter matches).
  Each filter, named or skipped, is `<module>::`, where the root of a test target the entry
  selects declares the module (at its top, out-of-line or inline), and no `cfg` naming
  `miri` may compile it out: on its declaration (or a `cfg_attr` there that yields a `cfg`),
  at the top of its file or at the top of the root. A module renamed, removed or compiled out
  under Miri is reported, where the filter would otherwise select nothing, or move the
  module's tests to the entry that skips it; a test of the module may still be gated or
  ignored under Miri on its own. A package in `[miri_split]` must name its targets: an entry
  that names none is refused, since cargo's default run then follows the manifest's `test` and
  `required-features`, which the counts do not model. An explicit `--test` runs a
  `test = false` target and fails the job on one whose `required-features` it does not enable,
  so neither needs a rule. A package outside `[miri_split]` that two or more entries name by
  target or filter is reported as a split the policy does not list; an entry that runs the
  package whole and unfiltered drops none of its tests, so entries that do, as
  `lactose-task`'s two do under two environments, are no split, and neither is one of them
  beside one entry by target. An empty `[miri_split]` is reported. A binary, bench or example
  target of a split package whose `test` is on, which `cargo test` would run and the split
  cannot select, is reported in its manifest. A `--test` value with none of `*?[]` is a target
  name, and any other a pattern of the `glob` crate as cargo 1.99 and the job's
  `MIRI_TOOLCHAIN` (nightly-2026-10-04, cargo 1.101.0-nightly) build it: case-sensitive, `?`,
  `*`, `**` only alone, `[...]` and `[!...]` with ranges and a leading `]`, no escape. The
  unit tests' cases are cargo's own selections and refusals, run through
  `cargo test --no-run --test` on cargo 1.99.0, and a set of them on both cargos. A value that
  selects nothing, which cargo refuses, is reported, as is a pattern with a path separator,
  which the check does not read.
- One test binary per package (`src/test_targets.rs`, D150): a package with more than one
  integration test file builds them as one target, `integration`, whose root
  `tests/integration.rs` declares every other file of `tests/` as a module, with
  `autotests = false` in the manifest; a package of one test file may leave it a target under
  its own name. The module is named after the file, except where the file's name holds a
  codename of `CLAUDE.md`'s Names table, which no module may carry: then the file is declared
  under a name for what it holds, with `#[path]` (`#[path = "sour.rs"] mod recorded;`). Shared
  support (`common`, `glue`) is declared once in the root, and a file reaches it as
  `crate::common`. The tests of one binary share a process under `cargo test` and
  `cargo miri test` (nextest alone gives each test its own), so a test file that installs what
  a process has one of (a `#[global_allocator]`, the tracing subscriber), measures what is one
  per process (its CPU time, rayon's global pool) or asserts when a file lock is let go (which
  a process another test spawns holds until its exec), stays a binary of its own, as does a
  target a tool builds alone; each is listed in `[test_targets]` `separate` as
  `package::target` under its reason. The check walks each test target's module tree from its
  root as rustc resolves it (out-of-line and inline modules and literal `include!`s, each file
  relative to the one that declares it). Findings: a package with more than one test target
  outside that list, an entry that names no test target, a file cargo would make a target of
  by itself (`tests/*.rs`, `tests/*/main.rs`) that no test target compiles, and a module of a
  test target whose name holds a codename (each `_`-separated word, lower-cased). Whether a
  file needs a process of its own is not checked: the list holds what review found. A test
  that runs its own binary again names itself by its path in the binary, `module_path!()`
  below the crate root, not by its bare name.
- Check groups (`src/check_groups.rs`, milestone 2 plan decision 5.15): each workspace
  package is in exactly one group of `[check_groups]`, which CI's check jobs build, lint and
  test one each. Findings: a package in no group, which no check job would build or test; a
  package in two groups, or twice in one; an entry that names no workspace package; and a
  group that names none, which as cargo arguments would select the whole workspace. The two
  group names, `base` and `physics`, are fixed in `src/policy.rs`. `boundary` also holds the
  `check` job of `.github/workflows/ci.yml` to the table, so the policy job reports a workflow
  that leaves a group untested, whichever groups the check jobs run: the job's head, whose
  matrix names each group in the table's order with no `include`, `exclude`, condition or
  other key, and its Packages, Clippy, Test and Doctests steps, which carry the group's
  packages with no condition. Each is compared as the whole text of its block, after
  whole-line comments are dropped, so a key added anywhere in one is a finding; an edit to
  those lines of the workflow is an edit to `src/check_groups.rs`. `cargo xtask group <name>`
  prints the group's packages as one line of cargo arguments, `-p <package>` each in the
  table's order, which the check job passes to clippy, nextest and the doc tests; it prints
  nothing and exits with 2 when the groups do not divide the workspace, so a job never tests
  a division that leaves a package out.

The scanner reads source text. It does not expand macros, so an attribute a macro writes is
not seen. In the lint check it resolves a `#[path]` inside an inline `mod` block as if it
stood outside it, where rustc adds the block's name to the directory, and reports a
`#[path]` on an inline block, which names a directory, as naming no source file; no such
attribute exists today. A plain
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

Every root's tree compiles glam for the target with its `scalar-math` feature (D119):
without it, glam's `Vec3A` and `Vec4` `min` and `max` are the target's SIMD instructions,
which treat NaN differently on x86-64 and aarch64. It also compiles glam with `libm`, so
glam's transcendentals are the pinned `libm` crate's rather than the platform's float
library (D105).

No workspace package but `lactose-task` names loom, read from `cargo metadata`: no
dependency on it under any rename, no dependency declaration or `[features]` value that turns
on loom or a feature named `loom`, and no feature named `loom`. Features unify across a
build, so one package turning on `lactose-task/loom` would make every executor in that build
loom's model. The one exception is the packages that build loom models of a lock they ship,
`lactose-input-host` and `lactose-net-host`: each may have an optional dependency on loom
and the feature `loom = ["dep:loom"]`, and nothing more.

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

## Line endings

`bash tools/xtask/line-endings.sh` checks the line-ending rule of architecture section 20
(D148) against git's own account of every tracked file: its line endings in the index and in
the working tree, and the attribute that governs them, from `git ls-files --format` (git
2.38 or later). It lists the whole tree, with paths from the root, whatever directory it
runs in. It is a script and not a subcommand, so that a runner checks its checkout before
any build. `merger` runs it at every merge, where the index and the attributes read the same
on every host; each of CI's check jobs runs it first, on the three hosts, since each working
tree is its host's own.

It accepts a file in one of three shapes and refuses every other:

| Path | Attribute | Index | Working tree |
|---|---|---|---|
| outside `vendor/` | `text=auto eol=lf` | `lf` or `none` | `lf` or `none` |
| under `vendor/` | `-text` | any | any |
| outside `vendor/` | `-text` | `-text` | any |

`none` is a file with no line break, and `-text` in the index a file git reads as binary,
which outside `vendor/` passes only under a `-text` or `binary` line. Under `vendor/` the
script asks only for `-text`; that the root file's vendor line exists is `vendor`'s check as
well. A file of a kind named `binary` whose bytes git reads as text is refused,
and the change that adds it admits its path in the script.

Each refusal prints `path: i/<index> w/<worktree> attr/<attribute>` and, below it, every
remedy that applies, the attribute's first, since `git add --renormalize` stores LF only
under the rule:

- Attribute `-text` outside `vendor/` over an index that is not `-text`: a `-text` or
  `binary` line reaches a text file, and is narrowed or removed.
- A path under `vendor/` whose attribute is not `-text`: the vendor line is restored as the
  last line of the root `.gitattributes` that reaches it, and a `.gitattributes` under
  `vendor/` that sets an attribute on it is removed (D123). For a vendored file this remedy
  is printed alone.
- Any other attribute: a line takes the file out of the rule, or the rule's line is gone.
  On a host that converts, that is why its working tree is CRLF.
- Index `crlf` or `mixed`: `git add --renormalize -- <path>`, the path quoted for the shell;
  a plain `git add` keeps the CR. A name git printed quoted is not repeated as an argument,
  since its quoting is git's, not the shell's.
- Working tree `crlf` or `mixed` over an index `lf`, under the rule: the checkout predates
  the rule, or a CRLF edit was committed. With nothing uncommitted and no merge in progress,
  `git rm --cached -r -q :/` then `git reset --hard` refresh it; `git add --renormalize`,
  `git checkout`, `git checkout-index -f -a` and `git reset --hard` alone do not. In CI,
  whose checkout is fresh, the rule does not hold on that host, and the decision returns to
  the planner.
- Index `-text` under the rule: a stray CR or NUL is removed from a text file, or a kind that
  is not text is named `binary`.
- An empty index or working-tree field: a symbolic link or submodule, which the rule does not
  admit, or a file missing from the checkout, which is restored.
- Anything else, which is a working tree `-text` over an index `lf` or `none`, or `crlf` or
  `mixed` over an index `none`: the working file holds an uncommitted edit, which is
  reverted. Committed instead, it is refused again by the remedy its new line matches.

The last line is `line-endings: <n> files read, none refused` with exit status 0, or
`line-endings: <r> of <n> files refused` with 1. It exits 2, with a message on standard
error, when `git ls-files` fails or lists no file, or when it reads fewer lines than git
listed. A copy of the script checked out with CRLF runs itself again without the carriage
returns, so it still refuses what git lists; run through standard input, a pipe or a process
substitution, such a copy cannot read itself again and exits 2. It cannot see a file that
is not text and that git took for text, since `git add` rewrote it; the change that adds a
kind names it `binary`. A copy whose lines end in a lone carriage return is one line to
bash, a comment, and it exits 0 with no output and no last line. git takes such a file for
binary and stores it as it is, so only the review of its diff, where the script collapses to
one line, sees it.

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
  with, and a chaos test that left no execution log. The tests that recorded in the baseline
  run must be exactly `[harness]` `tests`: one listed that recorded nothing was renamed,
  ignored, deleted or stopped recording, and one that recorded unlisted was added without
  the list. `--emit <file>` writes the baseline hashes. `--release` builds the tests with
  the release profile (`--cargo-profile release`), which drops debug assertions and overflow
  checks, and writes to `target/sour-release`.
- `sour compare <reference> <file>` compares two emitted files. Architecture mode is each
  machine emitting and one job comparing. A missing file, or a reference with no hashes, fails.
  Nightly also compares each platform's dev baseline with its release one, from
  `sour run --mode architecture --release` (project review 2, X5).
- `sour self-test` proves the runner through nextest on the stand-in simulations in
  `src/harness/stand_in.rs`: a sound one must pass every mode, and one flawed stand-in per
  mode must be reported by that mode at the tick its flaw starts. The nested build uses its
  own target directory, `target/xtask-self-test`, so it never replaces the running binary.

While `[harness]` is empty, `sour run` and `sour compare` say so and exit 0. Once
`lactose-sim` exists in the workspace an empty `[harness]` is a finding.

Runs write to `target/sour/<mode>/<variant>/` (`target/sour-release/...` under `--release`):
`hashes.tsv`, and `logs/<test>.execlog` for
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

## Measurements

`cargo xtask measure (--all | <name>…) [--short] [--out <dir>]` runs the measurements the
milestone criteria rest on and writes each one's result files to `docs/results` (or
`<dir>`, which must be inside the repository; an absolute one is written relative to its
root in the files), with the header of `docs/plans/milestone-1.md`, section 4. The names, what each
runs and where each threshold is stated are in `docs/results/README.md`; `--all` runs every
one in that order and, when one fails, still runs the rest and exits 2 at the end. It runs
`cargo nextest run`, `cargo bench` and `cargo run`, all `--locked`, and their output goes to
standard error as it comes.

- Before the measurements, an ignored test prints the core topology the platform reports
  and the default executor's frame workers, which every file's header carries.
- A measurement that is a test prints each figure as a line `measure: <name> <value>`. The
  files carry those figures, the `.json` with each value a number where it reads as one
  (`true`, `false`, `null` or an array where it reads as that), and the `.md` with the
  test's whole output. A test that prints no figure, a figure line that does not read, or a
  figure named twice fails its measurement. nextest runs it with `--retries 0`, so that no
  failed attempt's output or time is read.
- `spike-net` runs the netcode spike's ten-minute replays one `cargo nextest run` at a time;
  each prints its report as one JSON object, whose values the files carry under
  `<scene>.<clock>.` (nested keys joined by dots, an array as its JSON text), with the
  replay's seconds from its first line of output to its report, the build left out, as
  `<scene>.<clock>.wall_s`. A replay that prints no report, or two, or a
  line opening with `{` that is not one, fails the measurement, and so does one whose
  `server_frozen_ticks` is not 0 (a server frame with its region not `Simulated`) or whose
  `prediction_fixed_and_proxies` is over 64 (plan decision 5.5): no file is written then.
  The replays run on the spike's four workers whatever `LACTOSE_MEASURE_WORKERS` says, and
  the header's Workers row says so.
- The cost tests run on N frame workers: the performance cores the platform reports or, on
  a part that reports no core types, its physical cores, at most eight (the plan's section
  4). `LACTOSE_MEASURE_WORKERS=<n>` sets N instead: a count above zero, refused before
  anything builds otherwise, passed to the tests and recorded in the header (not to
  `spike-net`'s replays, above). Empty, it is unset, here and in the tests.
- `--short` sets `LACTOSE_MEASURE_SHORT=1` for the tests and `WHISK_BENCH_RUNS=100` for the
  benchmark, cuts `spike-net` to the dynamic capsule on each clock, since a replay is ten
  minutes of ticks however it runs, and names the files `…-short`. It cuts nothing of
  `headless-120`, which has no
  repetitions, or `edit-to-playing`, which keeps its five so that a runner's short run still
  yields the criterion's figure, and leaves their files unmarked.
- The machine in the file names is `LACTOSE_MEASURE_MACHINE` where it is set, else a slug
  of the CPU's model name, or of `PROCESSOR_IDENTIFIER` on Windows (the WHISK benchmark's
  rule, so that its files and these share the name). The benchmark's summary is prefaced
  with this command's rows: the command, the profile, the operating system and the load.
- The load is `uptime`'s averages before and after a run; on Windows, the processor's mean
  idle share over it, which Windows PowerShell samples every second from WMI's
  `Win32_PerfFormattedData_PerfOS_Processor` (`PercentIdleTime` of `_Total`), the first
  sample dropped. Where there is none the row says why: Windows PowerShell could not be
  started, or the run's seconds and that the sampler printed no sample, or only the first,
  in them, which a run shorter than PowerShell's start and two samples a second apart gets
  (the Windows laptop's `physics-phase` and `rollback-seven-ticks` files of 2026-11-15). Every
  command but `edit-to-playing`, whose builds are what it measures, first builds what it
  runs with cargo's `--no-run` form of the same command (`cargo build` for `headless-120`)
  before the window opens, so the window holds the run and not its build, on a cold tree
  too, though a load average taken just after that build still carries it; a failed build
  fails the measurement. The
  sampler wakes WMI once a second during a Windows run,
  a perturbation not measured. The Windows CPU row is the registry's
  `ProcessorNameString`. The Windows paths have not been run.
- `edit-to-playing` builds in a detached worktree of `HEAD` under
  `target/measure/edit-to-playing/`, removed again when it ends, with a target directory of
  its own that it keeps for the next run's warm build. Its Commit row is `HEAD` without
  `-dirty`: local changes are not measured.

It loads the machine for minutes: under other agents' load, wrap it in `heavy`, and read its
numbers as steering only. The numbers that prove a criterion come from a quiet machine.

## Slang

`cargo xtask slang` installs the Slang release that `tools/cook/slang.toml` pins (decision 5.8
of the milestone 2 plan) and prints its `slangc`, which `ricotta --slangc` takes. The pin is
the version and, for each of `linux-x86_64`, `macos-aarch64` and `windows-x86_64`, the
release archive's URL and SHA-256; the pin is refused if a host is missing or another is
named, a digest is not 64 lowercase hexadecimal digits, or a URL is not that version's and
host's archive.

- The archive is fetched with the host's `curl` into `target/slang/download/` and refused,
  and deleted, unless its SHA-256 is the pin's. A download whose digest is already the pin's
  is not fetched again.
- It is unpacked with the host's `tar` into `target/slang/<version>.partial/`, its `slangc`
  must report the pinned version, and the directory is then renamed to
  `target/slang/<version>/`: an interrupted install leaves nothing that passes for one.
- With the pinned release already installed it does nothing but print the path.
- On Windows `curl` and `tar` are the copies in `System32`, since a `tar` found first on the
  path (Git's) reads `C:` as a remote host.

The cook's own copy of the pin is the same file, read at build time: a `slangc` that reports
another version, the SDK's among them, is refused by the cook too.

## GPU runs

`cargo xtask gpu [--driver <lavapipe|moltenvk|kosmickrisp|system>] [--adapter <name>] [--only
<metal|vulkan>] [--gpu-assisted] [--cooked <dir>]` is the command decision 5.10 of the
milestone 2 plan names. It cooks, then runs the Metal step on macOS and the Vulkan step on
every host the layer's pin names; `--only` runs one of them, and the Metal step asked for on
another host than macOS exits 2. The Metal step is the same under each driver name, since the
driver selects a Vulkan loader's environment and the Metal step uses none.

With no `--driver`, on macOS, it is the M1's gate (the owner's ruling of 19 November 2026):
the Metal step once, then the Vulkan step under MoltenVK, with Metal API and shader
validation, and under KosmicKrisp, without them, one after the other, and a failure under
either fails the run. Each catches a missing barrier the other misses: MoltenVK the local
read's, by its pixels, and KosmicKrisp the one between two dispatches, which Metal's serial
compute encoder hides under MoltenVK. KosmicKrisp runs without Metal's validation because
its queue aborts under API validation and its heap's textures are not resident under shader
validation, defects of that driver. `gpu --gpu-assisted` with no `--driver` runs KosmicKrisp
alone, since the layer's instrumented shaders fail in MoltenVK's SPIRV-Cross. On another host
a run names its driver. `--driver <name>` runs one driver, for development, and `--adapter`
goes with `--driver system` alone. Every step runs before a failure fails the run, so each
writes its report.

1. It installs the pinned Slang as `slang` does, and cooks `shaders/` with `ricotta --slangc
   <it> --code target/gpu/code --shaders target/gpu/shaders` into
   `target/gpu/shaders.pack`, removing the code and shaders of an earlier run first. With
   `--cooked <dir>` it cooks nothing and takes the code in `<dir>`, which `ricotta --code`
   wrote on another runner; that is the Metal step's alone, so it needs `--only metal`.
2. It runs `swift tools/xtask/metal/pipelines.swift`, the command-line tools' `swift`, under
   Metal API validation (`MTL_DEBUG_LAYER=1`, warnings to the log) and Metal shader validation
   (`MTL_SHADER_VALIDATION=1`, reports to standard error), as the Metal spike set them:
   - first each validator's canary, in a process of its own: an unbound buffer, which API
     validation must abort on with `missing Buffer binding at index 0`, and the `canary`
     kernel's store past a buffer's end, at offset 4194304, which shader validation must
     report and let the process complete. The wording differs between Xcodes, so `CANARIES`
     in `src/main.rs` lists each phrasing seen, compared without regard to ASCII case (Xcode
     16.4 writes `buffer`): for the store, `Invalid store in 'device' or 'constant' address
     space at offset 4194304, executing compute function: "canary"` (Xcode 27.2) and `Invalid
     device store at offset 4194304, executing kernel function: "canary"` (Xcode 26.6). A
     canary that is silent, or says only something else, fails the run;
   - then the step: the device is the first of `MTLCopyAllDevices`, and the run fails with
     none. It prints the device's facts: its name, its argument buffers tier, its highest
     Apple GPU family and whether it is of the Metal 3 family. Code that takes the heap
     needs argument buffers tier 2, for 1024 textures in one argument buffer, and fragment
     code that reads a colour needs an Apple GPU family, for framebuffer fetch: the step
     fails naming each one the device lacks, before it makes any pipeline state;
   - each cooked MSL is compiled at run time as MSL 3.1 and each metallib loaded, and each
     makes a pipeline state with every function constant given the zero of its type, the
     `MTLDataType` of its MSL type, which the cook holds to the cooked format's
     `ConstantType` (the step reads the code alone, which `--cooked` takes from another
     runner, and not the pack the format is encoded in): a
     source's fragment entries each with its one vertex entry, a vertex entry alone as a
     depth-only pipeline, a kernel as a compute pipeline. No pipeline is run, so shader
     validation is loaded, as its canary shows, and sees no execution of the engine's code.
3. Every line the step writes on standard error counts, except the two validators' banners,
   and one fails the run. It writes `target/gpu/<driver>-metal.txt`: the driver, the Slang
   version or the directory the code was taken from, the validators and what each canary
   was heard to say, as the validator wrote it, so a run shows a runner's wording, the
   device's facts, the system's version, one line per pipeline state, and the messages
   counted.
4. The Vulkan step runs every device test: each ignored test of the test targets
   `DEVICE_TESTS` in `src/main.rs` lists, which are `lactose-rhi-vulkan`'s library's unit
   tests, of its `canary` module alone, so that no other ignored unit test runs as a device
   test, and its `integration` binary, each in a process of its own, built with `cargo test
   --no-run` and run by its exact name. Under each driver it sets the loader's environment for its children:
   - the layer is the Khronos validation layer of the Vulkan SDK that
     `tools/xtask/vulkan-layer.toml` pins, refused unless its library's SHA-256 and its
     manifest's version are the pin's. On Linux the SDK's archive is fetched into
     `target/vulkan-sdk/`, refused unless its SHA-256 is the pin's (LunarG's published
     digest), and its loader, layer and manifest unpacked; on the Mac the SDK is the one the
     installer put at `VULKAN_SDK`, or at `~/VulkanSDK/<version>/macOS`. The loader is the
     SDK's (`LD_LIBRARY_PATH`, or on macOS `DYLD_FALLBACK_LIBRARY_PATH`, since
     `DYLD_LIBRARY_PATH` would also load the SDK's copy of a driver the loader opens by its
     absolute path, and the loader reports the difference);
   - the layer is loaded as an implicit layer, from a copy of its manifest under
     `target/gpu/layers/` that names its library by its absolute path, with `VK_LAYER_PATH`
     set to an empty directory: a layer forced through `VK_INSTANCE_LAYERS` or
     `VK_LOADER_LAYERS_ENABLE`, or found twice, makes the loader warn, and the warning reaches
     the debug messenger, which counts it. Before the run sets its own, it removes these
     inherited variables (`CLEARED` and `CLEARED_PREFIXES` in `src/vulkan.rs`): the
     families `VK_KHRONOS_VALIDATION_*` and `VK_LAYER_*` (the layer's settings under both
     the names its manifest gives, and the layer paths), `VK_LOADER_*`, MoltenVK's `MVK_*`,
     Mesa's `MESA_*` (KosmicKrisp's and lavapipe's) and Metal's `MTL_*`, and by name
     `VK_INSTANCE_LAYERS`, `VK_ICD_FILENAMES`, `VK_DRIVER_FILES`, `VK_ADD_DRIVER_FILES`,
     `VK_ADD_LAYER_PATH`, `VK_IMPLICIT_LAYER_PATH`, `DYLD_LIBRARY_PATH`,
     `METAL_DEVICE_WRAPPER_TYPE` (which turns Metal's API validation on, as `MTL_DEBUG_LAYER`
     does), `LACTOSE_VALIDATION_OFF`, `LACTOSE_GPU_ADAPTER` and `LACTOSE_GPU_SHADERS`. It
     names each it removed in its file. A driver's other variables are not cleared. What a
     run needs of them it takes as an argument, or sets itself after the removal;
   - the driver is `VK_DRIVER_FILES`: lavapipe's manifest in `/usr/share/vulkan/icd.d/`,
     MoltenVK's or KosmicKrisp's in the SDK's `share/vulkan/icd.d/`; under `system` the
     loader's own search, and `--adapter <name>` selects the device by a part of its name
     (`LACTOSE_GPU_ADAPTER`, which the tests' request reads);
   - the validators are core, synchronisation (with its check of a load after a store) and
     thread-safety validation; with `--gpu-assisted`, GPU-assisted validation and
     thread-safety validation, with core and synchronisation validation off, as the layer
     advises beside it, and its note of the features it turns on for its instrumentation
     filtered (`WARNING-Setting-Limit-Adjusted`). Its safe mode is on, which skips an access
     its instrumentation finds invalid and still reports it: without it the access is made,
     and under lavapipe, a CPU driver, the heap canary's slot past the heap is read as a host
     pointer and the process dies before the layer reads its report back (the nightly of 23
     November 2026). The sanitizer, which safe mode would also turn on, is off, so the run
     checks what it checked before;
   - MoltenVK writes its warnings and errors (`MVK_CONFIG_LOG_LEVEL=2`), and Mesa's banner for
     a driver that is not conformant is off (`MESA_VK_IGNORE_CONFORMANCE_WARNING=1`): it is
     no message about a call;
   - under MoltenVK, Metal API and shader validation, set as the Metal step sets them
     (`METAL_VALIDATION` in `src/vulkan.rs`); under any other driver, none.
5. Each binary's canaries go first. The canaries (`Run`'s `canaries` in `src/vulkan.rs`) are
   thread-safety validation's (the graphics queue waited idle through `ash` with no lock under
   a lagging dispatch while another thread submits to it,
   `UNASSIGNED-Threading-MultipleThreads-Write`; a unit test in `src/canary.rs`, since no
   safe call can race a queue), core validation's (an image declared in a state it is not in,
   `VUID-vkCmdDraw-None-09600`), synchronisation validation's (two copies into one range,
   `SYNC-HAZARD-WRITE-AFTER-WRITE`), its check of a load after a store (a target stored by
   one render pass and kept by the next with no transition, `SYNC-HAZARD-READ-AFTER-WRITE`,
   which the layer sends only with `syncval_load_op_after_store_op_validation` on), its
   hazard across queues (a target a lagging graphics submit stores and a compute submit takes
   from `Undefined` with no semaphore, `SYNC-HAZARD-WRITE-RACING-WRITE`, which says `skipped,
   compute shares the graphics queue` on a device with one queue), its hazard across two
   submits of one queue, which every device runs (a buffer a lagging submit copies into and
   a second submit takes from a state it is not in and copies into again,
   `SYNC-HAZARD-WRITE-AFTER-WRITE`), the suite's `each_validator_reports_its_canary`, which
   provokes core's and synchronisation's, and under Metal's validation Metal's shader
   validation's (`METAL_CANARY`: a dispatch's read past a buffer, which must write `Invalid
   load in 'device' or 'constant' address space` from the test's own process, and which the
   layer hears nothing of); with
   `--gpu-assisted`, GPU-assisted validation's alone (a shader's read past a buffer through
   its address, `VUID-RuntimeSpirv-PhysicalStorageBuffer64-11819`, and a draw that samples
   slot `heap_slots`, one past the heap, by an index in push bytes,
   `VUID-vkCmdDraw-None-10068`). A canary passes when its test passes and every message it
   sent is one of its identifiers, each heard, or when it says why it is skipped. After every
   test, a listed canary that no binary holds fails the run. Every other test passes when it
   passes, its sinks printed no message and the driver wrote no line of its errors or
   warnings: MoltenVK's `[mvk-error]` and `[mvk-warn]`, Mesa's `MESA: error`, `MESA:
   warning` and any `WARNING: ` line. Under Metal's validation Metal must also write no
   report of shader validation's (`Invalid Metal usage`) but the Metal canary's, no
   assertion of API validation's (`failed assertion`) and no other line of the system log,
   which is where API validation's warnings go; and every test, canaries included, must
   print both validators' banners to the system log (`Metal API Validation Enabled`, `Metal GPU Validation Enabled`), which
   shows them loaded in its process, unless it says it skipped: a case of the null backend's
   alone skips before it makes a device, so Metal never loads, and a case may skip after
   making one (14 of MoltenVK's 44 skips do). The Metal step's canaries show what the two
   report, and the Vulkan step's Metal canary that a report reaches the judge from a test's
   process. Each test's process has a deadline (`DEADLINE`, 120 seconds): one still running
   then is killed and recorded failed, `past its deadline of 120 s`, so that a hung driver
   ends the test and not the run. The thread-safety canary's race may hang a driver: its
   watchdog aborts its process after 30 seconds, inside the deadline, which the file
   records as the canary failed (`the test failed`); an abort, since an exit runs the
   layer's teardown of the live device, which hangs where a stuck thread holds the layer's
   lock.
6. It writes `target/gpu/<driver>-vulkan.txt`, or `<driver>-vulkan-gpu-assisted.txt`: the
   driver, the shaders' Slang, the layer, the inherited variables it cleared (`inherited and
   cleared:`, or `none`), every variable it set, the canaries heard or skipped, the
   device's `DeviceInfo` and `Capabilities`, and each test with `passed`, the suite's
   `skipped, <why>` or `FAILED, <why>` and the messages and driver lines it printed. Under a
   failed test it writes how its process ended, its exit status or on unix the signal that
   killed it, and the last lines of its standard output and of its standard error, 40 at
   most between them (`TAIL_LINES` in `src/vulkan.rs`), so a process that died silently still
   says how. It prints the same to standard error, the job's log, followed by the test's whole
   standard output and standard error, each under a heading of its own: the log keeps all of
   a failed test's output, the file its tail. A failed test fails the run, under every driver.

The `shaders` CI job runs `gpu --driver system --only metal` on the macOS runner that cooks,
and the `shaders-oldest` job runs `gpu --driver system --only metal --cooked` on the oldest
macOS image, on the code the first cooked: the same canaries and count on both. The `gpu
(lavapipe)` job runs `gpu --driver lavapipe --only vulkan` on every pull request, and the
nightly runs `gpu --driver lavapipe --only vulkan --gpu-assisted`. On the M1, `gpu` and
`gpu --driver kosmickrisp --gpu-assisted` are run by hand: no runner has the M1's drivers.

Before either, `bash tools/xtask/runner-facts.sh [--metal-optional] <file>` records the
runner's facts in `<file>` and in the job's summary: its system, and on macOS its Xcode, its
SDK, its `metal`, its `swift` and its Metal device's facts, as
`swift tools/xtask/metal/pipelines.swift --device` prints them. Each is recorded before any is
judged, and a missing one fails the step, naming it. `--metal-optional`, for a runner that
compiles no metallib, records a missing `metal` and does not fail on it.
