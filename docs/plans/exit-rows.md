# Milestone 1 exit: the rows with no owner or no closer

Status: proposed, for the owner. Written at main `10cf89e` and revised at `6f0508a` after its
first review and its second, architecture revision 3.3. Nothing of section 6 is applied until
the owner approves it, and the coordinator applies it. This document changes no code. The
owner approved it on 16 November 2026, with the lint at milestone 4; it is D149.

Exit step 4 of `docs/plans/milestone-1.md`, section 7, needs every verification row that gates
milestone 1 Done or re-gated. The coordinator's pass at `10cf89e` left two rows that no
decision settles and six with a remainder nobody owns. This plan decides each one: closed on
evidence already on main, owned, or re-gated with its reason. As recommended it adds no lane
and no measurement to milestone 1. It proposes one decision entry, eleven amendments and one
result file to check in (section 6), and asks the owner one question (section 5).

A row is cited by its line in `docs/verification.md` at `6f0508a` and by its opening words.
Cost is counted as the milestone plan counts it.

## 1. The decisions

| Line | Opening words | Decision | Gate after |
|---|---|---|---|
| 89 | Float `%` inside the boundary | Decided: a float remainder is `Scalar`'s, and the operator is banned. The lint is re-gated | 4 |
| 98 | `lactose-net-api` is implemented | Milestone 1's part closed. The gather's time and the allocation question re-gated | 4 |
| 99 | `lactose-predict` is implemented | Remainder closed: the estimate at 10,000 statics is retired | 2, the presentation check's |
| 160 | The prediction slot as a copy (D143) | Remainder closed: not owed to criterion 5 | 1, Done |
| 139 | Mount cost (D136) | Closed as the plan closes it. Remainder re-gated | 2 and 3 |
| 140 | Lookups wait behind long steps | Re-gated; nobody owns "splitting long steps" now | 3 |
| 163 | The keeper's timed wait on Windows (D144) | It has its closer: the desktop boots Windows. Its unmeasured list re-gated | 1 and 2 |
| 64 | `lld-link` on Windows | The runner's file is named and checked in | 1 |

Two rows stay at gate 1 because a criterion reads them, and both wait on the owner's desktop:
163 and 64 (section 4). The cost is no lane, or five runs if the owner takes the lint of line
89 now, beside this plan's own review and answer.

## 2. Line 89, "Float `%` inside the boundary"

**Evidence.**

- The route through `math` exists and is proven. `Scalar`'s `Rem` and `Scalar::rem_euclid`
  call the `libm` crate's `fmod` (`crates/math/src/scalar.rs`, `crates/math/src/raw.rs`), and
  `crates/math/README.md` gives the reason: the primitive `%` compiles to a platform library
  call. The golden table holds 361 cases of each
  (`grep -c '^rem|' crates/math/tests/golden/vectors.txt`), and `golden_vectors_match` passes
  on x86-64 Linux and Windows (the row "Cross-architecture agreement of `lactose-math`").
- The route the row names is real on the pinned toolchain. A probe crate outside the tree,
  built by 1.99.0 against glam 0.33.12 with its `libm` and `scalar-math` features, imports
  `_fmod` and `_fmodf` on aarch64-apple-darwin: `f64 % f64` is a tail call of `fmod` and
  `DVec3 % DVec3` is three calls. `x % 2.0` is expanded inline with no call, so what the
  operator compiles to is the compiler's choice. simba's SIMD types take the same route lane by
  lane, as a `zip_map` over the primitive `%` in simba 0.10.2's `auto_simd_impl.rs` and
  `wide_simd_impl.rs`: `AutoSimd<[f64; 2]>`'s `%` is two calls. `lactose-physics-rapier` can
  name them as `rapier3d_f64::parry::simba`, and no `clippy.toml` bans `AutoSimd`. nalgebra
  0.35.0 has no `Rem`.
- No boundary crate uses it. `grep -rnE '[^/%]%[^%]|%=' crates/<crate>/src crates/<crate>/tests`
  over the eleven boundary crates that exist, read hit by hit, finds integer remainders,
  `tracing`'s `%field` sigils and, in `lactose-math`'s tests, `Scalar`'s own operator. The
  hits in `vendor/rapier3d-f64-0.36.0/src` are integer remainders.
- Inside Rapier and parry the platform library's callers are D145's. Its trace interposed
  `fmod` and saw no call from a step in any recorded scene or backend test
  (`docs/plans/physics-evidence-proposal.md`, section 6, last item), and a lane that widens
  what the backend builds owes the trace again.
- A stable lint sees the primitive operator in its plain form and no further. Three probes
  outside the tree ran clippy 0.1.99 with `clippy::modulo_arithmetic` denied: the planner's,
  the first review's and this revision's. `%` and `%=` whose left operand is an `f32` or `f64`
  value are refused, `f64 % 2.0` and `f64 % &f64` among them. A signed integer's `%` is refused
  and an unsigned one passes. The lint is silent on a float `%` whose left operand is a
  reference (`&f64 % f64`, `iter().map(|x| x % m)`), on `Rem::rem` and `rem_assign` called as
  functions, on a generic `T: Rem` used at a float, and on the `%` and `%=` of glam's vectors
  and of simba's `AutoSimd`. Each of those compiles to calls of the platform `fmod`. It is
  silent too on two constants of one sign, which the compiler folds with no call in a release
  and in a debug build.
- Other stable lints close part of that. `disallowed-methods` on `core::ops::Rem::rem` and
  `core::ops::RemAssign::rem_assign` refuses `Rem::rem` and `rem_assign` called or named as
  functions, on any type, and no operator, and no boundary crate calls either today.
  `clippy::float_arithmetic` refuses the reference forms as well. Neither sees the generic,
  glam's or simba's, and no lint tried does.

**Decision.** The row's third option, in the form the tree already has. A float remainder
inside the boundary is `Scalar`'s. `%` and `%=` on `f32` and `f64`, and the `Rem` and
`RemAssign` of glam's vectors and simba's SIMD types, are banned in boundary crates, and
section 8.2 says so. The Euclidean bans keep their reason. The lint is re-gated to milestone 4,
with architecture mode's move to every pull request and beside the Haswell-masked run, which
D139 gated there on the same ground: that is where agreement between platforms becomes a
criterion. Until then a review holds the ban. The lint does not end the review. It holds the
forms it sees, and a float `%` through a reference or a generic, and the operator in the
backend's tests, stay with review after it.

`fmod` is exact, so a breach of the ban changes no bit on a conforming platform library. That
is why the lint can wait. It is not a reason to accept the operator. Accepting it would make
`fmod` the one platform function the boundary trusts, from a library the build does not pin
and a client's operating system supplies, and would leave two bans whose stated reason is the
route just accepted.

**Rules out.** Accepting `%` on `fmod`'s exactness. A measurement of `fmod` on the three CI
targets: under the ban nothing reads its figure. A lane in milestone 1: no criterion's proof
reaches a float `%`.

**What would change it.** A boundary lane before milestone 4 that computes outside `Scalar`
at scale, or a nightly difference traced to a remainder. Either brings the lint forward. The
owner may also take it now (section 5).

## 3. Line 98, "`lactose-net-api` is implemented"

**Evidence.**

- The pin holds: `crates/app/tests/proof/registration.rs` asserts `ProtocolHash::of` over an
  empty game's types equal to 0x1458_0671_8FB1_1F86. The quantiser part is closed in the row
  by D146.
- The spike drives the gather path and checks it. Its packer absorbs `FrameOutput::gathered`
  alone (`crates/app/tests/spike.rs`, `absorb`), and `check_mirror` compares the mirror with
  every replicated row of the server world every 60 ticks.
- Nothing times a gather at `6f0508a`. `git grep -i gather -- docs/results` finds nothing,
  `crates/net-api/tests` holds no ignored cost test, and `cargo xtask measure` has no gather
  measurement (`docs/results/README.md`). The one figure, 0.46 ms on four workers and 0.61 ms
  on one against the design review's 0.8 ms, is the implementation review's at `eb5cf22`,
  before the three ECS hot-path merges (`917e64b`, `b1b8812`, `f75c35b`).
- No criterion or budget names the gather before milestone 4. Section 22's milestone 1 row
  holds none and section 9.3 gives it no figure. Milestone 4 holds the server tick to 16 ms
  with thirty-two clients.
- The allocation the row asks about is one `Vec` for each call of `World::columns_erased`,
  one call for each replicated component, holding one `ErasedColumn` per chunk: 20,640 values
  in 20 vectors at the review's scale, not 20,640 allocations. `LayoutConfig::order` reverses
  or shuffles that `Vec` under SOUR's layout mode (`crates/ecs/src/world.rs`), and
  `GatherState::gather` collects the kept columns into a `Vec` of its own to fork over by
  index (`crates/net-api/src/staging.rs`).

**Decision.** Milestone 1's part is closed, and the rest is re-gated to milestone 4. The
plan's "the spike measures the gather path" is read as what the spike does: it holds the path
to the server's rows. It is not a timing, and the plan's sentence is amended to say so. The
gather is timed when `net`'s packer consumes it and the server tick gives it a budget. The
iterator question is decided from that figure.

**Rules out.** A gather measurement in `cargo xtask measure` now: no decision reads it before
the packer exists. An allocation-free iterator built on the review's figure: the figure is
inside the 0.8 ms it was judged against, and the iterator changes how the layout mode orders
erased columns.

**What would change it.** A steering figure that shows a frame's passes as a visible share of
a server frame before milestone 4.

## 4. The remainders

**Lines 99 and 160, a prediction ring's memory.** Line 99 estimates a ring of sixty slots
beside 10,000 statics at 0.93 GB with Rapier. Section 10 admits at most 64 fixed colliders
and proxies in a prediction world, so no ring holds that scene, and what replaces the bound
is milestone 3's (the row "A prediction world holds at most 64 fixed colliders and proxies
together as rows"). The estimate is retired, not measured. The rings the bound admits are
recorded: result file `spike-net` at `c50b688` carries `<scene>.<clock>.ring.*`, 16.9 MB of
world, backend and record bytes for sixty slots of the thrown scene, 11.1 MB of it the
backend's. Line 160 leaves open "the memory on the criterion's scene". Lane `slot-copy` owed
the memory of a slot counted under the tagged allocator, and
`crates/physics-rapier/tests/memory.rs` counts it on a scene of the criterion's shape without
its character: 433 kB for a copy against 193 kB of bytes. The planner's probe put the
criterion's own scene at 537 kB against 262 kB. Criterion 5 reads time, and no budget reads
either figure before `app` sizes the ring at milestone 4. Both remainders close, and the row
"The frame re-simulation budget has no figure" gains the memory of the ring it sizes. Rules out
a memory measurement in milestone 1.

**Lines 139 and 140, SILO's lock.** Line 139 is closed as the plan closes it: recorded with
the implementation. Its remainder and line 140's are one question: what waits behind a step
that holds the store's lock. At `6f0508a` one crate depends on `lactose-asset`, the cooker:
`grep -n lactose-asset crates/*/Cargo.toml tools/*/Cargo.toml` finds the crate's own name,
`lactose-asset-api` and `tools/cook`'s line. The cooker takes the pack writer from it (D147),
and its sources mount nothing. D136 rules out an `app` dependency on it before milestone 3. The
verification rows place the first mount made beside a reader in milestone 2's watch loop, so
the mount cost on Linux and Windows x86-64 goes to that loop's dev session (the row "Mounted
packs are never unmounted and published versions never unloaded (D136)"). Whether the index's
insert leaves the lock, and whether lookups leave it or long steps are split, go to milestone
3's streaming design (the row "The lookup path's linearisation point (D136)"). D136 states what
a backend that takes lookups off the lock must keep; it does not decide that one is built, so
the coordinator's draft sentence "the milestone 3 backend takes lookups off the lock" is not
used. Rules out an owner for "splitting long steps" in milestone 1.

**Line 163, the keeper's timed wait on Windows.** The row has a closer. The owner answered on
15 November 2026 that the desktop boots Windows and another system (D144, "Driven by"). The
desktop's Windows `whisk` file closes the figure. It is written at the first commit of main
that holds the lane, `9c1e005`, and at D144's follow-up if one is made, and D138's carried
tick-start criterion reads it against 1 ms, or 2 ms after the follow-up. Exit step 6 takes a
miss to the owner. The row's "Not measured" list is owed to no criterion: D144 holds Windows to
1 ms on what the operating system gives and leaves frame pacing's own figure to the first
Windows client. The list is read there, at milestone 2.

**Line 64, the runner's file.** The `measure` workflow's run 43659357193 at `bef3455`, the
measurement commit, kept both runners' files as artifacts: `edit-to-playing-linux-x64` and
`edit-to-playing-windows-x64`, which expire on 13 February 2027. The desktop boots Windows, so
only linux-x64 can need a runner's file, and only if the desktop's other system is not Linux,
which no document records. The coordinator checks in the linux-x64 pair now, as a runner's:
slower median 3.21 s, sketch 0.93 s, on one frame worker. It is criterion 9's file on Linux
unless session B runs there. Rules out waiting for the fact and for the artifact's expiry.

**What the criteria depend on.**

The verdict on each criterion is exit step 5's reviewer's. This table says only what each row
gives a criterion.

| Line | Criterion that reads it | What the row gives it |
|---|---|---|
| 89 | None | No criterion's proof reaches a float `%` |
| 98 | Criterion 7, for the gather path | The mirror check in every recorded and measured run. Nothing reads the gather's time |
| 99, 160 | Criteria 5 and 7 | The result files at `c50b688`. Nothing reads a ring's memory |
| 139, 140 | None | SILO is a delivery of section 22, with no criterion of its own |
| 163 | D138's carried tick start, on Windows at 1 ms, or 2 ms after D144's follow-up | Nothing yet: it waits on the desktop's Windows `whisk` file, and is not re-gated |
| 64 | Criterion 9 | The M1's file. Windows waits on session B. Linux is session B's or the runner's |

## 5. For the owner

One question. The rest is folded into the decision.

```json
{"questions":[
{"header":"Float % lint","question":"A raw `%` on a float inside the determinism boundary calls the operating system's `fmod`, and no lint stops it. The plan bans it in architecture section 8.2 either way. No boundary crate holds one today, and `fmod` is exact, so a breach would change no bit on a conforming system. A lint can hold the operator's plain forms, not every form. When is it built?","multiSelect":false,"options":[
{"label":"At milestone 4 (Recommended)","description":"No lane now. The lint lands with architecture mode's move to every pull request, where agreement between platforms becomes a criterion. A review holds the ban until then, and milestone 1 gains no run."},
{"label":"Now, in milestone 1","description":"Lane `rem-lint`, five runs, in `tools/xtask`, the boundary crates' lint settings and their signed remainders: `clippy::modulo_arithmetic` and two `disallowed-methods` entries in every boundary crate, and a source check for the operator in the physics backend. The row closes at its merge; a float `%` through a reference or a generic, and the operator in the backend's tests, stay with review."}]}]}
```

## 6. Text for the coordinator (exact)

The number is allocated at application. Every OLD below occurs exactly once in its file at
`6f0508a` and every NEW is one line. `planner-exit-rows-check-rev2.py`, kept in the
coordinator's scratch area and not part of the tree, checks that, the cell count and gate of
each verification row, that each row a NEW cites by name exists, that the lint's coverage is
stated in the same words in its three places, that each substitution for the owner's second
answer matches once, and that this document holds each pair verbatim; with `--draft` it applies
the pairs with the other pairs of the coordinator's draft, in both orders. Its output closes
this section. The pairs were written at `10cf89e`. Main has taken the `ricotta` merge (D147)
and the `spike-rhi` merge since. They changed no OLD, and they moved the plan's row from line
88 to line 89. Run the script with `--worktree` on main before applying.

The pairs for lines 99, 139, 140 and 160 replace the pairs the coordinator's draft of the pass
holds for those lines, whose words they keep. The draft's other pairs are untouched.

The owner answered "At milestone 4" on 16 November 2026, and the coordinator applied this section
as D149 on main after D148, with the line 64 result file below. The section on the lint now
is not applied.

### Decision entry

```
**D<N>. Milestone 1's exit rows: a float remainder inside the boundary is `math`'s, and what no milestone 1 criterion reads is gated at the milestone that reads it** (§5.1, §8.2, §9.3, §10, §14, §18, §22, `lactose-math`, `lactose-net-api`, `lactose-predict`, `lactose-asset`, `lactose-task`, `docs/plans/exit-rows.md`)
- Why: the coordinator's verification pass for milestone 1's exit, at `10cf89e`, found two rows
  gated at milestone 1 that no decision settled and six whose remainder no lane, session or
  decision owned. `docs/plans/exit-rows.md` holds the evidence; these are the decisions.
  - *A float remainder inside the boundary is `Scalar`'s `%` or `Scalar::rem_euclid`, which are
    the `libm` crate's `fmod`. The `%` and `%=` operators on `f32` and `f64`, and the `Rem` and
    `RemAssign` of glam's vectors and simba's SIMD types, are banned in boundary crates.* They
    call the platform `fmod`, which is the stated reason of section 8.2's bans on `rem_euclid`
    and `div_euclid` and the reason `lactose-math` takes its remainder from `libm`, under
    golden bits generated on aarch64 that CI holds on x86-64 Linux and Windows. `fmod` is
    exact, so on a conforming platform library a breach changes no bit: the ban keeps the
    boundary off a library the build does not pin, and its gap is a rule that review holds, not
    a known divergence. At `6f0508a` a reading of the boundary crates' sources and tests finds
    no float `%` outside `lactose-math`'s tests of `Scalar`, and the trace of D145 saw no
    `fmod` from a physics step in a recorded scene. Rules out accepting `%` on `fmod`'s
    exactness, which would leave one platform function trusted inside the boundary and two bans
    without their reason; and a measurement of `fmod` on the CI targets, which nothing would
    read.
  - *The lint is owed at milestone 4, with architecture mode's move to every pull request, and
    it holds the forms it sees.* On the pinned 1.99.0 `clippy::modulo_arithmetic` refuses `%`
    and `%=` on a float value, a constant divisor included, and on signed integers, and passes
    unsigned ones and two constants of one sign, which the compiler folds; it does not see a
    float `%` whose left operand is a reference (`&f64 % f64`, which a closure over `iter()`
    computes), `Rem::rem` or `rem_assign` called as a function, a generic `T: Rem` used at a
    float, or the `Rem` and `RemAssign` of glam's vectors and simba's `AutoSimd` (probes
    outside the tree). `disallowed-methods` on `core::ops::Rem::rem` and
    `core::ops::RemAssign::rem_assign` sees the calls, and `lactose-physics-rapier`, the one
    boundary crate that can name glam or simba, gets a check of its `src` for the operator;
    the reference and generic forms, and the operator in that crate's tests, stay with
    review. Until the lints and the check exist a review holds the whole ban. Rules out a lane
    for it in milestone 1, where no criterion's proof reaches a float `%`; and reading the
    lint, once built, as the whole ban's proof.
  - *The replication gather is timed at milestone 4, with `net`'s packer.* Milestone 1 holds
    the gather path by the netcode spike's mirror check and gives its time no criterion and
    no budget. Whether `World::columns_erased` needs an allocation-free iterator is decided
    from the gather's time on the milestone 4 server; the vector it fills is also what the
    layout mode reorders. Rules out a gather measurement in milestone 1, and reading the
    plan's "the spike measures the gather path" as a timing.
  - *A prediction ring's memory is read when `app` sizes the ring, at milestone 4.* The
    estimate at 10,000 statics is retired: section 10 admits 64 fixed colliders and proxies,
    and what replaces that bound is milestone 3's. A slot's memory on the criterion's scene
    is not owed to the prediction-snapshot criterion, which reads time. Rules out both
    measurements in milestone 1.
  - *SILO's remainders go where a reader first waits.* The mount cost on Linux and Windows
    x86-64 goes to milestone 2's watch loop, the first mount made beside a reader. Whether
    the index's insert leaves the lock, and whether lookups leave it or long steps are split,
    go to milestone 3's streaming design, since `app` takes no dependency on `lactose-asset`
    before it (D136). A lane that calls a lookup from a frame-critical worker sooner owes
    that decision first.
  - *The Windows keeper's unmeasured list is milestone 2's*, read where the first Windows
    client's frame pacing is decided (D144). The desktop boots Windows, so the row keeps its
    closer and its gate: the desktop's Windows `whisk` file at the first commit of main that
    holds the lane and at D144's follow-up if one is made, read against 1 ms, or 2 ms after the
    follow-up.
  - *The linux-x64 runner's edit-to-playing file is checked in as a runner's*, from the
    `measure` workflow's run at the measurement commit. It is the edit-to-playing criterion's
    file on Linux unless session B runs there (`docs/plans/milestone-1.md`, decision 5.9).
    The desktop boots Windows, so no Windows runner's file is owed.
- Driven by: exit step 4 of `docs/plans/milestone-1.md` and the coordinator's pass for it;
  the review of lane `xtask-policy`, which found the `%` route; the planner's reading of the
  boundary crates' sources and its clippy probe on the pinned toolchain; the plan's first
  review, which found the forms `clippy::modulo_arithmetic` does not see and simba's remainder;
  the owner's answer of <date>, which left the lint at milestone 4.
- Replaced: D119's addendum's "`%` is an operator no lint sees", which holds for
  `disallowed-methods` only (the `clippy.toml` and `crate-tiers.toml` comments that repeat it
  change with the lint's lane); in `docs/plans/milestone-1.md`, section 2's "The spike measures
  the gather path". In section 8.2 nothing: its bans stand and gain one.
```

### Architecture amendment

`docs/architecture.md`, section 8.2, the first bullet (line 561).

OLD:

~~~
Transcendentals come from the `libm` crate. `math` owns
~~~

NEW:

~~~
Transcendentals come from the `libm` crate, and so does a float remainder, as `Scalar`'s `%` and `rem_euclid`: the primitive `%` and `%=` on `f32` and `f64`, and the `Rem` and `RemAssign` of glam's vectors and simba's SIMD types, call the platform `fmod` and are banned inside the boundary, by review until a lint holds them (D<N>). `math` owns
~~~

### Plan amendment

`docs/plans/milestone-1.md`, section 2, the row for line 93 (line 89 of the plan at `6f0508a`).

OLD:

~~~
The spike measures the gather path.
~~~

NEW:

~~~
The spike holds the gather path by its mirror check; a gather's time and the `columns_erased` allocation are re-gated to milestone 4 (D<N>).
~~~

### Verification rows

Line 64, "`lld-link` on Windows". OLD:

~~~
a runner's file is checked in for an operating system the desktop does not boot (`docs/plans/milestone-1.md`, decision 5.9) | 1 |
~~~

NEW:

~~~
a runner's file is checked in for an operating system the desktop does not boot (`docs/plans/milestone-1.md`, decision 5.9). The desktop boots Windows and another system (D144), so only linux-x64 can need one. That run's linux-x64 file is checked in as a runner's, `docs/results/edit-to-playing-github-runner-linux-x64-2026-11-15.md`: it is criterion 9's on Linux unless session B runs there, and then session B's file is (D<N>) | 1 |
~~~

Line 89, "Float `%` inside the boundary". OLD:

~~~
or route `%` through `math` and refuse it in boundary crates | 1 |
~~~

NEW:

~~~
or route `%` through `math` and refuse it in boundary crates. **Decided (D<N>): the third, and its first half is on main.** A float remainder inside the boundary is `Scalar`'s `%` or `Scalar::rem_euclid`, the `libm` crate's `fmod` (`crates/math/src/scalar.rs`), with 361 golden cases each that CI holds on linux-x64 and windows-x64 (the row "Cross-architecture agreement of `lactose-math`"). `%` and `%=` on `f32` and `f64` and the `Rem` and `RemAssign` of glam's vectors and simba's SIMD types are banned in boundary crates (architecture section 8.2), so the Euclidean bans keep their reason and nothing is measured on the CI targets. At `6f0508a` every `%` in the boundary crates' sources and tests is an integer remainder, a `tracing` sigil or `Scalar`'s own, the vendored Rapier's are integer remainders, and the trace of D145 saw no `fmod` from a step. Owed with architecture mode's move to every pull request: `clippy::modulo_arithmetic` denied in every boundary crate and held there by `cargo xtask boundary`, with fixtures in `tools/xtask/tests/boundary_lints.rs`. On the pinned 1.99.0 that lint refuses `%` and `%=` on a float value, a constant divisor included, and on signed integers, and passes unsigned ones and two constants of one sign, which the compiler folds; it does not see a float `%` whose left operand is a reference (`&f64 % f64`, which a closure over `iter()` computes), `Rem::rem` or `rem_assign` called as a function, a generic `T: Rem` used at a float, or the `Rem` and `RemAssign` of glam's vectors and simba's `AutoSimd` (probes outside the tree). `disallowed-methods` on `core::ops::Rem::rem` and `core::ops::RemAssign::rem_assign` sees the calls, and `lactose-physics-rapier`, the one boundary crate that can name glam or simba, gets a check of its `src` for the operator; the reference and generic forms, and the operator in that crate's tests, stay with review. A review holds the whole ban until then | 4 |
~~~

Line 98, "`lactose-net-api` is implemented". OLD:

~~~
claim changes and the join's full gather). | 1 |
~~~

NEW:

~~~
claim changes and the join's full gather). Milestone 1's part is **Done** (lanes `sour-evidence` and `spike-net`; D146): the hash is pinned, the quantiser follows the 64-bit scalar, and the spike holds the gather path by that mirror check. The spike does not time a gather, and no criterion or budget of milestone 1 reads its time; the figure above was taken at `eb5cf22`, before the ECS hot-path lanes (D137), and steers. Re-gated (D<N>): with `net`'s packer, time the gather on milestone 4's thirty-two-client server, whose tick section 22 holds to 16 ms, and decide from it whether `columns_erased` needs an allocation-free iterator. `World::columns_erased` collects one `ErasedColumn` per chunk into one `Vec` per component, which the layout mode reverses or shuffles (`LayoutConfig::order`), and the gather collects the kept columns into a `Vec` of its own to fork over, so an iterator that allocates nothing changes both | 4 |
~~~

Line 99, "`lactose-predict` is implemented (D122 addendum)". OLD:

~~~
figure". | 1 and 2 |
~~~

NEW:

~~~
figure". Milestone 1's part is **Done** (lanes `spike-net` and `measure`): the slot with Rapier is measured (result file `prediction-slot` at `c50b688`) and the absorbed figures are recorded (result file `spike-net` at `c50b688`, `<scene>.<clock>.absorbed.*`). The ring's memory at 10,000 statics is not measured and is not owed (D<N>): a prediction world holds at most 64 fixed colliders and proxies (architecture section 10), so no ring holds that scene, and what replaces the bound is milestone 3's (the row "A prediction world holds at most 64 fixed colliders and proxies together as rows"). The rings the bound admits are recorded as the bytes their slots would write (`<scene>.<clock>.ring.*`: 16.9 MB for sixty slots of the thrown scene, 11.1 MB of it the backend's), and what a ring holds as copies is read when `app` sizes it (the row "The frame re-simulation budget has no figure"). The presentation check is what remains | 2 |
~~~

Line 139, "Mount cost (D136)". OLD:

~~~
Still open: the same measurement on Linux and Windows x86-64, and whether the index's insert cost should leave the lock | 1 |
~~~

NEW:

~~~
**Done** (lane `silo`, merge `71f6296`), as the milestone 1 plan closes it: recorded with the implementation. Re-gated (D<N>), since no binary mounts a pack beside a reader before milestone 2's watch loop: the same measurement on Linux and Windows x86-64 is taken with that loop's dev session (the row "Mounted packs are never unmounted and published versions never unloaded (D136)"), and whether the index's insert cost should leave the lock is decided with the lookups' place on it (the row "The lookup path's linearisation point (D136)") | 2 and 3 |
~~~

Line 140, "Lookups wait behind long steps (D136, review C6)". OLD:

~~~
change | 1 and 3 |
~~~

NEW:

~~~
change. Milestone 1's part is **Done**: the waits are measured, above. The rest is milestone 3's (D<N>): `app` and its binaries depend on `lactose-asset-api` alone, and `app` takes no dependency on `lactose-asset` before milestone 3 (D136), so no driver calls a lookup on a frame thread until then, and the streaming design decides, from the lock's contention on its scenes, between taking lookups off the lock and splitting long steps (the row "The lookup path's linearisation point (D136)"). A lane that calls a lookup from a frame-critical worker sooner owes that decision first | 3 |
~~~

Line 160, "The prediction slot as a copy (D143)". OLD:

~~~
Open: the memory on the criterion's scene, and SOUR's architecture mode over the spike's records (not runnable on aarch64) | 1 |
~~~

NEW:

~~~
Closed (D<N>): the memory on the criterion's scene is not owed to milestone 1. Criterion 5 reads a slot's time, the lane's test counts a copy on a scene of the criterion's shape without its character (433 kB against 193 kB of bytes), the planner's probe put the criterion's scene at 537 kB against 262 kB, and no budget reads either before `app` sizes the ring (the row "The frame re-simulation budget has no figure"). SOUR's architecture mode over the spike's records, which aarch64 cannot run, passed in nightly 43703297881 at `c50b688`, the first nightly to hold the lane | 1 |
~~~

Line 163, "The keeper's timed wait on Windows (D144)". OLD:

~~~
says Windows 11 does not guarantee | 1 |
~~~

NEW:

~~~
says Windows 11 does not guarantee. The desktop boots Windows and another system (the owner's answer of 15 November 2026, D144), so the row has its closer: the desktop's Windows `whisk` file at the first commit of main that holds the lane, `9c1e005`, and at D144's follow-up if one is made, which D138's carried tick-start criterion reads against 1 ms, or 2 ms after the follow-up (D144). The list above is owed to no criterion (D<N>): it is read at milestone 2, where the first Windows client's frame pacing is decided (D144) | 1 (the desktop's file) and 2 (the list) |
~~~

Line 167, "The frame re-simulation budget has no figure". OLD:

~~~
record on the quiet M1 the smallest depth the client derives over a ten-minute thrown-scene replay,
~~~

NEW:

~~~
record on the quiet M1 the smallest depth the client derives over a ten-minute thrown-scene replay, and with it the memory a ring of that depth holds as copies, counted under the tagged allocator as `crates/physics-rapier/tests/memory.rs` counts one slot (the row "The prediction slot as a copy (D143)"),
~~~

### The result file

Line 64's NEW names a file this plan does not carry. The coordinator adds
`edit-to-playing-github-runner-linux-x64-2026-11-15.md` and its `.json` to `docs/results` in
the commit that applies the row, from the artifact:

```
gh run download 43659357193 -n edit-to-playing-linux-x64
```

Both files name the commit `bef3455b4a56` and the machine `github-runner-linux-x64`, and hold
no path outside the tree.

### If the owner takes the lint now

Three substitutions, and the lane of section 7. In the decision entry the second bullet
becomes:

```
  - *The lint lands in milestone 1, lane `rem-lint`, and it holds the forms it sees.* On the
    pinned 1.99.0 `clippy::modulo_arithmetic` refuses `%` and `%=` on a float value, a constant
    divisor included, and on signed integers, and passes unsigned ones and two constants of one
    sign, which the compiler folds; it does not see a float `%` whose left operand is a
    reference (`&f64 % f64`, which a closure over `iter()` computes), `Rem::rem` or
    `rem_assign` called as a function, a generic `T: Rem` used at a float, or the `Rem` and
    `RemAssign` of glam's vectors and simba's `AutoSimd` (probes outside the tree).
    `disallowed-methods` on `core::ops::Rem::rem` and `core::ops::RemAssign::rem_assign` sees
    the calls, and `lactose-physics-rapier`, the one boundary crate that can name glam or
    simba, gets a check of its `src` for the operator; the reference and generic forms, and
    the operator in that crate's tests, stay with review. The lane denies the lint in every
    boundary crate, holds it there with `cargo xtask boundary`, and adds the two entries and
    the check. Rules out leaving the whole ban to review until milestone 4; and reading the
    lint as the whole ban's proof.
```

Its "Driven by" ends "which took the lint in milestone 1". In line 89's NEW, "Owed with
architecture mode's move to every pull request:" becomes "Owed by lane `rem-lint`:", "A review
holds the whole ban until then" becomes "A review holds the whole ban until its merge", and the
gate stays 1: the NEW ends `| 1 |`. The lane's merge marks the row Done and narrows section
8.2's "by review until a lint holds them" to the forms its fixtures show unseen and the
backend's tests.

### The check

Run from the root of a checkout that holds `6f0508a`, with this document in its working tree
and the coordinator's draft of the pass as `<draft>`:

```
python3 -I planner-exit-rows-check-rev2.py --doc docs/plans/exit-rows.md --draft <draft>
```

~~~
docs/verification.md at 6f0508a: 167 lines
  row 64, lld-link and edit-to-playing: line 64, OLD 129 chars, count 1, gate '1' -> '1': ok
  row 89, float %: line 89, OLD 66 chars, count 1, gate '1' -> '4', cites 1: ok
  row 98, net-api: line 98, OLD 48 chars, count 1, gate '1' -> '4': ok
  row 99, predict: line 99, OLD 20 chars, count 1, gate '1 and 2' -> '2', cites 2: ok
  row 139, mount cost: line 139, OLD 125 chars, count 1, gate '1' -> '2 and 3', cites 2: ok
  row 140, lookups behind long steps: line 140, OLD 18 chars, count 1, gate '1 and 3' -> '3', cites 1: ok
  row 160, the slot as a copy: line 160, OLD 128 chars, count 1, gate '1' -> '1', cites 1: ok
  row 163, the keeper's timed wait on Windows: line 163, OLD 40 chars, count 1, gate '1' -> "1 (the desktop's file) and 2 (the list)": ok
  row 167, the frame re-simulation budget: line 167, OLD 99 chars, count 1, gate '4' -> '4', cites 1: ok
docs/architecture.md at 6f0508a: 1433 lines
  section 8.2, the float bullet: line 561, OLD 55 chars, count 1: ok
docs/plans/milestone-1.md at 6f0508a: 815 lines
  the plan's section 2, row 93: line 89, OLD 35 chars, count 1: ok
docs/verification.md: 167 lines after the edits, 167 before
docs/architecture.md: 1433 lines after the edits, 1433 before
docs/plans/milestone-1.md: 815 lines after the edits, 815 before
the owner's second answer:
  row 89, float %: "Owed with architecture mode's move to every pull request:": ok
  row 89, float %: 'A review holds the whole ban until then': ok
  row 89, float %: gate '4' -> '1': ok
the lint's coverage: 791 chars in the NEW of row 89, float %
the document states the lint's coverage in the same words in 3 places, 3 wanted
the owner question parses: 1 question, 2 options, the first 'At milestone 4 (Recommended)'
placeholders: the decision's number 12 times, the date of the owner's answer 1 time, others none
the document holds every OLD and NEW verbatim
the coordinator's draft: 14 pairs, 10 kept (rows 17, 20, 73, 75, 90, 116, 123, 124, 141, 132), 4 replaced by this document's
  this document's, then the draft's: 0 failing, 167 lines
  the draft's, then this document's: 0 failing, 167 lines
11 pairs, 0 failing
~~~

## 7. The lane, if the owner takes the lint now

`rem-lint`, five runs: the lane, its review, one fix round with its diff review, and the
merge. It edits `tools/xtask`, the boundary crates' lint settings and each boundary file with a
signed remainder the lint refuses, so it runs when no other lane edits `tools/xtask/src` or
those files. The same text is what milestone 4 owes if the lint waits.

- Deny `clippy::modulo_arithmetic` in every crate of `[boundary]`, by whichever of a
  crate-level attribute and a `[lints]` table the lane shows `cargo xtask boundary` can
  require of each crate and refuse a crate-level `allow` of. `lactose-math` needs no
  exemption: `Scalar`'s remainder calls `libm`.
- Add `disallowed-methods` entries for `core::ops::Rem::rem` and
  `core::ops::RemAssign::rem_assign` to the reference `clippy.toml`, its copies and the lists
  in `tools/xtask` that hold them. They refuse the trait's method called or named as a
  function, on any type. No boundary crate calls either at `6f0508a`.
- Rewrite or `#[expect]` each signed-integer `%` the lint then refuses in boundary crates'
  sources and tests, and report the count.
- For the `Rem` and `RemAssign` of glam's vectors and simba's SIMD types: a check in
  `cargo xtask boundary` that `lactose-physics-rapier`'s `src` holds no `%` or `%=` outside a
  `tracing` macro's field sigils, or a lint the lane shows to see it; the crate's tests stay
  with review. At `6f0508a` its `src` holds eight `%`, all sigils. Its tests hold integer
  remainders, which no untyped check can tell from a vector's.
- The lane may also measure `clippy::float_arithmetic`, which states section 8.2's "Floating
  point only through `math`" directly and refuses the reference forms in the probes. It reports
  what that lint refuses in boundary crates and denies nothing on it without a decision.
- Update the comments that say no lint sees the operator: the reference `clippy.toml` and its
  copies, and `crate-tiers.toml`. Narrow section 8.2's "by review until a lint holds them" to
  the forms the fixtures show unseen and the backend's tests. D119's addendum is not edited:
  D<N>'s "Replaced" names its sentence.

Proof: in `tools/xtask/tests/boundary_lints.rs`, a fixture for each of `f32 % f32`,
`f64 % f64`, `f64 %= f64`, `f64 % &f64`, a constant divisor, and `Rem::rem(a, b)`,
`a.rem(b)` and `a.rem_assign(b)` on `f64` is refused, and an unsigned remainder passes. A
fixture for each of `&f64 % f64`, `&f64 % &f64` and a generic `T: Rem` used at `f64` passes
and is asserted to pass: they are the forms left with review, and
a toolchain that starts to refuse one fails the test and shortens that list.
`cargo xtask boundary` fails when the lint or either entry is dropped from one boundary crate
and when the backend's `src` holds `DVec3 % DVec3`, and passes over its `tracing` sigils.
Clippy passes over the workspace on the three targets.

## 8. What this document could not verify

- The scan for a float `%` is a reading of grep hits over the boundary crates' `src` and
  `tests`, not a compiler's classification. It was read at `10cf89e`, and `git diff` shows
  those crates and the vendored Rapier unchanged at `6f0508a`. The lint is the proof for the
  forms it sees. For a float `%` through a reference or a generic there is no proof but the
  reading.
- The clippy probes ran on aarch64-apple-darwin, on crates outside the workspace. Clippy was
  not run with either lint over the workspace, so the signed-integer remainders
  `clippy::modulo_arithmetic` would refuse in boundary crates, and what
  `clippy::float_arithmetic` would refuse there, are not counted. At least
  `crates/sim/tests/cost/scene.rs` holds some of the first.
- What each form compiles to was read in a release build on aarch64-apple-darwin alone. That
  the compiler folds two constants is from that build and a debug build there. Code generation
  on x86-64 Linux and Windows was not read.
- simba's wide types were read in the source and not probed; the probe built
  `AutoSimd<[f64; 2]>`.
- `fmod`'s exactness on glibc, Apple's library and the Windows runtime is the C standard's
  requirement, not a measurement. The ban does not rest on it. The lint's wait until
  milestone 4 does, beside the scan above.
- Parry's, nalgebra's and simba's own `%` sites were not classified; a text match finds 133
  lines with a `%` in parry 0.31.1. D145's trace is the evidence that no recorded scene
  reaches `fmod`, and it ran on aarch64 alone.
- No gather was timed at `6f0508a`. The figure in line 98 is the review's at `eb5cf22`.
- Which system the desktop boots beside Windows is in no document, and no desktop file is
  under `docs/results` at `6f0508a`.
- The runner's files were read from the workflow's artifact, not produced again.
- CI and nightly states are the coordinator's reading; this document read one run's artifact
  list and no log.
- Every pair is checked against `6f0508a` and applied in memory alone. No pair was applied to a
  file.
