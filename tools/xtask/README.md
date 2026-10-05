# lactose-xtask

The CI driver. Every rule that CI enforces and that is not a compiler, clippy or cargo-deny
diagnostic is a subcommand here, so it runs the same on a laptop as on a runner.

| | |
|---|---|
| Tier | leaf |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

Run it as `cargo xtask <command>` from anywhere inside the repository.

| Command | Enforces |
|---|---|
| `tiers` | `crate-tiers.toml` against the workspace (architecture section 3) |
| `boundary` | the file facts of the determinism boundary and the unsafe policy (sections 8.2, 8.3, 20, 21) |
| `features` | identical boundary feature trees across shipped binaries and the SOUR test crate (section 8.3) |
| `sour run`, `sour compare`, `sour self-test`, `sour matrix` | SOUR, the determinism harness (section 8.3) |

Findings are printed to standard output as `path: message`, one per line, and nothing else
is printed there. The exit status is 0 for none, 1 for findings and 2 when a check could not
run.

## Where the rules live

| File | Holds |
|---|---|
| `crate-tiers.toml` | Tiers, the boundary list, the server denylist, exceptions. The only dependency rule |
| `tools/xtask/ci-policy.toml` | The unsafe list of section 21, the boundary dependency allowlist of section 8.2, the feature-tree roots and the SOUR packages |
| `deny.toml` | Licences, sources, advisories and duplicate versions, enforced by `cargo deny check` |
| `crates/core/clippy.toml` | The boundary lints; every boundary crate carries an identical copy |

A new shipped binary goes into `[feature_roots]`, and a package with simulation tests into
`[sour]`, both in `ci-policy.toml`. Nothing in the code names a crate of the workspace.

## `tiers`

The header comments of `crate-tiers.toml` are the rules. Where they are silent, the module
documentation of `src/tiers.rs` lists the reading taken. In short:

- every workspace crate is in exactly one tier section, and a crate the file lists but that
  does not exist yet is fine;
- foundation and game crates depend on foundation crates only; a backend crate is named only
  by leaf crates and through `[exceptions]`; only leaf crates depend on leaf crates;
- dev-dependencies obey the same rules, except a crate's dev-dependency on itself;
- `lactose-app-server` reaches no crate of `[server_denylist]`;
- no proc-macro crate and no build script reaches a boundary crate.

## `boundary`

- Each boundary crate's `clippy.toml` is byte-identical to `crates/core/clippy.toml`.
- A crate on the unsafe list has a `SAFETY.md`; every other crate has
  `#![forbid(unsafe_code)]` in each library, proc-macro and binary root. Integration tests,
  benches, examples and build scripts are separate crate roots and are not checked.
- Every crate has a `README.md`.
- No boundary crate lifts `clippy::disallowed_methods` or `clippy::disallowed_types` with an
  inner attribute. `#[expect(..., reason = "...")]` on one item is the sanctioned form.
- A boundary crate's dependencies outside the boundary are on the allowlist in
  `ci-policy.toml`.
- The x86-64-v3 pin is present in `.cargo/config.toml`, and no cargo configuration file,
  workflow or script names `RUSTFLAGS` or sets another `target-cpu`. Whole-line `#` comments
  are skipped; a trailing comment is scanned.

## `features`

For each root package, `cargo tree` gives the resolved features of every package. The check
keeps the boundary crates and everything below them and requires the result to be the same
for every root. `--package <name>` (repeatable) compares the named packages instead of
`[feature_roots]`.

## SOUR

SOUR runs every simulation test once per run configuration and requires the world hashes to
agree. `cargo xtask sour matrix` prints the modes; `src/sour/mod.rs` holds them as data.

| Mode | Configurations compared against the baseline | Pull request |
|---|---|---|
| workers | 1 worker (the baseline has 4) | yes |
| repeat | the baseline again | nightly |
| layout | 1, 7 and 64 rows and 16 KiB, each with seeded permutation | 7 rows; the rest nightly |
| chaos | three seeds | nightly |
| architecture | the baseline of another machine | nightly |

- `sour run --pr | --nightly | --mode <name>…` launches `cargo nextest run` over the packages
  of `[sour]` once per configuration and reports, per test, the first tick whose hash
  differs. `--emit <file>` also writes the baseline hashes.
- `sour compare <reference> <file>` compares two emitted files. Architecture mode is each
  machine emitting and one job comparing.
- `sour self-test` proves the runner through nextest on the stand-in simulation in
  `src/sour/stand_in.rs`: a sound one must pass and an order-dependent one must be reported
  at the tick its flaw starts.

Runs write to `target/sour/<mode>/<variant>/`: `hashes.tsv`, and `logs/<test>.execlog` for
the execution log of a failed chaos test.

### What a simulation test does

`src/sour/protocol.rs` is the contract, and its module documentation is the schema: six
`LACTOSE_SOUR_*` environment variables, version 1, and a tab-separated hash record. A test
calls `Session::from_env()`, builds its world and executor from `Session::config()`, and
calls `Recorder::record(tick, hash)` at every sync point it hashes. Outside the runner the
session is the production configuration and records nothing, so the same test runs under a
plain `cargo nextest run`.

The protocol module depends only on `std` and on nothing else in this crate. It is written
to move, unchanged, into a foundation crate outside the boundary that the SOUR test crate
and the tests of `sim` and the game crates depend on; a leaf crate such as this one cannot
be depended on by any of them. Until that crate exists the module lives here.
