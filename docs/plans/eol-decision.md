# The line-ending rule: LF in every checkout

Status: decision, written at main `437fcf6`, architecture revision 3.3, revised after its
first review, with main at `10cf89e`, and after its second, with main at `4d60326`. Nothing
of section 5 is applied until the owner answers section 4, and the coordinator applies it.
This document changes no code and no configuration. The owner approved answer 1 on
16 November 2026; it is D148.

A Windows checkout turns LF into CRLF, and code that reads a tracked file has failed on it
twice. Section 1 is the evidence. Section 2 is the decision and what each option changes.
Section 3 is the lane. Section 4 is the one question for the owner. Section 5 is the text for
the coordinator. Section 6 says what this document could not verify.

In brief:

- **Every tracked text file outside `vendor/` has LF line endings, in the index and in every
  checkout made or refreshed under the rule.** `.gitattributes` gains `* text=auto eol=lf`,
  which holds whatever `core.autocrlf` says.
- **`vendor/**` stays `-text`, and D123 does not change.** A kind of file that is not text is
  named `binary` in the change that adds its first file. None is tracked today.
- **Nothing changes in the index.** No tracked file holds CRLF there, so no blob, no hash and
  no checkout on the Mac moves.
- **One script checks the rule.** `merger` runs it at every merge, and CI's check job on the
  three hosts. Its first run on the Windows runner is the proof this document could not give.
- **The CRLF tolerance already in code stays, and a new reader owes none.**
- **One lane, one push made alone, and one refresh of each checkout on the Windows laptop.**

## 1. The evidence

**A Windows checkout today.** `.gitattributes` holds one rule, `vendor/** -text` (D123).
`git ls-files --eol` at `437fcf6` lists 870 files. The 622 outside `vendor/` are `i/lf` with
no attribute. The 248 under it are `-text`: 247 are `i/lf`, and `.cargo-checksum.json` is one
line with no ending. No tracked file is binary. No workflow sets `core.autocrlf`, and the
Windows runner's git converts. In run 43752979947 rust-cache's key ends in `446ff03d` on the
Linux and macOS jobs and in `a3af042e` on both Windows jobs, and what it hashes byte for byte
is `.cargo/config.toml` and `rust-toolchain.toml`. A scratch clone of `437fcf6`, checked out
again under `core.autocrlf=true` (git 2.39.5, macOS), is that checkout: 622 files `w/crlf`,
the 248 vendored ones as they were. Main has since moved to `10cf89e`, which holds the same
870 paths, and to `4d60326`, which holds 886: the sixteen new ones are lane `ricotta`'s, and
638 are outside `vendor/`. No blob of either holds a CR or a NUL byte, and neither tracks a
symbolic link or a submodule.

**The readers.** Every place that reads a tracked text file and compares, hashes or parses
it.

| Reader | What it reads | On a CRLF checkout today |
|---|---|---|
| `cargo xtask measure edit-to-playing` (`edited` in `tools/xtask/src/measure.rs`) | The anchor lines of `crates/app-headless/src/main.rs` and `crates/app/src/runtime.rs`, which it edits | Failed until `9790ce4`: the anchors ended in `\n`. Now a whole-line match that keeps the file's ending, with a CRLF test |
| The schema guard (`guard` in `crates/app/tests/schema_record.rs`) | `tests/schema_record/engine.txt`, compared as text with the build's rendering | Would have failed; its first review found it. It normalises CRLF since `d6efc8b`, with a CRLF test |
| `golden_vectors_match` (`crates/math/tests/golden.rs`) | `include_str!` of `golden/vectors.txt`, 17,801 lines | The test binary embeds CRLF. `str::lines` drops the CR |
| The unit tests of `tools/xtask` | `include_str!` of `tools/xtask/ci-policy.toml`, `crate-tiers.toml`, `crates/core/clippy.toml` and `.github/workflows/nightly.yml` | They embed CRLF. The TOML parser reads it, and so does the workflow reader of `miri.rs`, written for LF or CRLF |
| The tests of `tools/xtask/src/measure.rs`, through `repository` | At run time: `docs/results/README.md`, `crates/app/tests/spike.rs`, the two edit sites and each test file a measurement names | They normalise CRLF or search for a substring |
| `tools/xtask/tests/workspace.rs`, `boundary_lints.rs`, `vendor.rs` | `.cargo/config.toml`, the workflows, `Cargo.lock`, `crates/core/clippy.toml`, `deny.toml` | `repository_file` normalises CRLF; the rest read by lines or hand the file to cargo and clippy |
| `cargo xtask tiers`, `boundary`, `features`, `vendor` | Through `tools/xtask/src/repo.rs`: every `.rs`, `.toml`, `README.md` and `SAFETY.md`, `docs/architecture.md` for section 21's table, the workflows and the scripts | CI runs them on ubuntu alone. All four pass on the emulated CRLF checkout (this document's probe) |
| The hashes of `cargo xtask vendor` (D123) | Every file under `vendor/`, byte for byte, against `.cargo-checksum.json`; `git apply` under `core.autocrlf=false` | Unchanged, by `-text`: the vendored working tree hashes the same in the LF and the CRLF checkout |
| The boundary build ID (architecture section 9.1) | The boundary crates' sources and `vendor/` | Not computed yet: `lactose-net-host` holds the type alone, and the verification row "The boundary build ID hashes every vendored directory" gates milestone 4. From a CRLF checkout a Windows client would compute another ID than its Linux server, and the handshake refuses a mismatch |
| SOUR | No tracked file: it hashes world state and packets. `[harness]` of `ci-policy.toml` goes through the TOML parser | Equal on windows-x64 in nightly 43703297881 |
| The compile-fail tests of `lactose-ecs` and `lactose-reflect`; the two `include!`s of `crates/sim/tests/cost/scene.rs` | Rust sources | rustc reads a CRLF source as LF |
| cargo and rust-cache | `Cargo.lock` under `--locked`; the two files rust-cache hashes byte for byte | cargo passes. The Windows cache key differs, as above |
| The cooker | `tools/cook` and one `.gltf` source, in the tree since `4d60326` (lane `ricotta`). It parses the source as JSON, and an asset's version is the hash of its cooked bytes (`cook.rs` at `daaf347`) | Its output does not depend on the source's line endings |

Every reader in the tree tolerates CRLF at `437fcf6`. Run 43771184301 there is green on both
Windows jobs, which run the workspace's tests. The `measure` workflow passed on the Windows
runner in run 43659357193 at `bef3455`. The four policy checks, which no Windows job runs,
pass on the emulated checkout.

**What the two failures cost.** The first reached main: run 43654643980 at `d7274b2` failed
`each_edit_s_anchor_occurs_once_in_its_file` in check (windows-latest), where nextest stopped
with 1,691 of 1,816 tests run. Lane `measure-crlf` repaired it (merge `bef3455`), and its
emulated CRLF checkout on macOS found no other failure in the workspace's tests. The second
was one finding of the schema guard's first review. Each reader paid for its tolerance in
code and a CRLF test, and the only check that a reader has it is the Windows jobs, after a
push.

**What git does under the rule.** Probed with section 5's two lines in scratch clones of
`437fcf6`: the CRLF one above, and one left LF. The review ran its own (section 6).

- With `core.autocrlf=true` still set, checking every file out again gives 622 files `w/lf`
  under `attr/text=auto eol=lf`, and the 248 vendored files unchanged: the vendored working
  tree's hash is the same before and after.
- `git add --renormalize .` changes no file's blob. On the LF clone, with the new file not
  yet committed, `git status --porcelain` then prints `M  .gitattributes` and nothing else.
  On the CRLF clone, after the commit, it prints nothing.
- A pull alone refreshes nothing. After the commit the CRLF clone's status is clean, and
  every file outside `vendor/` but `.gitattributes` is still `w/crlf`: 621 files at
  `437fcf6`.
- `git rm --cached -r -q :/` and then `git reset --hard` refresh it: no file is `w/crlf`
  after them, from any directory. With `.` in place of `:/`, run in `crates/`, they refresh
  that directory alone. `git add --renormalize .`, `git checkout -- .`,
  `git checkout-index -f -a` and `git reset --hard` alone each leave every file CRLF.
- A CRLF file added under `vendor/` is stored and checked out byte for byte
  (`i/crlf w/crlf attr/-text`): `-text` wins over the `eol=lf` the first line also sets. The
  same file added under `docs/` is stored LF, with git's warning.
- A file with a NUL byte lists as `i/-text` under `attr/text=auto eol=lf`, and as
  `attr/-text` once a line `*.bin binary` names its kind. A text file with one stray CR
  lists as the first does (the review's probe).
- `cargo xtask tiers`, `boundary` and `vendor --offline` pass with the two lines, and
  `vendor --offline` also with a `binary` line above the vendor line. `keeps_vendor_bytes`
  in `tools/xtask/src/vendor.rs` requires `vendor/** -text` to be the last line that reaches
  `vendor/`, and its test already holds `* text=auto` above it. The comments of section 5's
  file have changed since those runs; `sets_text` there skips a comment line.
- `git ls-files --format` gives the same three facts as bare values, `lf`, `crlf`, `mixed`,
  `none` or `-text` for the index and the working tree, and the attribute as written. Its
  atoms need git 2.38. On git 2.39.5, with a root pathspec and run in a subdirectory, it
  prints garbage for the paths outside that directory unless `--full-name` is given.

## 2. The decision

**`.gitattributes` gains `* text=auto eol=lf`: option 1 of the brief.** Git stores every text
file with LF and checks it out with LF on every host. The reason is not the two failures. It
is the hashes that must agree between hosts. Section 9.1's build ID hashes the boundary
crates' sources, section 2 gives Windows clients a Linux server, and D123 made the bytes
equal for `vendor/` alone. The cooker's text sources will need the same once a cooked format
embeds source text, and so will milestone 2's golden files if they are compared as bytes.
One line gives it to every file.

What each alternative rules out, and why it is not taken:

- **`eol=lf` for named paths (option 2).** The list is then what nothing checks: a new
  reader's fixture is CRLF on Windows until someone names it. The files a hash must agree on
  are a large part of the tree already: the eleven boundary crates it holds are 278 of the
  622 files.
- **Every reader tolerates CRLF, with a check (option 3).** Tolerance cannot make two hosts'
  hashes agree; only a normalising hasher could, against D123's byte-for-byte reading. The
  check exists already, the Windows jobs, and runs after the push. It does not run the policy
  checks, and each new reader pays again.
- **`* -text`, no conversion anywhere.** Every host would also get the index's bytes, with no
  heuristic. Git would then store CRLF as an editor wrote it, so LF in the index would rest
  on a check alone.
- **`core.autocrlf=false` in the workflows.** It reaches the runners and no other clone.

Two facts would change it. If the script's first run on the Windows runner refuses a file
for its working tree, the rule is not in force there and the decision returns to the
planner. If a tracked file must hold CRLF on disk, a `.bat` script for example, its kind
gets its own line above the vendor line and the script admits it, in the change that adds
the file; the rule stands.

**What each option changes.**

| | 1. `* text=auto eol=lf` | 2. `eol=lf` for named paths | 3. Today's rule and a check |
|---|---|---|---|
| The `windows-latest` runner | Every text file LF: the bytes the Linux and macOS jobs already pass on. Each CRLF test builds its own CRLF | The named paths LF, the rest CRLF | CRLF, as today |
| The Windows laptop's checkout | Stale until refreshed, since a pull rewrites only the files it changes. Once, in each checkout and worktree, with nothing uncommitted and no merge in progress: `git rm --cached -r -q :/`, then `git reset --hard`. They write every tracked file again, so cargo builds the workspace's crates and the vendored Rapier again once. `git add --renormalize .`, `git checkout -- .`, `git checkout-index -f -a` and `git reset --hard` alone leave the checkout CRLF | The same step, or the named files stay CRLF | Nothing |
| The vendored tree's hashes | Unchanged: `-text` on the last line, the same blobs and the same working bytes | Unchanged | Unchanged |
| CI caches keyed on file contents | The Windows jobs' rust-cache key changes once, because `.cargo/config.toml` and `rust-toolchain.toml` become LF there. D125's rule applies: the push is made alone, and nothing follows until its Windows check completes | The same, if either file is named | None |
| A contributor's editor | An editor that writes CRLF changes its working file alone: git stores LF and warns. No `.editorconfig` is added | The same inside the named paths | No change |

**`git add --renormalize .` changes nothing in the index** (section 1). The Mac's checkouts
and every agent worktree need no step. A branch in flight merges as before, and owes a line
only if it adds a file that is not text.

**D123 does not change.** Its rule, its `-text` line and section 8.2's sentence stand as
written. The line moves to the end of the file, where `cargo xtask vendor` already requires
it. Row "The boundary build ID hashes every vendored directory" gains what the rule adds.

**A kind that is not text is named `binary`, in the change that adds its first file.**
`text=auto` leaves the choice to git's reading of the content. A file git takes for text is
rewritten on `git add` wherever it holds CRLF, which would corrupt a binary one. A NUL byte
settles it for the usual formats, and the check refuses a file git had to classify itself,
so each kind is named at its first file. No kind is named now, because none is tracked.

**The rule needs a check, and it is one tracked script that `merger` and CI both run.**
`tools/xtask/line-endings.sh` reads git's own account of each tracked file: its line endings
in the index, its line endings in the working tree, and its attributes. The index and the
attributes read the same on every host, so `merger` runs the script among its gates and a
merge it refuses stops before the push. The working tree is the host's own, so CI's check
job runs the same file on the three hosts. The first draft wrote the check into the workflow
alone. That is not taken: `merger` could not run it, and the lane's proof would have run a
copy of it. It is not Rust in `cargo xtask` either: it is a few lines over git's listing,
and the Windows run waits for no build. The cost is one gate, in `docs/agents.md` and in
`merger`'s definition.

**What the check refuses, and what it cannot.** Outside `vendor/` it accepts a file under
`text=auto eol=lf` whose index and working tree are LF, and a file under `-text` that git
itself reads as binary. It refuses the rest: CRLF in the index or in the working tree, a
file git reads as binary whose kind no line names, a `-text` or `binary` line that reaches a
text file, and any other attribute. Outside `vendor/`, a symbolic link, a submodule and a
text file missing from the checkout are refused as well, since git lists no line endings for
them; no link or submodule is tracked. The first draft accepted `-text` anywhere, and the
review stored a CRLF file under a line `*.md -text` with the check green. The narrowing has
a cost: a file of a kind named `binary` whose bytes git happens to read as text is refused,
and the change that adds it admits its path in the script. Under `vendor/` the script asks
only for `-text`; that the vendor line exists is `cargo xtask vendor`'s check. It cannot see
a file that is not text and that git took for text, since git rewrote it at `git add`; that
stays a rule for the lane that adds a kind.

**The CRLF tolerance in code stays.** That is `edited` in `measure.rs`, the schema guard's
normalising read, `repository_file`, the workflow reader of `miri.rs` and the `\r` handling
of `vendor.rs`, each with its test. A checkout is CRLF until it is refreshed, a hand copy of
such a tree carries CRLF, and each test builds its own CRLF, so none depends on the
checkout. Removing them would cost a lane and buy nothing. A new reader of a tracked file
owes no tolerance of CRLF from a checkout. It still owes what its other inputs bring: a child
process on Windows ends its lines with CRLF, as `replay_figures` in `measure.rs` handles.

## 3. The lane

Lane `eol-rule`, from main. Four files and no Rust.

- `.gitattributes` becomes section 5's text.
- `tools/xtask/line-endings.sh`, new. It lists every tracked file with
  `git ls-files --full-name --format`, four fields joined by git's own `%x09`, not by a tab
  the shell supplies: `%(eolinfo:index)`, `%(eolinfo:worktree)`, `%(eolattr)` and `%(path)`.
  The atoms need git 2.38; the Mac has 2.39.5 and the runner 2.56. It lists the whole tree
  whatever directory it is run in, with the root pathspec `:/`, and with full names
  (section 1's last probe). It accepts a file in one of three shapes and refuses every
  other, printing its line:
  1. a path outside `vendor/` with attribute `text=auto eol=lf`, index `lf` or `none`, working
     tree `lf` or `none`;
  2. a path under `vendor/` with attribute `-text`, whatever its index and working tree;
  3. a path outside `vendor/` with attribute `-text` and index `-text`.
- It exits non-zero if `git ls-files` does or lists no file, and its last line on success is
  the number of files it read.
- Its message gives the remedy for each refusal. Where more than one applies, the attribute's
  remedy comes first: `git add --renormalize` stores LF only under `text=auto eol=lf`.
  - index `crlf` or `mixed`: `git add --renormalize <path>`; a plain `git add` keeps the CR.
  - working tree `crlf` or `mixed` over an index `lf`, under `text=auto eol=lf`: the checkout
    predates the rule, or a CRLF edit was committed, and it is refreshed (section 2's table).
    In CI, whose checkout is fresh, it means the rule does not hold on that host, and the
    decision returns to the planner (section 2).
  - index `-text` under `text=auto eol=lf`: a stray CR or NUL is removed from a text file, or
    a kind that is not text is named `binary`.
  - an empty index or working-tree field: a symbolic link or submodule, which the rule does
    not admit, or a file missing from the checkout, which is restored.
  - attribute `-text` outside `vendor/` over an index that is not `-text`: a `-text` or
    `binary` line reaches a text file, and is narrowed or removed.
  - a path under `vendor/` whose attribute is not `-text`: the vendor line is restored as the
    last line of the root `.gitattributes` that reaches it, and a `.gitattributes` under
    `vendor/` that sets an attribute on it is removed (D123). For a vendored file this remedy
    is printed alone.
  - any other attribute: a line of `.gitattributes` takes the file out of the rule, or the
    rule's line is gone. On a host that converts, that is why its working tree is CRLF.
  - anything else, which is a working tree `-text` over an index `lf` or `none`, or `crlf` or
    `mixed` over an index `none`: the working file holds an uncommitted edit, which is
    reverted; committed instead, it is refused again by the remedy its new line matches.
- `.github/workflows/ci.yml`, job `check`: one step directly after the checkout, on the three
  hosts, `bash tools/xtask/line-endings.sh`. The step names `shell: bash`: the workflow sets
  no default shell, and the Windows runner would use PowerShell.
- `tools/xtask/README.md` gains the script: what it accepts, its remedies and who runs it.

Its proof. The script passes on the three hosts in CI, reading the same number of files on
each, and its Windows run is the proof of the rule. Each case below is a scratch clone made
as the runner makes its checkout, under a system `core.autocrlf=true`, and runs the script
as that clone checked it out. The script refuses six mutations:

1. the rule's line removed. Every file is then CRLF there, the script included, and the run
   must still fail;
2. a file with a NUL byte whose kind has no line;
3. a later line `*.md text eol=crlf`;
4. a line `*.md -text` above the vendor line, and a CRLF `.md` file;
5. no change to the files: a clone made under `core.autocrlf=true` before the rule and then
   fast-forwarded to it. Its index and attributes are right, so it is refused through the
   working-tree field alone;
6. the script's own blob stored CRLF, nothing else changed; it alone is refused, by the CRLF
   copy of itself that the clone checked out.

It accepts four controls: the rule as written; a file with a NUL byte whose kind a `binary`
line names; an empty file and a file with no line break; a CRLF file under `vendor/`. Run in
a subdirectory it gives the same answers. Run outside any repository, where `git ls-files`
fails, and in a repository that tracks no file, it exits non-zero. `cargo xtask boundary`
and `cargo xtask vendor`, which read the workflows, the scripts and `.gitattributes`, and
`tools/xtask/tests/workspace.rs` pass.

- The merge's brief names `bash tools/xtask/line-endings.sh` as an extra gate: the
  definition `merger` runs under gains the gate only with this merge. The merge carries
  section 5's documents. Its push is made alone (D125).
- It lands before any lane that adds a file which is not text, or that hashes a tracked
  file's bytes.

It costs three agent runs if the first review is clean: `lane`, `reviewer`, `merger`. A fix
round adds two. The owner then refreshes each checkout on the laptop once (section 2's
table).

## 4. The question for the owner

One question. The coordinator asks it in these words; the recommended option is first.

> **Header:** Line endings
>
> A Windows checkout turns LF into CRLF, and code that reads a tracked file has failed on it
> twice: the `measure` anchors on the Windows runner, and the schema guard at review. The
> boundary build ID will need the same bytes on every host, and so will a cooked format that
> embeds source text. No tracked file holds CRLF in the index, so the rule changes no
> committed file but `.gitattributes`. Do you approve the repository-wide rule?
>
> 1. **Approve the LF rule with its check (Recommended):** `.gitattributes` gains
>    `* text=auto eol=lf`, `vendor/**` stays `-text` (D123 unchanged), and one script checks
>    it. `merger` runs the script at every merge, which adds a gate to its definition, and
>    CI's check job runs it on the three hosts. One lane of four files and no Rust, three
>    agent runs if its review is clean. One push made alone, since the Windows cache key
>    changes once. You refresh each checkout on the laptop once, with two git commands. The
>    CRLF tolerance already in code stays.
> 2. **Approve the rule without the check:** The coordinator applies `.gitattributes` and the
>    documents, with no lane. Nothing then shows that the Windows runner checks out LF.
>    Nothing refuses a later line that takes a tracked text file out of the rule, or a file
>    git reads as binary whose kind is unnamed.
> 3. **Keep today's rule:** Every reader keeps tolerating CRLF and only the Windows jobs
>    check it, after a push. The build ID's source hash then needs its own answer at
>    milestone 4.

## 5. Text for the coordinator (exact)

The text is for answer 1. `D<N>` is the number the coordinator allocates at application, and
`<date>` is the date of the owner's answer. Each OLD is its file's own text at `4d60326` and
occurs exactly once there; a script checked it against that commit's files. They occur once
at `437fcf6` and `10cf89e` too, and at `6f0508a`, the merge of lane `spike-rhi` that follows
`4d60326` and edits other lines of `docs/verification.md`.

The owner approved answer 1 on 16 November 2026, and the coordinator applied it as D148 at the
merge of lane `eol-rule`, with the third review's wording in section 2, section 3's remedies
and the entry's sub-item on the check, and with the lane's first review's change to section
3: the first shape is outside `vendor/`, and a vendored file not under `-text` has its own
remedy.

Answer 2 has no lane, and the coordinator applies the same text with the check cut out of
it. The cuts: the last sentence of the `.gitattributes` comment's first paragraph; the entry's
sub-item "The check is one script", in its "Rules out" the clause on a check written into
the workflow alone, and in its "Driven by" the words ", which narrowed what the check
accepts"; in the architecture block, the words ", and the line-ending check" and the rule's
last sentence; the `docs/agents.md` block and the `merger` block, whole; in the new
verification row, the sentence that begins "The proof is" and the words "Beside it, ". The
entry's title and references then read:

```
**D<N>. Every tracked text file outside `vendor/` has LF line endings, in the index and in
every checkout made or refreshed under the rule: `.gitattributes` sets `* text=auto eol=lf`,
`vendor/**` stays `-text`, and a kind that is not text is named `binary`** (§8.2, §9.1, §20,
`.gitattributes`)
```

### `.gitattributes`, the whole file

Lane `eol-rule` writes it (answer 1), or the coordinator (answer 2). It names no decision
number, so the lane writes it as it stands.

```
# Every tracked text file outside `vendor/` has LF line endings, in the index and in every
# checkout made or refreshed under this rule, on every host and whatever `core.autocrlf` says
# (architecture section 20), so code that compares, hashes or parses a tracked file reads the
# same bytes everywhere. A file that is not text is named by kind, as `binary`, on a line
# between this rule and the vendor line, in the change that adds the first of its kind.
# `tools/xtask/line-endings.sh` checks the tree against this file, in CI and at every merge.
* text=auto eol=lf

# Vendored third-party sources (architecture section 8.2, D123) are hashed byte for byte by
# `cargo xtask vendor` and by the boundary build ID: no end-of-line conversion on any host.
# This line stays the last one, below every line that can match a file under `vendor/`.
vendor/** -text
```

### Decision entry

It goes at the end of section 9 of `docs/decisions-log.md`, before `## 10. Review history`.

```
=== docs/decisions-log.md ===
--- OLD ---
## 10. Review history
--- NEW ---
**D<N>. Every tracked text file outside `vendor/` has LF line endings, in the index and in
every checkout made or refreshed under the rule: `.gitattributes` sets `* text=auto eol=lf`,
`vendor/**` stays `-text`, a kind that is not text is named `binary`, and one script checks
it at every merge and on CI's three hosts** (§8.2, §9.1, §20, `.gitattributes`,
`tools/xtask/line-endings.sh`, `.github/workflows/ci.yml`, `docs/agents.md`)
- Why: a Windows checkout under `core.autocrlf=true` gave every file outside `vendor/` CRLF,
  and code that reads a tracked file failed on it twice: the edit-to-playing anchors of
  `cargo xtask measure` on the Windows runner (run 43654643980 at `d7274b2`, repaired by
  `9790ce4`), and the schema guard's comparison of its record, found at review (`d6efc8b`).
  `.gitattributes` held `vendor/** -text` alone (D123), so each reader owed its own tolerance
  and only the Windows jobs checked it, after a push. `docs/plans/eol-decision.md` holds the
  evidence; these are the decisions.
  - *One rule for the repository.* `* text=auto eol=lf`: git stores a text file with LF and
    checks it out with LF whatever `core.autocrlf` says. None of the 870 tracked files held
    CRLF in the index at `437fcf6`, so `git add --renormalize .` changed no blob. Probed on
    git 2.39.5 under `core.autocrlf=true`: the 622 files outside `vendor/` check out LF, and
    the 248 under it are unchanged.
  - *The reason is the hashes that must agree between hosts.* Section 9.1's boundary build ID
    hashes the boundary crates' sources, a Windows client must compute the ID its Linux
    server computes, and D123 made the bytes equal for `vendor/` alone. The cooker's text
    sources will need the same once a cooked format embeds source text, and so will
    milestone 2's golden files if they are compared as bytes. A reader of a tracked file
    owes no tolerance of CRLF from a checkout.
  - *`vendor/**` stays `-text`, on the last line; D123 is unchanged.* `cargo xtask vendor`
    already requires it to be the last line that reaches `vendor/`, and passes with the rule
    above it.
  - *A kind of file that is not text is named `binary` in the change that adds its first
    file.* `text=auto` leaves the choice to git's reading of the content, and a file it takes
    for text is rewritten on `git add`. None was tracked at `437fcf6`, so none is named.
  - *The check is one script, `tools/xtask/line-endings.sh`, which `merger` runs at every
    merge and CI's check job on the three hosts.* It reads git's own listing of each tracked
    file (`git ls-files --format`, git 2.38 or later). Outside `vendor/` it accepts a file
    under `text=auto eol=lf` whose index and working tree are LF, and a file under `-text`
    that git reads as binary. It refuses the rest: CRLF in the index or the working tree, a
    file git reads as binary whose kind no line names, a `-text` or `binary` line that
    reaches a text file, and any other attribute, which a later line or the rule line's
    removal gives. Outside `vendor/`, a symbolic link, a submodule and a text file missing
    from the checkout are refused as well, since git lists no line endings for them; no link
    or submodule was tracked at `4d60326`. Under `vendor/` it asks only for `-text`. It
    cannot see a file that is not text and that git took for text. Its first run on the
    Windows runner is the proof of the rule. It is a script, not Rust in `cargo xtask`, so
    that the Windows run waits for no build and the lane's proof runs the file CI runs.
  - *The CRLF tolerance in code stays.* `measure`'s whole-line anchors, the schema guard's
    normalising read, `repository_file` in `tools/xtask/tests/workspace.rs`, the workflow
    reader of `miri.rs` and `vendor.rs`'s handling of `\r`, each with its own test. A checkout
    not yet refreshed and a hand copy can still carry CRLF.
  - *One push, made alone, and one refresh of each older checkout.* rust-cache hashes
    `.cargo/config.toml` and `rust-toolchain.toml` byte for byte, so the Windows jobs' cache
    key changes once (D125). A checkout or worktree made before the rule under
    `core.autocrlf=true` keeps its CRLF until, with nothing uncommitted and no merge in
    progress, `git rm --cached -r -q :/` and `git reset --hard` check every file out again.
    `git add --renormalize`, `git checkout`, `git checkout-index -f -a` and `git reset
    --hard` alone do not.
  - Rules out: `eol=lf` for named paths, whose list nothing would check; every reader
    tolerating CRLF, which cannot make two hosts' hashes agree; `* -text`, which would leave
    LF in the index to a check alone; `core.autocrlf=false` in the workflows, which reaches
    no other clone; a check written into the workflow alone, which `merger` cannot run; an
    `.editorconfig`; removing the tolerance in code.
- Driven by: CI run 43654643980 and lane `measure-crlf`; the first review of the schema
  guard; the probes of `docs/plans/eol-decision.md` at `437fcf6` and its review, which
  narrowed what the check accepts; the owner's approval of <date>.
- Replaced: nothing stated. `.gitattributes` held the vendor line alone.

## 10. Review history
```

### Architecture amendment

`docs/architecture.md`, section 20: the CI rule names the check, and one rule follows it.
Section 8.2's sentence on `vendor/**` and section 9.1 are not changed.

```
=== docs/architecture.md ===
--- OLD ---
  boundary, feature and vendor checks.
--- NEW ---
  boundary, feature and vendor checks, and the line-ending check.
- Every tracked text file outside `vendor/` has LF line endings, in the index and in every
  checkout made or refreshed under the rule: `.gitattributes` sets `* text=auto eol=lf`,
  which holds whatever `core.autocrlf` says. Code that compares, hashes or parses a tracked
  file therefore reads the same bytes on every host and owes no tolerance of CRLF from a
  checkout. Two exceptions, each a line of `.gitattributes`: `vendor/**` is `-text` and keeps
  upstream's bytes (section 8.2), and a kind of file that is not text is named `binary` in
  the change that adds its first file (D<N>). `tools/xtask/line-endings.sh` checks git's
  listing of the tree against the rule: `merger` runs it at every merge, and CI's check job
  on the three platforms.
```

### The gate

`docs/agents.md`, section 4, and `merger`'s definition, step 5. The second is an agent's
definition: the owner's answer 1 is what approves it.

```
=== docs/agents.md ===
--- OLD ---
- `cargo xtask tiers`, `cargo xtask boundary`, `cargo xtask features`, `cargo xtask vendor`;
--- NEW ---
- `cargo xtask tiers`, `cargo xtask boundary`, `cargo xtask features`, `cargo xtask vendor`,
  `bash tools/xtask/line-endings.sh`;
```

```
=== .claude/agents/merger.md ===
--- OLD ---
   `cargo xtask tiers`; `cargo xtask boundary`; `cargo xtask features`; `cargo xtask vendor`;
--- NEW ---
   `cargo xtask tiers`; `cargo xtask boundary`; `cargo xtask features`; `cargo xtask vendor`;
   `bash tools/xtask/line-endings.sh`;
```

### Verification rows

`docs/verification.md`, one block: a fragment of the row "The boundary build ID hashes every
vendored directory", and a new row after it.

```
=== docs/verification.md ===
--- OLD ---
is the check that a Windows checkout hashes the same bytes | 4 |
--- NEW ---
is the check that a Windows checkout hashes the same bytes. Since D<N> the boundary crates' sources are LF as well in every checkout made or refreshed under the rule, by `* text=auto eol=lf`, so the ID can hash them byte for byte as it hashes `vendor/`. A CRLF checkout of `437fcf6` emulated on macOS (git 2.39.5, `core.autocrlf=true`) passed `cargo xtask vendor --offline`, which is not the windows-x64 run | 4 |
| The line-ending rule on a Windows host (D<N>): `* text=auto eol=lf` was probed on macOS alone, on git 2.39.5 with `core.autocrlf=true` set in scratch clones of `437fcf6`, where every text file outside `vendor/` then checked out LF. The proof is the first run of `tools/xtask/line-endings.sh` in check (windows-latest), on the runner's own git (2.56.0.windows.4 in run 43752979947): it passes, and the number of files it read equals the Linux job's, so no tracked file there is `w/crlf` or `w/mixed`. Beside it, rust-cache's key for the Windows jobs is expected to end in the hash the Linux and macOS jobs' keys end in (`a3af042e` against `446ff03d` in run 43752979947), since it hashes `.cargo/config.toml` and `rust-toolchain.toml` byte for byte. The owner refreshes each checkout and worktree on the Windows laptop once (with nothing uncommitted and no merge in progress, `git rm --cached -r -q :/` then `git reset --hard`), after which `git ls-files --eol :/` lists no `w/crlf` there | 2 |
```

## 6. What this document could not verify

- **No Windows host was used.** That `eol=lf` overrides the runner's `core.autocrlf` is git's
  documented behaviour, probed on git 2.39.5 on macOS with the setting in a scratch clone's
  own configuration, and by the first review under five settings of `core.autocrlf` and
  `core.eol`. The runner has git 2.56.0.windows.4 and the setting at system level. The first
  review compared the sources of 2.39.5 and 2.56.1 and found the line-ending logic and the
  listing unchanged; no review and no draft of this document ran 2.56. The script's first
  run there is the proof, and the new verification row holds it.
- **The laptop's configuration is the brief's statement.** Its git version, its
  `core.autocrlf` and the state of its checkouts were not read. The refresh was run on the
  macOS emulation alone, and file locks during `git reset --hard` on Windows were not
  tested. The script needs git 2.38 there; `git ls-files --eol` does not.
- **rust-cache's hashing was read through a summarising fetch** of `src/config.ts` on its
  `v2` branch on 16 November 2026, not at the commit the runs used. The keys of run
  43752979947 agree with it. That the Windows key will end as the other hosts' do, and that
  the first run restores the old entry by its prefix, are predictions.
- **The four policy checks were run on an emulated CRLF checkout, not on Windows.** Path
  separators and the Windows build of `cargo xtask` were not exercised, so the row "The
  boundary build ID hashes every vendored directory" stays open.
- **The workspace's tests were not run on the emulated checkout.** CI's Windows jobs at
  `437fcf6` are the evidence that every reader they run tolerates CRLF.
- **The script is not written.** The first review's two forms of the step and the second
  draft's two, one in awk and one in bash alone, are probes and are not in the tree. The
  second draft's two accepted the tree under the rule, run in a subdirectory. With the rule's
  line removed under `core.autocrlf=true`, each ran as the CRLF copy that clone checked out,
  on the Mac's bash 3.2.57: the awk form refused 625 files and exited 1, and the bash form
  stopped on a syntax error and exited 2. What the runner's bash and awk do with a CRLF copy
  was not run. `merger`'s run on the Mac refuses a change of attributes before the push, so
  the runner meets a CRLF copy only if that gate was skipped.
- **Section 3 was run as a script on the Mac alone.** The second review wrote section 3 as
  it then stood into a script and ran it from fresh clones made under a system
  `core.autocrlf=true`, at the root and in `crates/`: it refused the five mutations then
  listed and accepted the four controls. It also passed where `git ls-files` failed, and a
  CRLF copy that took its tab from the shell refused every file; section 3 now rules out
  both. This revision ran one form of section 3 as it now stands, in awk, over the 886 files
  of `4d60326` and the script. It read 887 files at the root and in `crates/`. Mutations 1,
  4, 5 and 6 refused 639, 88, 637 and 1 files, the first and the last as a CRLF copy.
  Outside a repository and in an empty one it exited 1, as an LF copy and as a CRLF one. A
  symbolic link and a gitlink each listed with both fields empty, and a file deleted from
  the checkout with an empty working-tree field. An uncommitted edit listed as section 3's
  last remedy names it: `-text` over an index `lf`, and `crlf` over an index `none`. Neither
  that review nor this revision ran bash 5, an awk other than the Mac's, or git 2.56. That
  Git Bash hands `:/` and the format to git unconverted was read from its source by the
  second review, and not run.
- **`git ls-files --format` in a subdirectory was read on git 2.39.5 alone.** Whether 2.56
  still needs `--full-name` there was not run; the script gives it either way.
- **Branches in flight.** The first review read `ricotta` at `daaf347` and `spike-rhi`:
  neither adds a file with a CR or a NUL. The second draft read `cook.rs` and the parse in
  `gltf.rs` at `daaf347`, and nothing else of either. Both have merged since, at `4d60326`
  and `6f0508a`, and this revision found no CR and no NUL in any blob of either commit.
- **Git refused to run under the project scratchpad from the planner's worktree**, so the
  scratch clones lived in the session scratchpad, this revision's as well; the first draft's
  build directory was in the project one. All are deleted.
