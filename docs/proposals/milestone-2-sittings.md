# Proposal: the sittings that close milestone 2

Status: decided. The owner ruled on its five questions, on the order of the sittings, on 26 November
2026 on the order of the three runs before the commit and on the work desktop's rehearsal, and on 28
November 2026 on a commit that moves and on who dispatches the workflows at the commit, and this is
the proposal as ruled, for the coordinator to apply as D165. It is the tenth version. The fifth
answered the fourth's diff review, which found one serious defect, in a commands file and not in
this text, five minor ones and five nits. The sixth narrowed one rule of the fifth's, when the pin
runs again after a fix round, and took back a claim about `record-stall`'s rows that a lane's review
showed false. The seventh answered the diff review of those two, which found one serious defect,
three minor ones and two nits: the sixth's rule contradicted itself, and the coordinator replaced it
with a simpler one, that the pin passes at the commit's code. The eighth answered the seventh's diff
review, which found one serious defect, two minor ones and two nits: the seventh's commit was "the
first commit of main that holds" the lanes, which left a state with no next step once the pass had
to be at the commit's code, and the commit became the head of main at which the three runs are last.
The ninth answered the eighth's diff review, which found no serious defect, five minor ones and two
nits: the pin passes at the commit itself, a push made against the hold moves the commit, the head
is chosen after the early runs' last report, and at a moved commit the laptop's record follows the
M1's hour there. The tenth answers the ninth's diff review, which found one minor defect and two
nits: a moved commit's states now cover a defect that the laptop's record shows and one that only
session C's driver shows, and the rehearsal's complement says when it can arise. The answers to each
review are near the end. It answers the exit pass's audit of main at `ad1a99d` (26 November 2026):
what stands between main and milestone 2's exit (`docs/plans/milestone-2.md`, section 7), and in
what order it is done. It settles four things: the measurement commit, the order of the sittings and
what each asks of the owner, the Khronos layer's pin on Windows, and the stamp verification row 21's
Windows item waits for. The rulings are near the end, where the questions were, and after them what
the reviews changed. It amends no sentence of `docs/architecture.md`. It is applied as one decision
entry, the plan's and the verification rows' text below, and two lanes, of which `park-stamp` is
running.

This proposal ran no measurement and no device. It read the tree, the audit, `reviewer-gpu`'s
reading for row 34, the two reviews with their probes and the diff review of the second version. It
ran two probes of its own: the Windows layer's digest, read on the development Mac out of LunarG's
installer, which was fetched and never run; and a detached power assertion, to see that it outlives
the shell that starts it. For the seventh version it searched main at `1c62676` for code that reads
a document, tried the quiet window's command of that version, which the ninth drops, on commits of
this branch and of main, and read lane `park-stamp`'s fix round on its branch at `6bdea44`. For the
eighth it read the `ci` and `measure` workflows at `1c62676` for what a push and a dispatch do to a
run, the list of measurements in `tools/xtask/src/measure.rs` for which of them need a device, and
the lane's README at its branch's head `35c2120` for a listener's rule. For the ninth it read that
README again at the lane's commit `19f1b1b`, and the laptop's commands for where its record falls at
a moved commit. For the tenth it read the laptop's commands again for the parts that follow its
short run, and the work desktop's for when the pin run's first message goes out. The work desktop's
facts are its own agent session's inventory, as the coordinator relayed it, and this proposal
checked none of them. The rows below are built against main at `1c62676`, which is `b0c5f44` with
lane `present-encoder` merged.

## What stands between main and the exit

| What section 7 asks | State at `1c62676` |
|---|---|
| Step 1: a measurement commit, and the quiet window, the laptop's session and session C on it | No commit is named. No result file of milestone 2 is on main |
| Step 1: the desktop's first sitting | Not run. Its commands are written, at `a539d60` (D151, decision 5.19), and are revised with this proposal to ask for what the later sittings read from it. The desktop is out of reach until 7 December 2026 |
| Step 3: `gpu (lavapipe)` and `shaders` green, no check job's median past ten minutes | Met. Lane `exit-pass-rows` wrote the run ids into the rows |
| Step 3: a nightly that holds the client's recording test | Not run. The last nightly, 45609400690 at `98d11cf`, is older than the test, which merged with `frame-impl` at `55ffdda` |
| Step 4: section 2's 26 rows Done or re-gated | Five were the pass's to write (35, 95, 99, 134, 170): `exit-pass-rows` merged at `b0c5f44`, and 170 takes this proposal's block. Three the laptop's (63, 109, 150). Six the first sitting's (64, 77, 80, 81, 164, 169). Six the measurement commit's files (15, 16, 19, 21, 34, 102). One the freeze decision's (18). Five re-gated already (97, 136, 139, 140, 143) |
| Criterion 1 on Windows | Cannot run. `tools/xtask/vulkan-layer.toml` pins no layer for Windows, and `triangle` and `cargo xtask gpu` both stop at `vulkan::host` |
| Row 21's Windows item | Cannot be measured. Both of its instants are inside `lactose-task` (D162's addendum of `measure-m2`, round two). Since the ruling, lane `park-stamp` runs from `b0c5f44`, and its two spans are on its branch at `a5ce394` |
| Lane `present-encoder` | Merged at `1c62676`. It edited the client's present call, which `triangle` runs through and `frame-pace` reads, and added `cargo xtask gpu --client` |
| The owner's Windows machines | Out of reach until 7 December 2026, the laptop and the desktop both. A work desktop is in reach from 26 November, through its own agent session, for steering alone |

**What the audit did not read.** Six things, found while placing the sittings and in the reviews.

- *Section 2 lists 26 rows, and 85 rows of `docs/verification.md` are gated at milestone 2.* The 26
  are those gated at `a561b36`, and 21 of them still are. The other 64 were opened by the plan and
  its lanes. Exit step 4 names section 2's rows alone, and the audit read those. By their own text
  27 of the 64 owe a run on the laptop or the desktop: 176, 189, 190, 208 to 210, 228, 229, 234,
  236, 243, 246 and 247 the laptop's; 212, 214, 216, 218 to 220, 222, 224, 225, 238 to 241 and 244
  the desktop's. This proposal places those. The other 37 it read by their heads and status marks
  alone, and none against the tree.
- *No line of `lactose-rhi-vulkan` has run on a Windows driver.* Rows 210 and 236 say of Win32's
  surface "not yet run anywhere", and `cargo xtask gpu` stops on Windows for want of the pin. The
  first run there is the step of this plan most likely to find a defect of the engine, and such a
  defect's fix changes what every device measurement reads.
- *Nothing writes `rhi-conformance`.* Section 4 says `cargo xtask gpu --driver <name>` writes
  `rhi-conformance-<driver>-<machine>-<date>.md`, and criterion 2's proof and row 171 name the file.
  The command writes `target/gpu/<driver>-vulkan[-gpu-assisted].txt` (`tools/xtask/src/main.rs`,
  `run_vulkan`). Its lines hold the SDK's and the environment's paths as the machine has them, and
  under `--driver system` a second adapter's run overwrites the first's.
- *Session C as the plan has it is one command.* The rows owe the desktop more than `cargo xtask
  measure --all`: the device tests on a desktop's driver (rows 212, 214, 216, 218, 219, 222, 224 and
  244, two of them on a device whose present family is not its graphics family), the renderer's cost
  tests (rows 220 and 238) and the three barrier mutations (rows 225 and 239 to 241). The mutation
  script is in no checkout: lane `renderer-impl` left it outside the repository. Its verdict reads
  the gate's console output for a line the gate has not printed since `7424e7c`, so every mutation
  would read as caught. Row 244's probe is a build of a scratch copy, in no checkout either.
- *Nothing skips a window on a system that is not a client platform.* Section 4 says the
  measurements that need a window are skipped there and the file says so. `cargo xtask measure` has
  no such rule, and on Linux `lactose-platform` "builds over X11 and is not run" (its README,
  "Status").
- *Row 243 names a selection the tool does not make.* It has the laptop's GPUs "chosen by
  `VK_LOADER_DRIVERS_SELECT`". A run clears every `VK_LOADER_*` variable
  (`tools/xtask/src/vulkan.rs`, `CLEARED_PREFIXES`) and chooses the device by `--adapter`, whose
  name the device's name must contain (`crates/rhi-vulkan/README.md`, `VulkanDevice::new`).

## The measurement commit

**The decision.** The measurement commit is the head of main at which `check`, the `measure`
workflow and the Windows pin run last, once main holds the three lanes that change a measured input
after `measure-role`, which are `present-encoder`, `park-stamp` and `layer-windows`, and the fix
round of each defect the commit waits for. Whatever else main holds when that head is chosen is in
the commit. The coordinator records its hash in a verification row of its own, which this proposal
adds, once three things hold: `check` is green at it, the `measure` workflow is green at it, and the
work desktop's run of the Windows pin on its RX 9060 XT has passed at it. Five rules hold the three
together. Three parts of them are the owner's rulings, of 26 and 28 November 2026: the order in the
second, the fourth's first three sentences with its sentence on the cost, and the fifth down to the
work desktop. The fourth and the fifth each say where the coordinator's part starts. The rest is the
coordinator's rule, as the diff reviews of the seventh and eighth versions corrected it.

1. *The pin passes at the commit itself.* The pin's report names the commit it ran at, and the
   coordinator compares that hash with the head `check` and `measure` are green at. Where the two
   are one, the hash is recorded. A pass at another commit does not count. The eighth version let a
   pass stand at a commit of main that differed from the commit in documents alone, and gave a
   command to check it. Its diff review showed that no state uses that allowance once pushes are
   held. The allowance, the command and the term "the commit's code" are dropped.
2. *The cheap runs come first, and main is held from the first of them.* The order is the owner's
   ruling: the lanes the commit holds merge first, `check` and `measure` run at that head, and the
   pin runs there last, so that the owner's remote session is spent once where it can be. The hold
   is the coordinator's step. The ruling has only documents merge "from the pin's run" until the
   commit is recorded. The hold starts sooner, at `check`, and covers pushes: it forbids more than
   the ruling does, in the ruling's own direction. The coordinator chooses the head only once the
   work desktop's early runs have sent their last report, so that no defect of theirs is still to
   come. The hold starts when `check` starts at that head: at the push of the head, or at the choice
   where the head is pushed already. It ends when the hash is recorded, or sooner by the third rule.
   While it stands nothing merges to main but documents, which are the files under `docs/` and the
   `*.md` files, and nothing is pushed to main, a document neither: a document merged meanwhile
   waits in the main checkout and is pushed when the hold ends. A push made against the hold moves
   the commit to the new head, and the hold stands there: `check` runs there by the push, the
   coordinator dispatches `measure` there, and the pin runs there last, again where it had passed at
   the old head. A run that is cut has not failed: a job of `check` or of `measure` that lost its
   runner, or a block of the pin's that a control of the work desktop's or a dropped remote session
   cut. It runs again at the same head, and the hold stands.
3. *The commit waits for a defect that an early run of the work desktop's finds, or that `check`,
   `measure` or the pin fails on.* It is a fix round of the lane whose code it is, whichever crate
   that is. A run that fails ends the hold. So does a defect the commit waits for that is reported
   while the hold stands, as an early run's reported late: no hash is recorded on the runs under
   way. Once the fix round has merged the coordinator chooses main's head again, and the three run
   there in the same order, so the pass is always at the commit that is recorded.
4. *The commit does not wait for the rehearsal.* This is the owner's ruling. A defect that only the
   work desktop's rehearsal finds, in its runs after the pin's own, and that fails none of the pin's
   four conditions does not hold the commit. The owner accepted the rule's cost: where such a defect
   touches a measured input the laptop's session meets it again, and a file written after its fix is
   steering until the owner rules which is the record (section 7, step 7). The rest of this rule is
   the coordinator's. Session C meets such a defect as the laptop's session does. The fix round
   merges outside the hold: before the head is chosen, or after the hash is recorded, as a lane that
   merges after the commit does, which runs the measurements it touches as steering. It never merges
   while the hold stands. Once it has merged, the part of the rehearsal that found the defect runs
   again on the work desktop while the machine is lent, as steering: the rehearsal is there so that
   the laptop's one session does not meet the defect first. A defect of the rehearsal's that fails
   one of the four, as a message that a measured run of `triangle`, `gpu-cost`, `frame-pace` or
   `rhi-conformance` hears on the RX 9060 XT in the rehearsal's full run, is the pin's failure where
   it is found before the hash is recorded. The coordinator records the hash on the first message of
   the pin run's report, which goes out when the pin's own part ends and before the rehearsal
   starts. So this case covers only a defect reported before that recording is made: while the
   coordinator reads the message, or while a failed case waits for the owner's ruling. It stays for
   that case: the coordinator relays that the owner's ruling of 26 November was framed beside it.
   Found after, it is a defect found after the commit, and the rule for a file of a later commit
   holds.
5. *A commit that moves after 7 December is recorded with the laptop standing for the pin.* This is
   the owner's ruling of 28 November 2026. A commit that a defect only NVIDIA's or Intel's driver
   shows has moved is recorded once `check`, `measure` and the laptop's own gate and short run have
   passed at the moved head. Only documents merge from `check` there until it is recorded. No
   further remote session of the work desktop's is needed: it may no longer be lent by then. The
   rest of this rule is the coordinator's. The first four rules hold at the moved head with the
   laptop's gate and short run in the pin's place: the hold covers pushes, and a failure of one of
   the three ends it. And the laptop's record at the moved commit comes after the M1's hour has run
   again there, or after the owner has ruled that the M1's files stand, which is the order's ruling
   read for a moved commit. Where the laptop's record had begun when the defect showed, its files so
   far are steering, since they are not at the commit, and its sitting at the moved head runs the
   record whole. A defect that only session C's driver shows moves the commit as the laptop's does.
   Whether session C then sits again at the moved head or its files stand is the owner's to rule
   when it arises, and it is not ruled: the owner's desk time comes last and once, and no rule here
   spends it twice.

**Why documents may merge, and why none is pushed.** The coordinator's own work between the pass and
the recording is documents: this decision's rows, a review's answers, a result file. They merge into
the main checkout and wait there, so main's head on the remote stays the head the three runs are at.
The quiet window checks the commit out by its hash, so a document pushed after the hold reaches no
file's Commit row. Three things of the workflows are why the hold covers pushes, a document's too.
The `ci` workflow, which holds `check`, runs on every push (`.github/workflows/ci.yml`), so a push
starts `check` at a new head. A push to main also cancels a `ci` run still in progress on main (its
`concurrency` group). And `measure` runs by dispatch alone, at the head of the branch it is
dispatched on (`.github/workflows/measure.yml`). After a push against the hold, then, `check` has
run last at a head where `measure` and the pin have not run. The eighth version gave that state two
next steps. One of them, running the cancelled run again, would have cancelled the new head's run in
turn, as its diff review showed. This version gives it one: the commit is the new head, and the
three run there. That costs a second run of the pin where the pin had passed, which is the price of
a push the hold forbids. The seventh version searched main for a document that the pin's run reads,
to show that a pass at a commit that differs in documents is a pass at the same code. Nothing rests
on that search now.

**Every state has a next step.** The seventh version's diff review reached a state its texts gave no
way out of, and the eighth's found one state with two next steps and one with none. The ninth's
named two the table lacked. These are the states between the lanes' merges and the last sitting,
each with the one thing that is done next.

| State | Next step |
|---|---|
| Main holds the three lanes and the fix round of each defect the commit waits for, and the work desktop's early runs have not sent their last report | No head is chosen. The coordinator waits for that report, and a defect of a measured input in it is a fix round before the head is chosen |
| The same, with the early runs' last report read, and no hold stands | A rehearsal's fix that is ready merges. The coordinator then chooses main's head, pushes it and holds main. `check` runs there by the push, and the coordinator dispatches `measure` on main, and the nightly with it if the schedule did not run on that head |
| `check` or `measure` fails at the head | The hold ends. What failed is fixed where it is: by a fix round of the lane whose code it is, or by the coordinator's own edit where a document fails a lint. The head is then chosen again |
| `check` and `measure` are green at the head | The pin's brief goes to the work desktop with the head's hash. The owner approves the SDK's install where it is not there yet, and keeps the remote session open |
| A run is cut and has not failed: a job of `check` or of `measure` that lost its runner, or a block of the pin's that a control of the work desktop's or a dropped remote session cut | It runs again at the same head, and the hold stands |
| A defect the commit waits for is reported while the hold stands, as an early run's reported late | The hold ends, as at a run that fails, and no hash is recorded on the runs under way. A fix round of the lane whose code it is, and the head is chosen again |
| The pin fails on a defect of the engine's or of the tool's | The hold ends. A fix round of the lane whose code it is. The head is chosen again, and `check`, `measure` and the pin's own parts run there |
| The pin fails on a case that presents into the remote session, and on nothing else | The coordinator asks the owner whether the case is the machine's. Ruled the machine's, the pin has passed with it, and the laptop's gate runs that case first. Not ruled so, it is the pin's failure on a defect of the engine's, as above |
| The pin fails and no fix round ends it, or the machine cannot run it | The hold ends. The owner rules whether the commit is recorded on `check` and `measure` alone or waits for the laptop ("What would change it") |
| The pin passes, and its report names the head | The coordinator records the hash. The hold ends: the documents merged meanwhile are pushed, and the held merges go on |
| The pin passes, and its report names another commit than the head | That pass does not count. The pin runs again at the head `check` and `measure` are green at, and the hold stands |
| A push reached main against the hold | The commit moves to the new head, and the hold stands there. `check` runs there by the push, the coordinator dispatches `measure` there, and the pin runs there last, again where it had passed. No run of the old head is run again |
| The rehearsal finds a defect that fails none of the four | The commit does not wait. Its fix round merges once no hold stands: after the hash is recorded, or before the head is next chosen where the hold ended without a hash. The part that found it then runs again, as steering |
| The rehearsal finds a defect that fails one of the four, before the hash is recorded. The coordinator records on the first message of the pin run's report, which goes out before the rehearsal starts, so this is only a defect reported before that recording is made | The pin has failed, as where it fails on a defect of the engine's |
| The same, after the hash is recorded | A defect found after the commit. The commit stands, and the rule for a file of a later commit holds |
| A sitting from 7 December shows a defect of a measured input that only NVIDIA's or Intel's driver shows | The commit moves to its fix. Where the laptop's record has not begun, its session leaves it out. Once the fix round has merged the coordinator chooses the head that holds it and holds main, `check` and `measure` run there, and a second sitting of the laptop runs its gate and short run there, which stand for the pin, as the owner ruled on 28 November 2026 |
| The same, where the laptop's record has begun | The commit moves as above. The laptop's files so far are steering, and its second sitting runs the record whole, which is part F of its commands, after its gate, its short run and its wait |
| The same, where the sitting that shows the defect is session C and only its driver shows it | The commit moves as at a defect the laptop shows, and the laptop sits again as the two states above have it. The coordinator asks the owner whether session C sits again at the moved head or its files stand. That is not ruled: the owner's desk time comes last and once, and no text has session C sit twice of its own accord |
| At a moved head, `check`, `measure` and the laptop's gate and short run have passed | The coordinator records the moved commit's hash, and the hold ends. The laptop's session waits. The M1's hour runs again there unless the owner rules that its files stand, and the coordinator then tells the laptop's session to take its record |
| At a moved head, `check`, `measure` or the laptop's gate or short run fails | As at the first commit: the hold ends, a fix round merges, and the head is chosen again |

**How the rule came to this.** The fourth version wrote "has run the Windows pin" and sent a failure
to "the lane it names". A pin run that failed met those words, a defect of the engine's that the pin
found was no lane's by them, and nothing made the pin run again after a fix in a crate that is not
`layer-windows`'. The fifth version had the pin run again after every fix round before the commit
and fixed no order, so a `check` failure after the pin's pass spent the owner's remote session
twice. The sixth named a set of the pin's inputs and let a fix round outside it leave the pass
standing. Its diff review showed that rule contradicting itself for a defect the rehearsal finds
after the pass, and the set short of what the pin's run builds and reads, by the cook's crates,
`.cargo/config.toml`, the root manifest, `.gitattributes` and the harness's modules. The seventh
named no set and held the pass to the commit's code, and it kept the commit as "the first commit of
main that holds" the lanes and the fix rounds. Its diff review showed what that left. Anything that
merged after the last fix round the commit waits for, a rehearsal's fix, another lane or a document,
moved main's head off that first commit. `check` and `measure` then ran at the head and not at the
commit, and where the merge was code no pass of the pin at the head could count for the commit. The
eighth defined the commit by where the three runs are, started the hold where the first of them
starts, and held pushes with merges. It kept the seventh's allowance for a pass at a commit that
differs in documents alone, with the command that checks it, for a push made against the hold. Its
diff review showed that no state uses the allowance, that such a push had two next steps, and that
the definition named no one commit after it. This version has the pass at the commit itself,
compares two hashes, and moves the commit at such a push. The rule has a first case already: the
coordinator relays that the work desktop's first early run found the shader cook failing on Windows.
That is `tools/cook`, a measured input and none of the three lanes' code. This proposal did not read
the defect. The third version's third condition was the laptop's first session. The owner has no
Windows machine until 7 December 2026, and ruled that the work desktop's pin run takes its place:
the commit is recorded once the pin has passed there, and the M1's hour follows.

**A measured input** is anything a measurement builds, runs or computes a figure with. Among them:
the crates under `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and layer pins,
`shaders/`, `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`,
`.config/nextest.toml` and `.gitattributes`. Where the list is short the definition holds. The
second version's list ended at `.cargo/config.toml` and read as complete. It was not: every
measurement that is a test runs as `cargo nextest run` (`measure.rs`, `nextest_args`), under the
timeout and the kill `.config/nextest.toml` sets, and `.gitattributes` sets the bytes a Windows
checkout gives the shaders and the sources. The first version of this proposal said "a measured
crate" and held `tools/xtask` outside it. That was wrong. `tools/xtask` is `edit-to-playing`'s
stopwatch (`measure.rs`), computes `frame-pace`'s figures (`pace.rs`), cooks the shaders every
device measurement draws with, and sets every device run's environment (`vulkan.rs`).
`layer-windows` changes that environment on every host, so it is in the commit by the rule and not
by preference.

**How a row names it.** "The measurement commit", with its short hash beside it once it exists. A
file is the record at that commit. A file of a later commit is the record as `-at-<commit>`
(`docs/results/README.md`) only where the diff from the measurement commit changes no measured input
the file's measurement reads, and the row that cites the file gives the diff. Otherwise the file is
steering until the owner's ruling at section 7's step 7 names the record.

**What a figure taken before it is.** The rule is about section 4's measurements and their files of
record. A figure one of them gave before the commit is steering: it stays in the row it steered,
proves no criterion and stands for no file of record. The lanes' runs at `7e3b62a` and `16f45fc` in
rows 21 and 176 are such figures, and so is any measurement run before the commit. A row that asks a
machine for a run and names no commit is another thing: that machine's run closes it, and the row
records the commit it ran at. Two sittings so keep commits of their own: the desktop's first sitting
at `a539d60` (D151's addendum), and the before-and-after runs of `eco-qos` at `a75596b` and its
first parent (row 63). The laptop's session closes rows 109, 189, 208 to 210, 234 and 236 and the
laptop's half of 190 and 247 at the measurement commit, which it has checked out. None of those
names a commit, and row 236 asks for "the merged head". The third version had a first laptop session
close them at `layer-windows`' merge, before the commit; the laptop is not in reach before the
commit now. The second version said a figure before the commit "closes no row", which its own table
of sittings contradicted. A run on the work desktop is a third thing: it is steering at any commit,
it closes no row, and none of its files is checked in.

**After it.** A lane that merges later and changes a measured input runs the measurements it touches
and reports them as steering. Where it answers a finding of a sitting, the owner's ruling on the
finding says which file is the record (section 7, step 7).

**A defect a later sitting finds.** The work desktop shows AMD's driver and no other. A defect of a
measured input that only NVIDIA's or Intel's driver shows is found from 7 December, after the M1's
hour: on the laptop, in its gate, its short run or its record, or in session C. It moves the commit
to its fix: the M1's hour runs again there, or the owner rules that its files stand. The fifth rule
says how the moved commit is recorded, and it is the owner's ruling of 28 November 2026: on `check`,
`measure` and the laptop's own gate and short run at the moved head, with no further remote session
of the work desktop's. The machine may no longer be lent by then, and its driver is not the one that
showed the defect. The laptop's commands gain one rule for it. Its gate and its short run already
come before its record, and a second sitting of the laptop, which the owner starts, runs them at the
moved head and sends its first four messages. The coordinator records the hash on those. The sitting
then waits for the coordinator's word before its record: the M1's hour runs again at the moved
commit first, or the owner rules that its files stand. The eighth version let the sitting go on to
its record at once. Its diff review showed that this is a sitting at the commit before the M1's
files there are read, which the decision rules out. The ninth version's states covered a defect that
the laptop's gate or short run shows, and no other. Its diff review named two more. Where the defect
shows once the laptop's record has begun, in a measurement of the record, in the client's run or in
the mutations, the files written so far are steering and the second sitting runs the record whole.
Where only session C's driver shows it, the commit moves as it does for the laptop's, and whether
session C then sits again at the moved head or its files stand is the owner's to rule when it
arises. The desktop's GPU is not known before its first sitting, and the owner's desk time comes
last and once, so this text does not have session C sit twice of its own accord. A case of the pin's
that the owner ruled the work desktop's own and that then fails in the laptop's gate is such a
defect. The owner's ruling takes that cost for the days before 7 December it saves, and the fourth
option below names it.

**The nightly.** Exit step 3's nightly is the first at or after the measurement commit whose four
`sour` jobs pass: `sour (linux-x64)`, `sour (windows-x64)`, `sour (macos-arm64)`, and `sour
(architecture)`, which compares the three. Those jobs are what step 3 asks: the recording test equal
on three platforms, and each platform's release baseline equal to its dev one. Another job of that
run that fails is a finding in its own row and holds nothing of step 3. The first version said "the
first green nightly", which is stricter than the step: its review found a nightly that failed on
`gpu (lavapipe, GPU-assisted)` alone. `park-stamp` edits the executor and `present-encoder` edits
the package that holds the recording test, so a nightly before them compares a tree the exit does
not ship. The coordinator dispatches the `measure` workflow at the head the commit is to be, before
the pin runs there, and the nightly with it if the schedule did not run on that head, as section 9's
step 6 did at the re-run's commit. The owner allowed the coordinator these two dispatches on 28
November 2026: `gh workflow run measure.yml` and `gh workflow run nightly.yml` on main, for the
measurement commit and a moved one, and for nothing else. A fix round that moves the head sends both
again. The `measure` workflow there is the first run of `record-stall` on Windows with
`park-stamp`'s spans. Its figures are a runner's and are discarded
(`.github/workflows/measure.yml`).

| Option | What it rules out |
|---|---|
| A. The commit holds the three lanes and is recorded once the Windows pin has passed in its first run, which is the work desktop's (ruled) | Sittings at the commit before the lanes merge and the work desktop's pin run has passed. A defect that only NVIDIA's or Intel's driver shows is not found before the M1's hour |
| B. Main's head now. The lanes merge after it | One commit for the record. `present-encoder` then changes what `frame-pace` and `triangle` measured, `park-stamp` changes the executor under all eighteen measurements and `layer-windows` every device run's environment: each is run again as steering, and the files the criteria rest on are of a tree the exit does not ship |
| C. The commit holds `present-encoder` and `layer-windows` alone | Row 21's Windows item measured at the commit. It was the right commit had the owner re-gated that item |
| D. Recorded before any Windows run, as the first version had it | A defect of the first Windows run found before the record is taken. Its fix moves the commit and the M1's hour is run again, or the owner rules that the M1's files stand. It was the third question's second option, and for NVIDIA's and Intel's drivers it is what the ruling takes |
| E. Recorded after the laptop's first session, as the third version recommended | Any sitting before 7 December 2026: the commit and the M1's hour wait until then for machines the owner does not have before it |

**What would change it.** `park-stamp` running past two days or failing its review: the commit is
taken without it, and its item is re-gated. The other two cannot be left out: `present-encoder`
changes the client the criteria run, and without `layer-windows` nothing validated runs on Windows.
The work desktop unable to run the pin at all, for a control of the machine, or left with a failure
that no fix round ends and that is not the owner's to rule the machine's: the pin has not passed,
and the commit is then recorded on `check` and `measure` alone, which is the fourth option, or waits
for the laptop. The owner rules which, and neither is ruled here. The work desktop no longer lent
before the pin has passed: that is the pin unable to run, and the same ruling. Whether the commit
waits for the rehearsal is no longer open: the owner ruled on 26 November 2026 that it does not.

## The sittings, in order

Four sittings of record, the work desktop's two runs, which are steering, and two things that need
no machine.

| Order | Sitting | Machine and commit | What it needs from the owner | What it writes | Rows it closes or advances |
|---|---|---|---|---|---|
| Now | The work desktop's early runs | The work desktop, at main's head | Its installs approved at their prompts: the MSVC C++ build tools, rustup, nextest and Python. The remote session connected, its window open and not minimised, while a run opens a window | Nothing of record and nothing checked in. As steering: the gate's device tests, without their canaries, by hand on the RX 9060 XT, with no validation layer on the machine, which is the first run of `lactose-rhi-vulkan` on a Windows driver; the window cases, fifteen on Windows, in a remote session; `cargo xtask measure --all --no-device` as `work-desktop-windows`, thirteen measurements on an x86 desktop part | None closed. The row of the work desktop's runs. A defect goes to a fix round of the lane whose code it is, before the commit |
| Now | The lanes, and the rows not yet read | None | Nothing | `layer-windows` and `park-stamp` in parallel, `present-encoder` having merged. `park-stamp` runs from `b0c5f44` and takes that merge for its `record-stall` part. One read-only pass over the 37 rows gated at milestone 2 that neither the audit nor this proposal read, so that what they owe a machine is in a commands file before the commit | None yet |
| 1 | The work desktop's pin run | The work desktop, at the head the commit is to be recorded at: the lanes the commit holds have merged, `check` and `measure` are green there, and the coordinator holds main | The Vulkan SDK 1.4.363.1 installed there, approved at its prompt. The remote session connected, its window open and not minimised, while a run opens a window | Nothing of record and nothing checked in. The pin: the installed manifest's version and the layer's digest, the loader's implicit layers from its own log, the gate, plain and GPU-assisted, on the RX 9060 XT, and a short run of `triangle`, `gpu-cost`, `frame-pace` and `rhi-conformance`. Then, as rehearsal and as steering: the same on the integrated Radeon under a name of its own; all eighteen measurements; `cargo xtask gpu --client`; the three mutations' outcome | None closed. The pin's row and the row of the work desktop's runs. The measurement commit is not recorded before the pin has passed, and does not wait for a defect that only the rehearsal finds and that fails none of the pin's four conditions |
| 2 | The quiet window | The M1, at the measurement commit, as soon as it is recorded: before 7 December if the lanes and each fix round merge in time | The Mac on mains, awake, its screen unlocked, and unused for about an hour. One hand at the start: a window resized by its corner for a few seconds (row 209) | Eighteen measurements' files: milestone 1's twelve again, `extract-50k`, `remote-upload-5k` and `record-stall`, and `triangle`, `gpu-cost` and `frame-pace` under MoltenVK and under KosmicKrisp. The `rhi-conformance` files, under both drivers and under KosmicKrisp with GPU-assisted validation. The renderer's cost tests' figures, into their rows. The system daemon's CPU time before and after | The M1's half of 15, 16, 19, 21, 34, 102, 176, 192, 220, 228, 229 and 238; 171's files; 173's tick start read again; 209's M1 half |
| From 7 December 2026 | The desktop's first sitting | The desktop, under each system it boots, at `a539d60` | The owner at the desk for all of it. Mains power. Git, rustup with the pinned toolchain installed by name, a C toolchain, nextest. Under Windows, as administrator: `LocalDumps` before the run and, after it, the Vulkan SDK 1.4.363.1 installed, so session C needs no second setup | Milestone 1's twelve files for each system, the hash file `cargo xtask sour run --emit` writes, nextest's summary for `lactose-task`. The owner returns the CPU model, the systems, the GPU and `vulkaninfo`'s whole output; the `whisk` file's topology row says whether the processor is a hybrid one (row 80) | 64, 77, 80, 81, 164, 169; the desktop's half of 190; the desktop's queue families and memory types for 176 and 246; the facts decision 5.19 wants, which make session C's commands exact |
| 3, from 7 December 2026 | The laptop's session | The laptop, at the measurement commit; row 63's runs at `a75596b` and its first parent | Once, as administrator: the SDK's installer and `LocalDumps`. Then the laptop on mains, logged in, its screen unlocked, its screen and sleep set to Never for the session and put back after it, and the agent session opened in a shell that is not elevated, with the session's brief. The session is about five hours, an estimate. One hand in its first ten minutes: a window dragged by its title bar (row 209). The display's refresh rate read out once | With the owner there: the checkouts' refresh, the SDK as installed, the loader's implicit layers, each GPU's queue families and memory types, and the latency tool's lines. Then the gate's reports, plain and GPU-assisted, on each GPU, and a short run of `triangle`, `gpu-cost`, `frame-pace` and `rhi-conformance`, thrown away. Then the record: all eighteen measurements on the RTX 4070 as `laptop-windows`, its `triangle` file criterion 1's proof on Windows; `triangle`, `gpu-cost` and `frame-pace` on the integrated GPU as `laptop-windows-intel`; `rhi-conformance`, plain and GPU-assisted, on each GPU. Last: `cargo xtask gpu --client` on each GPU, row 63's three `whisk` files at their own commits, the window cases' output, and the three mutations' outcome on the RTX 4070, as labelled data | 243; 63, 109, 189, 208, 209, 210, 234 and 236; the laptop's half of 190 and 247; 150's files and the machine's facts for 150, 176 and 246; the laptop's half of 21, 164's two items that `frame-pace` reads, 176, 228 and 229; labelled figures for 15, 16, 19 and 102; the pin's row, on NVIDIA's driver and Intel's |
| 4, from 7 December 2026 | Session C | The desktop, under each system it boots, at the measurement commit; row 63's runs at their own commits if its processor is a hybrid one | The owner at the desk for all of it. Mains power. Python 3 for the mutation script | All eighteen measurements under Windows; under a system that is not a client platform, the sixteen that need no window. `rhi-conformance`, plain and GPU-assisted, on each GPU. `cargo xtask gpu --client`. The renderer's cost tests' figures. The three mutations' outcome, each beside its control | The x86 desktop's half of 15, 16, 19 and 102; 176; 212, 214, 216, 218 to 220, 222, 224, 225, 238 to 241 and 244; 246 and 247; 63's desktop half if it has one |
| After | Files, rows and the exit's steps | None | The rulings of step 7 | The coordinator checks in the M1's, the laptop's and the desktop's files, scanned for paths as a public repository needs, and none of the work desktop's. The pass writes the rows. The first nightly at or after the commit whose `sour` jobs pass gives rows 170 and 232 their run id. Then section 7's steps 5 to 8 | Every row gated at milestone 2, Done or re-gated |

**Why this order.** The owner has no Windows machine until 7 December 2026, and ruled that the
Windows tests wait until then. The third version put the laptop's first session before the commit:
it would have been the first run of `lactose-rhi-vulkan` on a Windows driver and of the Windows pin,
and a defect found there is a fix of a measured input that costs no file of record. The work desktop
takes that place. It runs the backend on AMD's Windows driver now, by hand and with no validator,
and the pin once the lanes the commit holds have merged and `check` and `measure` are green at that
head. It is on the commit's critical path as the laptop was: the commit waits for its pin run, and
for a fix and a second run if an early run or the pin finds a defect. What it cannot stand for is
NVIDIA's driver and Intel's. The owner ruled that the commit and the M1's hour do not wait for them:
the hour runs once the pin has passed on the RX 9060 XT, likely before 7 December, and a defect that
only NVIDIA's or Intel's driver shows then moves the commit, with the M1's hour run again or its
files ruled to stand. The quiet window is first at the commit because it is the cheapest and it
reads the M1's half of every criterion a result file proves: a criterion that misses there needs an
engine lane, and the files of a laptop's session and a desk sitting taken before that lane would be
steering. From 7 December the desktop's first sitting waits for nothing but the machine: its commit
is fixed, its commands are written, and what it returns of the desktop's GPU and second system is
what session C's commands need. The laptop is before session C because it needs no owner after its
first minutes. Session C is last because the owner's time at a desk is the scarcest thing in the
plan, and it then runs once, on a tree two machines have measured and with a mutation script that
has run on Windows.

**The laptop is opened once.** The third version opened it twice, the first time before the commit.
Nothing is left that must come before the commit: the pin's first run is the work desktop's, and the
laptop's rows that name no commit are closed by a run at whatever commit it has checked out. One
session at the measurement commit holds what the two held, and the owner is at it once: as
administrator and then for about ten minutes, with the laptop's screen and sleep set to Never before
it and put back after. It holds, in this order. While the owner is still there: the checkouts'
refresh (row 109), before anything is built; the installed manifest's version, the installed layer's
digest, the loader's implicit layers and the devices' facts; and the latency tool, which is the one
step that needs a hand (row 209). Then, with nobody at it: the gate, plain and GPU-assisted, on each
GPU, which is the first run of the backend on NVIDIA's and on Intel's Windows driver and fails in
its first minutes if the tool cannot use the pin; and a short run of `triangle`, `gpu-cost`,
`frame-pace` and `rhi-conformance` under each GPU's machine name, which checks the adapters' names
and the files' names and is thrown away. A case the owner ruled the work desktop's own is read first
among the gate's. A gate run or a short run that fails stops the record there: the failure is read
first, so that no file of record is written at a commit that will move, and what needs no
measurement commit still runs. Then the record: all eighteen measurements on the RTX 4070, and the
short run's four on the integrated GPU. Last: `cargo xtask gpu --client` on each GPU; row 63's three
runs at their own commits; the window cases (rows 208, 210, 234 and 236), fifteen on Windows; and
the three mutations on the RTX 4070. `frame-impl`'s by-hand form of criterion 1 is dropped: `cargo
xtask measure triangle` is that run once the pin exists (row 243). A session cut short, or stopped
before the record, runs what is left in a second sitting of the laptop, which needs the owner only
to start it. Where the commit has moved, that sitting's gate and short run at the moved head are
what the moved commit is recorded on, as the owner ruled. Its record there waits for the
coordinator's word: the M1's hour runs again at the moved commit first, or the owner rules that its
files stand.

**The work desktop.** It is the owner's desktop at work, lent for early runs and reached through its
own agent session over a remote desktop session. Its inventory, as that session gave it: an AMD
Ryzen 9 9950X, sixteen cores of one kind; 64 GB; an AMD Radeon RX 9060 XT on AMD's driver 26.11.2,
which reports Vulkan 1.4.359, and the processor's integrated Radeon; Windows 11 Enterprise 25H2; the
system's loader at 1.4.357; and one implicit layer registered for the whole machine, a capture
program's `VK_LAYER_OBS_HOOK`. The owner ruled its role: early runs, as steering. Its files carry
the machine name `work-desktop-windows` and are never the record. This proposal adds what follows
from that on a machine that is not the owner's own and in a repository that is public: none of its
files is checked in, a figure of its is quoted in the row it steers, and its reports name no host,
user or domain. It adds a second machine name too, `work-desktop-windows-igpu`, for the rehearsal on
the integrated Radeon: two GPUs under one name write over each other's files, and that rehearsal is
there to show the laptop's two-GPU commands do not, before the laptop's one session. Only a
measurement's file carries a machine name: the gate's report and a test's log carry none.

It runs twice. *From 26 November*, at main's head, with the toolchain the owner approved and no
validation layer on the machine. The toolchain is installed by name, `rustup toolchain install` in
the clone: on this machine rustup warned that its install of a pinned toolchain on first use is
deprecated, and the desktop's and the laptop's commands had rested on that install. `cargo xtask
gpu` and every device measurement stop on Windows for want of the pin (`tools/xtask/src/vulkan.rs`,
`host`), so the device tests run by hand: the pinned Slang installed by `cargo xtask slang`, the
shaders cooked with `ricotta` as the gate cooks them (`tools/xtask/src/slang.rs`, `cook`), and the
gate's list of device tests (`tools/xtask/src/main.rs`, `DEVICE_TESTS`) run under nextest with
`LACTOSE_GPU_SHADERS` and `LACTOSE_GPU_ADAPTER` set. The canaries are left out: with no validator a
canary's misuse reaches the driver unheard, and its own assert fails
(`crates/rhi-vulkan/src/canary.rs`). A clean run so proves no barrier. What it shows is whether the
backend makes a device, draws, reads back and presents on a Windows driver at all. With it:
`lactose-platform`'s window cases, and `cargo xtask measure --all --no-device`, the first figures of
milestone 2's tree on an x86 desktop part. *Once the lanes the commit holds have merged*, the fix
round of each defect the early runs found among them, and once the early runs' last report is read,
at the head `check` and `measure` are green at and with the SDK installed: the pin, and then the
laptop's and session C's commands rehearsed at full length. The commit does not wait for what only
the rehearsal finds, as the decision's fourth rule has it.

**What the pin's pass is.** Four things, all on the RX 9060 XT: the tool takes the installed layer,
its manifest's version and then its digest; each canary is heard, seven under the plain validators
and two under GPU-assisted validation (`tools/xtask/src/vulkan.rs`, `Validators::canaries`); no case
of either gate run fails; and in a short run of `triangle`, `gpu-cost`, `frame-pace` and
`rhi-conformance` the measured runs hear no message. Six measurements need a device once
`layer-windows` has merged: five at `1c62676`, and `rhi-conformance` (`tools/xtask/src/measure.rs`,
`on_device`). The short run takes four of the six and leaves `extract-50k` and `remote-upload-5k` to
the full run. The seventh version called the four "the four device measurements", and its examples
of a failed fourth condition named the full run, so the condition had two meanings. It has one. A
message that a measured run of one of the four hears on the RX 9060 XT fails it, in the short run or
at full length in the rehearsal. A message in a measured run of `extract-50k` or of
`remote-upload-5k` fails none of the four: it is the rehearsal's defect, and the commit does not
wait for it. The third version wrote "the seven canaries", which is the plain run's count alone. The
fourth condition is a sharper test there than on a clean machine: the registry's implicit layer
would join a validated run under the old form and fail a measured one, which G3 found by reading the
loader's source and no run has shown. A failure is reported with whose it looks to be, and beside
the engine's there are three kinds: AMD's driver's, as KosmicKrisp has defects of its own; the
remote session's, where a window or a present behaves as no panel does; and a control's, where the
scanning, the lock or an update cuts a run. The four conditions are not equally showable on this
machine, which the fourth version did not say. The layer's check and the canaries open no window,
and a control shows in them as a refusal or a timeout: that run is run again. The gate's swapchain
cases and the short run's `triangle` and `frame-pace` present into a remote session, and a failure
of theirs cannot be told there from a Win32 present defect of the engine's. AMD's driver's cannot be
told apart at all: both of the machine's GPUs run the one driver, and the first other Windows driver
is the laptop's, after the commit. So the ruling is bounded. The owner may rule a failed case the
machine's, and the pin passed with it, only where the case presents into the remote session: never
the layer's check, a canary or a message. The laptop's gate runs that case first, and where it fails
there it is a defect of a measured input and moves the commit. Any other failure that no fix round
ends leaves the pin not passed, and what the commit then waits for is the owner's to rule, as "What
would change it" has it.

**What the work desktop cannot show.** It has no hybrid part: nothing of row 63 and nothing of an
efficiency core. Its windows are a remote session's: a present's timing, frame pacing and the
latency tool's lines mean nothing there, and a run that opens a window needs the session connected
and its window open and not minimised: the diff review expects a minimised remote session to stop
updating and a present to stall there, which would read as the engine's, and no run has shown it.
Its endpoint protection scans in real time, so no figure of its is quiet. It locks after fifteen
minutes without input, and on Windows `cargo xtask gpu --client` reads no lock
(`tools/xtask/src/main.rs`, `screen_locked`), so a run a lock fell in is named by the session and
not by the tool. Its controls stand: no command asks for an exclusion from scanning, a changed lock,
a changed GPU timeout or an elevated run, and `LocalDumps` is not set there. PowerShell's execution
policy is one of them. No command overrides it for a process or for the machine, and a script it
refuses stops the run and is reported as the machine's. It shows nothing of NVIDIA's driver or
Intel's.

**Row 150's four hardware questions are not shown.** The spike's file leaves four to hardware
(`docs/results/spike-rhi-apple-m1-2026-11-15.md`, "Design questions for the lavapipe run"): an
ownership transfer's effect, memory that is not coherent, a heap indexed without the non-uniform
decoration, and a heap image in the wrong layout. The plan's table of what the spike left open says
of two of them: "Probed on the laptop's devices and under GPU-assisted validation, as labelled data
(decision 5.17)". No probe of any of the four exists. `ownership_transfer_keeps_the_contents`
performs the release and the acquire and checks the contents
(`crates/rhi-api/src/conformance/outputs.rs`). It never drops the transfer, so it cannot show what a
dropped one does, and where compute has no queue family of its own `lactose-rhi-vulkan` gives it a
second queue of the graphics family (`crates/rhi-vulkan/src/raw.rs`, `families`), across which there
is nothing to transfer. `unflushed_range_at_a_submit_is_refused` skips where `coherent_host_memory`
is set (`conformance/buffers.rs`), and `MemoryTypes::choose` takes a coherent host-visible type
wherever a device offers one, so the capability is set on every device that offers one. The cook
refuses an undecorated heap index (`tools/cook/README.md`, "The checks"), and null's rule alone
holds the wrong layout. The first version said the gate's two runs "answer the first two where the
device allows". That was wrong.

The ruling is that row 150 and the plan's table say so: all four are recorded as not shown, and "a
run that shows nothing is recorded as that and discharges no rule" (decision 5.17). The gate's runs
on each GPU stay: they are criterion 2's evidence on those drivers. Each sitting records what bears
on the four and costs nothing: each device's queue families, so that a later reader knows whether
the ownership case crossed two families, and its memory types, so that a host-visible type that is
not coherent is known where one exists. The alternative is a lane that writes the four as device
probes: the transfer dropped across two families, a host-visible type forced non-coherent and left
unflushed, an undecorated heap index past the cook, and a heap image in the wrong layout. It is
about ten agent runs with `reviewer-gpu`, it edits `lactose-rhi-vulkan`, which is a measured input,
so the commit waits for it, and what it gives is labelled data on two drivers that proves no rule
either way. It was the owner's fourth question, and the owner ruled for the record as not shown.

**The row of `frame-impl`'s runs on x86 (row 244).** It owes the desktop criterion 1 and "the
probe's paced runs under synchronisation validation", and lavapipe a run. The probe is "a probe
build of a scratch copy" (`crates/app-client/README.md`): no checkout holds it, so session C cannot
run it. No runner of lavapipe's presents to a window. Session C runs what a checkout holds: `cargo
xtask measure triangle`, which is criterion 1, and `present-encoder`'s `cargo xtask gpu --client`,
which starts the release client on a window under the gate's validators, as shipped and with the GPU
made to run behind the present thread. That is none of the probe's three faults: a present call that
sleeps 40 ms, every 13th making no submit, and every 97th draw torn (`crates/app-client/README.md`).
The probe's paced runs and lavapipe's half move to a row of their own, gated at milestone 3: the
first change after the exit that edits the client's present path checks the probe in or says why
not. The alternative is a lane now that adds the probe's three faults to the client behind a switch:
about six agent runs, which are the lane, `reviewer` and `reviewer-gpu`, a fix round,
`reviewer-diff` and `merger`, in a measured input, so the commit waits for it. It gives the
desktop's paced runs and not lavapipe's half. The re-gate was the recommendation, and the owner
ruled for it on the fifth question. The second version had the decision entry take it with no
question. It is a cut of an exit row's scope with a costed alternative that holds the commit, which
is what the fourth question was for row 150, so it was asked.

**A second GPU on one machine.** `LACTOSE_MEASURE_MACHINE` names the machine in a file's name
(`docs/results/README.md`). The laptop's RTX 4070 runs as `laptop-windows`, the name its files of 15
November carry, and the integrated GPU as `laptop-windows-intel`, each with `--adapter`. The desktop
runs as `desktop-windows` and `desktop-<system>`, in its first sitting as in session C: the first
sitting's commands did not name the machine, and its two systems' files of one day would have shared
a name. A run under the wrong name overwrites the other GPU's file of the same day (`measure.rs`,
`write`). The guard is in the commands. Each GPU's run is one command that sets the name itself,
carries the adapter's name written out, copies the files it wrote into the target directory and
counts them; after the second GPU's run the first one's files are compared with their copies; and
the `rhi-conformance` file's header names its adapter. The laptop's short run checks all of it
before the long one. The work desktop's pin run rehearses the same blocks first, on its two GPUs, as
`work-desktop-windows` and `work-desktop-windows-igpu`. The second version set the name in one
command and ran in another. That rests on a shell that keeps its state between an agent's calls,
which the laptop's session may not have, and a name that is lost is silent: both GPUs' files take
the machine's default name and the second run overwrites the first. For the same reason no command
rests on a changed directory: the laptop's session detaches the clone itself at its commit, where
the second version made a worktree beside it.

**A system that is not a client platform.** Session C there names its measurements and leaves out
`triangle` and `frame-pace`. Nothing is built for it: the plan's sentence is amended to say so.

**What would change the order.** The lanes not merging before 7 December: the work desktop's pin run
and the M1's hour then fall beside the owner's own machines' sittings, and the order from 7 December
stands. The work desktop unable to run the pin: the owner's ruling, as the section on the commit has
it. Or the owner preferring one day to three from 7 December: the laptop's session and session C
then run without waiting for each other's reading, and a miss that needs an engine lane costs the
other machine a second sitting.

## The Khronos layer on Windows

D153 has the layer "pinned on every host". `vulkan-layer.toml` pins Linux by an archive the tool
fetches by digest and the Mac by the digest of the library the owner's installer leaves.

**The decision.** Windows takes the Mac's form. The pin is the SHA-256 of
`Bin\VkLayer_khronos_validation.dll` of the SDK the owner installs. The tool finds the SDK at
`VULKAN_SDK`, which LunarG's installer sets, or at `C:\VulkanSDK\<version>`. It reads the manifest
beside the library, `Bin\VkLayer_khronos_validation.json`, for the layer's name and version before
it takes the digest, so that another SDK at `VULKAN_SDK` is named as another version and not as a
wrong file. It takes the system's loader and the system's drivers, so `--driver system` is the only
driver there, and it sets no library path.

~~~toml
# LunarG's installer, which an owner runs as administrator: the SDK at `VULKAN_SDK`, or at
# `C:\VulkanSDK\<version>` where the installer puts it. Nothing is fetched. `url` and `sha256`
# name the installer, for the error that says the SDK is not there.
[host.windows-x86_64]
url = "https://sdk.lunarg.com/sdk/download/1.4.363.1/windows/vulkansdk-windows-X64-1.4.363.1.exe"
sha256 = "c02932bd9a968b22d404536108adaacac908c081cd6b0113f5bfefc523dd0a6a"
library_sha256 = "451ed11cbc799f858cbc0e843766d2990527c03f93f7a85f03bc1e67b605a435"
~~~

**How the digest is obtained with no Windows host in reach.** LunarG publishes each SDK file's
SHA-256 in one list, and the list's digest for the Linux archive is the one the pin already holds.
The Windows installer for 1.4.363.1 is 303,066,832 bytes with the SHA-256 above. It is a Qt
Installer Framework installer that carries ten 7z archives. The first holds
`Bin/VkLayer_khronos_validation.dll`, 21,867,448 bytes, and its manifest, which names
`VK_LAYER_KHRONOS_validation` at `1.4.363` with the library beside it. This proposal's probe fetched
the installer, checked its digest against the list, cut the archive out by its 7z signature and
header, and read the two files with the Mac's own `bsdtar`. Nothing of the installer was run.
`reviewer-gpu` repeated the derivation and got the same digest, found the file signed by LunarG, and
found it the installer's only copy. The lane derives it a third time by the same steps and writes
them into `tools/xtask/README.md`.

**What the installer does to the machine.** It needs an administrator. Beside the SDK's folder it
installs LunarG's Vulkan runtime for the whole machine, which is the loader every Vulkan program
there then finds; it sets `VULKAN_SDK` and the path for every user; and it registers the SDK's
explicit layers in the registry. The layer's library needs the Visual C++ runtime on the machine.
The owner's second question said so, and the owner ruled for the install. On the work desktop it is
the same install, approved at its prompt: it changes that machine's Vulkan runtime for every
program, and the run records the loader's version before and after.

**Which layers join a run.** Today a validated run adds its layers directory with
`VK_ADD_IMPLICIT_LAYER_PATH`, and a measured run sets `VK_LOADER_LAYERS_DISABLE=~implicit~`
(`tools/xtask/src/vulkan.rs`, `Run::environment` and `Run::measured`). On Windows both fail, as
`reviewer-gpu` read in the loader's source at the pinned SDK's tag. The first adds to the registry's
implicit layers, so a layer a driver or an overlay registered joins every validated run. The second
makes the loader warn once for each implicit layer it disabled, the device hears the warning as a
message, and a measured run that hears a message fails (`crates/app-client/tests/timing.rs`,
`Messages::assert_none`). The lane sets the exclusive `VK_IMPLICIT_LAYER_PATH` in place of both: the
tool's layers directory for a validated run, and its empty directory for a measured one. It does so
on every host, so that the form the laptop runs is one the M1 and CI have run. The device files'
Validation line, which names `~implicit~` today (`measure.rs`), follows. The loader ignores every
such variable in an elevated process, so the session's shell that is not elevated is what makes the
pin work, and each session checks it. The work desktop's registry holds such a layer for the whole
machine, `VK_LAYER_OBS_HOOK`, so its runs are the first to show the exclusive path against one.

**How it is checked.** On a Windows machine, by the tool itself: `vulkan::sdk` refuses a manifest of
another version and then a library whose digest is not the pin's, so the first `cargo xtask gpu
--driver system` is the check. The work desktop's pin run is the first, on AMD's driver. The
laptop's session repeats it on NVIDIA's and Intel's, and session C on the desktop's. A digest that
differs is a finding for the planner, and no session re-pins. A second pin is proposed only for a
file whose version is 1.4.363.x and whose Authenticode signature is LunarG's, and it is the owner's
to approve.

| | The Mac can do it | Only a Windows machine can |
|---|---|---|
| The pin | Its form and parse; the digest out of the installer; the paths, the environment, the manifest's check and the implicit manifest's JSON for Windows as unit tests; clippy for `x86_64-pc-windows-msvc` | Run the installer, which needs an administrator. Show that the installed file is the archived one |
| The loader | Show that the exclusive path gives the M1's and CI's runs no message and no lost canary | Show that the system's loader, at the version the run records, takes the manifest from the exclusive path in a shell that is not elevated and loads nothing of the registry's with it |
| The run | The M1's `rhi-conformance` files | Hear each canary on each of the machine's GPUs, and no message in a measured run. Record the registry's implicit layers from the loader's own log (`VK_LOADER_DEBUG=layer`), since `vulkaninfo`'s list does not mark which layers are implicit |

| Option | What it rules out |
|---|---|
| A. Installed, as the Mac (ruled) | A run on a Windows machine with no administrator, and a machine whose system loader and environment the SDK has not touched. The owner installs the SDK once on each Windows machine: the work desktop, the laptop and the desktop |
| B. The tool fetches the installer by digest and cuts the layer out | Nothing of the owner's, where the Visual C++ runtime is already there. The installer's layout is the Qt Installer Framework's, which is public, and the archive's own CRC checks each cut. It needs a 7z reader in the tool, a new dependency, and the loader stays whatever the GPU driver installed. About nine agent runs |
| C. No pin. Criterion 1 on Windows by hand, with whatever layer the machine has | Section 4's "its `triangle` file is criterion 1's proof on Windows", D153's sentence, and `cargo xtask gpu` on Windows at all: rows 210, 234 and 236 and the desktop's rows under Windows stay open |

**What the lane also carries.** Two things of the same crate, each too small for a lane.

- *A measurement `rhi-conformance` in `cargo xtask measure`.* It writes the file section 4 names,
  with section 4's four contents: the cases run, the capabilities reported, each canary's message,
  and the count of other messages by source. Its header is its own and not a timed measurement's,
  which would say "Validation off" of a validated run: the machine's rows, the adapter's name, the
  device as `DeviceInfo` reports it, and a Validation row with the layer's version and the
  validators' settings. No path of the machine is in it.
- *`renderer-impl`'s mutation script, checked in as `tools/xtask/barrier-mutations.py`*, beside
  `line-endings.sh`, so that the desktop needs one clone. The lane fixes its verdict. Today it looks
  in the gate's console output for "`<test>: passed`", which `run_vulkan` has not printed since
  `7424e7c`: only failed entries go to the console, so every mutation reads as caught, a mutation
  that was missed among them. The verdict is read from the gate's report under the target directory:
  `caught` only on the named test's `FAILED` line, `MISSED` on its `passed` line, and `NOT RUN` on
  neither. The lane shows it on the M1 with the row's own record: `between-dispatches` is missed
  under MoltenVK, whose serial compute encoder orders the dispatches, and caught under KosmicKrisp
  (row 239). For Windows it reads and writes its files as UTF-8 and makes each copy into a fresh
  directory.

**What would change it.** An installed file not being the archived one, on any of the three Windows
machines: then B, which cuts the layer out of the archive the digest was read from. The system's
loader not honouring the exclusive path is not cured by B, under which the loader stays the
driver's: it is a finding for the planner, and its fix is in how the tool names its layers. The
second version said B "can bring its own loader", which the option does not do.

## Row 21's Windows item

D162 sends the `SetEvent` that `Pool::open_gate` makes for each frame worker to `record-stall`'s
Windows rows, as "the time from each `block_in_place`'s end to the record's next job on a worker
that was parked". `record-stall` is a test of `lactose-app-client` and reaches neither instant.
D162's addendum names two forms of a stamp it could read, two process-wide hooks or the same behind
a cargo feature, and leaves the choice as an interface decision.

**The decision.** Neither. The stamp is two `tracing` spans of `lactose-task`, at debug level,
target `whisk`.

- `open_gate`: around `Pool::open_gate` in `JobContext::block_in_place` (`crates/task/src/job.rs`),
  from the return of the closure to the gate's return.
- `park`: around each of the two waits a worker makes without a permit (`crates/task/src/gate.rs`),
  entered before the wait and exited when it returns, with a field that says which wait it is.
  - The idle protocol's, `sleep.park` in `gate::idle_park`, whatever the worker's look saw. A worker
    parks after a look that found nothing, and also after a look that found work with no permit to
    take it. The second is the worker the gate's opening is for. The first version's "entered after
    the look that found nothing" left it out.
  - A scope opener's on its latch, `opener.park(latch)` in `gate::wait_for_children`. An opener
    whose children other workers took parks there, and so does one the gate stopped beside its own
    children. It is outside the idle protocol, and `Pool::open_gate` wakes it with every other
    worker (`crates/task/src/pool.rs`). The second version spanned the idle protocol alone: such an
    opener's wake was not counted, and a next job it took would have read as taken by a worker that
    was awake. A replay's wait on a latch (`job.rs`) is not spanned: under a replay the log decides
    when each worker runs, and `record-stall` runs under none.

The first review's probe put spans at `open_gate` and at the idle protocol's wait: 53 of 53 loom
models and Miri's five `block_in_place` tests passed. No probe has run a span at the latch's wait,
and the lane's loom and Miri runs are the first. The owner ruled for the spans, and the lane runs
from `b0c5f44`: its spans are on its branch at `a5ce394`, and its `record-stall` part takes
`present-encoder`'s merge first.

**What it is and is not.** Three things the first version understated.

- *Both spans are in every build.* Architecture section 5.1 says "Finer levels compile out". Nothing
  builds that: the workspace's `tracing` sets no static maximum level. Until it is built, each span
  costs about 1 ns where nothing listens at debug (the crate's README). When it is built, it is set
  where `record-stall`'s build does not inherit it.
- *A listener's callbacks for `park` run inside a wait's protocol*, on a worker that is going to
  sleep or has just been woken, in the idle protocol after it has announced it is asleep. The sixth
  version said that a listener that blocks there delays a wake sent to that worker. Lane
  `park-stamp`'s concurrency review showed worse: a listener on the latch's `park` span could take
  the opener's unpark token, and the opener then slept on a set latch for good. A callback that
  waits now delays that worker's wake and loses none, for a reason of its own at each wait. At the
  latch it is the lane's two fixes. The wait checks its latch, its permit and shutdown again once
  the span is entered, after the callbacks and before it parks. And a `run_job` scope's opener looks
  again where its children are queued: the diff review of the eighth version and the coordinator
  relay that the lane's review found the first check not enough for such an opener, and the lane's
  commit `19f1b1b` adds the second look. At the idle wait the reason is the protocol's own: the
  worker parks only while its sleeper bit says nobody has claimed it. A listener's callbacks must
  return promptly and never call into the executor. `open_gate`'s callbacks for the span's creation
  and for its entry both run with the gate still closed, and every frame worker the gate holds waits
  for them. The decision entry has the rule in the lane's words, checked against the lane's README
  at `19f1b1b`, "Tracing". The eighth version's entry had it as of `35c2120`, one look short. The
  lane table's row and the spans' row keep the seventh version's sentences: the coordinator relays
  that the lane's merge writes both rows again. The crate's README holds the full rule under
  "Tracing", which on main says "One `tracing` span per node run and one per range split off it" and
  nothing of a debug level.
- *A hook would be safe code too.* `gate::Hook`, the test-only hook the crate has, is an atomic flag
  and a mutex. The first version costed the hooks with an `unsafe` they do not need and with "always
  compiled", which the spans are as well.

What is left in the spans' favour is smaller and real. `record-stall` already reads this crate's
spans through a subscriber it installs (`crates/app-client/tests/stall.rs`), so no public item is
added, and no cargo feature, which unifies across the workspace's builds. Architecture section 21's
entry for `task` stands either way.

**What the file gains**, at each reservation:

- for each worker parked across an opening, in the idle protocol or on a latch, the time from the
  opening's start to that worker's `park` exit, with the count of each kind, the median, P99 and the
  worst. This is the item: what one `SetEvent` for each frame worker costs each of them;
- the opening's own length, on the opener's side: it is not the item, and the woken worker's wake
  comes after it;
- D162's own figure, for the openings after which a worker that was parked took the record's next
  job: that job's start from the opening's start and its start wait, with the count, the median and
  the worst, and no percentile under a hundred samples. In the review's probe on the M1 the opener
  took the next job itself in 1,859 to 1,924 of 2,000 openings, so this figure has a sample in under
  a tenth of openings;
- the count of openings after which a worker that was awake took it.

A full run makes 360 ticks at each reservation where it makes 60 today, so that the first row can
hold over 1,000 wakes on the laptop's six frame workers: up to four are parked across an opening at
a reservation of one and up to three at two. The six are its performance cores
(`docs/results/whisk-laptop-windows-2026-11-15.md`, "Detected topology"), and the test takes a frame
worker for each, up to eight (`crates/app-client/tests/stall.rs`). The 300 more ticks at each
reservation add seconds to a run: about four on the M1, by the diff review's arithmetic from the
short files' ticks. Every tick asserts that it saw an `open_gate` span, and each reservation that it
saw `park` spans, so a build that compiles debug spans out fails and writes no file with empty rows.
The rows are written on every host. On Windows they are the `SetEvent` item.

**What the lane owes for the spans' own cost.** On the M1, as steering, interleaved and three times
each: the record phase's figures and the time from an opening to the record's next job with nothing
listening at debug, with `open_gate` listened to, and with both. The three are within one another's
spread, or the README states the difference. The first version had the quiet window's WHISK file
judge the spans by "a row that moved beyond the spread". That is gone: `whisk` installs no listener
(`crates/task/benches/whisk.rs`), so it cannot see a listener's cost, and the quiet window's file is
read against D156 alone.

| Option | What it rules out |
|---|---|
| A. Two debug spans, lane `park-stamp` before the commit (ruled) | A measurement commit before the lane merges. A subscriber of `whisk`'s debug level that blocks in `park` |
| B. Re-gate the item to the change that revisits the keeper's wait | Knowing what opening the gate costs a record on Windows when `RECORD_RESERVATION` is set. D162 says the frame uses that `SetEvent`. No other run shows it: the client binary links no physics backend (`crates/app-client/Cargo.toml`), so `frame-pace` never opens the gate |
| C. Two public hooks, or the same behind a feature | A crate surface with no item added for a measurement. The same rows, the same rule for what a callback may do, about two agent runs more |

**What would change it.** The lane's paired runs showing that listening moves the record phase
beyond the spread: then the file says so beside its rows, or B.

## Two points for the freeze decision

`reviewer-gpu`'s reading for row 34 found that the pinned MoltenVK 1.4.2 (`v1.4.2`, `db66022`)
encodes a multi-draw as a loop on the CPU, one Metal indirect draw for each record, inside
`vkQueueSubmit` under the defaults, and that KosmicKrisp loops inside recording. Row 34 then waits
on the quiet window's `gpu-cost` files alone. The reading raises two points. This proposal places
them and settles neither.

- *`maxDrawIndirectCount`.* `lactose-rhi-vulkan`'s `draw_indexed_indirect` never checks its count
  against the device's limit, and `lactose-rhi-api` carries no such limit. The fix the reading names
  is a field of `Capabilities` that every backend and null refuse above. That is a change to the
  surface the freeze fixes, so it belongs to the freeze decision's amendments (decision 5.13, row
  18), built by a lane the decision names. No driver in reach has a limit a frame meets, so the
  sittings do not wait for it. If it is built before the exit it merges after the measurement commit
  and changes a measured input, under section 3's rule.
- *D42's "emulated indirect-count still encodes the maximum draw count".* It fits KosmicKrisp. It
  does not fit the pinned MoltenVK, which reports `drawIndirectCount` false and records nothing for
  the count entry points. The correction of D42's sentence is a correction of record: it joins the
  conflicts row 18 already holds for the freeze decision. Its consequence, a culling path under
  MoltenVK where the count is not offered, is an input to milestone 3's plan and to no lane of
  milestone 2.

## The rows (exact text)

Each OLD is a run of whole lines of the named file at `1c62676` that occurs once in it, and NEW
replaces the run. Row 150's block is the whole row, as lane `exit-pass-rows` left it. The rows name
each other by D165 and by their heads, never by a row number this proposal adds. There are
twenty-one blocks: fourteen in the plan, six in the verification file, and one that appends five
rows to it. The third version had twenty: the new one is the work desktop's row in section 4's table
of machines, and the block of appended rows grew from four rows to five, by the row of the work
desktop's runs. The fifth version adds no block and changes six for its diff review: the lane
table's, the measurement commit's, the machine's row, the work desktop's paragraph, the owner's
actions and the appended rows. The sixth changed six: the lane table's, the measurement commit's,
the work desktop's paragraph and the appended rows for its rule, and the lane table's again,
`record-stall`'s row in section 4 and verification row 21 for the opening's length. The seventh
changed four: the lane table's, the measurement commit's, the work desktop's paragraph and the
appended rows, for the coordinator's rule and for what a listener on the two spans may do. The
eighth changed four: the measurement commit's, the laptop's session's, the work desktop's paragraph
and the appended rows, for where the commit is, the hold, the owner's two rulings of 26 November, a
moved commit and the short run's four names. The ninth changes three: the lane table's, where
`layer-windows`' Windows run is at the commit itself, the measurement commit's, and the appended
rows, of which the commit's row alone changes. They take the pass at the commit itself, the head's
choice after the early runs' last report, a push made against the hold, what ends the hold, and a
moved commit as the owner ruled it with the laptop's record after the M1's hour there. The tenth
changes one, the measurement commit's bullet in the plan, which gains the sentence on a defect that
only session C's driver shows. Every block was applied in memory to the two files of `1c62676`: each
OLD was found once, at the lines its head states. If a merge before the decision is applied moves a
line, the blocks are built again from the merged files. `present-encoder`'s merge at `1c62676`
inserted an addendum of D162 into the decisions log and replaced verification row 229, one line by
one. It did not touch the plan, it moved no line of the verification file, and no block here names
row 229, so the third version's blocks kept their lines and the next decision number is the one
after D164. The plan's own record of the text it once appended to row 150, in its section
"Verification rows", still reads "probed on the laptop's devices": it records an edit that was made,
and it stays.

The text is written for the owner's rulings.

~~~text
=== docs/plans/milestone-2.md line 114: section 2's entry for row 150: the four hardware questions are recorded as not shown
OLD:
| 150 | The RHI spike ran under MoltenVK alone | `vulkan`. **On the M1** (branch `vulkan`): device-only staging runs as a case under both drivers, and KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. lavapipe's run is the `gpu (lavapipe)` job's first, after the merge | The suite under lavapipe settles the spike's eight design questions where lavapipe can: it has one coherent memory type and one queue. The four questions the spike left to hardware are probed on the laptop's devices and recorded, and a run that shows nothing discharges no rule of null (decision 5.17). Question 2 needs a lazily allocated memory type, which lavapipe and the PC drivers surveyed do not offer: KosmicKrisp answers it on the M1 if it offers one, and `spike-metal` answers it for Metal |
NEW:
| 150 | The RHI spike ran under MoltenVK alone | `vulkan`. **On the M1** (branch `vulkan`): device-only staging runs as a case under both drivers, and KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. lavapipe's run is the `gpu (lavapipe)` job's first, after the merge | The suite under lavapipe settles the spike's eight design questions where lavapipe can: it has one coherent memory type and one queue. The four questions the spike left to hardware are recorded as not shown: no test makes any of the four misuses on a device (D165), and a run that shows nothing discharges no rule of null (decision 5.17). The laptop's session records each of its GPUs' queue families and memory types beside them. Question 2 needs a lazily allocated memory type, which lavapipe and the PC drivers surveyed do not offer: KosmicKrisp answers it on the M1 if it offers one, and `spike-metal` answers it for Metal |

=== docs/plans/milestone-2.md line 146: section 3's table of what the RHI spike left open: two questions no run probes
OLD:
| An ownership transfer's effect; a heap image in the wrong layout | Null's rules. Probed on the laptop's devices and under GPU-assisted validation, as labelled data (decision 5.17) |
NEW:
| An ownership transfer's effect; a heap image in the wrong layout | Null's rules. No test makes either misuse on a device, so no run on the laptop's devices can show an effect: both are recorded as not shown, which discharges no rule (decision 5.17, D165) |

=== docs/plans/milestone-2.md line 185: the lane table: two lanes after its last row
OLD:
| `measure-role` | Two safe calls for measurement in `lactose-task` (D164): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |
NEW:
| `measure-role` | Two safe calls for measurement in `lactose-task` (D164): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |
| `layer-windows` | The Khronos layer pinned on Windows (D165; D153's "pinned on every host"), in `tools/xtask`. `vulkan-layer.toml` gains `[host.windows-x86_64]` in the Mac's form: the SDK the owner installs, found at `VULKAN_SDK` or at `C:\VulkanSDK\<version>`, its `Bin\VkLayer_khronos_validation.dll` held to `library_sha256`, and its manifest `Bin\VkLayer_khronos_validation.json` read for the layer's name and version before the digest is taken, so that another SDK at `VULKAN_SDK` is named as that; the installer's `url` and `sha256` beside it, named in the error for an SDK that is not there and never fetched. The lane derives the digest on the Mac out of LunarG's installer, fetched by the SHA-256 LunarG publishes and never run, and writes the steps into `tools/xtask/README.md`; the proposal's reading, which its GPU review repeated, is `451ed11cbc799f858cbc0e843766d2990527c03f93f7a85f03bc1e67b605a435`. On Windows a run takes the system's loader and drivers, `--driver system` alone, and sets no library path. On every host a run sets the exclusive `VK_IMPLICIT_LAYER_PATH`, to the tool's layers directory for a validated run and to its empty directory for a measured one, in place of `VK_ADD_IMPLICIT_LAYER_PATH` and of `VK_LOADER_LAYERS_DISABLE=~implicit~`: a layer the Windows registry enables then joins no run, and the loader has nothing to warn of in a measured one. The device files' Validation line, the README and the unit tests follow. A measurement `rhi-conformance` in `cargo xtask measure`: the gate's Vulkan step under each driver `measure` takes, and under GPU-assisted validation where the host's gate has it, written as `rhi-conformance-<driver>[-gpu-assisted]-<machine>-<date>.md` with section 4's four contents under a header of its own: the machine's rows, the adapter's name, the device as `DeviceInfo` reports it, and a Validation row with the layer's version and the validators' settings, every path cut as a device file's is; `--all` runs it last and `--no-device` leaves it out. `renderer-impl`'s barrier-mutation script, checked in as `tools/xtask/barrier-mutations.py` beside `line-endings.sh`, with its verdict read from the gate's report under the target directory, `caught` only on the named test's `FAILED` line, `MISSED` on its `passed` line and `NOT RUN` on neither, since the gate has printed no passing line since `7424e7c`; its files read and written as UTF-8 and each copy made into a fresh directory, for Windows | Nothing | On the M1: the pin's parse and the Windows paths, environment and manifest check as unit tests; a unit test that no run sets `VK_ADD_IMPLICIT_LAYER_PATH` or `VK_LOADER_LAYERS_DISABLE`; clippy for `x86_64-pc-windows-msvc`; the digest derived by the lane and equal to the proposal's; under the exclusive path, `cargo xtask gpu` in both forms and `triangle`, `gpu-cost` and `frame-pace` under both drivers with no message, as steering, and `gpu (lavapipe)` green in CI; the three `rhi-conformance` files, with no path of the machine in them; the script's control, and `between-dispatches` read `MISSED` under MoltenVK and `caught` under KosmicKrisp, as the row of that barrier records the two drivers. The Windows run is the work desktop's pin run, on its RX 9060 XT at the measurement commit itself, as steering and before the measurement commit is recorded, and a defect it finds is a fix round of the lane whose code it is; the laptop's and the desktop's runs follow from 7 December 2026 | 7 |
| `park-stamp` | Two `tracing` spans of `lactose-task` at debug level, target `whisk` (D165, D162): `open_gate`, around `Pool::open_gate` in `JobContext::block_in_place`, from the return of the closure to the gate's return; and `park`, around each of the two waits a worker makes without a permit, entered before the wait and exited when it returns: `sleep.park` in `gate::idle_park`, whenever a worker parks in the idle protocol, whatever its look saw, nothing or work with no permit to take it; and `opener.park(latch)` in `gate::wait_for_children`, a scope's opener on its latch, which `Pool::open_gate` wakes with every other worker. A field of the span says which wait it is. A replay's wait on a latch is not spanned. The loom models still run the shipped functions. No public item, no `unsafe`, no hook and no cargo feature; architecture section 21's entry stands. The crate's README, "Tracing", names both spans and says that the node and range spans are the info level's, that both debug spans are in every build until section 5.1's compile-out is built, and that a listener's callbacks for `park` run on a worker that is going to sleep or has just been woken, in the idle protocol after it has announced it is asleep, so they must return promptly and never call the executor, and that `open_gate`'s enter callback runs with the gate still closed. A scope's opener checks its latch, its permit and shutdown again once it has entered its `park` span, after the callbacks and before it parks, so a listener that blocks there delays a wake and cannot lose it. `record-stall`'s subscriber reads the two, and its file gains at each reservation: for each worker parked across an opening, in the idle protocol or on a latch, the time from the opening's start to that worker's `park` exit, with the count of each kind, the median, P99 and the worst; the opening's own length, on the opener's side; for the openings after which a worker that was parked took the record's next job, that job's start from the opening's start and its start wait, with the count, the median and the worst; and the count of openings after which a worker that was awake took it. A full run makes 360 ticks at each reservation, so that the first row can hold over 1,000 wakes on the laptop's six frame workers. Every tick asserts that it saw an `open_gate` span, and each reservation that it saw `park` spans, so a build that compiles debug spans out fails. The rows are written on every host; on Windows they are the `SetEvent` item | `measure-role` merged | The rows in `record-stall`'s file on the M1. On the M1, as steering, what listening costs: the record phase's figures and the time from an opening to the record's next job with nothing listening at debug, with `open_gate` listened to and with both, interleaved and three times each, by a means that is the lane's; the three within one another's spread, or the difference stated in the README. `cargo xtask measure whisk` before and after. The loom models, Miri and the cross-target clippy runs for `lactose-task`, with the span at the latch's wait, which no probe has run. `tests/spans.rs` holds both spans' names, target and level, `park` at each of its two waits, and that neither is emitted at info. On Windows the `measure` workflow at the commit shows the test passing with the spans, the work desktop's run gives the rows first, as steering, and the rows of record are the laptop's session's | 7 |

=== docs/plans/milestone-2.md lines 285 to 287: section 3: the measurement commit, a measured input, and what a figure before it is
OLD:
- The measurement commit is the first commit of main that holds `frame-impl` and `measure-m2`. The
  quiet window, the laptop's session and session C run there (section 4). A lane that merges after
  it and edits a measured crate runs the measurements it touches and reports them as steering.
NEW:
- The measurement commit is the head of main at which `check`, the `measure` workflow and the
  Windows pin run last, once main holds the three lanes that change a measured input after
  `measure-role`, which are `present-encoder`, `park-stamp` and `layer-windows`, and the fix round
  of each defect the commit waits for (D165). A measured input is anything a measurement builds,
  runs or computes a figure with, among them the crates under `crates/` and `game/`, `tools/cook`
  and `tools/xtask`, the Slang and layer pins, `shaders/`, `vendor/`, the manifests and
  `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, `.config/nextest.toml` and
  `.gitattributes`; where the list is short the definition holds. The coordinator records the
  commit's hash in the verification row that names it once `check` and the `measure` workflow are
  green at it and the work desktop's run of the Windows pin on its RX 9060 XT has passed at it
  (section 4). The pass is at the commit itself: the commit the pin's report names is the commit
  recorded, and a pass at another commit does not count. The order is the owner's ruling: the lanes
  the commit holds merge first, `check` and `measure` run at that head, and the pin runs there last,
  so that the owner's remote session is spent once where it can be. The coordinator chooses that
  head once the work desktop's early runs have sent their last report, and holds main from the start
  of `check` there until the hash is recorded: while the hold stands nothing merges to main but
  documents, which are the files under `docs/` and the `*.md` files, and nothing is pushed to it, a
  document neither. A push made against the hold moves the commit to the new head: `check`,
  `measure` and the pin run there, each again where it had run. The commit waits for a defect that
  an early run of the work desktop's finds, or that `check`, `measure` or the pin fails on: it is a
  fix round of the lane whose code it is, whichever crate that is. A run that fails ends the hold,
  and so does such a defect reported while it stands: once the fix round has merged the head is
  chosen again, and the three run there in the same order. The commit does not wait for a defect
  that only the work desktop's rehearsal finds and that fails none of the pin's four conditions
  (section 4): the owner ruled so. Its fix round merges outside the hold: before the head is chosen,
  or after the hash is recorded, as a lane that merges after the commit does. One that fails one of
  the four, found before the hash is recorded, is the pin's failure. The quiet window checks the
  commit out as soon as it is recorded, and the laptop's session and session C from 7 December 2026
  (section 4). A defect of a measured input that only NVIDIA's or Intel's driver shows, neither of
  which the work desktop has, moves the commit to its fix: the quiet window runs again there, or the
  owner rules that its files stand. A commit moved so is recorded as the first is, with the laptop's
  own gate and short run at the moved head standing for the pin: the owner ruled so, the work
  desktop is not needed, and the hold runs from `check` at that head to the hash's recording. The
  laptop's record at the moved commit follows the quiet window's run there, or the owner's ruling
  that the quiet window's files stand. A defect that only session C's driver shows moves the commit
  as the laptop's does, and whether session C then sits again at the moved head or its files stand
  is the owner's to rule when it arises. A file is the record at the commit. A file of a later
  commit is the record as `-at-<commit>` only where the diff from the measurement commit changes no
  measured input the file's measurement reads, and its row gives the diff; otherwise the owner's
  ruling says which file is the record (section 7). A figure one of section 4's measurements gave
  before it is steering: it stays in its row, proves no criterion and stands for no file of record.
  A run on the work desktop is steering at any commit: it closes no row, and none of its files is
  checked in. A row that asks a machine for a run and names no commit is closed by that machine's
  run and records the commit it ran at. A lane that merges after it and changes a measured input
  runs the measurements it touches and reports them as steering.

=== docs/plans/milestone-2.md line 297: section 4's table of machines: the work desktop, after the desktop's row
OLD:
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until its first sitting, which returns them (decision 5.19) | The x86 desktop figures section 20 asks for, and the x86 half of milestones 0 and 1 (D151). A second discrete GPU | Anything without the owner at it |
NEW:
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until its first sitting, which returns them (decision 5.19) | The x86 desktop figures section 20 asks for, and the x86 half of milestones 0 and 1 (D151). A second discrete GPU | Anything without the owner at it |
| The work desktop: the owner's desktop at work, which is not the plan's desktop, lent for early runs (D165) and driven through its own agent session over a remote desktop session. AMD Ryzen 9 9950X, sixteen cores of one kind, 64 GB, an AMD Radeon RX 9060 XT on AMD's driver 26.11.2, which reports Vulkan 1.4.359, and the processor's integrated Radeon, Windows 11 Enterprise 25H2, the system's loader at 1.4.357, and a capture program's implicit layer, `VK_LAYER_OBS_HOOK`, registered for the whole machine. Its controls stand, and no command asks for an exception to one or overrides one: endpoint protection that scans in real time, PowerShell's execution policy as the machine has it, a lock after fifteen minutes without input, updates installed on a daily schedule, and the default two-second GPU timeout | As steering alone, under the machine name `work-desktop-windows`, and `work-desktop-windows-igpu` on its integrated Radeon: `lactose-rhi-vulkan` on a Windows driver while the owner's machines are out of reach, first by hand with no validator and then under the Windows pin; the tool's check of the installed layer; the exclusive layer path against a registered implicit layer; the canaries on AMD's driver; the laptop's and session C's commands, rehearsed; and a first look at the measurements on an x86 desktop part | The record: none of its files is checked in and no run of its closes a row. NVIDIA's driver or Intel's. A hybrid part: nothing of row 63 or of an efficiency core. A quiet figure: the scanning is in every one. A panel: its windows are a remote session's, so a present's timing, frame pacing and the latency tool's lines mean nothing there, and a run that opens a window needs the session connected and its window not minimised. A crash dump: `LocalDumps` is not set there |

=== docs/plans/milestone-2.md lines 308 to 312: section 4: the quiet window
OLD:
- *The quiet window*, the coordinator's, at the measurement commit. Every track stops, the machine
  holds `caffeinate -i -s -d`, and `cargo xtask measure --all` runs once. It writes milestone 2's
  six files beside milestone 1's, `gpu-cost` under MoltenVK and under KosmicKrisp. The WHISK
  file's 60 Hz P99 is read against D156: under 30 µs, the planner proposes 35 µs again; the system
  daemon's CPU time is read before and after the run, as the remeasurement of 17 November 2026 did.
NEW:
- *The quiet window*, the coordinator's, at the measurement commit, the first sitting there, as soon
  as the commit is recorded (D165). Every track stops, the machine is on mains power with low power
  mode off and its screen unlocked, and a detached `caffeinate -dims -t <seconds>` holds it awake
  for the window's length. The owner first resizes the latency tool's window by a corner, for row
  209, and then leaves the machine. `cargo xtask measure --all` runs once. It writes milestone 2's
  six files beside milestone 1's, with `triangle`, `gpu-cost` and `frame-pace` under MoltenVK and
  under KosmicKrisp, and the `rhi-conformance` files. The renderer's cost tests run once after it,
  for rows 220 and 238. The WHISK file's 60 Hz P99 is read against D156: under 30 µs, the planner
  proposes 35 µs again; the system daemon's CPU time is read before and after the run, as the
  remeasurement of 17 November 2026 did. Its files are read before the laptop's session starts.

=== docs/plans/milestone-2.md lines 313 to 315: section 4: the laptop's session
OLD:
- *The laptop's session*, at the same commit, with nothing else running there: the same command,
  and `cargo xtask gpu --driver system`. Its `triangle` file is criterion 1's proof on Windows
  (the owner's answer 2).
NEW:
- *The laptop's session*, from 7 December 2026, at the measurement commit, after the quiet window's
  files are read, with nothing else running there (D165). The owner installs the pinned Vulkan SDK
  and sets `LocalDumps` (row 190) as administrator, and stays for its first minutes. The session
  runs in a shell that is not elevated. First, with the owner there: the checkouts' refresh (row
  109), before anything is built; the SDK as installed, the loader's implicit layers, and each GPU's
  queue families and memory types; and the latency tool under the owner's hand (row 209). Then
  `cargo xtask gpu --driver system`, plain and under GPU-assisted validation, on each GPU chosen by
  `--adapter`, which is the first run of `lactose-rhi-vulkan` on NVIDIA's and on Intel's Windows
  driver, and a short run of `triangle`, `gpu-cost`, `frame-pace` and `rhi-conformance`, thrown
  away. A run that fails there is read before any file of record is written, and a defect of a
  measured input moves the commit (section 3). Then the record: `cargo xtask measure --all` on the
  RTX 4070, and the same four again on the integrated GPU under a machine name of its own.
  `rhi-conformance` is the gate's run, plain and under GPU-assisted validation. Its `triangle` file
  on the RTX 4070 is criterion 1's proof on Windows (the owner's answer 2). Last: `cargo xtask gpu
  --client` on each GPU, as labelled data; `eco-qos`'s before-and-after runs at their own commits
  (row 63); the window cases (rows 208, 210, 234 and 236); and the barrier mutations once.

=== docs/plans/milestone-2.md lines 326 to 330: section 4: session C
OLD:
- *Session C*, the owner's, at the measurement commit, on the desktop under each system it boots:
  `cargo xtask measure --all`. Its `extract-50k` and `remote-upload-5k` files are the x86 desktop's
  figures. The command writes milestone 1's files again, which shows what milestone 2's lanes moved
  on x86. Under a system that is not a client platform, the measurements that need a window are
  skipped and the file says so.
NEW:
- *Session C*, the owner's, at the measurement commit, last and from 7 December 2026, on the desktop
  under each system it boots, after its first sitting (D165): `cargo xtask measure --all`, whose
  `rhi-conformance` is the gate's run, plain and under GPU-assisted validation; lane
  `present-encoder`'s `cargo xtask gpu --client`, for the row of `frame-impl`'s runs on the x86
  desktop; the renderer's cost tests; the barrier mutations, each beside its control (rows 225 and
  239 to 241); and `eco-qos`'s three `whisk` runs at their own commits if the first sitting shows a
  hybrid processor (row 63). Its `extract-50k` and `remote-upload-5k` files are the x86 desktop's
  figures. The command writes milestone 1's files again, which shows what milestone 2's lanes moved
  on x86. Under a system that is not a client platform the session names its measurements and leaves
  out `triangle` and `frame-pace`: `lactose-platform` has not been run there, and nothing skips
  them.

=== docs/plans/milestone-2.md lines 332 to 333: section 4: the work desktop's runs, and what each sitting needs of the owner
OLD:
Only the desktop's two sittings need the owner at a desk. The laptop's runs need the laptop awake
with its agent session open, and a run that opens a window needs a person logged in at it.
NEW:
**The work desktop's runs are steering, and are no sitting (D165).** The owner's Windows machines
are out of reach until 7 December 2026, and the work desktop runs first what they would have. From
26 November 2026, at main's head and with no validation layer on the machine: the device tests
`cargo xtask gpu` lists, without their canaries, by hand on its RX 9060 XT, which is the first run
of `lactose-rhi-vulkan` on a Windows driver; `lactose-platform`'s window cases; and `cargo xtask
measure --all --no-device`. Once the lanes the measurement commit holds have merged, `check` and the
`measure` workflow are green at that head and the pinned Vulkan SDK is installed there, the Windows
pin, at that head: the installed layer as the tool checks it, `cargo xtask gpu --driver system`
plain and under GPU-assisted validation, and a short run of `triangle`, `gpu-cost`, `frame-pace` and
`rhi-conformance`. After the pin, as rehearsal of the laptop's session and of session C: `cargo
xtask measure --all`, `cargo xtask gpu --client` and the barrier mutations. The pin has passed when
four conditions hold: the tool takes the installed layer, each canary is heard on the RX 9060 XT,
seven under the plain validators and two under GPU-assisted validation, no case of either gate run
fails, and the short run's measured runs hear no message. A failure is reported with whose it looks
to be: the engine's, AMD's driver's, the remote session's or one of the machine's controls'. A run
that a control cut is run again. The owner may rule a failed case the machine's, and the pin passed
with it, only where the case presents into the remote session: never the layer's check, a canary or
a message. The laptop's gate runs that case first, and where it fails there it is a defect of a
measured input and moves the commit (section 3). A defect that an early run finds, or that the pin
fails on, is a fix round of the lane whose code it is, whichever crate that is: it merges before the
measurement commit is recorded, and after its merge `check` and `measure` run again, and then the
pin. The commit does not wait for a defect that only the rehearsal finds and that fails none of the
four conditions, and section 3 says when its fix round merges. Every measurement's file carries the
machine name `work-desktop-windows`, or `work-desktop-windows-igpu` where it ran on the integrated
Radeon, none is checked in, and a report names no host, user or domain.

Only the desktop's two sittings need the owner at a desk. The laptop's session needs the owner as
administrator once and the owner's hand once in its first minutes. The quiet window needs the
owner's hand once at its start, and then the Mac unused. The work desktop's runs need the owner to
approve its installs at their prompts and to keep its remote session connected, with its window open
and not minimised, while a run opens a window. The laptop's runs need the laptop awake with its
agent session open, and a run that opens a window needs a person logged in at it. On each machine
the toolchain `rust-toolchain.toml` pins is installed by name before the first build, by `rustup
toolchain install` in the clone: rustup's install of it on a first use is deprecated.

=== docs/plans/milestone-2.md line 341: section 4, row `record-stall`: the gate's rows come from two spans, on every host
OLD:
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without, and on Windows the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is the `SetEvent` item of row 164's unmeasured list (D162) | Nothing: recorded. It sets `lactose-app`'s default reservation, `RECORD_RESERVATION`, which is one until the file is read (decision 5.16, D162) |
NEW:
| `record-stall` | The composed tick of milestone 1, 100k rows and 2,000 awake bodies, stepping beside a frame's record of the minimal renderer's passes on null, at a physics reservation of one frame worker and of two. The file gives the record phase's median and P99 with the physics phase beside it and without, and, from `lactose-task`'s two debug spans, for each worker parked across an opening of the gate at a `block_in_place`'s end, in the idle protocol or on a scope's latch, the time from the opening's start to that worker's wake, beside it the opening's own length, on the opener's side, and the time to the record's next job where a worker that was parked took it, each with its count, which on Windows is the `SetEvent` item of row 164's unmeasured list (D162, D165) | Nothing: recorded. It sets `lactose-app`'s default reservation, `RECORD_RESERVATION`, which is one until the file is read (decision 5.16, D162) |

=== docs/plans/milestone-2.md lines 355 to 356: section 4: what writes `rhi-conformance`
OLD:
`cargo xtask gpu --driver <name>` writes `rhi-conformance-<driver>-<machine>-<date>.md`: the cases
run, the capabilities reported, each canary's message, and the count of other messages by source.
NEW:
`cargo xtask measure rhi-conformance` writes `rhi-conformance-<driver>-<machine>-<date>.md`, with
`-gpu-assisted` after the driver for that run: the cases run, the capabilities reported, each
canary's message, and the count of other messages by source, under a header that names the adapter,
the device as it reports itself and the validators the run was under (D165). `cargo xtask gpu` keeps
its own report under the target directory.

=== docs/plans/milestone-2.md lines 1021 to 1024: section 5: the owner's actions, in order and with their dates
OLD:
The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); the desktop's first sitting, once the M1's re-run is on main, with `LocalDumps`
set on the desktop; the laptop's checkouts refreshed once (D148) and `LocalDumps` set on the laptop,
before its first milestone 2 run; session C at the measurement commit.
NEW:
The owner's actions, in order: the upgrade to macOS 27.2.1 and the Xcode install, before the first
lane (section 9); from 26 November 2026, on the work desktop, its installs approved at their
prompts, which are the MSVC C++ build tools, rustup, nextest and Python, and its remote session kept
connected, its window open and not minimised, while a run opens a window; once `layer-windows` has
merged, the pinned Vulkan SDK installed there, approved at its prompt; at the measurement commit,
one window resized at the quiet window's start and the Mac then left unused; from 7 December 2026,
the desktop's first sitting, with `LocalDumps` set on the desktop and the pinned Vulkan SDK
installed under its Windows; the laptop's session, with the pinned Vulkan SDK installed and
`LocalDumps` set as administrator, its checkouts refreshed (D148) and one window dragged in its
first minutes; then session C (D165).

=== docs/plans/milestone-2.md lines 1884 to 1887: section 7, step 3: which nightly
OLD:
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and a nightly that holds the client's
   recording test has compared it equal on three platforms, with each platform's release baseline
   equal to its dev one. The run ids are in the rows. No check job's median over its three most
   recent warm runs of main is past ten minutes, or the owner has ruled on the budget.
NEW:
3. The `gpu (lavapipe)` and `shaders` jobs are green on main, and the first nightly at or after the
   measurement commit whose four `sour` jobs pass, which holds the client's recording test, has
   compared it equal on three platforms, with each platform's release baseline equal to its dev one
   (D165). Another job of that nightly that fails is a finding in its own row and holds nothing of
   this step. The run ids are in the rows. No check job's median over its three most recent warm
   runs of main is past ten minutes, or the owner has ruled on the budget.

=== docs/plans/milestone-2.md line 1888: section 7, step 4: every row gated at milestone 2
OLD:
4. The coordinator's pass has left every row of section 2 Done or re-gated.
NEW:
4. The coordinator's pass has left every row of `docs/verification.md` whose gate names milestone 2
   Done or re-gated, section 2's rows among them (D165).


=== docs/verification.md line 21: verification row 21: the Windows item is two debug spans' (lane `park-stamp`)
OLD:
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153). Until the file is read the default is one frame worker: `lactose_app::RECORD_RESERVATION`, which `WorldConfig::shown()` gives the physics configuration (D162). The file is taken at a reservation of one and of two, and on Windows it also gives the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is what the `SetEvent` for each frame worker in `Pool::open_gate` costs a record (the keeper's unmeasured list, read by D162) | 2. **In part** (branch `measure-m2`, round one and its fix round; not the record): `stall::the_record_beside_the_physics_phase` of `lactose-app-client`'s `integration` target records `ClientRender` on null, each record one `run_job` timed from the job's start, its start wait from `run_job`'s call beside it, beside the composed tick timed inside its bounded future; the physics phase is `physics_api::sync`, `step` and `write_back`, from the first entry into any to the last exit, matched by name in the executor's node spans and `lactose-ecs`'s system spans. In the lane's fix-round `--all` run on the M1 (commit 7e3b62a, after fix rounds one and two), at load averages 4.32 5.02 6.16 before and 4.53 5.05 6.16 after, 60 ticks at each reservation: at one, the record's median and P99 beside the physics phase 7.7 and 49.5 µs, their start wait's 1.1 and 20.8 µs, the phase's median 2,502.7 µs; at two, 7.6 and 43.3 µs, the start wait's 1.1 and 19.6 µs, the phase's 2,737.1 µs; with no tick, 6.8 and 37.3 µs, the start wait's 0.8 and 12.3 µs. The Windows item is not measured: it needs a stamp where a frame-critical worker parks and wakes, inside `lactose-task`, whose hooks only its own unit tests compile, so a public surface of `lactose-task` first (D162 addendum at `measure-m2`, round two). `RECORD_RESERVATION` stays one until the quiet window's file is read |
NEW:
| Physics phase stall | The stall the physics phase imposes on render recording during the overlap; nothing records a frame before `render-graph` exists; result file `record-stall`, recorded with no threshold, sets `lactose-app`'s default reservation (D153). Until the file is read the default is one frame worker: `lactose_app::RECORD_RESERVATION`, which `WorldConfig::shown()` gives the physics configuration (D162). The file is taken at a reservation of one and of two, and on Windows it also gives the time from each `block_in_place`'s end to the record's next job on a worker that was parked, which is what the `SetEvent` for each frame worker in `Pool::open_gate` costs a record (the keeper's unmeasured list, read by D162) | 2. **In part** (branch `measure-m2`, round one and its fix round; not the record): `stall::the_record_beside_the_physics_phase` of `lactose-app-client`'s `integration` target records `ClientRender` on null, each record one `run_job` timed from the job's start, its start wait from `run_job`'s call beside it, beside the composed tick timed inside its bounded future; the physics phase is `physics_api::sync`, `step` and `write_back`, from the first entry into any to the last exit, matched by name in the executor's node spans and `lactose-ecs`'s system spans. In the lane's fix-round `--all` run on the M1 (commit 7e3b62a, after fix rounds one and two), at load averages 4.32 5.02 6.16 before and 4.53 5.05 6.16 after, 60 ticks at each reservation: at one, the record's median and P99 beside the physics phase 7.7 and 49.5 µs, their start wait's 1.1 and 20.8 µs, the phase's median 2,502.7 µs; at two, 7.6 and 43.3 µs, the start wait's 1.1 and 19.6 µs, the phase's 2,737.1 µs; with no tick, 6.8 and 37.3 µs, the start wait's 0.8 and 12.3 µs. The Windows item is lane `park-stamp`'s (D165): `lactose-task` emits two debug spans under target `whisk`, `open_gate` around `Pool::open_gate` at a `block_in_place`'s end and `park` around every wait a worker makes without a permit, the idle protocol's whatever the worker's look saw and a scope opener's on its latch, and the test's subscriber gives, for each worker parked across an opening, the time from the opening's start to that worker's wake, with its count, and beside it the opening's own length, on the opener's side, and, where a worker that was parked took the record's next job, that job's start and start wait, with their count. The rows are written on every host, and the laptop's and the desktop's Windows files hold the item. No hook and no cargo feature is added, which D162's addendum at `measure-m2`, round two, named as the two forms. `RECORD_RESERVATION` stays one until the quiet window's file is read |

=== docs/verification.md line 150: verification row 150, the whole row: the laptop's four hardware questions are not shown
OLD:
| The RHI spike ran under MoltenVK alone (D139): its result file marks each "no" as MoltenVK's, against MoltenVK's published limits and the feature's Vulkan requirement, or as a design question. **The MoltenVK half is done:** `docs/results/spike-rhi-apple-m1-2026-11-15.md` shows the decision's ten items under MoltenVK 1.4.2; its four "no"s on the list and GPU-assisted validation's are MoltenVK's, and its design questions are the lavapipe run's to settle, or the desktop GPU's or KosmicKrisp's where it names them. Run the same list of architecture section 12.1 under lavapipe with `rhi-vulkan`'s first Linux job, as the conformant reference, and settle from it each design question the file gives it. Lane `vulkan` closes it (D153): the design questions lavapipe can answer, from its run; the four hardware questions probed on the laptop's devices and recorded, a run that shows nothing discharging no rule of the null backend; device-only staging as a case on the M1; question 2 answered under KosmicKrisp on the M1 if that driver offers a lazily allocated type, and open on Vulkan if not. **On the M1** (branch `vulkan`): a buffer in device-only memory filled through a staging copy and read back through a copy runs as a case of the suite under MoltenVK and KosmicKrisp (`device_memory_is_filled_through_a_staging_copy`, and `device_buffer_is_filled_through_a_staging_copy` among the device tests). KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. The lavapipe run is the `gpu (lavapipe)` job's first run, after the merge, and the laptop's four hardware questions are owed | 2. **The lavapipe half is done** (CI 45116598474 at `21631f5`, the `gpu (lavapipe)` job's first green run, job 137500764827; the run at `ae399a7`, 45108422263, failed): its `gpu-lavapipe` report, under the layer 1.4.363.1's core, synchronisation and thread-safety validation with every canary heard but the race across queues, skipped since compute shares the graphics queue, and no case failed, gives llvmpipe of Mesa 26.0.8 one queue (compute and present `Shared`), coherent host memory, no lazily allocated type (`memoryless_colour`, `memoryless_depth` and `memoryless_survives_local_read` false), multi-draw and indirect count. Of the spike's eight design questions: 1, the local-read barrier dropped, is not shown, since its case is a check the null backend alone makes and skips under lavapipe, so `rhi-null`'s `local` rule stays the only guard; 2 stays open on Vulkan, lavapipe offering no lazily allocated type; 3 is settled, memoryless being a request: lavapipe reports it not honoured (`memoryless_outcome_is_queryable_and_as_reported`), the G-buffer is lit in its pass in ordinary memory (`memoryless_g_buffer_is_lit_in_its_pass`), and a memoryless attachment stored or kept is refused; 4 is settled for the facade: a byte flipped, a truncation, a wrong checksum and a stale header each start the cache `Rejected` before the driver sees it, and garbage sealed behind lavapipe's own header makes the device with no message (`cache_that_fails_a_check_starts_empty`), whether lavapipe took or refused it not being in the report, so the seal and the header check are the guard; 5 is not this run's, which runs without GPU-assisted validation: the nightly's `gpu (lavapipe, GPU-assisted)` (45609400690 at `98d11cf`, job 139188694423) heard the address canary's read past a buffer and the heap canary's slot past the heap, and no case failed; 6 is not shown, compute having no queue of its own, so the pairing stays null's rule and the transfer's effect a hardware question; 7 is settled: indirect count is reported, and a count of one capped at two draws the first record alone, as the CPU reference does (`indirect_count_draws_where_reported`); 8 is not shown, lavapipe's host memory being coherent, where flush and invalidate are accepted (`flush_and_invalidate_are_accepted_where_memory_is_coherent`), so the non-coherent path is a hardware question |
NEW:
| The RHI spike ran under MoltenVK alone (D139): its result file marks each "no" as MoltenVK's, against MoltenVK's published limits and the feature's Vulkan requirement, or as a design question. **The MoltenVK half is done:** `docs/results/spike-rhi-apple-m1-2026-11-15.md` shows the decision's ten items under MoltenVK 1.4.2; its four "no"s on the list and GPU-assisted validation's are MoltenVK's, and its design questions are the lavapipe run's to settle, or the desktop GPU's or KosmicKrisp's where it names them. Run the same list of architecture section 12.1 under lavapipe with `rhi-vulkan`'s first Linux job, as the conformant reference, and settle from it each design question the file gives it. Lane `vulkan` closes it (D153): the design questions lavapipe can answer, from its run; the four hardware questions, which were to be probed on the laptop's devices, recorded as not shown (D165), a run that shows nothing discharging no rule of the null backend; device-only staging as a case on the M1; question 2 answered under KosmicKrisp on the M1 if that driver offers a lazily allocated type, and open on Vulkan if not. **On the M1** (branch `vulkan`): a buffer in device-only memory filled through a staging copy and read back through a copy runs as a case of the suite under MoltenVK and KosmicKrisp (`device_memory_is_filled_through_a_staging_copy`, and `device_buffer_is_filled_through_a_staging_copy` among the device tests). KosmicKrisp offers no lazily allocated type, so question 2 stays open on Vulkan. The lavapipe run is the `gpu (lavapipe)` job's first run, after the merge, and the laptop's four hardware questions are not shown (D165): no test makes any of the four misuses on a device. The ownership case always performs the transfer, and where compute has no queue family of its own it crosses two queues of one family; the backend takes a coherent host-visible type wherever a device offers one, so the unflushed-range case skips; the cook refuses an undecorated heap index; and null's rule alone holds the wrong layout. The laptop's session runs the gate, plain and GPU-assisted, on each of its GPUs as criterion 2's evidence there, and records each GPU's queue families and whether a host-visible type of it is not coherent. A run that shows nothing discharges no rule of null (decision 5.17) | 2. **The lavapipe half is done** (CI 45116598474 at `21631f5`, the `gpu (lavapipe)` job's first green run, job 137500764827; the run at `ae399a7`, 45108422263, failed): its `gpu-lavapipe` report, under the layer 1.4.363.1's core, synchronisation and thread-safety validation with every canary heard but the race across queues, skipped since compute shares the graphics queue, and no case failed, gives llvmpipe of Mesa 26.0.8 one queue (compute and present `Shared`), coherent host memory, no lazily allocated type (`memoryless_colour`, `memoryless_depth` and `memoryless_survives_local_read` false), multi-draw and indirect count. Of the spike's eight design questions: 1, the local-read barrier dropped, is not shown, since its case is a check the null backend alone makes and skips under lavapipe, so `rhi-null`'s `local` rule stays the only guard; 2 stays open on Vulkan, lavapipe offering no lazily allocated type; 3 is settled, memoryless being a request: lavapipe reports it not honoured (`memoryless_outcome_is_queryable_and_as_reported`), the G-buffer is lit in its pass in ordinary memory (`memoryless_g_buffer_is_lit_in_its_pass`), and a memoryless attachment stored or kept is refused; 4 is settled for the facade: a byte flipped, a truncation, a wrong checksum and a stale header each start the cache `Rejected` before the driver sees it, and garbage sealed behind lavapipe's own header makes the device with no message (`cache_that_fails_a_check_starts_empty`), whether lavapipe took or refused it not being in the report, so the seal and the header check are the guard; 5 is not this run's, which runs without GPU-assisted validation: the nightly's `gpu (lavapipe, GPU-assisted)` (45609400690 at `98d11cf`, job 139188694423) heard the address canary's read past a buffer and the heap canary's slot past the heap, and no case failed; 6 is not shown, compute having no queue of its own, so the pairing stays null's rule and the transfer's effect a hardware question; 7 is settled: indirect count is reported, and a count of one capped at two draws the first record alone, as the CPU reference does (`indirect_count_draws_where_reported`); 8 is not shown, lavapipe's host memory being coherent, where flush and invalidate are accepted (`flush_and_invalidate_are_accepted_where_memory_is_coherent`), so the non-coherent path is a hardware question |

=== docs/verification.md line 170: verification row 170: its nightly is not exit step 3's
OLD:
| SOUR hashes come from a dev build (project review 2, X5): a result that depends on debug assertions or overflow checks, such as a stale arena read that poisons only under `debug_assertions` (`crates/core/src/arena.rs`), could differ in release unseen. Lane `exit-tooling` (`1dd5dab`) adds `cargo xtask sour run --release` (nextest `--cargo-profile release`, output in `target/sour-release`); each nightly sour job runs one architecture-mode run with it and compares that baseline with the platform's dev baseline by `sour compare`. Shown on the Apple M1: the release baseline and `sour run --pr`'s dev baseline agree on all 18,446 hashes (load averages 14.72 22.97 15.18 to 13.05 19.79 15.02). The same lane records the bone scene (X4), `lactose_app::cost::bone_bodies_on_rapier_are_recorded` and its `::physics` stream: 20 characters with bone bodies on Rapier over 80 ticks, 0.229 s under nextest (load 1.93 5.73 5.28), among those hashes. **Done**: nightly run 44067232715 at `a539d60`, the first after the merge, held release equal to dev on linux-x64, windows-x64 and macos-arm64 (each platform's `sour compare` of its release baseline with its dev baseline), and its cross-platform job found linux-x64's hashes equal to windows-x64's and to macos-arm64's, the bone scene's streams among them | 2 |
NEW:
| SOUR hashes come from a dev build (project review 2, X5): a result that depends on debug assertions or overflow checks, such as a stale arena read that poisons only under `debug_assertions` (`crates/core/src/arena.rs`), could differ in release unseen. Lane `exit-tooling` (`1dd5dab`) adds `cargo xtask sour run --release` (nextest `--cargo-profile release`, output in `target/sour-release`); each nightly sour job runs one architecture-mode run with it and compares that baseline with the platform's dev baseline by `sour compare`. Shown on the Apple M1: the release baseline and `sour run --pr`'s dev baseline agree on all 18,446 hashes (load averages 14.72 22.97 15.18 to 13.05 19.79 15.02). The same lane records the bone scene (X4), `lactose_app::cost::bone_bodies_on_rapier_are_recorded` and its `::physics` stream: 20 characters with bone bodies on Rapier over 80 ticks, 0.229 s under nextest (load 1.93 5.73 5.28), among those hashes. **Done**: nightly run 44067232715 at `a539d60`, the first after the merge, held release equal to dev on linux-x64, windows-x64 and macos-arm64 (each platform's `sour compare` of its release baseline with its dev baseline), and its cross-platform job found linux-x64's hashes equal to windows-x64's and to macos-arm64's, the bone scene's streams among them | 2. **Done** for release against dev. The nightly of the plan's exit step 3 is a later one, the first at or after the measurement commit whose four `sour` jobs pass (D165), and its run id is in the row of the client's recording test |

=== docs/verification.md line 232: verification row 232: the nightly exit step 3 waits for
OLD:
| `lactose-app-client` is both a shipped binary and a harness package (D162: criterion 3(c)'s recorded test is in the module `recorded` of its `integration` target, since that package alone may name the real render side in normal code). `cargo xtask features` then compares its boundary tree in full with its dev-dependencies included, against `lactose-app-headless`'s. Owed by `frame-impl`: the comparison passes with `lactose-rhi-null` and the test's other dev-dependencies in the tree. If a dev-dependency moves the boundary tree, that is a finding for the planner, and D162's placement of the test is what changes **Answered** (branch `frame-impl`, fix rounds 1 and 2). At round two `cargo xtask features` reported six findings, one cause: `bitflags` 2.13.2, in the boundary tree through `lactose-physics-rapier` (`parry3d-f64` builds it without `std`), was built with `std` for `lactose-app-client` and without it for the other roots, on aarch64-apple-darwin and x86_64-unknown-linux-gnu. No dev-dependency moved it, so D162's placement of the test did not: with normal edges alone (`cargo tree -e normal,features -i bitflags`) the window's dependencies ask `std`, `objc2-app-kit` under winit through `lactose-platform` and `objc2-quartz-core` under `raw-window-metal` through `lactose-rhi-vulkan` on macOS, and `rustix` on Linux, so any feature root that links a window moves the tree. `lactose-physics-rapier` names `bitflags` through the workspace, pinned `=2.13.2` with `default-features = false, features = ["std"]`, so every root builds it alike, and `crate-tiers.toml` lists it among the crate's direct dependencies with that reason. `std` keeps a parse error's text and implements `std::error::Error` for it, and changes no flag value, bit operation or iteration order. On the branch `cargo xtask features` and `cargo xtask boundary` both pass, and with them `lactose-xtask`'s `workspace::the_feature_check_passes_with_no_roots_and_reads_real_trees`; `boundary::tests::the_physics_backend_may_not_name_the_solvers_math`, which reads the real tier file, expects `bitflags` on the crate's list with the rest; `cargo xtask vendor` and `cargo deny check` accept the pin. A pin moves with `parry3d-f64`'s `bitflags` | 2 |
NEW:
| `lactose-app-client` is both a shipped binary and a harness package (D162: criterion 3(c)'s recorded test is in the module `recorded` of its `integration` target, since that package alone may name the real render side in normal code). `cargo xtask features` then compares its boundary tree in full with its dev-dependencies included, against `lactose-app-headless`'s. Owed by `frame-impl`: the comparison passes with `lactose-rhi-null` and the test's other dev-dependencies in the tree. If a dev-dependency moves the boundary tree, that is a finding for the planner, and D162's placement of the test is what changes **Answered** (branch `frame-impl`, fix rounds 1 and 2). At round two `cargo xtask features` reported six findings, one cause: `bitflags` 2.13.2, in the boundary tree through `lactose-physics-rapier` (`parry3d-f64` builds it without `std`), was built with `std` for `lactose-app-client` and without it for the other roots, on aarch64-apple-darwin and x86_64-unknown-linux-gnu. No dev-dependency moved it, so D162's placement of the test did not: with normal edges alone (`cargo tree -e normal,features -i bitflags`) the window's dependencies ask `std`, `objc2-app-kit` under winit through `lactose-platform` and `objc2-quartz-core` under `raw-window-metal` through `lactose-rhi-vulkan` on macOS, and `rustix` on Linux, so any feature root that links a window moves the tree. `lactose-physics-rapier` names `bitflags` through the workspace, pinned `=2.13.2` with `default-features = false, features = ["std"]`, so every root builds it alike, and `crate-tiers.toml` lists it among the crate's direct dependencies with that reason. `std` keeps a parse error's text and implements `std::error::Error` for it, and changes no flag value, bit operation or iteration order. On the branch `cargo xtask features` and `cargo xtask boundary` both pass, and with them `lactose-xtask`'s `workspace::the_feature_check_passes_with_no_roots_and_reads_real_trees`; `boundary::tests::the_physics_backend_may_not_name_the_solvers_math`, which reads the real tier file, expects `bitflags` on the crate's list with the rest; `cargo xtask vendor` and `cargo deny check` accept the pin. A pin moves with `parry3d-f64`'s `bitflags` | 2. **Owed, a nightly's**: the first nightly at or after the measurement commit whose `sour (linux-x64)`, `sour (windows-x64)`, `sour (macos-arm64)` and `sour (architecture)` jobs pass (D165) compares the test's two streams equal on the three platforms, with each platform's release baseline equal to its dev one, and its run id is written here; another job of that run that fails is a finding in its own row. Nightly 45609400690 at `98d11cf`, the last before `ad1a99d`, is older than the test, which merged at `55ffdda` |

=== docs/verification.md line 243: verification row 243: `--adapter`, the pin's lane, and whose run it is
OLD:
| Criterion 1 on the Windows laptop (decision 5.1, D162): `lactose-app-client --fixed 600` on each of the laptop's GPUs, chosen by `VK_LOADER_DRIVERS_SELECT`, under the Khronos layer's core, synchronisation and thread-safety validation, each validator's canary run in a process of its own beside it. **Owed, the owner's**: the commands from a clean checkout are the coordinator's, from lane `frame-impl`, and `cargo xtask gpu` does not run there, since the pinned layer has no Windows build. It passes where every canary reports its own identifier and nothing else, and the run prints one `Outcome` with `counts`, `surely` with every entry above zero and `messages: 0`, and nothing at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live, and every pixel is as the reference rasterised from those placements says. The M1 gave `Counts { drawn: 1044, clear: 195304, margin: 260 }` under both of its drivers; the counts are the reference's, from the placements and the latched camera, so the same are expected. The run's two `info` lines give the device and its queue layout, which say whether the laptop's devices serve the present role from the graphics queue (row 176). `cargo xtask measure triangle` (branch `measure-m2`, round two) is the run on a host the layer is pinned for, and met the criterion on the M1 under both drivers with those counts; `tools/xtask/vulkan-layer.toml` pins no layer for Windows, so the laptop's file needs a pin for `windows-x86_64` first | 2 |
NEW:
| Criterion 1 on the Windows laptop (decision 5.1, D162): `lactose-app-client --fixed 600` on each of the laptop's GPUs, chosen by `--adapter`, which the device's name must contain, under the Khronos layer's core, synchronisation and thread-safety validation, each validator's canary run in a process of its own beside it. **Owed, the laptop's, from 7 December 2026** (D165): lane `layer-windows` pins the layer for Windows, the work desktop runs the pin first, as steering, and `cargo xtask measure triangle --driver system --adapter <name>` on each GPU in the laptop's session at the measurement commit is the run; lane `frame-impl`'s by-hand form is dropped. It passes where every canary reports its own identifier and nothing else, and the run prints one `Outcome` with `counts`, `surely` with every entry above zero and `messages: 0`, and nothing at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live, and every pixel is as the reference rasterised from those placements says. The M1 gave `Counts { drawn: 1044, clear: 195304, margin: 260 }` under both of its drivers; the counts are the reference's, from the placements and the latched camera, so the same are expected. The run's two `info` lines give the device and its queue layout, which say whether the laptop's devices serve the present role from the graphics queue (row 176). `cargo xtask measure triangle` (branch `measure-m2`, round two) is the run on a host the layer is pinned for, and met the criterion on the M1 under both drivers with those counts; `tools/xtask/vulkan-layer.toml` pins no layer for Windows at `1c62676`; lane `layer-windows` adds the pin (D165), and the RTX 4070's `triangle` file at the measurement commit is the criterion's proof on Windows | 2 |

=== docs/verification.md line 244: verification row 244: what session C runs of it, and what it cannot
OLD:
| Lane `frame-impl`'s runs on the x86 desktop (D162, decision 5.1): criterion 1 and the probe's paced runs under synchronisation validation (row 230), on a device whose present family is not its graphics family, and on lavapipe, whose roles share one queue. **Owed to the desktop sitting**, as `measure-m2` is; nothing of this lane ran on x86 | 2 |
NEW:
| Lane `frame-impl`'s runs on the x86 desktop (D162, decision 5.1): criterion 1 and the probe's paced runs under synchronisation validation (row 230), on a device whose present family is not its graphics family, and on lavapipe, whose roles share one queue. **Owed to session C** (D165), as `measure-m2` is: criterion 1 is `cargo xtask measure triangle` there, and the client's present under the gate's validators is lane `present-encoder`'s `cargo xtask gpu --client`, as shipped and lagged, on the desktop's GPU, with that device's queue families beside it. The probe's paced runs and lavapipe's half are not session C's: the probe is a build of a scratch copy that no checkout holds, and no runner of lavapipe's presents to a window. They are the row of the client's fault probe, gated at milestone 3. Nothing of this lane ran on x86 | 2 |

=== docs/verification.md: five rows appended at the end of the file, after the table's last row
NEW:
| The measurement commit of milestone 2 (D165): the head of main at which `check`, the `measure` workflow and the Windows pin run last, once main holds `present-encoder`, `park-stamp` and `layer-windows`, the three lanes that change a measured input after `measure-role`, of which `present-encoder` merged at `1c62676`, and the fix round of each defect the commit waits for. It is recorded once `check` and the `measure` workflow are green at it and the work desktop's run of the Windows pin on its RX 9060 XT has passed at it. The pass is at the commit itself: the commit the pin's report names is the commit recorded, and a pass at another commit does not count. The order is the owner's ruling: the lanes the commit holds merge first, `check` and `measure` run at that head, and the pin runs there last. The coordinator chooses that head once the work desktop's early runs have sent their last report, and holds main from the start of `check` there until the hash is recorded: while the hold stands nothing merges to main but documents, which are the files under `docs/` and the `*.md` files, and nothing is pushed to it, a document neither. A push made against the hold moves the commit to the new head: `check`, `measure` and the pin run there, each again where it had run. The commit waits for a defect that an early run of the work desktop's finds, or that `check`, `measure` or the pin fails on: it is a fix round of the lane whose code it is, whichever crate that is. A run that fails ends the hold, and so does such a defect reported while it stands: once the fix round has merged the head is chosen again, and the three run there in the same order. The commit does not wait for a defect that only the work desktop's rehearsal finds and that fails none of the pin's four conditions, which D165 names: the owner ruled so. Its fix round merges outside the hold: before the head is chosen, or after the hash is recorded, as a lane that merges after the commit does. One that fails one of the four, found before the hash is recorded, is the pin's failure. A measured input is anything a measurement builds, runs or computes a figure with, among them the crates under `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and layer pins, `shaders/`, `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, `.config/nextest.toml` and `.gitattributes`; where the list is short the definition holds. The quiet window on the M1 checks it out as soon as it is recorded, and from 7 December 2026 the laptop's session and session C on the desktop, in that order, each read before the next. A defect of a measured input that only NVIDIA's or Intel's driver shows, neither of which the work desktop has, moves the commit to its fix: the quiet window runs again there, or the owner rules that its files stand. A commit moved so is recorded as the first is, with the laptop's own gate and short run at the moved head standing for the pin: the owner ruled so, the work desktop is not needed, and the hold runs from `check` at that head to the hash's recording. The laptop's record at the moved commit follows the quiet window's run there, or the owner's ruling that the quiet window's files stand. A file is the record at it. A file of a later commit is the record as `-at-<commit>` only where the diff from it changes no measured input the file's measurement reads, and the row that cites the file gives the diff; otherwise the owner's ruling at the plan's exit step 7 names the record. A figure a measurement gave before it is steering and stands for no file of record, and a run on the work desktop is steering at any commit. A row that asks a machine for a run and names no commit is closed by that machine's run and records the commit it ran at. Two sittings keep commits of their own: the desktop's first at `a539d60` (D151), and `eco-qos`'s runs at `a75596b` and its first parent. To record here: the commit's hash; the date of the work desktop's pin run that passed, and the commit its report names, which is that hash; each fix round before the commit; each defect the rehearsal found that the commit did not wait for, with its fix round's merge; where the commit moved, the laptop's gate and short run that stood for the pin, and the quiet window's run there or the owner's ruling that its files stand; for each sitting its date, its load and the files it wrote; the ids of the `measure` workflow's run and of the first nightly at or after the commit whose four `sour` jobs pass | 2. **Owed**: no such commit exists at `1c62676` |
| The Khronos layer on Windows is pinned as installed (D165, D153; lane `layer-windows`): `library_sha256` is the SHA-256 of `Bin/VkLayer_khronos_validation.dll` as read on the development Mac, by the proposal and again by its GPU review, out of LunarG's installer `vulkansdk-windows-X64-1.4.363.1.exe`, which was fetched by the SHA-256 LunarG publishes for it and never run. No Windows host has checked it. The work desktop's pin run is the first to, on AMD's driver and as steering, and the measurement commit's row waits for its pass; the laptop's session, from 7 December 2026, is the check on NVIDIA's driver and Intel's, and the desktop's Windows is the third host. What a host's first run shows, and nothing before it: that the installed manifest names the layer at `1.4.363` and the installed file is the archived one, which the tool checks in that order; that the system's loader, whose version the run records, takes the layer's manifest from the exclusive `VK_IMPLICIT_LAYER_PATH` in a shell that is not elevated and loads no layer of the registry's with it, the loader ignoring every such variable in an elevated process; that each canary is heard on each of the host's GPUs, seven under the plain validators and two under GPU-assisted validation; that a measured run under the empty path hears no message; and which implicit layers the registry holds, read from the loader's own log (`VK_LOADER_DEBUG=layer`), each with its enable and disable variables. The work desktop's registry holds one for the whole machine, `VK_LAYER_OBS_HOOK`, so its runs show the exclusive path against a layer that would otherwise join. A digest that differs is a finding, and no run re-pins: a second pin is proposed only for a file whose version is 1.4.363.x and whose signature is LunarG's, and it is the owner's to approve | 2 |
| The work desktop's runs (D165): steering alone, under the machine name `work-desktop-windows`, and `work-desktop-windows-igpu` on its integrated Radeon, on a machine that is not the owner's own and is reached over a remote session: an AMD Ryzen 9 9950X and a Radeon RX 9060 XT, with endpoint scanning in every figure. None of its files is checked in, no run of its closes a row, and its reports name no host, user or domain. Owed, in two parts. Before lane `layer-windows` merges, at main's head and with no validation layer on the machine: the device tests `cargo xtask gpu` lists, without their canaries, by hand on the RX 9060 XT, which is the first run of `lactose-rhi-vulkan` on a Windows driver; `lactose-platform`'s window cases; and `cargo xtask measure --all --no-device`. Once the lanes the measurement commit holds have merged and `check` and `measure` are green at that head, with the pinned SDK installed: the Windows pin there, whose pass the measurement commit's row waits for; then `cargo xtask measure --all`, `cargo xtask gpu --client` and the barrier mutations, as rehearsal of the laptop's session and of session C. The measurement commit does not wait for a defect that only the rehearsal finds and that fails none of the pin's four conditions. The owner may rule a failed case of the pin's the machine's only where the case presents into the remote session, never the layer's check, a canary or a message, and the laptop's gate runs that case first. The rehearsals are owed while the machine is lent: the exit's read re-gates what is left of them if it no longer is. To record here: each run's date and commit; each failure and whose it was, the engine's, AMD's driver's, the remote session's or one of the machine's controls'; the lane each defect's fix round went to, whether the measurement commit waited for it, and the pin's run after it where it did; and each case the owner ruled the machine's, with what the laptop's gate then gave for it. A figure of its is quoted in the row it steers, labelled as the work desktop's | 2 |
| `lactose-task`'s two debug spans (D165; lane `park-stamp`): `open_gate` and `park`, target `whisk`, which `record-stall` reads. `park` is around both waits a worker makes without a permit, the idle protocol's and a scope opener's on its latch, which an opening of the gate wakes too. Both are in every build: section 5.1's compile-out of finer levels is not built, and a span costs about 1 ns where nothing listens at debug. `park`'s callbacks run on a worker that is going to sleep or has just been woken, in the idle protocol after it has announced it is asleep. A scope's opener checks its latch, its permit and shutdown again once it has entered the span, after the callbacks and before it parks, so a listener that blocks there delays a wake and cannot lose it. A listener's callbacks must return promptly and never call the executor, and `open_gate`'s enter callback runs with the gate still closed. Owed by the lane, on the M1 as steering: the record phase's figures and the time from an opening to the record's next job with nothing listening at debug, with `open_gate` listened to and with both, interleaved; `cargo xtask measure whisk` before and after; the loom models, Miri and the cross-target clippy runs, with the span at the latch's wait, which no probe has run; a test in which a listener's callback takes the opener's unpark token and the opener still wakes. Owed by the quiet window: the WHISK file at the measurement commit, read against D156 as the plan has it. `record-stall` fails where a build compiles debug spans out, so when section 5.1's compile-out is built it is set where `record-stall`'s build does not inherit it | 2 |
| The client's fault probe on x86 and on lavapipe (D162, D165; from the row of lane `frame-impl`'s runs on the x86 desktop): the paced runs of a probe build, in which each present call sleeps 40 ms, every 13th makes no submit and every 97th draw is torn, ran on the M1 under both drivers and nowhere else. The probe is a build of a scratch copy that no checkout holds, so session C cannot run it, and no runner of lavapipe's presents to a window. Owed to the first change after milestone 2's exit that edits the client's present path, which checks the probe in or says why not, and to the first run of `lactose-platform` on Linux | 3 |
~~~

## Decision entry (exact text): D165

The entry is written for the owner's rulings, as the rows are, and the coordinator commits it as
D165.

**D165. Milestone 2's sittings: the measurement commit holds every lane that changes a measured
input and is recorded once the Windows pin has passed on the work desktop, the owner's Windows
machines sit from 7 December 2026, the Khronos layer is pinned on Windows as installed, and the
gate's wake is read from two debug spans** (§5.1, §20, §21, §22, `lactose-task`, `tools/xtask`,
`docs/plans/milestone-2.md`)
- Why: the exit pass's audit of main at `ad1a99d` found no measurement commit named, no result file
  of milestone 2 on main, the laptop unable to run criterion 1 for want of a pinned layer, and row
  21's Windows item unmeasurable for want of a stamp. The plan's section 4 named the sittings and
  not their order, and its session C was one command where the rows owe the desktop the device tests
  and the barrier mutations too. The owner then had no Windows machine until 7 December 2026, and
  lent a work desktop for early runs.
  - *The measurement commit holds every lane that changes a measured input.* It is the head of main
    at which `check`, the `measure` workflow and the Windows pin run last, once main holds
    `present-encoder`, `park-stamp` and `layer-windows`, and the fix round of each defect the commit
    waits for. A measured input is anything a measurement builds, runs or computes a figure with,
    among them the crates under `crates/` and `game/`, `tools/cook` and `tools/xtask`, the Slang and
    layer pins, `shaders/`, `vendor/`, the manifests and `Cargo.lock`, `rust-toolchain.toml`,
    `.cargo/config.toml`, `.config/nextest.toml` and `.gitattributes`; where the list is short the
    definition holds. `tools/xtask` is one: it times `edit-to-playing`, computes `frame-pace`'s
    figures, cooks the shaders and sets every device run's environment. A file is the record at that
    commit. A file of a later commit is the record as `-at-<commit>` only where the diff changes no
    measured input its measurement reads; otherwise the owner's ruling names the record. A figure a
    measurement gave before the commit is steering: it stays in its row, proves no criterion and
    stands for no file of record. A row that asks a machine for a run and names no commit is closed
    by that machine's run and records the commit it ran at. Two sittings keep commits of their own:
    the desktop's first, at `a539d60`; and `eco-qos`'s runs, at `a75596b` and its first parent.
  - *The commit is recorded once the Windows pin has passed, and the pin's first run is the work
    desktop's.* No line of `lactose-rhi-vulkan` had run on a Windows driver, and the owner's laptop
    and desktop are out of reach until 7 December 2026. The work desktop runs the backend on AMD's
    Windows driver first, by hand and with no validator, then the pin, on its RX 9060 XT, and after
    the pin the laptop's and session C's commands, as rehearsal. The pin has passed when four
    conditions hold: the tool takes the installed layer, each canary is heard, seven under the plain
    validators and two under GPU-assisted validation, no case of either gate run fails, and the
    measured runs of a short run of `triangle`, `gpu-cost`, `frame-pace` and `rhi-conformance` hear
    no message. The pin passes at the commit itself: the commit its report names is the commit
    recorded, and a pass at another commit does not count. The owner ruled the order of the three:
    the lanes the commit holds merge first, `check` and `measure` run at that head, and the pin runs
    there last, so that the owner's remote session is spent once where it can be. The coordinator
    dispatches `measure` there, and the nightly where its schedule has not run there: the owner
    allowed those two dispatches on main for the measurement commit and a moved one, and for nothing
    else. The coordinator chooses that head once the work desktop's early runs have sent their last
    report, and holds main from the start of `check` there until the hash is recorded: while the
    hold stands nothing merges to main but documents, which are the files under `docs/` and the
    `*.md` files, and nothing is pushed to it, a document neither. A push made against the hold
    moves the commit to the new head: `check`, `measure` and the pin run there, each again where it
    had run. The commit waits for a defect that an early run of the work desktop's finds, or that
    `check`, `measure` or the pin fails on: it is a fix round of the lane whose code it is,
    whichever crate that is. A run that fails ends the hold, and so does such a defect reported
    while it stands: once the fix round has merged the head is chosen again, and the three run there
    in the same order. The owner ruled that the commit does not wait for a defect that only the work
    desktop's rehearsal finds and that fails none of the pin's four conditions, and accepted the
    cost: where such a defect touches a measured input the laptop's session meets it again, and a
    file written after its fix is steering until the owner rules which is the record. Its fix round
    merges outside the hold: before the head is chosen, or after the hash is recorded, as a lane
    that merges after the commit does. One that fails one of the four, found before the hash is
    recorded, is the pin's failure: the coordinator records on the first message of the pin run's
    report, sent before the rehearsal starts, so this covers only a defect reported before that
    recording. The owner may rule a failed case the machine's, and the pin passed with it, only
    where the case presents into the remote session: never the layer's check, a canary or a message.
    The laptop's gate runs that case first, and a failure of it there is a defect of a measured
    input and moves the commit. The hash is recorded once `check` and the `measure` workflow are
    green at the commit and the pin has passed at it.
  - *The work desktop is steering.* It is no machine the criteria name, and it is not the owner's
    own. Its measurements' files carry the machine name `work-desktop-windows`, or
    `work-desktop-windows-igpu` on its integrated Radeon, and are never the record: none is checked
    in, no run of its closes a row, and a figure of its is quoted in the row it steers. It has no
    hybrid part, its windows are a remote session's, and its endpoint scanning is in every figure.
    Its controls stand, PowerShell's execution policy among them: no command asks for an exception
    to one or overrides one, and a step a control refuses is reported as the machine's. Its reports
    name no host, user or domain.
  - *The sittings at the commit run in one order, and each is read before the next.* The M1's quiet
    window, as soon as the commit is recorded; then, from 7 December 2026, the laptop's session,
    opened once, and session C. A criterion that misses on the M1 needs an engine lane, and a
    sitting taken before that lane is steering. A defect of a measured input that only NVIDIA's or
    Intel's driver shows is found after the M1's hour: it moves the commit to its fix, and the hour
    runs again there or the owner rules that its files stand. A commit moved so is recorded as the
    first is, with the laptop's own gate and short run at the moved head standing for the pin: the
    owner ruled so, the work desktop is not needed, and the hold runs from `check` at that head to
    the hash's recording. The laptop's record at the moved commit follows the M1's hour there, or
    the owner's ruling that the M1's files stand. A defect that only session C's driver shows moves
    the commit as the laptop's does, and whether session C then sits again at the moved head or its
    files stand is the owner's to rule when it arises. The owner's desk time comes last and once.
    The desktop's first sitting waits for the machine alone. The nightly of the exit is the first at
    or after the measurement commit whose four `sour` jobs pass; another job's failure in that run
    is a finding in its own row.
  - *Windows takes the layer as installed.* `vulkan-layer.toml` pins the SHA-256 of the SDK's
    `VkLayer_khronos_validation.dll`, read on the development Mac out of LunarG's installer, which
    is fetched by the digest LunarG publishes and never run. The owner installs the SDK on each
    Windows machine, the work desktop among them. The tool checks the installed manifest's version
    and then the file's digest. A digest that differs is a finding: no session re-pins, and a second
    pin is the owner's to approve. The run takes the system's loader and drivers. This is D153's
    "pinned on every host" on the host it did not reach.
  - *A run names its implicit layers, on every host.* A run sets the exclusive
    `VK_IMPLICIT_LAYER_PATH`: the tool's layers directory where it validates and an empty directory
    where it measures. `VK_ADD_IMPLICIT_LAYER_PATH` and `VK_LOADER_LAYERS_DISABLE=~implicit~` go. On
    Windows the first let the registry's implicit layers into a validated run, and the second made
    the loader warn for each layer it disabled, which a measured run hears as a message and fails
    on.
  - *`rhi-conformance` is a measurement.* `cargo xtask measure rhi-conformance` runs the gate's
    Vulkan step and writes the file criterion 2 names, under a header of its own that names the
    adapter, the device and the validators, with no path of the machine in it. `cargo xtask gpu`
    keeps its report under the target directory.
  - *The gate's wake is read from two debug spans.* `lactose-task` emits `open_gate`, around
    `Pool::open_gate` at a `block_in_place`'s end, and `park`, around every wait a worker makes
    without a permit: the idle protocol's, whatever the worker's look saw, and a scope opener's on
    its latch, which an opening wakes too. Both are at debug level under target `whisk`.
    `record-stall`'s subscriber reads them and gives, for each worker parked across an opening, the
    time from the opening's start to its wake. 5.1's compile-out is not built. Until it is, the two
    spans are in every build, at about 1 ns each with no debug listener. When it is built, it is set
    where `record-stall`'s build does not inherit it. A listener's callbacks for `park` run on a
    worker that is going to sleep or has just been woken. They must return promptly and never call
    into the executor; one that waits delays that worker's wake and loses none, since the latch's
    wait checks again once the span is entered, a `run_job` scope's opener looks again where its
    children are queued, and the idle wait parks only while its sleeper bit is unclaimed.
    `open_gate`'s callbacks for the span's creation and for its entry run with the gate still
    closed, and every frame worker the gate holds waits for them. Section 21's entry for `task`
    stands.
  - *The laptop's four hardware questions are not shown.* No test makes any of the four misuses on a
    device: the ownership case always performs the transfer, the backend takes coherent memory
    wherever a device offers it, the cook refuses an undecorated heap index, and null's rule alone
    holds the wrong layout. The gate's runs on the laptop are criterion 2's evidence there and
    answer none of the four. Each sitting of the laptop and the desktop records its devices' queue
    families and memory types. Null's rules and the cook's check stand (decision 5.17).
  - *The client's fault probe is re-gated.* `frame-impl`'s probe build is in no checkout. Session C
    runs criterion 1 and `cargo xtask gpu --client` for the row of that lane's runs on x86, and the
    probe's paced runs and lavapipe's half move to a row gated at milestone 3.
  - *The exit reads every row gated at milestone 2.* Section 2 lists the 26 gated at `a561b36`; 85
    are gated at `ad1a99d`. Step 4 reads them all.
  - Rules out: a measurement commit named before a lane that changes a measured input has merged, or
    before `lactose-rhi-vulkan` has run on a Windows driver; a merge to main while the hold stands
    that is not a document's, and a push to main while it stands; the commit's head chosen before
    the work desktop's early runs have sent their last report; the Windows pin counted as passed at
    a commit other than the one recorded, or on a ruling that the layer's check, a canary or a
    message was the machine's; the commit made to wait for a defect that only the work desktop's
    rehearsal found and that fails none of the pin's four conditions; a commit moved after 7
    December 2026 made to wait for the work desktop, and the laptop's record taken there before the
    M1's hour has run there or the owner has ruled that its files stand; a command that overrides a
    control of the work desktop's; a file of the work desktop's as the record, checked in, or
    closing a row; a sitting of the laptop or the desktop before 7 December 2026; a file at another
    commit cited as the record where the diff changes what its measurement reads; a sitting at the
    measurement commit on the laptop or the desktop before the M1's files are read, the parts B, C
    and E that record a moved commit apart; a layer on Windows taken unchecked from whatever the
    machine has, and a pin changed by the session that found it wrong; a download the tool unpacks
    out of an installer; an implicit layer of the machine's in a validated or a measured run; a
    public hook in `lactose-task` for row 21's stamp, and a cargo feature for it; a probe program
    written for row 150's four questions; a lane for the client's fault probe before the exit; a
    window measurement run on a system `lactose-platform` has not been run on.
- Driven by: the exit pass's audit at `ad1a99d`; `docs/proposals/milestone-2-sittings.md`, its probe
  of LunarG's installer and its eight reviews; `reviewer-gpu`'s reading for row 34; lane
  `park-stamp`'s reviews, for the opening's length and for a listener; the owner's rulings on its
  five questions, on the sittings' order, on 26 November 2026 on the order of the three runs and on
  the rehearsal, and on 28 November 2026 on a moved commit and on the coordinator's dispatch of
  `measure` and the nightly; the work desktop's inventory, from its own agent session.
- Replaced: in the plan's section 2, row 150's four hardware questions as "probed on the laptop's
  devices"; in its section 3, the measurement commit as "the first commit of main that holds
  `frame-impl` and `measure-m2`", "a measured crate", and two of the spike's open questions as
  "Probed on the laptop's devices"; in its section 4, `cargo xtask gpu --driver <name>` as what
  writes `rhi-conformance`, the laptop's session as "the same command, and `cargo xtask gpu --driver
  system`", session C as `cargo xtask measure --all` alone, the quiet window's `caffeinate -i -s
  -d`, and the window measurements "skipped" on a system that is not a client platform; in its
  section 5, the owner's actions with no date and no work desktop; in its section 7, step 3's "a
  nightly" and step 4's "every row of section 2"; in D162's addendum of `measure-m2`, round two, the
  two forms of a stamp it names, two process-wide hooks and the same behind a cargo feature; in
  `tools/xtask`'s runs, `VK_ADD_IMPLICIT_LAYER_PATH` and `VK_LOADER_LAYERS_DISABLE=~implicit~`.

## The owner's rulings

The third version asked five questions, in two `AskUserQuestion` calls. They are in this file's
history, at `975ae53`. The owner ruled on all five, four as recommended.

| Question | Ruling |
|---|---|
| Row 21: how the gate's wake is stamped | Two debug spans, lane `park-stamp`, as recommended |
| Windows pin: how the layer is pinned | The installed SDK, as recommended |
| Row 150: the four hardware questions | Recorded as not shown, as recommended |
| Fault probe: row 244's paced runs | Re-gated to milestone 3, as recommended |
| Order: when the laptop is first opened, and the sittings' order | Changed by two facts, below |

**The order's ruling.** Two facts came after the question. The owner has no Windows machine, laptop
or desktop, until 7 December 2026: "delay windows tests until then". And the owner made a work
desktop available through its own agent session. The owner then ruled two things. The work desktop's
role is early runs, as steering: its files are labelled `work-desktop-windows` and are never the
record, it runs the Vulkan backend on a Windows driver first and later the Windows pin, and a defect
it finds goes to a fix round before the measurement commit. And the Mac's hour comes "After the work
desktop's pin": the commit is recorded once the pin has passed on the RX 9060 XT and `check` and
`measure` are green, the Mac's quiet hour then runs, the laptop's session and session C follow from
7 December, and a defect that only NVIDIA's or Intel's driver shows then moves the commit, with the
Mac's hour run again or its files ruled to stand.

**The rulings on the pin's run.** On 26 November 2026 the owner ruled twice more, through
`AskUserQuestion`, and the coordinator relayed both. The first: a defect that only the work
desktop's rehearsal finds, and that fails none of the pin's four conditions, does not hold the
commit. It was the recommended option, and the owner accepted its stated cost: where such a defect
touches a measured input, the laptop's session meets it again, and files written after its fix are
steering until the owner rules which is the record. The second confirmed the order: the lanes the
commit holds merge, then `check` and `measure` run, then the pin runs last. Its span for merges is
"from the pin's run" until the commit is recorded: only documents merge then, so the pin's pass is
at the exact code recorded. The coordinator's hold starts sooner, at `check`, and covers pushes.
That narrows the ruling in its own direction and takes nothing it grants: a merge of code between
`check` and the pin would only move the head and send `check` and `measure` again.

**The rulings on a moved commit and on dispatch.** On 28 November 2026 the owner ruled twice again,
through `AskUserQuestion`, each time for the recommended option, and the coordinator relayed both.
The first is on a commit that a defect only NVIDIA's or Intel's driver shows has moved: the laptop
stands in for the pin. The moved commit is recorded once the Mac's checks and measurements and the
laptop's gate and short run pass at that head, only documents merge from the Mac's checks until it
is recorded, and there is no further remote session on the work desktop, which may not be lent by
then. This text reads "the Mac's checks and measurements" as `check` and the `measure` workflow, as
the diff review of the eighth version does. The eighth version had this rule as the coordinator's
and asked for the ruling. The second is on who dispatches `measure` and the nightly at the
measurement commit: the coordinator may run `gh workflow run measure.yml` and `gh workflow run
nightly.yml` on main, for the measurement commit and a moved one alone. Nothing else of the
coordinator's rule for `gh` changes.

**What this text adds to the rulings.** Eight things the rulings do not say and this text decides,
each the smallest reading of them.

- *The laptop is opened once*, from 7 December, where the third version opened it twice. Nothing it
  runs must come before the commit now.
- *A failed run on the laptop is read before its record is taken.* The laptop is the first to show
  NVIDIA's driver and Intel's, and it is at the commit when it does. Its gate runs and a short run
  come before its files of record, and a failure there stops the record until it is read.
- *None of the work desktop's files is checked in*, and its reports name no host, user or domain. It
  is not the owner's own machine and the repository is public. A figure of its is quoted in the row
  it steers, as a lane's figure before the commit is.
- *The pin's pass is four things*, and the owner's ruling of a failure as the machine's is bounded:
  a case that presents into the remote session alone, never the layer's check, a canary or a
  message, and the laptop's gate runs that case first.
- *Where the commit is, and the hold.* The rulings give the order of the three runs, that only
  documents merge from the pin's run to the recording, that the commit does not wait for the
  rehearsal, and how a moved commit is recorded. The rest of "The decision" is the coordinator's
  rule, as the diff reviews of the seventh and eighth versions corrected it, and the owner has not
  ruled on it. The commit is the head of main at which the three run last, and not the first commit
  that holds the lanes. The pass is at the commit itself, by the hash the pin's report names. The
  head is chosen after the early runs' last report. The hold starts with `check` at that head,
  sooner than the ruling's span, and covers every push, a document's too. It ends at the recording,
  at a failed run, or at a defect the commit waits for that is reported while it stands. A push made
  against it moves the commit to the new head. A rehearsal's fix merges before the head is chosen or
  after the hash is recorded, and the part that found the defect then runs again as steering. A
  rehearsal's defect that fails one of the four, found before the recording, is the pin's failure.
  At a moved commit the hold covers pushes as at the first, and the laptop's record waits for the
  M1's hour there or for the owner's ruling that its files stand, which is the order's ruling read
  for a moved commit. Where that record had begun, its files so far are steering. A defect that only
  session C's driver shows moves the commit as the laptop's does. Whether session C then sits again
  at the moved head or its files stand is left to the owner, and nothing here rules it.
- *The work desktop's controls stand.* No command asks for an exception to one or overrides one,
  PowerShell's execution policy among them.
- *A second machine name*, `work-desktop-windows-igpu`, for the rehearsal on the work desktop's
  integrated Radeon. The ruling names `work-desktop-windows` alone.
- *The toolchain is installed by name* on every machine, since rustup's install of a pinned
  toolchain on first use is deprecated.

One thing was never a question. The desktop's first sitting: the plan has two desktop sittings
(decision 5.19, D151's addendum), and the first one's commands are written. It runs from 7 December,
as soon as the desktop is in reach, and its install of the SDK comes after its measurements.

## Cost and order

| Step | Who | Agent runs |
|---|---|---|
| This round's check | The coordinator, by diff | None |
| The decision, the plan's lines and the rows, applied | The coordinator | None |
| The work desktop's toolchain and its early runs | The work desktop's own agent session; the owner approves the installs | 1 |
| The rows gated at milestone 2 that nothing has read, 37 of them | One read-only agent | 1 |
| `present-encoder`'s fix round and merge | Merged at `1c62676` | Spent |
| `layer-windows`: the pin, the exclusive path on every host, `rhi-conformance`, the script | A lane from main, `reviewer` and `reviewer-gpu`, a fix round and a second where the diff review asks for one, `reviewer-diff`, `merger` | 7 |
| `park-stamp`: the two spans, `record-stall`'s rows, the paired runs | A lane from `b0c5f44`, running, `reviewer` and `reviewer-concurrency`, a fix round and a second where the diff review asks for one, `reviewer-diff`, `merger` | 7 |
| The fix round of each defect the commit waits for, the shader cook's the first, and of each that only the rehearsal finds, which the commit does not wait for | A lane for the crate whose code it is, a review and `merger`, opened by the coordinator | About 3 each, an estimate |
| The work desktop's pin run and its rehearsal, and the pin again where a fix round the commit waits for merges after the pin has run | The work desktop's own agent session; the owner approves the SDK's install and keeps the remote session open for each run | 1, and 1 more for each fix round the commit waits for that merges after a run of the pin, and for each part of the rehearsal run again after its defect's fix; none for a document's merge, and none for a fix round that merges before the pin first runs |
| The nightly and the `measure` workflow at the commit, and at a moved one | The coordinator, by the two dispatches the owner allowed | None |
| The quiet window | The coordinator, with every agent stopped | None |
| The desktop's first sitting, from 7 December 2026 | The owner | None |
| The laptop's session, from 7 December 2026 | The laptop's own agent session | 1 |
| A commit moved after 7 December, as the owner ruled: `check`, `measure`, and the laptop's gate and short run at the moved head; then the M1's hour again there, or the owner's ruling that its files stand; then the laptop's record | The coordinator, and the laptop's own agent session in a second sitting the owner starts, which waits between its short run and its record | 1 for each move, beside its fix round |
| Session C | The owner | None |
| The files checked in and the rows written | The coordinator's pass, or a lane as `exit-pass-rows` was | 2 |
| The freeze decision, the exit's read and the milestone 3 plan | As section 7 has them | Already counted |

About twenty agent runs are to spend, some of `park-stamp`'s seven already spent, and about three
more for each defect the work desktop finds. `park-stamp` and `layer-windows` touch different
crates: `crates/task` with `crates/app-client/tests/stall.rs`, and `tools/xtask`. `present-encoder`
merged at `1c62676`: it edited `crates/app-client/src` and, for its client step,
`tools/xtask/src/main.rs`, so `layer-windows` starts from main now and `park-stamp` takes that
merge. The two lanes each edit `docs/results/README.md`, and the later merge takes the earlier
one's.

**The calendar.** From 26 November 2026: the lanes, and the work desktop's early runs. Once the
lanes the commit holds have merged, the fix round of each defect the early runs found among them,
and the early runs' last report is read: `check` and `measure` at that head, with main held from
`check`'s start, then the work desktop's pin run there, then the commit, the nightly and the M1's
hour, before 7 December if the lanes and each fix round merge in time. From 7 December 2026: the
desktop's first sitting, the laptop's session and session C, about three days if nothing misses. The
exit is not read before then.

## What the reviews changed

`reviewer` attacked the first version and found no blocking defect, four serious, ten minor and four
nits. `reviewer-gpu` read the Windows pin and row 150 and found no blocking defect, three serious,
four minor and two nits. All are accepted but one half of G5 and the number in M3. The answers name
the owner's questions as the third version asked them, and those are in this file's history at
`975ae53`.

| Finding | Answer |
|---|---|
| S1. The hooks were costed with what the spans also carry | Accepted. The first question's first option says that a listener's `park` callbacks run on a worker inside a wait, and that both spans are in every build. Its third option loses "always compiled" and the `unsafe` clause, which was false: `gate::Hook` is safe code. What is left against the hooks is a public item or a feature, and about two runs |
| S2. `park` "after the look that found nothing" misses the worker that parks beside work it has no permit for | Accepted. `park` is entered whenever `gate::idle_park` parks, whatever its look saw. Both arms of `gate.rs` reach `sleep.park`. Since the diff review it is at a scope opener's wait on its latch too (R8) |
| S3. `tools/xtask` is measured, and the rule missed the lock file, the toolchain, the shaders and the vendored crates | Accepted, and taken further. A measured input is defined. `layer-windows` is in the commit by the rule. `-at-<commit>` needs a diff that changes nothing its measurement reads. The gap the finding shows, a fix of the pin after the laptop's first run, is closed by running the pin before the commit is recorded |
| S4. `caffeinate -w $$ &` dies with the shell that started it | Accepted. A detached `caffeinate -dims -t <seconds>`, checked alive before the run and ended after it. A probe showed the form outlive its shell |
| M1. Session C cannot run row 244's probe | Accepted. Session C runs criterion 1 and `cargo xtask gpu --client`. The probe's paced runs and lavapipe's half move to a row gated at milestone 3, where the owner's fifth question is answered as recommended (R6) |
| M2. The laptop does not send back the memory types row 246 waits for | Accepted. The laptop's first session, the desktop's first sitting and session C return each device's memory types. They serve row 150 too |
| M3. The window cases' count is wrong | Accepted, with another number: fifteen on Windows. The first version's twelve was row 208's count before the swapchain's three. The review's fourteen is one short: `CASES` in `crates/platform/tests/window.rs` has sixteen entries and one is macOS's alone |
| M4. The parked-worker figure has no sample size, and the opening's length is a bound, not the item | Accepted. The item is each parked worker's wake, from the opening's start to its `park` exit, with its count. The opening's length is stated beside it, as the opener's side alone. The versions before this one called it a bound, and lane `park-stamp`'s review showed that false: the woken worker's wake comes after the opening, and the coordinator relays a worst wake of 43.6 µs on the M1 against a worst opening of 7.6 µs. D162's figure keeps its count, median and worst. A full run makes 360 ticks a reservation |
| M5. Only `open_gate` is asserted, and the README's "Tracing" is not amended | Accepted. Each reservation asserts `park` spans too. The lane amends "Tracing" |
| M6. "Finer levels compile out covers them" overclaims | Accepted, in the review's words: the compile-out is not built, the spans are in every build, and when it is built it is set where `record-stall` does not inherit it |
| M7. "The first green nightly" is stricter than step 3 | Accepted. The first nightly at or after the commit whose four `sour` jobs pass. Another job's failure is a finding in its own row |
| M8. Nothing checks what the spans cost the wake | Accepted. The lane owes paired runs with nothing listening, one span and both. "A row that moved beyond the spread" is gone from the quiet window |
| M9. Question 3's "about three days" has no condition | Accepted: "if nothing misses; a miss holds the later sittings for the lane it needs" |
| M10. Question 2 does not say what follows a digest that differs | Accepted. The run stops, the session re-pins nothing, and a second pin is the owner's |
| N1. Question 1's second option said "the M1's and the desktop's files" | Accepted: "the three machines' files" |
| N2. D165 ruled out every hook and feature in `lactose-task` | Accepted: "for row 21's stamp" |
| N3. Row 63 has a desktop half | Accepted. Session C adds `eco-qos`'s three runs if the first sitting shows a hybrid processor |
| N4. `typeperf` never stops | Accepted. It takes a sample count and ends by itself |
| G1. The mutation script's verdict is vacuous | Accepted. Lane `layer-windows` fixes it and checks it in as `tools/xtask/barrier-mutations.py`, with the verdict read from the gate's report, and shows `between-dispatches` missed under MoltenVK and caught under KosmicKrisp |
| G2. The gate's runs answer none of the four hardware questions | Accepted. Row 150, the plan's table, the decision entry and question 4 say all four are not shown. The paragraph below settles the two reviews' disagreement |
| G3. The registry's implicit layers join validated runs, and `~implicit~` fails every measured run on a machine that has one | Accepted, on every host: the exclusive `VK_IMPLICIT_LAYER_PATH` in place of both forms |
| G4. `vulkaninfo`'s list does not mark implicit layers | Accepted. The session reads the loader's own log, with each layer's enable and disable variables |
| G5. `rhi-conformance` under the measurements' header would misstate the run, and a second GPU's run overwrites the first's | Accepted for the header: its own, with the adapter, the device and a Validation row. Not accepted: a refusal to overwrite for this one measurement. `write` overwrites for every measurement, and a lane's second run of a day relies on it. A refusal for one measurement is a special case, and for all of them it changes every lane's re-run. The guard is each GPU's run as one command that sets the machine name itself, copies what it wrote and counts it (R4), and the adapter in the header |
| G6. A wrong SDK at `VULKAN_SDK` reads as a wrong digest, and the fallback re-pinned on the owner's word alone | Accepted. The manifest's version is checked first. No session re-pins. A second pin needs a 1.4.363.x file signed by LunarG and the owner's approval |
| G7. Question 2 left out what the install does to the machine, and overstated the other option's risk | Accepted. Both options are restated |
| G8. The manifest's path on Windows was not named, and "sets no library path" contradicted the tool | Accepted. `Bin\VkLayer_khronos_validation.json`. On Windows the run sets no library path, which the lane's unit test holds |
| G9. The script on Windows: encodings, a copy over an existing folder, and `python3` | Accepted. UTF-8 for the log and the report, a fresh directory for each copy, and `py -3` in the commands |

**The two reviews disagree on question 4, and this proposal follows `reviewer-gpu`.** `reviewer`
judged the question fair as worded and left row 150's substance to the GPU review. `reviewer-gpu`
read the two cases and the backend and found that the gate's runs answer none of the four. The tree
agrees with it, by the three files the section on row 150 cites: the ownership case never drops the
transfer, and the unflushed-range case skips wherever a device offers coherent memory. A question
that said "the device tests answer two of them" stated something false, so it was not fair, however
even its options were. The question then said that no test makes any of the four misuses, and its
options were to record that or to build the probes.

**The diff review of the second version.** `reviewer-diff` read the second version against the two
reviews and found no blocking and no serious defect, eight minor and six nits. It judged questions
1, 2 and 4 fair, the fourth once R1's block is added, and question 3 not fair until its recommended
option carries its costs. All fourteen are accepted.

| Finding | Answer |
|---|---|
| R1. The plan's section 2 still has row 150's four questions "probed on the laptop's devices" | Accepted. A block for the plan's line 114, which makes twenty. The plan's record of the text it once appended to row 150 stays: it records an edit that was made |
| R2. The desktop's first sitting is credited with work its commands do not ask for | Accepted, by revising the commands and keeping the claims. They now ask for `LocalDumps` before the run, `vulkaninfo`'s whole output, the processor's kinds of core from the `whisk` file's topology row and, after the measurements, the SDK's install where the second question is answered as recommended. They also name the machine for each system, which the review did not ask for. Session C's commands read the `LocalDumps` setting back |
| R3. "Closes no row" contradicts the rows the laptop's first session closes | Accepted, in both of the review's forms. The rule is about measurements and their files of record. A row that asks a machine for a run and names no commit is closed by that run and records its commit, and the laptop's first session is the third sitting with a commit of its own |
| R4. The laptop's commands rest on shell state an agent's calls may not keep | Accepted, and taken further. Every block is one command that sets what it needs: the machine name, the adapter's name written out, the commit. Each GPU's run copies the files it wrote into the target directory and counts them, and the first GPU's files are compared with their copies after the second's run, so an overwrite is seen and can be undone. A changed directory is shell state too, so the worktree beside the clone is gone and the clone itself is detached at the session's commit. Row 63's three runs each check out their commit in their own block and stop if the checkout missed. Where the session's shell is not PowerShell, a block is saved as a script and run as one |
| R5. Question 3's recommended option left out what it adds before the commit | Accepted. The option says the commit waits for the session, about three hours, and for a fix and a second run of the pin with the laptop opened again if it finds a defect, and that the owner sets the laptop's screen and sleep before each session and back after it. The second option says what it saves |
| R6. Row 244's re-gate was taken without a question | Accepted. It is the fifth question, in a second call |
| R7. The list of measured inputs left out nextest's configuration | Accepted. `.config/nextest.toml` and `.gitattributes` join the list, and the definition holds where the list is short |
| R8. A scope opener parked on its latch is woken by an opening and was not counted | Accepted, by the first of the review's two fixes. `park` is around both waits a worker makes without a permit, with a field that says which. The second fix, a figure that says it leaves such openers out, would keep a known wrong count: `Pool::open_gate` wakes every worker, and a next job taken by a woken opener would read as taken by a worker that was awake |
| n1. The first session cannot run "while the other two lanes finish" | Accepted: while `park-stamp` finishes at most, and on the commit's critical path |
| n2. Question 2's second option and the section disagree on the loader | Accepted. The loader stays the driver's under that option, and a loader that does not honour the exclusive path is not cured by it |
| n3. The quiet window's commands add a condition the plan does not have | Accepted: dropped. A fix of the pin is a round of `layer-windows` and moves its last merge |
| n4. Row 150's sentence for lane `vulkan` still says "probed" | Accepted: "which were to be probed on the laptop's devices, recorded as not shown" |
| n5. The caveat for 360 ticks overstates seconds | Accepted: seconds, in the proposal and in the quiet window's commands |
| n6. `park-stamp` holds the commit by nothing | Accepted. The first option says the lane is not expected to hold the commit, and what follows if it runs long |

**What the rulings changed in the answers above.** The two tables are the answers as the third
version gave them. Three no longer hold as written. R3 made the laptop's first session a third
sitting with a commit of its own: the laptop now sits once, at the measurement commit, and two
sittings keep commits of their own. R5 gave question 3's recommended option its costs: the question
was then overtaken by the two facts in the ruling. n1 placed the laptop's first session on the
commit's critical path: that place is the work desktop's pin run. Where another answer names the
laptop's first session, it is the work desktop's pin run for what had to come before the commit, as
in S3, and the laptop's one session for the rest, as in M2.

**The diff review of the fourth version.** `reviewer-diff` read the fourth version, the decision
entry, the twenty-one blocks, the checker and the five commands files against the rulings and main
at `1c62676`. It found no blocking defect, one serious, five minor and five nits, and all eleven are
accepted. The serious one and three of the minor ones are defects of the work desktop's and the
laptop's commands files, which are not checked in: they are fixed there, and this text gains the
rule each broke.

| Finding | Answer |
|---|---|
| W1. The work desktop's commands told the session to override PowerShell's execution policy for a process, and to run a block another way where that was refused | Accepted. Both Windows commands files run a saved block with no policy argument, and a script the policy refuses stops the run and is reported as the machine's. No command passes an execution policy or an encoded command, sets the policy or unblocks a file, and each Windows commands file says so once in its rules. The decision entry, the machine's row and this text name the execution policy among the controls that stand, and say that no command overrides one |
| W2. The commit waited for a pin run that has "run", not one that has "passed", and a defect went to "a fix round of this lane" | Accepted. The plan's bullet, the commit's row, the decision entry and this text say the pin's run has passed. A defect goes to the lane whose code it is, in the review's words. The review's wording named the fix round a failure of the pin went to; a defect an early run finds is held to the same rule, and the shader cook's is the first. Where the pass must be has changed twice since: the fifth version held it to every fix round before the commit, the sixth to a set of the pin's inputs, and the seventh to the commit's code, as "The decision" has it |
| W3. The lock's reading sees every session's sign-in process, and a minimised remote window is not seen at all | Accepted. The commands read the sign-in process of the one Windows session they run in, as the review wrote it, still as a heuristic that no run has checked. The owner's actions, the machine's row, the paragraph on what each sitting needs and the table of sittings ask for the remote session's window open and not minimised |
| W4. The pin's four conditions are not equally showable, and a failure ruled the machine's had no rule after it | Accepted, with the review's rule. The owner may rule a failed case the machine's only where it presents into the remote session, never the layer's check, a canary or a message. The laptop's gate runs that case first, and a failure of it there moves the commit. It is in the decision entry, the plan's paragraph, the row of the work desktop's runs and the section on the pin's pass, and the commands files carry it. A run a control cut is run again, and any other failure that no fix ends leaves the pin not passed |
| W5. `rustup show active-toolchain` prints the clone's path, and the commands had it sent | Accepted. The Windows commands print the line's first word, which is the toolchain, and whether the line names `rust-toolchain.toml`, and send neither the line nor a path. The desk sittings' commands say to send the first word alone |
| W6. An environment variable set by one block reached the next where the shell keeps state | Accepted, for every block and not for the two the review named: a block that sets an environment variable removes it at its end, in both Windows commands files |
| n-a. `work-desktop-windows-igpu` is a second label, not listed among the additions, and not every file carries a machine name | Accepted. The second name is kept, listed as an addition and named in the decision entry and the rows. "Every measurement's file" carries a name, and the gate's report and a test's log are said to carry none |
| n-b. "Only the laptop's or the desktop's sitting shows" is not the ruling's "only NVIDIA's or Intel's driver" | Accepted: the ruling's words, in the plan's bullet, the commit's row and the paragraph on the order. A defect another sitting shows falls under the rule for a file of a later commit, as before |
| n-c. The row of the work desktop's runs is gated at 2 and owes rehearsals of a lent machine | Accepted. The row says the rehearsals are owed while the machine is lent, and that the exit's read re-gates what is left of them if it no longer is |
| n-d. `work-desktop-windows-2` stood for the first digit of the date | Accepted: `work-desktop-windows-<date>`, and `laptop-windows-<date>` in the laptop's commands, which had the same form |
| n-e. The integrated Radeon's files went to a folder the scrub does not read | Accepted. They are copied into the scrubbed folder under a prefix of their own, so that a name the two GPUs share overwrites nothing there |

**What the sixth version changed.** Two things, neither from a review of this text. The fifth
version held the pin's pass to every fix round found before the commit, so that a fix round a
`check` or a `measure` failure went to sent the work desktop's pin again. The coordinator cut that
back: the pass was held to the fix round of each defect the work desktop itself found, and after it
the pin ran again only for a fix round that changed one of a set of the pin's inputs, to which this
text added a clause on tests. The seventh version replaced that rule, as the next table says. The
lane row's form from the review, "a fix round of the lane whose code it is", stands. The second
thing stands too: `record-stall`'s rows. Three of them, and this text's list of what the file gains,
called the opening's own length a bound on a parked worker's wake, and lane `park-stamp`'s review
showed that it is none: the opening is timed on the opener's side alone, and the wake comes after
it. The lane table's row, `record-stall`'s row in section 4 and verification row 21 give the length
beside the wake, as the opener's side, and call it nothing more.

**The diff review of the fifth and sixth versions.** `reviewer-diff` read the two versions' diff,
the decision entry, the twenty-one blocks, the checker and the two commands files that changed,
against the fourth version's review and main at `1c62676`. It found that review's eleven findings
closed, and no blocking defect, one serious, three minor and two nits. The serious one and the three
minor ones are all of the sixth version's set of the pin's inputs and of the rule around it. The
coordinator answered them with one change: the set is gone, and the five rules of "The decision"
stand in its place, in the decision entry, the plan's bullet, the commit's row, the work desktop's
paragraph and both commands files.

| Finding | Answer |
|---|---|
| D1. The two halves of the rule for running the pin again contradict each other for a defect found after the pass, and "Rules out" forbids both outcomes | Falls away with the set. The pass is at the commit's code, the pin runs again after every fix round the commit waits for, and the commit does not wait for a defect that only the rehearsal finds |
| D2. "Of their tests those the gate runs" drops the modules that make the gate's Vulkan device | Falls away with the set: the clause is gone |
| D3. The rule has three meanings, and the quiet window's command sends the pin again for a README | Falls away with the set: one rule in every text, and one command, which leaves documents out |
| D4. The set leaves out the cook's crates, `.cargo/config.toml`, the root manifest, `.gitattributes` and the harness's modules | Falls away with the set: no file but a document changes between the pass and the commit |
| n1. The set is written out in four places | Falls away with the set |
| n2. "Its four reviews" becomes five | Accepted: five |

**What else the seventh version changed.** Two things. The first is from lane `park-stamp`'s
concurrency review and not from a review of this text: what a listener on the two spans may do, as
the section on the stamp has it. The spans' row, the lane table's row and the decision entry say
that a listener that blocks in `park`'s callbacks delays a wake and cannot lose it, since the opener
checks again once it has entered the span, that a listener's callbacks must return promptly and
never call the executor, and that `open_gate`'s enter callback runs with the gate still closed. The
spans' row owes a test of it; the coordinator relays that the lane's fix round has since written
that test, and the row's text is left for the lane's merge. The second followed from the fourth
rule: the commit was "the first commit of main that holds" the three lanes "and the fix round of
each defect found before it", and became that of each defect it waits for, since the rehearsal's
defect may be found before the commit and is not waited for.

**The diff review of the seventh version.** `reviewer-diff` read the seventh version's diff, the
decision entry, the twenty-one blocks, the checker and the two commands files that changed, against
its review of the sixth and main at `1c62676`. It found that review's findings closed, and no
blocking defect, one serious, two minor and two nits. With the review came the owner's two rulings
of 26 November 2026, on the order of the three runs and on the rehearsal, and the coordinator took
the review's first fix for the serious one.

| Finding | Answer |
|---|---|
| R1. "The first commit of main that holds" cannot be squared with a pass "at its code" once anything but a fix the commit waits for merges after the last such fix, and the fourth rule allowed such a merge | Accepted, with the review's first fix. The commit is the head of main at which `check`, `measure` and the pin run last. The hold starts with `check` at that head, covers pushes with merges, and ends at the recording or at a failed run. A rehearsal's fix merges before the head is chosen or after the hash is recorded, and no longer "before a run of the pin where it is ready". "Every state has a next step" lists the states, the review's three among them. It is in the decision entry, the plan's bullet, the commit's row and the quiet window's commands |
| R2. A commit moved by a defect that only NVIDIA's or Intel's driver shows is under none of the five rules, and the work desktop may no longer be lent | Accepted, with the coordinator's rule, which the owner then ruled on 28 November 2026: the laptop's own gate and short run at the moved head stand for the pin, the work desktop is not needed, and the hold runs from `check` there to the recording. One sentence in the decision entry, the plan's bullet and the commit's row, and "A defect a later sitting finds" gives its cost |
| R3. "One of the four conditions" has two meanings: the fourth is the short run's, and the examples named a measured run of the full set | Accepted. Each example names the short run's four measurements, and so does the condition, in the decision entry, the plan's section 4, the section on the pin's pass and the two commands files that changed. The seventh version's name for them, "the four device measurements", is dropped there: six measurements need a device, and the short run takes four. The laptop's and session C's commands kept the name beside the command that lists the four until the ninth version, which names the four there too |
| n1. The reason a listener that blocks loses no wake covered the latch's wait alone | Accepted, in the decision entry and the section on the stamp: the sleeper bit at the idle wait, the opener's second check at the latch. With it, from the lane's fix round, `open_gate`'s two callbacks. The lane table's row and the spans' row are left for the lane's merge |
| n2. The brief for a second pin run sends part A again, which ends in the owner's install | Accepted, in the work desktop's commands: where the pinned SDK is installed already, part A reads the loader and goes on to part B |

The review also noted that this text did not say whether the part of the rehearsal that found a
defect runs again after its fix. It does, as steering and while the machine is lent, and the fourth
rule now says so.

**The diff review of the eighth version.** `reviewer-diff` read the eighth version's diff, the
decision entry, the twenty-one blocks, the checker and the commands files, against its review of the
seventh and main at `1c62676`. It found that review's findings closed, and no blocking or serious
defect, five minor ones and two nits. With it came the owner's two rulings of 28 November 2026, on a
moved commit and on dispatch, and the coordinator decided each finding.

| Finding | Answer |
|---|---|
| N1. After a push made against the hold the definition gives no one commit and the texts give two next steps, one of which cancels the new head's `check`; the allowance for documents is never used | Accepted, with the simpler text the eighth version's report offered. The pin passes at the commit itself, and two hashes are compared: the allowance, the command and "the commit's code" are gone. A push made against the hold moves the commit to the new head, and `check`, `measure` and the pin run there. A cut run that runs again at the same head is a runner's, a control's or a dropped session's, and no longer a `check` that a push cancelled. "Every state has a next step" gives each state one. It is in the decision entry, the plan's bullet, the commit's row, the lane table's row for `layer-windows`, and the quiet window's and the work desktop's commands |
| N2. At a moved commit the laptop's session takes its record before the M1's hour runs again there, which "Rules out" forbids | Accepted. The laptop's record at the moved commit follows the M1's hour there, or the owner's ruling that the M1's files stand, in the decision entry, the plan's bullet and the commit's row. In the laptop's commands a sitting at a moved head waits for the coordinator's word after part E. The eighth version's reading, that those commands needed no change, was wrong |
| N3. An early run's defect reported while the hold stands has no next step | Accepted, both halves. The head is chosen only once the early runs have sent their last report. And a defect the commit waits for that is reported while the hold stands ends it, as a failed run does: a fix round, and the three run again |
| N4. Two places record more as the owner's than the owner ruled | Accepted. "The decision" names as ruled the order in the second rule, the fourth's first three sentences with its sentence on the cost, and the fifth down to the work desktop, and each of the two says where the coordinator's part starts. The rulings' section quotes the second ruling's span, "from the pin's run", with the coordinator's sooner start beside it as a narrowing in the ruling's own direction |
| N5. The quiet window's commands use two forms of `gh` outside the coordinator's list | Accepted. A run that lost its runner is run again by `gh run rerun <run id> --failed`. The two dispatches stand, as the owner allowed them on 28 November 2026 |
| n-a. The laptop's and session C's commands still say "the four device measurements" | Accepted: both name the four |
| n-b. The reason a listener's wait loses no wake is one look short once lane `park-stamp` has fixed its review's finding | Accepted now, and not at the lane's merge. The decision entry and the section on the stamp have the rule in the lane's words, with a `run_job` scope's opener's second look, checked against the lane's README at `19f1b1b`. The lane table's row and the spans' row are still left for the lane's merge |

The review also listed where the moved commit's rule had to become the owner's: this text's status,
"A defect a later sitting finds", the rulings' section, the table of states, the cost table, R2's
answer above, the decision entry, the plan's bullet, the commit's row and the quiet window's
commands. Each now says so.

**The diff review of the ninth version.** `reviewer-diff` read the ninth version's two commits, the
decision entry, the twenty-one blocks, the checker and the four commands files that changed, against
its review of the eighth and main at `1c62676`. It found every finding of that review closed, and no
blocking or serious defect, one minor one and two nits. The coordinator decided each, and for
session C set another text than the review's.

| Finding | Answer |
|---|---|
| M1. The moved commit's states cover only a defect that the laptop's gate or short run shows. One can come from the laptop's record, or from session C | Accepted, with the coordinator's text for session C. The state's trigger is a sitting from 7 December. Where the laptop's record has begun, its files so far are steering and its second sitting runs the record whole: in the table, the fifth rule and the laptop's commands. A defect that only session C's driver shows moves the commit as the laptop's does, and whether session C then sits again at the moved head or its files stand is the owner's to rule when it arises: in the decision entry, the plan's bullet, the fifth rule and a state of its own. The review's sentence had session C run again or the owner rule. "The owner's desk time comes last and once" is the owner's, so no text has session C sit twice of its own accord, and none gives the question as ruled |
| n1. "Rules out" excepts the gate and the short run, where the sitting that records a moved commit also runs parts B and C before the M1's files are read | Accepted: "the parts B, C and E that record a moved commit apart" |
| n2. The complement for a rehearsal's defect that fails one of the four cannot arise, since the coordinator records on the pin run's first message | Kept, with its bound said. The coordinator records on that message, so the complement covers only a defect reported before the recording is made: in the fourth rule, its state, the decision entry and the work desktop's commands. It is not dropped: the coordinator relays that the owner's ruling of 26 November was framed beside it |

The coordinator also ruled on three readings of the ninth version's report. The quiet window's two
commands that show no push was made against the hold stay. The line that names message 7 the early
runs' last is the coordinator's to send to that session. And lane `park-stamp`'s merge writes the
lane table's row and the spans' row again.

## What this proposal could not verify

- That the file LunarG's installer leaves on a Windows disk is the file in its archive. The digest
  was read from the archive on the Mac, twice. The work desktop's pin run is the first check and the
  laptop's session the second, and the pin's row says what follows a difference.
- The Windows loader, on any of the three machines. `reviewer-gpu` read the loader's source at the
  pinned SDK's tag: the exclusive path keeps the registry's implicit layers out, and an elevated
  process ignores every layer variable. Which loader a machine has after the install, and which
  loader version first honours the exclusive variable, were not read. The work desktop's is at
  1.4.357 before the install. Each run records the loader's version, and the canaries and the
  loader's log show whether the path was honoured.
- That a discrete GPU on a hybrid laptop presents to the panel with the driver's own implicit layer
  kept out of the run. The first version's `~implicit~` kept it out of measured runs as well. The
  laptop's short `triangle` run is the check, and the work desktop cannot stand for it.
- Whether the Vulkan SDK is already on the laptop, and at which version, and whether the installer
  brings the Visual C++ runtime the layer's library imports. A layer that does not load fails the
  first gate run with no canary heard.
- The desktop: its CPU and whether it is a hybrid one, its GPU, its second system, and its devices'
  queue families and memory types. Rows 212 and 244 need a device whose present family is not its
  graphics family, and row 214 a family without the graphics bit. The first sitting's `vulkaninfo`
  output says whether the desktop has them, and a row it cannot run is the exit read's to re-gate.
- The laptop's second device. The plan expects an Intel integrated GPU and none was listed (section
  3's table of machines). The session's device list answers, and with one device each GPU's commands
  run once.
- `present-encoder`'s `cargo xtask gpu --client` on a desktop GPU. As merged at `1c62676` it takes
  `--driver system` on any host and gives it KosmicKrisp's lag of 200 passes, set on the M1
  (`tools/xtask/src/main.rs`, `client_lag`). Its lagged run fails where no call found a read
  pending, which a faster GPU may give. Such a failure on the work desktop, the laptop or the
  desktop is a finding about the lag, recorded as that, and the work desktop's run is the first
  look.
- That the probe of row 244 cannot be rebuilt. Only the client's README was read for what it is. No
  patch of it was looked for.
- How long `cargo xtask measure --all` runs at eighteen measurements. "About an hour" on the M1 is
  an estimate from the lanes' runs on a loaded machine, and `record-stall` at 360 ticks a
  reservation adds seconds to it. The laptop's session at about five hours is an estimate too:
  nothing of its length has been timed but row 63's runs. The work desktop's rehearsal times the
  Windows commands first.
- The cost of the two spans, with a listener and without. The crate's README gives a span about 1 ns
  with no subscriber, and the review's probe on a loaded M1 showed no ordered difference. The lane's
  paired runs are the first figures.
- The nightly the review cites as failing on one job alone. Its number was not checked against the
  run.
- The 37 rows gated at milestone 2 that neither the audit nor this proposal read. One of them may
  owe a machine something no commands file holds yet.
- A span at the latch's wait under loom and Miri. The first review's probe ran the other two places.
  Whether `record-stall` leaves an opener parked on its latch across an opening was not shown
  either: the file's count of each kind of wait answers it. Lane `park-stamp`'s commit `a5ce394` was
  found on its branch and not read.
- The laptop's agent shell and the work desktop's: which shell runs the commands, and whether it
  keeps a variable or a directory between two of them. The commands rest on it in one way. Where the
  shell is not PowerShell a block is saved as a script, and Windows' default execution policy for a
  client refuses a script file, as remembered. The run then stops, it is reported as the machine's,
  and the owner says how the blocks are run.
- The lock's reading on the work desktop. The check of the sign-in process of the one Windows
  session a command runs in is the diff review's, from Windows' behaviour and not run. Whether a
  locked, a disconnected or a minimised remote session shows in it, and what each does to a window
  that is presenting, is found by the first part that opens a window.
- The shader cook's defect on Windows. The coordinator relayed it from the work desktop's early run,
  and it was not read here.
- That no document is an input of the pin's run. A search of main at `1c62676` found none that a
  test of the gate's list or a device measurement reads, and no run has shown it. Nothing rests on
  it since the ninth version: the pin passes at the commit itself.
- Lane `park-stamp`'s figures for the wake and the opening. The coordinator relayed them from the
  lane's review, and neither the review nor the lane's files were read here.
- Lane `park-stamp`'s concurrency finding and its fix. The coordinator relayed the finding, and the
  review was not read here. The fix round's commit `6bdea44` was read on the lane's branch, for its
  message and for what its README says of a listener. It was neither reviewed nor merged when this
  was written, and its tests were not run here. For the eighth version the lane's README was read
  again at the branch's head `35c2120`, "Tracing", for the reason at each wait and for `open_gate`'s
  callbacks. The lane was in review then. For the ninth version the README was read at the lane's
  commit `19f1b1b`, for a `run_job` scope's opener's second look. That commit was in review too. The
  lane's review, whose finding it answers, was not read here, and neither was anything the lane's
  worktree held beyond that commit.
- The owner's four rulings of 26 and 28 November 2026. They are here as the coordinator relayed
  them, and this proposal saw none of the questions. The second ruling's span, "from the pin's run",
  is quoted as the coordinator and the diff review of the eighth version give it. That "the Mac's
  checks and measurements" in the moved commit's ruling are `check` and the `measure` workflow is
  this text's reading.
- What a push and a dispatch do to a run. That a push cancels a `check` in progress on main, that a
  cancelled run comes back only where it is run again, and that a dispatch of `measure` runs at the
  branch's head are read from the two workflow files at `1c62676` and from GitHub's behaviour as
  remembered. None was tried. `gh run rerun <run id> --failed` was not tried here either: the diff
  review of the eighth version found it in this repository's run history, running again one job that
  had lost its runner, which is the one use the quiet window's commands make of it.
- A moved commit. The laptop's gate and short run standing for the pin is a rule no run has met. The
  laptop's commands now hold a wait after part E for it, and no session has run them: whether an
  agent session there outlasts a wait of hours is untried. Nothing is known of the desktop's GPU
  before its first sitting, so whether session C's case can arise is not known either.
- Whether the CI runner's Mesa registers an implicit layer of its own, which the exclusive path
  would drop from the `gpu (lavapipe)` jobs. The diff review says so from memory. Nothing chooses a
  device by it, and the lane's `gpu (lavapipe)` run is the check.
- Where `vulkaninfo` is on a Windows machine before the SDK is installed, and the SDK's name for its
  own copy. The commands say what to do where it is not found.
- The work desktop. Nothing of its inventory was checked here. Not known: whether the device tests
  run by hand as the gate runs them, which nobody has done on Windows; whether a remote session lets
  a Vulkan surface be made and presented to, and what the lock does to a window that is presenting;
  whether its network lets `cargo xtask slang` fetch Slang's archive; what LunarG's installer does
  to its loader; and whether its endpoint protection lets each test's process start unhindered. Each
  is found by its first run and reported as the machine's.
- That `rustup toolchain install` with no name installs what `rust-toolchain.toml` pins, with its
  components, on the rustup each machine has. The Mac's rustup, 1.29.3, says of the command that it
  installs "the given toolchains, or by default the active toolchain", and it was not run here. The
  commands check the active toolchain after it and give the form with the version written out for a
  rustup that wants a name.
- The dates. 7 December 2026 is the owner's. When the lanes merge is not known, so the M1's hour
  before 7 December is likely and not promised.
- Any figure. Every figure quoted is a row's, a lane's or a review's.
