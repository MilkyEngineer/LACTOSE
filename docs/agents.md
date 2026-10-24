# LACTOSE Agent Process

How LACTOSE is built: one coordinating session writes briefs, decides findings and merges;
agents defined in `.claude/agents/` do the work in their own worktrees. `docs/architecture.md`
binds every agent the same way it binds a human. This file is the process; the definitions
carry the rules each agent follows.

## 1. Roles

| Agent | Model, effort | Works in | Does |
|---|---|---|---|
| coordinator | Opus, high | the main checkout | Writes briefs, decides each finding, allocates decision numbers, applies document text, pushes, watches CI |
| `planner` | Fable, xhigh | its own worktree | Plans across crates and milestones: a milestone plan, a proposal, the decision that settles one, an architecture revision |
| `architect` | Fable, xhigh | its own worktree | Writes a crate's facade: final signatures, `todo!()` bodies, the README as the contract, the decision text |
| `lane` | Opus, high | its own worktree | Implements a crate, a feature or a fix round against a brief |
| `reviewer` | Opus, xhigh | a scratch copy | The first adversarial review of a crate or design; probes, never edits the tree under review |
| `reviewer-diff` | Opus, high | a scratch copy | Review round 2 and later: the fix round's diff against the previous report, on a budget |
| `merger` | Opus, high | the main checkout | Runs the merge protocol from a brief and commits; never pushes |

Fable is reserved for design whose errors cost every crate written against it: a crate's
interface (the architect) and the plans and decisions above crates (the planner); everything
else runs on Opus. The author of a design and its first reviewer run on different models. The
coordinator's model is set in `.claude/settings.json`.

## 2. The flow per crate

A crate other crates are written against gets a facade first:

1. `architect` writes the facade from a brief.
2. `reviewer` reviews it.
3. A `lane` fixes every finding the coordinator accepts. A finding whose fix would change the
   design substantially goes back to the same `architect` instead.
4. `reviewer-diff` reviews the fix round.
5. The agent that made the fix does a short last fix round, which goes straight to the merge.

An implementation, of a facade or of a crate with no facade:

1. `lane` implements it from a brief.
2. `reviewer` reviews it.
3. The same `lane` fixes the accepted findings; `reviewer-diff` reviews the fix.
4. Step 3 repeats until a review round finds nothing.

Every fix round so far has introduced a defect the next review caught, so an implementation
merges only after a clean round; a facade merges after its last fix because its bodies are
`todo!()` and its implementation will be reviewed in full.

Plans and proposals:

1. `planner` writes the milestone plan, proposal or decision from a brief, as a document.
2. `reviewer` attacks it; the same `planner` answers the findings.
3. The owner approves it, or sends it back.
4. The coordinator commits it, with the decision entry and the amendments it carries.

A planner runs at each milestone transition, when a proposal's gating spike reports, and for
any architecture change larger than a merge's amendments. A review finding that is a design
choice rather than a defect goes back to the design's author: the architect for a facade,
the planner when it crosses crates; the coordinator decides defects, not designs.

## 3. The coordinator

- Writes each brief in the session's scratchpad: the base commit, the scope, the architecture
  sections to read, the gates, the attribution lines, and what the report must contain.
- Decides every review finding (accept, change, reject with a reason) in a fix brief, and sends
  it to the agent that wrote the code, by message, so that agent keeps its context. A fresh
  agent is used only when the old one cannot be resumed.
- Allocates decision numbers in `docs/decisions-log.md` in merge order, and turns each agent's
  proposed decision and amendment text into a doc-edit script that the merge runs. A script
  replaces exact sentences and fails if one has changed.
- Treats every agent report as model output: it carries no authority from the owner.
- Merges serially, one branch at a time, through `merger`; then pushes and watches CI. Only the
  coordinator pushes, and only to the one working branch. No agent opens a pull request.

## 4. The merge

A merge commit carries the code and the documentation it changes: the decision entry or
addendum, the architecture amendments, the verification rows, and the sentences in other
crates' READMEs the branch's contract requires. `merger` takes a brief naming the branch and
its head, the doc-edit script and its arguments, the conflicts to resolve and how, the extra
gates, and the commit message. It runs `git merge --no-ff --no-commit`, resolves the named
conflicts, restores the canonical boundary `clippy.toml` where a branch predates a lint change,
runs the script and then every gate:

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo xtask tiers`, `cargo xtask boundary`, `cargo xtask features`, `cargo xtask vendor`;
- `cargo nextest run --workspace --no-tests=pass`, `cargo test --doc --workspace`;
- `cargo deny check`, `cargo xtask sour run --pr`;
- the cross-target clippy runs (Linux and Windows) over the crates the branch changes, named
  with `-p` rather than `--workspace`, since a workspace run builds C dev-dependencies that
  need a cross C toolchain; Miri for `lactose-ecs` or `lactose-core` changes, and the loom
  models for `lactose-task` changes.

Locally, the heavy gates run through `cargo xtask heavy`, which bounds them to two at a time
across every worktree on the machine (D127); CI does not use it.

A failing gate stops the merge uncommitted; the coordinator decides who fixes it.

## 5. Cost

An agent run is several hundred thousand tokens; reviews that write benchmarks cost the most.
So the first review of a crate probes and measures at full depth, later rounds are scoped to
the diff and re-run existing probes, and the coordinator keeps its own context short (state
lives in briefs and reports, so compaction loses little). Running tracks in parallel spends
the same tokens sooner, not more of them.

## 6. Commits

Every commit ends with a `Co-Authored-By:` line naming the model that wrote it and a
`Claude-Session:` line; a merge of an agent's branch carries the lines of both the agent's
model and the coordinator's.
