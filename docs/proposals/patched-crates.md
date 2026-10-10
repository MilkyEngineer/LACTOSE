# Decision: a third-party crate built from a patched copy

Status: decision proposed, not applied; numbered D123 provisionally, the coordinator renumbers
at merge. It lands with the `lactose-physics-rapier` merge, whose branch holds the first
patched crate, and binds every crate from then on. Until it is applied no document says
whether a dependency may be patched; the lane brief's "prefer the patch if it is under fifty
lines, else refuse" is the only rule in use.

## Evidence

- The lane (`worktree-agent-ab8dd6e2785fde5c4` at `ca34412`) builds `rapier3d-f64` 0.36.0
  from `vendor/rapier3d-f64-0.36.0` through `[patch.crates-io]` in the root `Cargo.toml`,
  with `exclude = ["vendor"]`. The patch moves the angular joint limit's `sin_cos`
  (`AngularLimitParams::new`, `src/dynamics/solver/joint_constraint/joint_constraint_helper.rs`)
  from the inherent `f64::sin_cos` to `simba::scalar::ComplexField::sin_cos`, which
  `libm_force` routes to `libm`: two lines removed, four added, one file. It closes the first
  of the two Rapier gaps D119 names. The second, the sign of zero in SIMD `min` and `max`, is
  about 26 sites over rapier, parry, simba and wide and is not patched.
- The vendored tree is the archive plus that diff and nothing else. `diff -r` against cargo's
  own unpack of the archive (`~/.cargo/registry/src/…/rapier3d-f64-0.36.0`) shows the one
  file. The archive's sha256 is the `c28c8d3f…` the lane's README states, in the registry
  cache and freshly downloaded from `static.crates.io`. The README's diff block
  reverse-applies with `git apply --reverse` in a copy under `target/` and leaves the
  upstream tree.
- The lane's `Cargo.lock` entry for `rapier3d-f64` has no `source` and no `checksum` line.
  Section 9.1's build ID, "the `Cargo.lock` entries", therefore no longer pins the upstream
  archive of a patched crate.
- `cargo-deny` 0.20.2 does not check advisories for a patched crate. A scratch workspace
  depending on `smallvec = "=1.6.0"` fails `cargo deny check advisories` with
  RUSTSEC-2021-0003; the same version vendored with `cargo vendor`, patched by one line and
  reached through `[patch.crates-io]` passes with `advisories ok`. `bans`
  (`deny-multiple-versions` on the name) and `sources` pass and still match the crate by name
  and version; `licenses` reads the vendored manifest's `license` field.
- A `[patch.crates-io]` path inside the workspace root does not become a workspace member
  and cargo prints no warning (probed with `libm` 0.2.16 and no `exclude`): `cargo metadata`
  lists the member only, the copy is a package with `source: null`,
  `[profile.dev.package."*"]` applies to it (`opt-level=3` beside the member at 1) and
  `cargo fmt --all` ignores it. Cargo caps no lints on a path source, so `cargo clippy
  --workspace` prints the copy's own warnings (32 for `libm`); `-- -D warnings` reaches the
  selected packages only, so the run exits 0.
- `cargo vendor --versioned-dirs` writes `<name>-<version>/.cargo-checksum.json`: a `files`
  map of every file's sha256 (155 for `libm`; the manifest, `Cargo.toml.orig` and
  `.cargo_vcs_info.json` among them, never itself) and `package`, the archive's sha256.
  Cargo's own unpack writes only `.cargo-ok`.
- `cargo xtask tiers` and `boundary` already treat the copy as third-party:
  `tools/xtask/src/workspace.rs` classifies a declared dependency by its manifest's `path`
  key and a resolved package by membership, and the copy is neither; `[boundary_third_party]`
  names it as `rapier3d-f64`. `cargo xtask boundary` reads members' sources only, so the
  copy's code is outside every lint check, as the registry build's is (D31: clippy never
  lints dependencies).
- Rapier's master (`dimforge/rapier`, `joint_constraint_helper.rs`, read 2026-10-11) still
  calls the inherent `sin_cos`. The patch is not upstream.

## The decision

Patching is allowed, for every crate, by one mechanism, under one check. What a patch may do
is narrower inside the boundary.

**Allowed, with a cap.** A patch fixes a defect of the pinned version: inside the boundary, a
route that breaks a rule of section 8.2 (a transcendental from the platform library, a
numerics path selected at run time, an order dependence); anywhere, a defect that blocks a
milestone criterion; never a feature, an optimisation or an API change. It changes at most
fifty lines, counting every line it adds or removes, in files under `src/` only, adds no
`unsafe` and no dependency, and leaves the manifest, so the crate's features, dependencies
and the closure the allowlist review saw stay upstream's. Over fifty lines is not a patch:
the feature the route serves is refused (`Joint::limited` returning `Unsupported` was the
alternative here), or the dependency is changed, by a decision of the owner. The cap is the
lane brief's; it separates a fix from a fork, and the sign-of-zero sites are what it
excludes. Refusing `Joint::limited` for six lines was rejected because `anim`'s ragdoll uses
limits. A git fork was rejected because the source would leave the repository and
`deny.toml`'s `unknown-git = "deny"`. What would change the cap: SOUR's architecture mode
failing on the sign-of-zero sites with no fix but patches across four crates; that is a new
decision between a fork and another backend route, not a stretch of this one.

**Where it lives and how it is recorded.** `vendor/<name>-<version>/` holds the published
archive as `cargo vendor --versioned-dirs` unpacks it, `.cargo-checksum.json` included, with
the patch applied. `vendor/<name>-<version>.patch` holds the patch as a unified diff that
`git apply` consumes. `vendor/README.md` holds the prose: the upstream, the reason, the size,
the upstream report and what drops it; the archive checksum lives in the JSON only, so there
is one copy. The root `Cargo.toml` names the directory under `[patch.crates-io]` and keeps
`exclude = ["vendor"]`, which states what cargo does anyway. The crate keeps its crates.io
name in `crate-tiers.toml` and `deny.toml`. Rules out: a diff quoted in prose as the record
(the lane's README, which nothing could check); a tool that applies patches at build time
(cargo has no hook, and the tree must exist before `cargo metadata`); a directory source
replacement (the whole registry, and cargo refuses a modified file); a git fork.

**Proof.** `cargo xtask vendor`, below, proves offline that each copy is its archive plus its
patch, holds the patch to the rule, and runs `cargo deny check advisories` over the upstream
versions, since `cargo-deny` skips a path source. The review of a vendoring or a bump
verifies `package` in `.cargo-checksum.json` against the crates.io archive once
(`shasum -a 256` over the registry cache's `.crate`, or a download from `static.crates.io`);
the check holds the tree to that record from then on. A bump replaces the directory with the
new version's `cargo vendor` output, re-applies the patch or drops it when upstream carries
the change, moves the `[patch.crates-io]` path, regenerates `Cargo.lock` and updates the
README section; a patch that no longer applies is a new patch and a new review. A Rapier
bump is a protocol and replay break already (section 16, D58).

**Upstream.** Every patch is reported upstream, as an issue or a pull request, and the link
is in `vendor/README.md` before the merge that brings the patch in. A patch nobody reports is
a fork by instalments. It is dropped at the first bump to a version that carries the change.
A project that declines the report keeps the patch with the refusal recorded, and the owner
re-decides the feature at the next bump.

**Scope.** The mechanism and the check are workspace-wide, because `[patch.crates-io]`
applies to every user of the crate, inside the boundary or not, and `cargo xtask boundary`
can prove the boundary's closure only if every patch is visible. Admission is what differs:
inside the boundary a patch fixes a section 8.2 route and is reviewed as a determinism review
like a new allowlist name (`crate-tiers.toml` header); outside, a defect that blocks a
milestone criterion.

## `cargo xtask vendor` (exact behaviour)

A new subcommand beside `tiers`, `boundary` and `features`, run by the merge and by CI.
Findings are `path: message` lines as for every check; exit 0 clean, 1 findings, 2 when it
could not run. With no `vendor/` directory it prints `vendor: nothing is vendored` to
standard error and still checks that the root manifest holds no `[patch]` table.

It reads the root `Cargo.toml`, the `vendor/` listing, each copy's `Cargo.toml`,
`.cargo-checksum.json`, the `.patch` beside it, `vendor/README.md`, and the resolved graph
(`cargo metadata --locked`, the non-member packages whose manifest lies under `vendor/`).
The findings, in order:

1. `Cargo.toml: [patch.crates-io] "<name>" is not a path under vendor/` (a `git` patch or a
   path elsewhere); `Cargo.toml: [patch."<source>"] is not allowed; only crates-io is
   patched`; `<member>/Cargo.toml: holds a [patch] table; only the root may` (cargo ignores
   it with a warning; here it is a finding).
2. `vendor/<dir>: no [patch.crates-io] entry names it`; `Cargo.toml: [patch.crates-io]
   "<name>" names vendor/<dir>, which does not exist`.
3. `vendor/<dir>: holds <name> <version>, so it must be named vendor/<name>-<version>`;
   `Cargo.toml: [patch.crates-io] "<name>" names vendor/<dir>, which holds <other>`.
4. `vendor/<dir>: no package in the resolved graph is built from it (the patch is unused)`.
5. `vendor/<dir>/.cargo-checksum.json: missing, or without a "files" map and a "package"
   checksum; write it with cargo vendor --versioned-dirs`.
6. `vendor/<dir>.patch: missing; a copy without a patch has no reason to exist, use the
   registry`; `vendor/<dir>.patch: not a unified diff` (no `---`, `+++` and `@@` lines).
7. `vendor/<dir>.patch: <N> changed lines, over the fifty of architecture section 8.2`
   (every hunk line beginning `+` or `-`; the `---` and `+++` headers are not counted);
   `vendor/<dir>.patch: names <path>, outside src/`; `vendor/<dir>.patch: adds unsafe at
   line <n>` (a `+` line holding the token `unsafe`).
8. The copy is copied afresh to `target/vendor/<dir>` and `git apply --reverse
   <patch>` runs there (git is present wherever the repository is; `patch` is not promised on
   a Windows runner). On failure: `vendor/<dir>.patch: does not reverse-apply to vendor/<dir>
   (<git's first error line>)`. Then every file under the copy except `.cargo-checksum.json`
   is hashed with sha256 and compared with `files`: `vendor/<dir>/<file>: differs from the
   published archive`; `vendor/<dir>/<file>: is not in the published archive`;
   `vendor/<dir>: the archive's <file> is missing`.
9. `vendor/README.md: has no section "## <name> <version>"`; `vendor/README.md: section
   "## <name> <version>" names no directory under vendor/`.
10. The advisory shadow. It writes `target/vendor/upstream/Cargo.toml` (package
    `vendor-upstream`, `publish = false`, one `[dependencies]` line `<name> = "=<version>"`
    per copy) and an empty `src/lib.rs`, copies the workspace `Cargo.lock` beside them so the
    shadow resolves to the workspace's versions wherever it can, and runs
    `cargo deny --manifest-path target/vendor/upstream/Cargo.toml check --config deny.toml
    advisories`. Each `error[` line of its output is a finding against `vendor/<dir>`; the
    whole output goes to standard error. This step resolves the shadow's lock without
    `--locked`, the one exception to the tool's rule, and needs what `cargo deny check`
    needs anyway: cargo-deny installed, the index entry of the upstream version and the
    advisory database. An advisory for another package in the shadow is reported as well: it
    comes from the resolution the workspace would make without the patch.

Unit tests cover the patch reading (the count, the paths, `unsafe`), the hash comparison
against a JSON on a temporary directory, and every finding's text; the `git apply` and
`cargo deny` steps run only over the real `vendor/`. The tool gains the `sha2` crate; `toml`
and `serde_json` it has. One run costs a copy of each vendored tree (3.2 MB for Rapier), one
`git apply`, the hashing, one lock resolution and one `cargo deny` run: seconds.

## What the `physics-rapier` branch changes to fit

For the coordinator's fix brief, beside the review's findings:

1. `vendor/rapier3d-f64-0.36.0/.cargo-checksum.json`, produced by `cargo vendor
   --versioned-dirs` from a scratch manifest that depends on `rapier3d-f64 = "=0.36.0"` with
   no `[patch]`; the tree is otherwise unchanged (it already equals the archive plus the
   patch).
2. The diff moves from `vendor/README.md` into `vendor/rapier3d-f64-0.36.0.patch`, as it is
   (it applies).
3. `vendor/README.md` in the shape below, with the upstream report's link; the root
   `Cargo.toml` comment below.
4. `cargo xtask vendor` in `tools/xtask` (`src/vendor.rs`, the `USAGE` line, the README
   row and section below, `sha2` in the manifest and `Cargo.lock`), the `ci.yml` step below.
   The subcommand lands with this merge so the rule is enforced from the first vendored
   crate.
5. The D119 addendum the lane proposed stands; the sentence "vendored under vendor/ with one
   patch" is now the case of D123 rather than its own rule.

## Architecture amendments (exact text)

### §8.2 Rules inside the boundary, insert a bullet after the one ending "because they compile for the host without the pinned target features."

- A third-party crate may be built from a patched copy: `[patch.crates-io]` in the root
  `Cargo.toml` names `vendor/<name>-<version>`, the published archive unpacked with the
  `.cargo-checksum.json` that `cargo vendor` writes, and the patch, a unified diff, sits
  beside it as `vendor/<name>-<version>.patch`. A patch fixes a defect of the pinned version:
  inside the boundary, a route that breaks a rule of this section, such as a transcendental
  from the platform library; anywhere, a defect that blocks a milestone criterion; never a
  feature, an optimisation or an API change. It changes at most fifty lines, counting every
  line it adds or removes, in files under `src/` only, so the manifest, the build script and
  the crate's features, dependencies and closure stay upstream's; it adds no `unsafe`; it is
  reported upstream, and `vendor/README.md` records the link, the reason and what drops it.
  Over fifty lines is not a patch: the feature the route serves is refused, or the dependency
  is changed, by a decision. A bump re-applies the patch or drops it once upstream carries
  the change; a patch that no longer applies is a new patch and a new review. The copy is a
  dependency, not a member (`exclude = ["vendor"]`): no workspace lint, format or test runs
  over it, cargo caps none of its own warnings, and the crate keeps its crates.io name on the
  allowlist and in `deny.toml`. The same mechanism and check serve a crate outside the
  boundary, since a patch applies to every user of the crate in the workspace.

### §8.3 Enforcement, in the paragraph beginning "Static enforcement uses stable tooling", replace "`cargo-deny` with `multiple-versions = "deny"` for shared crates."

`cargo-deny` with `multiple-versions = "deny"` for shared crates; `cargo xtask vendor` for
each crate built from `vendor/` (section 8.2), which proves the copy is its upstream archive
plus its patch, holds the patch to the rule, and runs the advisory check over the upstream
versions, since `cargo-deny` skips a path source.

### §9.1 Topology and scale, replace "(the `Cargo.lock` entries, resolved feature set and source hash of boundary crates)"

(the `Cargo.lock` entries, the resolved feature set, the source hash of boundary crates, and
for each crate built from `vendor/` its upstream archive checksum and its patch, since the
lock entry of a patched crate names neither)

### §16 Physics, append after "any bump is a protocol and replay break."

The pinned `rapier3d-f64` is built from a patched copy under `vendor/` by the rule of section
8.2: the angular joint limit's `sin_cos` goes through `simba`'s `ComplexField`, which
`libm_force` routes to `libm`, in place of the platform library's; the patch travels with
every bump until upstream carries it.

### §20 Agent workflow, replace the last bullet's end ", `cargo-deny`, the tier check."

, `cargo-deny`, the tier check, the vendor check.

## Process and file amendments (exact text)

### `docs/agents.md` section 4, replace the gate bullet

- `cargo xtask tiers`, `cargo xtask boundary`, `cargo xtask features`, `cargo xtask vendor`;

### `.github/workflows/ci.yml`, insert after the `cargo-deny` step

```yaml
      - name: Vendored crates
        if: success() || failure()
        run: cargo xtask vendor
```

### `crate-tiers.toml` header, append after "is held to these rules wherever it comes from."

```
# A crate built from a patched copy under vendor/ (architecture section 8.2, D123) is not a
# path dependency: it is named by its crates.io name, replaced through [patch.crates-io] in
# the root Cargo.toml, and held to the lists below under that name; `cargo xtask vendor`
# proves the copy is the published archive plus its patch.
```

### `deny.toml` header, append a paragraph after the one ending "other than the x86-64-v3 pin."

```
#
# A crate built from a patched copy under vendor/ (architecture section 8.2, D123) keeps its
# name and version, so [bans] and [licenses] match it as they match the registry build and
# [sources] sees no path source. The advisories check skips a path source, so `cargo xtask
# vendor` runs it over the upstream versions in a manifest it writes under target/.
```

### Root `Cargo.toml`, the comment above `[patch.crates-io]`

```
# Architecture section 8.2 (D123): each entry names a directory under vendor/ that
# `cargo xtask vendor` proves is the published archive plus the patch beside it.
```

### `tools/xtask/src/main.rs`, `USAGE`, after the `features` entry

```
  vendor                     prove each crate built from vendor/ is its published archive
                             plus its patch, within the rule of section 8.2, and run the
                             advisory check over the upstream versions
```

### `tools/xtask/README.md`, a row after `features` and a section after `## features`

```
| `vendor` | each crate built from `vendor/` is its published archive plus its patch, the patch is within the rule, and the advisory check covers the upstream versions (sections 8.2, 8.3) |
```

```
## `vendor`

- Every `[patch.crates-io]` entry of the root manifest names `vendor/<name>-<version>`, no
  other `[patch]` table exists, every directory under `vendor/` is named by such an entry,
  holds that name and version, and is built by some package of the resolved graph.
- Each directory holds the `.cargo-checksum.json` that `cargo vendor --versioned-dirs`
  writes, and `vendor/<name>-<version>.patch` beside it is a unified diff of at most fifty
  changed lines, under `src/` only, adding no `unsafe`.
- Reversing the patch on a copy under `target/vendor/` gives exactly the files and sha256
  hashes the JSON records: nothing changed, added or missing.
- `vendor/README.md` has a section per directory and none for a directory that is gone.
- `cargo deny check advisories` over a manifest under `target/vendor/upstream/` that depends
  on each upstream version reports nothing; `cargo-deny` skips a path source, so the
  workspace run cannot see these. This step resolves the shadow's lock without `--locked`.
```

### `vendor/README.md` (the whole file)

```
# Vendored crates

Third-party crates the workspace builds from a patched copy, under the rule of architecture
section 8.2 (D123) and checked by `cargo xtask vendor`. Each `<name>-<version>/` is the
published archive as `cargo vendor --versioned-dirs` unpacks it, `.cargo-checksum.json`
included, with `<name>-<version>.patch` applied; nothing else. The crate keeps its crates.io
name in `crate-tiers.toml` and `deny.toml`, and the root `Cargo.toml` names the directory
under `[patch.crates-io]`. The directory is excluded from the workspace: no workspace lint,
format or test runs over it.

To add or bump a crate: vendor the version from a scratch manifest that depends on it
(`cargo vendor --versioned-dirs` into a temporary directory, with no `[patch]` entry), move
`<name>-<version>/` here, apply the patch with `git apply`, point the `[patch.crates-io]`
entry at it, regenerate `Cargo.lock`, write the section below and run `cargo xtask vendor`.
A patch that no longer applies is rewritten and reviewed as new; a version that carries the
change upstream drops the directory, the patch and the entry.

## rapier3d-f64 0.36.0

- Upstream: crates.io `rapier3d-f64` 0.36.0 (git `b716d375efc0201003f0cd9ef7168eee0b62c177`,
  path `crates/rapier3d-f64`); the archive's sha256 is `package` in `.cargo-checksum.json`.
- Used by: `lactose-physics-rapier`, inside the determinism boundary.
- Why: the angular joint limit (`AngularLimitParams::new`,
  `src/dynamics/solver/joint_constraint/joint_constraint_helper.rs`) took the sine and cosine
  of the limit centre with the inherent `f64::sin_cos`, the platform maths library's, on the
  solver path of every limited joint. `enhanced-determinism` turns on `simba/libm_force`,
  which routes `ComplexField::sin_cos` to `libm`; the inherent method ignores it. The patch
  calls the trait method, so the value is `libm`'s on every target (architecture section 8.2).
- Patch: `rapier3d-f64-0.36.0.patch`, one file, six changed lines (two removed, four added).
- Upstream report: <the issue or pull request on dimforge/rapier; filed before the merge>.
- Dropped when: a bump reaches a version whose `AngularLimitParams::new` takes the trait
  route.

Not patched: the sign of a zero returned by `max` and `min` (`f64::max` and the `wide` SIMD
lanes differ between x86-64 and AArch64 when both operands are zeros of opposite sign). The
call sites are spread over `rapier3d-f64`, `parry3d-f64`, `simba`, `wide` and `glam`; no
patch under fifty lines closes them, so the item stays open in `docs/verification.md`.
```

## Decision entry (exact text; number at merge)

**D123. A third-party crate is built from a patched copy only under `vendor/`, by one rule
for every crate, checked by `cargo xtask vendor`** (§8.2, §8.3, §9.1, §16, §20,
`crate-tiers.toml`, `deny.toml`, `docs/agents.md`, `tools/xtask`)
- Why: the Rapier backend needed one route moved from the platform library to `libm` (the
  angular joint limit's `sin_cos` in `joint_constraint_helper.rs`, six changed lines) and no
  document said whether a boundary crate may be patched, so the lane vendored the crate and
  proposed a rule. Patching is admitted because the alternative, refusing `Joint::limited`,
  drops a feature `anim`'s ragdoll needs for a six-line fix, and a git fork would move source
  out of the repository and past `deny.toml`'s `unknown-git = "deny"`.
  - *One mechanism, every crate.* `[patch.crates-io]` applies to every user of the crate in
    the workspace, inside the boundary or not, so the mechanism and its check are
    workspace-wide; admission differs: inside the boundary a patch fixes a section 8.2 route,
    elsewhere a defect that blocks a milestone criterion, never a feature.
  - *Fifty changed lines, under `src/` only, no `unsafe`, no dependency, no manifest.* The cap
    the lane brief used; it separates a fix from a fork. The sign of zero in SIMD `min` and
    `max` (about 26 sites over rapier, parry, simba and wide) is what it excludes, and that
    stays a verification item for SOUR's architecture mode, not a patch. The manifest stays
    upstream's so the features, dependencies and closure the allowlist review saw do not move.
  - *The record is what `cargo vendor` writes plus a patch file.* `.cargo-checksum.json`
    carries every upstream file's sha256 and the archive's, in the format cargo itself reads;
    the patch is a unified diff `git apply` consumes. So the check is offline: reverse the
    patch on a copy, hash, compare. Rules out a diff quoted in prose (the lane's README), a
    checksum in prose, and a tool that applies patches at build time (cargo has no hook; the
    tree must exist before `cargo metadata`).
  - *`cargo-deny` sees the name but not the advisories.* Probed on cargo-deny 0.20.2: `bans`,
    `licenses` and `sources` match a patched crate by name and version; `advisories` skips it
    as a path source (`smallvec` 1.6.0, RUSTSEC-2021-0003: reported from the registry, silent
    when vendored and patched). `cargo xtask vendor` therefore writes a manifest under
    `target/` that depends on each upstream version and runs `cargo deny check advisories`
    over it.
  - *The build ID names the vendored record.* A patched crate's `Cargo.lock` entry has
    neither `source` nor `checksum`, so section 9.1's build ID takes the archive checksum and
    the patch beside the lock entries.
  - *A dependency, not a member.* Cargo does not make a patch path a member, but
    `exclude = ["vendor"]` states it, so no workspace lint, format or test runs over the copy
    and `[profile.dev.package."*"]` applies as to the registry build. Cargo caps no lints on a
    path source, so the copy's own warnings show in a clippy run; `-D warnings` reaches the
    selected packages only, so they fail nothing.
  - *Reported upstream, dropped at the first bump that carries it.* A patch nobody reports is
    a fork by instalments. Rapier's master still called the inherent `sin_cos` on 2026-10-11.
  - Rules out: a git fork; a patch over the cap; a patch to a manifest or build script; a
    vendored copy with no patch; prose as the record; a per-boundary mechanism.
- Driven by: the `physics-rapier` lane (2026-10-10), its report's proposed rule and the review
  brief's vendored-patch item; the planner's probes (2026-10-11).
- Replaced: nothing stated; the lane brief's "prefer the patch if it is under fifty lines,
  else refuse".

## Verification items (exact text)

| Item | Gates milestone |
|---|---|
| `cargo-deny` 0.20.2 skips a path source in `check advisories`: `smallvec` 1.6.0 fails from the registry (RUSTSEC-2021-0003) and passes vendored and patched, while `bans` and `sources` match by name and version. `cargo xtask vendor` carries the advisory check over the upstream versions; at each `cargo-deny` upgrade re-check whether an option covers path sources, and drop the shadow manifest when one does | 1 |
| The `sin_cos` patch is not upstream (`dimforge/rapier` master, `joint_constraint_helper.rs`, 2026-10-11): report it and record the link in `vendor/README.md` before the `physics-rapier` merge; the patch is dropped at the first bump that carries it | 1 |
| The boundary build ID takes each vendored crate's archive checksum and patch beside the `Cargo.lock` entries (section 9.1) when `app` computes it; a patched crate's lock entry has no `source` or `checksum` | 4 |

## Cost

One `lane` run on the `physics-rapier` branch, after the review's fix brief or folded into
it: `cargo xtask vendor` with its tests is about the size of `features.rs`, and the four
branch changes above are minutes. One `reviewer-diff` round covers both with the review's
other findings. The owner or coordinator files the upstream report; that is not an agent run.
The merge and every CI run gain one gate of a few seconds plus one `cargo deny` run.
