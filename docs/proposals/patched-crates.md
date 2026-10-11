# Decision: a third-party crate built from a patched copy

Status: approved by the owner on 2026-10-11 as revised; D123. Applied with the `cargo xtask
vendor` lane, merged first on main, except what belongs to the first patched crate: the
section 16 sentence, the root `Cargo.toml` comment, `vendor/README.md`, and the row 83 and
`sin_cos` verification rows land with the `lactose-physics-rapier` merge, whose branch holds
that crate. The rule binds that merge and every crate from then on. The owner files the
upstream report; the Rapier merge does not wait for its link (`vendor/README.md` says "to be
filed" until then).
Steps 1, 2, 6, 7 and 8 amended by the check's reviews; see the D123 addendum.

## Evidence

- The lane (`worktree-agent-ab8dd6e2785fde5c4` at `ca34412`) builds `rapier3d-f64` 0.36.0
  from `vendor/rapier3d-f64-0.36.0` through `[patch.crates-io]` in the root `Cargo.toml`,
  with `exclude = ["vendor"]`. The patch moves the angular joint limit's `sin_cos`
  (`AngularLimitParams::new`, `src/dynamics/solver/joint_constraint/joint_constraint_helper.rs`)
  from the inherent `f64::sin_cos` to `simba::scalar::ComplexField::sin_cos`, which
  `libm_force` routes to `libm`: two lines removed, four added (two of them a `LACTOSE patch:`
  comment), one file. It closes the first of the two Rapier gaps D119 names. The second, the
  sign of zero in SIMD `min` and `max`, is about 26 sites over rapier, parry, simba and wide
  and is not patched. The vendored manifest is `license = "Apache-2.0"`, whose section 4(b)
  asks a modified file to say it was changed.
- The vendored tree is the archive plus that diff and nothing else. `diff -r` against cargo's
  own unpack of the archive shows the one file. The archive's sha256 is the `c28c8d3f…` the
  lane's README states, in the registry cache and freshly downloaded from `static.crates.io`.
  The README's diff block reverse-applies with `git apply --reverse` in a copy under `target/`
  inside the repository and leaves the upstream tree; the review confirmed git prefixes the
  working directory, so the subdirectory case does not bite.
- `cargo vendor --versioned-dirs` writes `<name>-<version>/.cargo-checksum.json`: a `files`
  map of every file's sha256 (the manifest, `Cargo.toml.orig` and `.cargo_vcs_info.json`
  among them, never itself) and `package`, the archive's sha256. Its output for
  `rapier3d-f64` 0.36.0 equals the registry unpack byte for byte; it drops a crate's
  `.gitignore` and `.gitattributes`. Cargo's own unpack writes only `.cargo-ok`.
- The record alone proves nothing about crates.io. The review's emulation of the offline
  steps passes a tree whose `src/lib.rs` was edited and whose hash in `files` was rewritten
  to match, with `package` untouched; re-deriving the record with `cargo vendor` from a
  shadow manifest reports `src/lib.rs` in about two seconds from the cache.
- The lane's `Cargo.lock` entry for `rapier3d-f64` has no `source` and no `checksum` line.
  Section 9.1's build ID, "the `Cargo.lock` entries", therefore no longer pins the upstream
  archive of a patched crate.
- `cargo-deny` 0.20.2 does not check advisories for a patched crate. A scratch workspace
  depending on `smallvec = "=1.6.0"` fails `cargo deny check advisories` with
  RUSTSEC-2021-0003; the same version vendored with `cargo vendor`, patched by one line and
  reached through `[patch.crates-io]` passes with `advisories ok`. `bans`
  (`deny-multiple-versions` and `bans.features` on the name) and `sources` still match the
  crate by name and version; `licenses` reads the vendored manifest's `license` field.
  `--config` is a global option of `cargo deny`, not of `check`; a manifest under `target/`
  with no `[workspace]` table is refused by cargo as "believes it's in a workspace when it's
  not". With both right, a shadow of the workspace's `rapier3d-f64` line resolves with zero
  drift from the workspace lock (58 shared packages identical, one added) and prints
  `advisories ok`.
- Cargo honours `[patch.crates-io]` in a committed `.cargo/config.toml`, and a path package
  outside `vendor/` resolves with no warning; a crate at two versions with one of them
  patched resolves the other from the registry with no warning either. Patching both versions
  needs a renamed key with a `package` field. `deny-multiple-versions` covers the math crates
  only.
- A `[patch.crates-io]` path inside the workspace root does not become a workspace member
  and cargo prints no warning: `cargo metadata` lists the member only, the copy is a package
  with `source: null`, `[profile.dev.package."*"]` applies to it (`opt-level=3` beside the
  member at 1) and `cargo fmt --all` ignores it. Cargo caps no lints on a path source, so
  `cargo clippy --workspace` prints the copy's own warnings; `-- -D warnings` reaches the
  selected packages only, so the run exits 0.
- `cargo xtask tiers` and `boundary` already treat the copy as third-party:
  `tools/xtask/src/workspace.rs` classifies a declared dependency by its manifest's `path`
  key and a resolved package by membership, and the copy is neither; `[boundary_third_party]`
  names it as `rapier3d-f64`. `cargo xtask boundary` reads members' sources only, so the
  copy's code is outside every lint check, as the registry build's is (D31).
- `libm` 0.2.16, the boundary's transcendental source, selects its paths (`arch_enabled`,
  `optimizations_enabled`, `x86_no_sse`) in `configure.rs` and `build.rs` at the crate root,
  outside `src/`; `libm` cannot be refused.
- The repository has no `.gitattributes`. A Windows checkout converts line endings, which
  would change the hashes of vendored files.
- Rapier's master (`dimforge/rapier`, `joint_constraint_helper.rs`, read 2026-10-11) still
  calls the inherent `sin_cos`. The patch is not upstream.

## The decision

Patching is allowed, for every crate, by one mechanism, under one check. What a patch may do
is narrower inside the boundary.

**Allowed, with a cap.** A patch fixes a defect of the pinned version: inside the boundary, a
route that breaks a rule of section 8.2 (a transcendental from the platform library, a
numerics path selected at run time, an order dependence); anywhere, a defect that blocks a
milestone criterion; never a feature, an optimisation or an API change. It changes at most
fifty lines, counting every line it adds or removes, comments included, so the count is the
diff's and nothing else. It never touches `Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock` or
`.cargo_vcs_info.json`, so the crate's version, features, dependencies and the closure the
allowlist review saw stay upstream's. It may touch a build script: `libm` selects its paths
in `configure.rs`, and a defect there could otherwise be met only by changing the one crate
the boundary cannot change; such a patch alters what the crate compiles on every target and
is reviewed as that. It adds no `unsafe` and no dependency. Every change carries a
`LACTOSE patch:` comment, the notice Apache-2.0 asks of a modified file; the comment lines
count, and the Rapier patch is six lines with them. Inside the boundary a patch is reviewed
as a determinism review, like a new allowlist name. Over fifty lines is not a patch: the
feature the route serves is refused (`Joint::limited` returning `Unsupported` was the
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
`exclude = ["vendor"]`, which states what cargo does anyway; no other `[patch]`, `[source]`
or `paths` setting exists in a manifest or a cargo configuration file, and a crate is patched
at one version only. `.gitattributes` marks `vendor/**` `-text`, so every host hashes the
same bytes. The crate keeps its crates.io name in `crate-tiers.toml` and `deny.toml`. Rules
out: a diff quoted in prose as the record (the lane's README, which nothing could check); a
tool that applies patches at build time (cargo has no hook, and the tree must exist before
`cargo metadata`); a directory source replacement (the whole registry, and cargo refuses a
modified file); a git fork.

**Proof.** `cargo xtask vendor`, below, proves offline that each copy is its patch applied to
its record and that the patch is within the rule, then online that the record is the
crates.io archive's, by re-deriving it with `cargo vendor` from a shadow manifest, and runs
`cargo deny check advisories` over the same shadow, since `cargo-deny` skips a path source.
The review of a vendoring or a bump reads the patch and the reason; it has no checksum to
verify by hand. A bump replaces the directory with the new version's `cargo vendor` output,
re-applies the patch or drops it when upstream carries the change, moves the
`[patch.crates-io]` path, regenerates `Cargo.lock` and updates the README section; a patch
that no longer applies is a new patch and a new review. A Rapier bump is a protocol and
replay break already (section 16, D58).

**The build ID.** Section 9.1 hashes the vendored directory's files, as it hashes the
boundary crates' sources, rather than naming the archive checksum plus the patch. The file
hash names what the binary compiled whether or not the check ran; an archive checksum plus a
patch stays equal for a tree whose record was rewritten, the review's tamper case, until the
online step catches it, and it names nothing the file hash does not already cover. The
`.gitattributes` line keeps the bytes the same on every host, the same exposure the existing
source hash has.

**Upstream.** Every patch is reported upstream, as an issue or a pull request; the owner
files it, since it is outward-facing, and `vendor/README.md` records the link. A patch
nobody reports is a fork by instalments. It is dropped at the first bump to a version that
carries the change. A project that declines the report keeps the patch with the refusal
recorded, and the owner re-decides the feature at the next bump. Question for the owner:
whether the `physics-rapier` merge waits for the report's link, or `vendor/README.md` carries
"to be filed" until it is; the rule requires the link to be recorded, not when, and the
verification row carries the obligation either way.

**Scope.** The mechanism and the check are workspace-wide, because `[patch.crates-io]`
applies to every user of the crate, inside the boundary or not, and `cargo xtask boundary`
can prove the boundary's closure only if every patch is visible. Admission is what differs:
inside the boundary a patch fixes a section 8.2 route and is reviewed as a determinism
review; outside, a defect that blocks a milestone criterion.

## `cargo xtask vendor` (exact behaviour)

A new subcommand beside `tiers`, `boundary` and `features`, run by the merge and by CI.
Findings are `path: message` lines as for every check; exit 0 clean, 1 findings, 2 when it
could not run. With no `vendor/` directory it prints `vendor: nothing is vendored` to
standard error and still runs steps 1, 2 and 5's `.gitattributes` line.

It reads the root `Cargo.toml`, every cargo configuration file the repository holds
(`.cargo/config.toml` and `.cargo/config` at any depth, as `cargo xtask boundary` finds
them), `.gitattributes`, the `vendor/` listing, each copy's `Cargo.toml`,
`.cargo-checksum.json`, the `.patch` beside it, `vendor/README.md`, and the resolved graph
(`cargo metadata --locked --all-features`: every package with its `source`, version and
manifest path, with every feature on, since a version behind a feature is still compiled by
whoever enables it), and `git apply --numstat -z` of each patch. A
`[patch.crates-io]` entry's crate name is its `package` key when present, else the key. The
findings, in order:

1. Patch sources. `Cargo.toml: [patch.crates-io] "<key>" is not a path under vendor/` (a
   `git` patch or a path elsewhere); `Cargo.toml: [patch."<source>"] is not allowed; only
   crates-io is patched`; `<member>/Cargo.toml: holds a [patch] table; only the root may`
   (cargo ignores it with a warning; here it is a finding); `<config>: sets [patch],
   [source] or paths; a patch lives in the root Cargo.toml only`.
   Cargo must read nothing the check does not: `<config>: has an include key; cargo reads
   the included files, which no check does`; `<path>: is a symbolic link; cargo follows it,
   which no check does, so a cargo configuration file or directory is committed as itself`
   (a `.cargo` directory, `.cargo/config` or `.cargo/config.toml` anywhere; `cargo xtask
   boundary` refuses both through the same function); `<config>: alias <name> passes
   --config or -Z, through which a [patch], [source] or paths setting reaches cargo unseen`;
   in each workflow and script `cargo xtask boundary` scans, `<file>: line <n>: passes
   --config, through which a [patch], [source] or paths setting reaches cargo unseen`, and
   in those and each cargo configuration file, `<file>: line <n>: sets CARGO_HOME, whose
   cargo configuration no check reads`; and for a cargo configuration file or a path under
   a `.cargo` directory that git tracks (`git ls-files`) beneath a directory the file walk
   skips, `<path>: tracked under <dir>/, which no check reads; cargo reads a cargo
   configuration file it runs beneath`.
2. Correspondence. Every resolved package with no `source` that is not a workspace member
   has its manifest at `vendor/<name>-<version>/Cargo.toml` and is named by a root
   `[patch.crates-io]` entry, and every directory under `vendor/` is such a package:
   `<manifest>: non-member path package <name> <version> outside vendor/`; `vendor/<dir>: no
   [patch.crates-io] entry names it`; `Cargo.toml: [patch.crates-io] "<key>" names
   vendor/<dir>, which does not exist`; `vendor/<dir>: no package in the resolved graph is
   built from it (the patch is unused)`. Nothing else lives directly under `vendor/`:
   `vendor/<entry>: neither a copy, its patch nor README.md; nothing else lives under
   vendor/`; `vendor/<x>.patch: no copy vendor/<x> beside it`.
3. Names. `vendor/<dir>: holds <name> <version>, so it must be named
   vendor/<name>-<version>`; `Cargo.toml: [patch.crates-io] "<key>" names vendor/<dir>,
   which holds <other>`.
4. One version. `vendor/<dir>: <name> also resolves from crates.io at <v>; a patch covers one
   version` (the graph holds the name at another version from a registry source).
5. The record. `vendor/<dir>/.cargo-checksum.json: missing, or without a "files" map and a
   "package" checksum; write it with cargo vendor --versioned-dirs`; `.gitattributes: does
   not mark vendor/** -text`.
6. The patch file. `vendor/<dir>.patch: missing; a copy without a patch has no reason to
   exist, use the registry`; `vendor/<dir>.patch: not a unified diff` (no `---`, `+++` and
   `@@` lines, or a binary change); `vendor/<dir>.patch: not a patch git applies (<git's
   first error line>)`.
7. The rule. git is the authority on what a patch touches: the files and the line counts
   are `git apply --numstat -z`'s, and the check's own reading, which finds the tokens and
   the notice, must agree with it file by file: `vendor/<dir>.patch: git applies it to
   <path>, which the check does not read`; `vendor/<dir>.patch: the check reads a change to
   <path>, which git does not apply`; `vendor/<dir>.patch: git reads <path> as <a> added
   and <r> removed lines, the check as <b> and <c>`. Then `vendor/<dir>.patch: <N> changed
   lines, over the fifty of architecture section 8.2` (every line git counts as added or
   removed, comments included); `vendor/<dir>.patch: names <path>, which a patch never
   touches` (`Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock`, `.cargo_vcs_info.json`,
   `.cargo-checksum.json`, by git's paths and the check's); `vendor/<dir>.patch: touches
   <path>, which holds unsafe; a patch never touches a file that does` (the copy's file, or
   a line the patch removes, holds the word `unsafe`); `vendor/<dir>.patch: creates <path>,
   a build script; a patch may change one, never add one` (`build.rs` or what
   `package.build` names, absent from the record); `vendor/<dir>.patch: adds <token> at
   line <n>` (a `+` line holding `unsafe`, `no_mangle`, `export_name`, `link_section`,
   `link_name`, `naked`, `asm`, `global_asm`, `naked_asm`, `include`, `include_str` or
   `include_bytes` as a whole identifier, with or without `!`; or, reading each hunk's
   added lines joined, as written and with comments blanked, `a path attribute` for `#`,
   `[`, then `path =`, and `an out-of-line module` for `mod <name>;`);
   `vendor/<dir>.patch: line <n> adds <N> characters, over the 200 an added line may hold`;
   `vendor/<dir>.patch: <path> carries no "LACTOSE patch:" notice` (no `+` line of that
   file's hunks holds the text).
8. Reverse and hash. The copy is copied afresh to `target/vendor/<dir>` and `git apply
   --reverse <patch>` runs there (git is present wherever the repository is; `patch` is not
   promised on a Windows runner). On failure: `vendor/<dir>.patch: does not reverse-apply to
   vendor/<dir> (<git's first error line>)`; a hunk git places away from where the patch
   says, which repeated context allows: `vendor/<dir>.patch: reverse-applies to vendor/<dir>
   only away from where it says (<git's line>)`. Then every file under the copy except
   `.cargo-checksum.json` is hashed with sha256 and compared with `files`:
   `vendor/<dir>/<file>: differs from the record`; `vendor/<dir>/<file>: is not in the
   record`; `vendor/<dir>: the record's <file> is missing`. When they match, the patch is
   applied forward to that tree, again with no offset or fuzz, and the result must equal the
   copy byte for byte: `vendor/<dir>/<file>: is not the record with the patch applied`.
9. Prose. `vendor/README.md: has no section "## <name> <version>"`; `vendor/README.md:
   section "## <name> <version>" names no directory under vendor/`.
10. Online: the record and the advisories. It writes `target/vendor/upstream/Cargo.toml`: a
    package `vendor-upstream`, `publish = false`, an empty `[workspace]` table so cargo does
    not look for a parent, and for each copy the root manifest's `[workspace.dependencies]`
    entry for `<name>` with its version pinned exactly (`=<version>`, the same
    `default-features` and `features`); an empty `src/lib.rs`; and a copy of the workspace
    `Cargo.lock` beside them, so the shadow resolves to the workspace's versions. Then:
    - `cargo vendor --versioned-dirs --manifest-path target/vendor/upstream/Cargo.toml
      target/vendor/upstream-src`. Each committed `.cargo-checksum.json` must equal
      `target/vendor/upstream-src/<name>-<version>/.cargo-checksum.json`, `package` and
      `files` both: `vendor/<dir>/.cargo-checksum.json: does not match the crates.io archive
      (<file>)`.
    - `cargo deny --manifest-path target/vendor/upstream/Cargo.toml --config deny.toml check
      advisories`. cargo-deny's exit status decides: 0 is clean; non-zero with `error[` lines
      gives one finding per line against `vendor/<dir>`; non-zero with none is exit 2 with
      its output. An advisory for another package in the shadow is reported as well; the
      shadow is the resolution the workspace makes without the patch.
    Both resolve the shadow's lock without `--locked`, the one exception to the tool's rule,
    and need what `cargo deny check` needs anyway: cargo-deny on PATH, the index entry and
    the `.crate` of each upstream version, the advisory database. `cargo xtask vendor
    --offline` passes `--offline` to both, for the integration test and a machine with the
    cache.

Unit tests cover the patch reading (the count, the paths, `unsafe`, the notice), the hash
comparison against a record on a temporary directory, and every finding's text. An
integration test vendors a small cached crate into a temporary workspace nested under a
`[workspace]` root, patches one line and runs all ten steps `--offline`; it also covers the
tamper cases: a file edited with its hash rewritten in the record (step 10 catches it), a
path package outside `vendor/` and a `[patch]` in `.cargo/config.toml` (steps 1 and 2), and
a crate at two versions with one patched (step 4). The tool gains the `sha2` crate; `toml`
and `serde_json` it has. One run costs a copy of each vendored tree (3.2 MB for Rapier), one
`git apply`, the hashing, one lock resolution, one `cargo vendor` from the cache (about two
seconds) and one `cargo deny` run.

## Sequencing

`cargo xtask vendor` is its own `lane` on main, reviewed by `reviewer` first and then as
`docs/agents.md` section 2 requires for an implementation, and merges before Rapier; on main
it finds nothing vendored and nothing patched, and it brings `.gitattributes`. The
`physics-rapier` fix round carries data only: `vendor/rapier3d-f64-0.36.0/.cargo-checksum.json`
from `cargo vendor --versioned-dirs` of a scratch manifest with no `[patch]` (the tree is
otherwise unchanged; it already equals the archive plus the patch), the diff moved from
`vendor/README.md` into `vendor/rapier3d-f64-0.36.0.patch` as it is, `vendor/README.md` in
the shape below, and the root `Cargo.toml` comment below. The Rapier merge gate then runs the
check over the real tree. The D119 addendum the lane proposed stands; its "vendored under
vendor/ with one patch" is now the case of D123 rather than its own rule.

## Architecture amendments (exact text)

### §8.2 Rules inside the boundary, insert a bullet after the one ending "because they compile for the host without the pinned target features."

- A third-party crate may be built from a patched copy: `[patch.crates-io]` in the root
  `Cargo.toml` names `vendor/<name>-<version>`, the published archive as `cargo vendor`
  unpacks it with the `.cargo-checksum.json` it writes, and the patch, a unified diff, sits
  beside it as `vendor/<name>-<version>.patch`. No other `[patch]`, `[source]` or `paths`
  setting exists, in a manifest or a cargo configuration file, and a crate is patched at one
  version only. A patch fixes a defect of the pinned version: inside the boundary, a route
  that breaks a rule of this section, such as a transcendental from the platform library;
  anywhere, a defect that blocks a milestone criterion; never a feature, an optimisation or
  an API change. It changes at most fifty lines, counting every line it adds or removes,
  comments included. It never touches `Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock` or
  `.cargo_vcs_info.json`, so the crate's version, features, dependencies and closure stay
  upstream's; it may touch a build script, since `libm` selects its paths in `configure.rs`
  and cannot be refused, and such a patch alters what the crate compiles on every target and
  is reviewed as that; it adds no `unsafe` and no dependency; every change carries a
  `LACTOSE patch:` comment, the notice Apache-2.0 asks of a modified file. Inside the
  boundary a patch is reviewed as a determinism review, like a new allowlist name. The owner
  reports it upstream, and `vendor/README.md` records the link, the reason and what drops it.
  Over fifty lines is not a patch: the feature the route serves is refused, or the dependency
  is changed, by a decision. A bump re-applies the patch or drops it once upstream carries
  the change; a patch that no longer applies is a new patch and a new review. The copy is a
  dependency, not a member (`exclude = ["vendor"]`): no workspace lint, format or test runs
  over it, cargo caps none of its own warnings, the crate keeps its crates.io name on the
  allowlist and in `deny.toml`, and `.gitattributes` keeps `vendor/**` free of line-ending
  conversion so every host hashes the same bytes. The same mechanism and check serve a crate
  outside the boundary, since a patch applies to every user of the crate in the workspace.

### §8.3 Enforcement, in the paragraph beginning "Static enforcement uses stable tooling", replace "`cargo-deny` with `multiple-versions = "deny"` for shared crates."

`cargo-deny` with `multiple-versions = "deny"` for shared crates; `cargo xtask vendor` for
each crate built from `vendor/` (section 8.2): offline, the copy is its patch applied to its
record and the patch is within the rule; online, the record is re-derived from the crates.io
archive and the advisory check runs over the upstream versions, which `cargo-deny` skips
behind a path source.

### §9.1 Topology and scale, replace "(the `Cargo.lock` entries, resolved feature set and source hash of boundary crates)"

(the `Cargo.lock` entries, the resolved feature set, and the source hash of boundary crates
and of every crate built from `vendor/`, since the lock entry of a patched crate names
neither a source nor a checksum)

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

### `.gitattributes` (new file, lands with the vendor lane)

```
# Vendored third-party sources (architecture section 8.2, D123) are hashed byte for byte by
# `cargo xtask vendor` and by the boundary build ID: no end-of-line conversion on any host.
vendor/** -text
```

### `crate-tiers.toml` header, append after "is held to these rules wherever it comes from."

```
# A crate built from a patched copy under vendor/ (architecture section 8.2, D123) is not a
# path dependency: it is named by its crates.io name, replaced through [patch.crates-io] in
# the root Cargo.toml, and held to the lists below under that name; `cargo xtask vendor`
# proves the copy is the published archive plus its patch. Patching a crate on these lists
# is a determinism review, as adding one is.
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
  vendor [--offline]         prove each crate built from vendor/ is its patch applied to its
                             record, the patch within the rule of section 8.2, the record
                             the crates.io archive's, and the upstream versions free of
                             advisories
```

### `tools/xtask/README.md`, a row after `features` and a section after `## features`

```
| `vendor` | each crate built from `vendor/` is its patch applied to its record, the patch is within the rule, the record is the crates.io archive's, and the advisory check covers the upstream versions (sections 8.2, 8.3) |
```

```
## `vendor`

- Every `[patch.crates-io]` entry of the root manifest names `vendor/<name>-<version>`; no
  other `[patch]` table exists, and no cargo configuration file sets `[patch]`, `[source]`
  or `paths`. Every resolved package with no source that is not a member lives at
  `vendor/<name>-<version>/` and is named by such an entry; every directory under `vendor/`
  is such a package, holds that name and version, and the name resolves at no other version.
- Each directory holds the `.cargo-checksum.json` that `cargo vendor --versioned-dirs`
  writes, `.gitattributes` marks `vendor/**` `-text`, and `vendor/<name>-<version>.patch`
  beside it is a unified diff of at most fifty changed lines, comments included, that never
  names `Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock` or `.cargo_vcs_info.json`, adds no
  `unsafe`, and marks every file it changes with a `LACTOSE patch:` comment.
- Reversing the patch on a copy under `target/vendor/` gives exactly the files and sha256
  hashes the record holds: nothing changed, added or missing.
- `vendor/README.md` has a section per directory and none for a directory that is gone.
- Online, over a manifest under `target/vendor/upstream/` that pins each upstream version as
  the root `[workspace.dependencies]` entry does: `cargo vendor` re-derives each record and
  it must equal the committed one, and `cargo deny check advisories` reports nothing;
  `cargo-deny` skips a path source, so the workspace run cannot see these. This step
  resolves the shadow's lock without `--locked`; `--offline` passes through to both.
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

To add or bump a crate: vendor the version from a scratch manifest that depends on it as the
root `[workspace.dependencies]` entry does (`cargo vendor --versioned-dirs` into a temporary
directory, with no `[patch]` entry), move `<name>-<version>/` here, apply the patch with
`git apply`, mark every change with a `LACTOSE patch:` comment, point the `[patch.crates-io]`
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
- Patch: `rapier3d-f64-0.36.0.patch`, one file, six changed lines (two removed, four added,
  two of them the notice).
- Upstream report: <filed by the owner; the issue or pull request on dimforge/rapier>.
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
`crate-tiers.toml`, `deny.toml`, `.gitattributes`, `docs/agents.md`, `tools/xtask`)
- Why: the Rapier backend needed one route moved from the platform library to `libm` (the
  angular joint limit's `sin_cos` in `joint_constraint_helper.rs`, six changed lines) and no
  document said whether a boundary crate may be patched, so the lane vendored the crate and
  proposed a rule. Patching is admitted because the alternative, refusing `Joint::limited`,
  drops a feature `anim`'s ragdoll needs for a six-line fix, and a git fork would move source
  out of the repository and past `deny.toml`'s `unknown-git = "deny"`.
  - *One mechanism, every crate.* `[patch.crates-io]` applies to every user of the crate in
    the workspace, inside the boundary or not, so the mechanism and its check are
    workspace-wide; admission differs: inside the boundary a patch fixes a section 8.2 route
    and is reviewed as a determinism review, like a new allowlist name; elsewhere it fixes a
    defect that blocks a milestone criterion; never a feature. A crate is patched at one
    version only, and no `[patch]`, `[source]` or `paths` setting lives anywhere but the root
    manifest, since cargo honours one in `.cargo/config.toml` silently.
  - *Fifty changed lines, comments included; never the manifest records; a build script if
    it must; no `unsafe`; a notice on every change.* The cap the lane brief used; it
    separates a fix from a fork. The sign of zero in SIMD `min` and `max` (about 26 sites
    over rapier, parry, simba and wide) is what it excludes, and that stays a verification
    item for SOUR's architecture mode, not a patch. `Cargo.toml`, `Cargo.toml.orig`,
    `Cargo.lock` and `.cargo_vcs_info.json` stay upstream's so the version, features,
    dependencies and closure the allowlist review saw do not move; a build script may be
    patched because `libm` selects its paths in `configure.rs` and cannot be refused, and
    such a patch is reviewed as a change to what every target compiles. The `LACTOSE patch:`
    comment is the notice Apache-2.0 section 4(b) asks of a modified file, and counting it
    keeps the cap the diff's line count.
  - *The record is what `cargo vendor` writes plus a patch file, and the check re-derives
    it.* `.cargo-checksum.json` carries every upstream file's sha256 and the archive's, in
    the format cargo itself reads; the patch is a unified diff `git apply` consumes. Offline
    the check reverses the patch on a copy, hashes and compares; that proves the copy against
    its record only, since a rewritten hash in the record passes it, so online the check
    re-derives the record from the crates.io archive with `cargo vendor` from a shadow
    manifest and requires equality. Rules out a diff quoted in prose (the lane's README), a
    checksum in prose, and a tool that applies patches at build time (cargo has no hook; the
    tree must exist before `cargo metadata`).
  - *`cargo-deny` sees the name but not the advisories.* Probed on cargo-deny 0.20.2: `bans`,
    `licenses` and `sources` match a patched crate by name and version; `advisories` skips it
    as a path source (`smallvec` 1.6.0, RUSTSEC-2021-0003: reported from the registry, silent
    when vendored and patched). The check runs `cargo deny check advisories` over the same
    shadow manifest, pinned as the root `[workspace.dependencies]` entry pins it and resolved
    against a copy of the workspace lock, and its exit status decides.
  - *The build ID hashes the vendored files.* A patched crate's `Cargo.lock` entry has
    neither `source` nor `checksum`, so section 9.1 hashes the vendored directory as it
    hashes the boundary crates' sources: that names what the binary compiled whether or not
    the check ran, where an archive checksum plus a patch would stay equal for a tampered
    record until the online step caught it. `.gitattributes` marks `vendor/**` `-text` so a
    Windows checkout hashes the same bytes.
  - *A dependency, not a member.* Cargo does not make a patch path a member, but
    `exclude = ["vendor"]` states it, so no workspace lint, format or test runs over the copy
    and `[profile.dev.package."*"]` applies as to the registry build. Cargo caps no lints on a
    path source, so the copy's own warnings show in a clippy run; `-D warnings` reaches the
    selected packages only, so they fail nothing.
  - *Reported upstream by the owner, dropped at the first bump that carries it.* A patch
    nobody reports is a fork by instalments. Rapier's master still called the inherent
    `sin_cos` on 2026-10-11.
  - Rules out: a git fork; a patch over the cap; a patch to a manifest record; a vendored
    copy with no patch; prose as the record; a per-boundary mechanism; a patch declared
    anywhere but the root manifest.
- What the proposal's review changed (0 blocking, 3 serious, 5 minor, 2 process): the shadow
  manifest's `[workspace]` table, pinning and command, and cargo-deny's exit status as the
  verdict; the online re-derivation of the record; cargo configuration files and non-member
  path packages outside `vendor/` as findings; the one-version rule and `package` keys; the
  determinism review in the binding text; the file list in place of "under `src/`"; the
  notice; the row 83 replacement; the check as its own lane before Rapier; the owner files
  the upstream report.
- Driven by: the `physics-rapier` lane (2026-10-10), its report's proposed rule and the review
  brief's vendored-patch item; the planner's and the reviewer's probes (2026-10-11).
- Replaced: nothing stated; the lane brief's "prefer the patch if it is under fifty lines,
  else refuse".

## Verification items (exact text)

Replace row 83 of `docs/verification.md` (the row beginning "`lactose-physics-api` (D119) is
implemented and reviewed against a fake backend. Two known gaps") with:

| `lactose-physics-api` (D119) is implemented and reviewed against a fake backend, and `lactose-physics-rapier` is its backend. Of the two Rapier gaps between the backend and section 2's cross-architecture obligation, the limited angular joint's `sin_cos` through the platform library, bypassing `libm_force`, is closed by the patch under `vendor/` (D123, six changed lines); the sign of zero in Rapier's SIMD `min`/`max`/`clamp` between SSE and NEON, canonicalised only in stored impulses, is about 26 sites over rapier, parry, simba and wide, cannot be closed by a patch under D123's cap, and stays open until SOUR's architecture mode measures it through the conformance scene across linux-x64, windows-x64 and macos-arm64. Until then the property is unproven, not excused | 1 |

Add to section 4:

| Item | Gates milestone |
|---|---|
| `cargo-deny` 0.20.2 skips a path source in `check advisories`: `smallvec` 1.6.0 fails from the registry (RUSTSEC-2021-0003) and passes vendored and patched, while `bans` and `sources` match by name and version. `cargo xtask vendor` carries the advisory check over the upstream versions; at each `cargo-deny` upgrade re-check whether an option covers path sources, and drop the shadow's advisory run when one does | 1 |
| The `sin_cos` patch is not upstream (`dimforge/rapier` master, `joint_constraint_helper.rs`, 2026-10-11): the owner files the report and the link goes into `vendor/README.md`; the patch is dropped at the first bump that carries it | 1 |
| The boundary build ID hashes every vendored directory beside the boundary crates' sources (section 9.1) when `app` computes it; `.gitattributes` marks `vendor/**` `-text`, and the first windows-x64 run of `cargo xtask vendor` (today it runs on ubuntu only) is the check that a Windows checkout hashes the same bytes | 4 |

## Cost

The check: one `lane` run on main, one `reviewer` round, and the fix rounds `docs/agents.md`
section 2 requires until a round is clean, usually one `reviewer-diff`; `cargo xtask vendor`
with its tests is about the size of `features.rs`. The Rapier fix round carries the four data
changes above, minutes inside the round the review already requires, and its `reviewer-diff`
covers them. The owner files the upstream report; that is not an agent run. The merge and
every CI run gain one gate of a few seconds plus one `cargo vendor` and one `cargo deny` run.
