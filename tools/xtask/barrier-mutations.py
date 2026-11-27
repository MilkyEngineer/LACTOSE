#!/usr/bin/env python3
"""The renderer's barrier mutations (docs/verification.md, the rows of the barrier between
dispatches, the scatter-to-vertex barrier and the barrier across frames).

Every dependency between the scatter and the draw passes through device addresses, which the
Khronos layer's synchronisation validation does not track, and lavapipe runs each dispatch to
completion, so neither shows a dropped barrier. Each run here copies this checkout (no target,
no .git) into a fresh directory, drops one barrier in `lactose-rhi-vulkan`'s encoder, adds to
the scatter shader the lag it needs where it needs one, and runs `cargo xtask gpu --only
vulkan` there, which cooks the copy's `shaders/` and runs every device test. A mutant is
caught where the test named for it fails and its control passed. Each mutant's control runs
before it: its lag alone, or the checkout unpatched where it adds no lag. A control keeps
every barrier and must pass every test. `--run` of a mutant runs its control too. The
checkout is never touched.

The verdict is read from the gate's report under the copies' target directory,
`<out>/target/gpu/<driver>-vulkan[-gpu-assisted].txt`, removed before each run, under each
driver the run took: `caught` on the named test's `FAILED` line alone, `MISSED` on its
`passed` line, `KILLED` where the test ran past its deadline, which shows nothing of the
barrier, `NOT JUDGED` where its control did not pass under that driver, and `NOT RUN` on no
such line (its `skipped` line, or no report). The gate prints no passing test's line to the
console, so its log says nothing of a mutant that was missed.

The barrier between dispatches is the one that matters: nothing else orders two dispatches of
the scatter. On the M1 it is missed under MoltenVK, whose serial compute encoder orders the
dispatches, and caught under KosmicKrisp. The other two cannot fail as the frame is declared
(reviewer-gpu's round-1 G5 of `renderer-impl`): dropping the instance buffer's transition
leaves other imports' transitions from `Undefined` before the same pass, each a full execution
dependency; across frames, a write after a read is all the hazard needs, so that mutation is
correct by the specification, and before the triangle pass it drops only the memory dependency
on the instance buffer, which desktop drivers carry out as a global cache operation. They run
to record that; their survival does not show the barriers unneeded.

Usage, from anywhere, under the GPU lock the host uses:

    python3 -B tools/xtask/barrier-mutations.py <out dir> [--run <name>]... [-- <gpu args>...]
    py -3 -B tools\\xtask\\barrier-mutations.py <out dir> [--run <name>]... [-- <gpu args>...]

with `<out dir>` a directory outside the checkout (or under its `target` or `.scratchpad`),
made if it is not there, and the gpu arguments what `cargo xtask gpu` takes, such as
`--driver system --adapter "<the GPU's name>"`; with none on macOS the gate runs MoltenVK and
KosmicKrisp both, and under `--gpu-assisted` the driver must be named. `--run` runs the named
runs and their controls alone, in the order below. It writes each run's log,
`<out>/<run>.log`, and the verdicts as lines of tab-separated values to standard output and to
`<out>/barrier-mutations.tsv`: the run, its control, the driver, the gate's exit status, the
verdict and what the report says of it. It exits 1 where a control failed or a mutant was
killed, not judged or not run, 2 where a patch does not apply, and 0 otherwise: a mutant
missed is the record, not a failure of the script.
"""

import glob
import os
import shutil
import subprocess
import sys
import tempfile

CHECKOUT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENCODER = "crates/rhi-vulkan/src/encoder.rs"
SCATTER = "shaders/renderer/scatter.slang"
# What the copy leaves out, at any depth; an output directory under one of the first three is
# left out with it.
LEFT_OUT = ("target", ".scratchpad", ".claude", ".git")

# Rounds of a dependent multiply-add per invocation: tens of milliseconds on a desktop GPU,
# far past the draw that follows. Its result is written to the record's reserved word only
# where the chain meets a value it is not expected to meet, so it cannot be removed, and
# `InstanceRecord::from_bytes` reads no reserved word. Only one invocation in 64 spins, so
# that a dispatch of 65,536 entries lags for tens of milliseconds and not for minutes.
LAG = "(id.x % 64u == 0u ? (1u << 22) : 0u)"
SPIN = f"""uint spin = id.x;
    for (uint k = 0; k < {LAG}; k++)
        spin = spin * 1664525u + 1013904223u;
"""

# The scatter's writes land late: a dropped scatter-to-vertex barrier lets the draw read the
# records before them.
LAG_SCATTER = (SCATTER, """    StagingRow row = push.rows[entry.row];
""", f"""    {SPIN}    StagingRow row = push.rows[entry.row];
""")
LAG_SCATTER_END = (SCATTER, """    record.reserved = 0;
""", """    record.reserved = spin == 0x9e3779b9u ? 1u : 0u;
""")
# A `FREE` entry's write lands late: a dropped barrier between dispatches lets a later
# dispatch's write to the same slot land first, and the free then lands over it.
LAG_FREE = (SCATTER, """        record.flags = 0;
        return;
""", f"""        {SPIN}        record.flags = 0;
        record.reserved = spin == 0x9e3779b9u ? 1u : 0u;
        return;
""")

# The graph's transition of a buffer from `ShaderWrite` to `ShaderRead`: the scatter to the
# triangle pass's vertex stage, through the instance buffer.
DROP_WRITE_TO_READ = (ENCODER, """            let Some(object) = self.buffer(transition.buffer) else { return };
""", """            let Some(object) = self.buffer(transition.buffer) else { return };
            if transition.from.state == BufferState::ShaderWrite
                && transition.to.state == BufferState::ShaderRead
            {
                continue;
            }
""")
# The import's transition from `ShaderRead` to `ShaderWrite`: the frame before's draw to this
# frame's scatter.
DROP_READ_TO_WRITE = (ENCODER, """            let Some(object) = self.buffer(transition.buffer) else { return };
""", """            let Some(object) = self.buffer(transition.buffer) else { return };
            if transition.from.state == BufferState::ShaderRead
                && transition.to.state == BufferState::ShaderWrite
            {
                continue;
            }
""")
# The barrier between two dispatches of a compute pass that declares a buffer `ShaderWrite`.
DROP_BETWEEN_DISPATCHES = (ENCODER, """        let ordered = pass.writes && pass.dispatched;
""", """        let ordered = false && pass.writes && pass.dispatched;
""")

PACKAGE = "lactose-renderer"
WORLD = "device::a_world_stepped_and_extracted_renders_as_the_reference"
GIVEN = "device::slots_freed_by_one_run_and_given_by_the_next_hold_the_new_instances"
MOVED = "device::a_held_frame_draws_its_own_records_though_the_next_frame_moves_them"

# (name, patches, the test that must fail, or None for a control, which must pass every
# test; its control, run before it; whether G5 says it cannot fail)
RUNS = [
    ("control-unpatched", [], None, None, False),
    ("control-lag-scatter", [LAG_SCATTER, LAG_SCATTER_END], None, None, False),
    ("control-lag-free", [LAG_FREE], None, None, False),
    ("between-dispatches", [DROP_BETWEEN_DISPATCHES, LAG_FREE], GIVEN, "control-lag-free", False),
    ("scatter-to-vertex", [DROP_WRITE_TO_READ, LAG_SCATTER, LAG_SCATTER_END], WORLD,
     "control-lag-scatter", True),
    # The lag is the gate the test holds frame 1 behind, so the checkout unpatched is its
    # control.
    ("across-frames", [DROP_READ_TO_WRITE], MOVED, "control-unpatched", True),
]


class PatchMiss(Exception):
    """A patch whose text is not once in its file."""


def read(path):
    with open(path, encoding="utf-8", newline="") as source:
        return source.read()


def write(path, text):
    with open(path, "w", encoding="utf-8", newline="") as out:
        out.write(text)


def inside(path, root):
    """Whether `path` is `root` or under it, each with its links resolved; never where the two
    are on different drives."""
    path, root = os.path.normcase(os.path.realpath(path)), os.path.normcase(os.path.realpath(root))
    try:
        return os.path.commonpath([path, root]) == root
    except ValueError:
        return False


def same(path, other):
    """Whether two paths name one directory, each with its links resolved."""
    return (os.path.normcase(os.path.realpath(path))
            == os.path.normcase(os.path.realpath(os.path.abspath(other))))


def copy(out, name):
    """A copy of the checkout in a directory made fresh for it, which no earlier run's files,
    nor a handle Windows still holds on one, can reach."""
    tree = tempfile.mkdtemp(prefix=f"tree-{name}-", dir=out)
    shutil.copytree(CHECKOUT, tree, ignore=shutil.ignore_patterns(*LEFT_OUT), dirs_exist_ok=True)
    # copytree keeps each file's time, and cargo judges a crate fresh by its sources' times
    # alone: the encoder is made newer than any earlier run's build, so that each run, a
    # control too, builds its own and never the last mutant's left in the target directory.
    os.utime(os.path.join(tree, *ENCODER.split("/")))
    return tree


def patch(tree, patches):
    for path, old, new in patches:
        full = os.path.join(tree, *path.split("/"))
        text = read(full)
        if text.count(old) != 1:
            raise PatchMiss(f"the text to patch is not once in {path}")
        write(full, text.replace(old, new))


def drivers(gpu_args):
    """The drivers the gate runs under, as it names its reports."""
    if "--driver" in gpu_args:
        at = gpu_args.index("--driver")
        if at + 1 < len(gpu_args):
            return [gpu_args[at + 1]]
    if sys.platform != "darwin":
        sys.exit("barrier-mutations: on this host the gate takes a driver: pass "
                 "`-- --driver system`")
    if "--gpu-assisted" in gpu_args:
        # The gate's GPU-assisted form leaves MoltenVK out, and the absence of its report
        # would read as a control's failure.
        sys.exit("barrier-mutations: under `--gpu-assisted` on macOS name the driver: pass "
                 "`-- --gpu-assisted --driver kosmickrisp`")
    return ["moltenvk", "kosmickrisp"]


def entry(report, test):
    """The named test's word in the gate's report, or `None` and why there is none."""
    if report is None:
        return None, "no report"
    prefix = f"{PACKAGE} {test}: "
    words = [line[len(prefix):] for line in report.splitlines() if line.startswith(prefix)]
    if len(words) != 1:
        return None, f"{len(words)} entries for {test}"
    return words[0], words[0]


def verdict(report, test, masked, control_passed):
    """The mutant's verdict on the named test's line in the gate's report, and that line."""
    word, detail = entry(report, test)
    if word is None:
        return "NOT RUN", detail
    if word.startswith("FAILED") and "past its deadline" in word:
        return "KILLED", detail
    if not control_passed:
        return "NOT JUDGED, its control did not pass", detail
    if word.startswith("FAILED"):
        return "caught", detail
    if word == "passed":
        return ("MISSED, as G5 expects" if masked else "MISSED"), detail
    return "NOT RUN", detail


def failed_count(report):
    for line in (report or "").splitlines():
        if line.startswith("failed: "):
            return line[len("failed: "):]
    return None


def parse(argv):
    if not argv or argv[0].startswith("-"):
        sys.exit(__doc__)
    out, rest, chosen = argv[0], argv[1:], []
    gpu_args = []
    while rest:
        first, rest = rest[0], rest[1:]
        if first == "--":
            gpu_args = rest
            break
        if first == "--run" and rest:
            if rest[0] not in [run[0] for run in RUNS]:
                sys.exit(f"barrier-mutations: no run is named `{rest[0]}`")
            chosen.append(rest[0])
            rest = rest[1:]
        else:
            sys.exit(f"barrier-mutations: unrecognised argument `{first}`\n\n{__doc__}")
    out = os.path.abspath(out)
    if inside(out, CHECKOUT):
        under = os.path.relpath(out, CHECKOUT).split(os.sep)[0]
        if under not in LEFT_OUT[:3]:
            sys.exit("barrier-mutations: the output directory is inside the checkout, which "
                     "each copy would take in: give one outside it, or under its `target`, "
                     "`.scratchpad` or `.claude`")
    # Cargo names a workspace's crates by their paths within it and judges them fresh by
    # their sources' times, so a mutant built into the checkout's own target directory is
    # what the checkout's next build runs, unrebuilt.
    target = os.path.join(out, "target")
    owns = [os.path.join(CHECKOUT, "target"), os.environ.get("CARGO_TARGET_DIR")]
    if any(own and same(target, own) for own in owns):
        sys.exit("barrier-mutations: `<out dir>/target` is the checkout's own target "
                 "directory, which a mutant's build would leave its crates in: give an "
                 "output directory whose `target` is its own")
    # A mutant's verdict stands on its control's pass, so a chosen mutant brings its control.
    controls = [run[3] for run in RUNS if run[0] in chosen and run[3]]
    runs = [run for run in RUNS if not chosen or run[0] in chosen or run[0] in controls]
    return out, runs, gpu_args


def main():
    out, runs, gpu_args = parse(sys.argv[1:])
    names = drivers(gpu_args)
    os.makedirs(out, exist_ok=True)
    target = os.path.join(out, "target")
    suffix = "-gpu-assisted" if "--gpu-assisted" in gpu_args else ""
    reports = {driver: os.path.join(target, "gpu", f"{driver}-vulkan{suffix}.txt")
               for driver in names}
    rows, failing, passed = [], False, set()
    for name, patches, must_fail, control, masked in runs:
        tree = copy(out, name)
        try:
            patch(tree, patches)
        except PatchMiss as miss:
            shutil.rmtree(tree, ignore_errors=True)
            print(f"barrier-mutations: {name}: {miss}", file=sys.stderr)
            sys.exit(2)
        # An earlier run's report would be read as this one's.
        for stale in glob.glob(os.path.join(target, "gpu", "*-vulkan*.txt")):
            os.remove(stale)
        log = os.path.join(out, f"{name}.log")
        env = dict(os.environ, CARGO_TARGET_DIR=target)
        print(f"barrier-mutations: {name} in {tree}", file=sys.stderr, flush=True)
        with open(log, "wb") as sink:
            code = subprocess.run(["cargo", "xtask", "gpu", "--only", "vulkan", *gpu_args],
                                  cwd=tree, env=env, stdout=sink,
                                  stderr=subprocess.STDOUT).returncode
        shutil.rmtree(tree, ignore_errors=True)
        for driver in names:
            path = reports[driver]
            report = read(path) if os.path.isfile(path) else None
            if must_fail is None:
                # The exit status is the whole gate's, under every driver; the report is this
                # driver's.
                failed = failed_count(report)
                passes = failed == "0"
                said = "control passes" if passes else "CONTROL FAILED"
                detail = "no report" if report is None else f"failed: {failed}"
                failing |= not passes
                if passes:
                    passed.add((name, driver))
            else:
                said, detail = verdict(report, must_fail, masked, (control, driver) in passed)
                failing |= not (said == "caught" or said.startswith("MISSED"))
            rows.append((name, control or "none", driver, str(code), said, detail))
            print("\t".join(rows[-1]), flush=True)
    with open(os.path.join(out, "barrier-mutations.tsv"), "w", encoding="utf-8",
              newline="") as tsv:
        for row in rows:
            tsv.write("\t".join(row) + "\n")
    sys.exit(1 if failing else 0)


if __name__ == "__main__":
    main()
