# Measured results

Bare-metal measurements that CI cannot make, checked in as the architecture requires (section
20). Each benchmark writes a machine-readable file and a short summary, named by benchmark,
machine and UTC date:

```text
whisk-<machine>-<date>.json
whisk-<machine>-<date>.md
```

| Benchmark | Command | Thresholds |
|---|---|---|
| WHISK (`lactose-task`) | `cargo bench -p lactose-task --bench whisk` | `crates/task/README.md` |
| Scalar precision (`scalar-precision-<machine>-<date>.md`) | The probe crate the file carries (manifest and source; the lock file is not carried, so pin the versions the file lists), built outside the workspace: `cargo build --release --manifest-path <probe>/Cargo.toml` from the repository root, then `<probe>/target/release/scalar-probe` | Architecture section 16; `docs/plans/milestone-1.md`, decision 5.4 |

Run on a quiet machine: the numbers are wall-clock. A file records the commit it was measured
at, with `-dirty` appended if the tree had uncommitted changes, and the core topology the
executor detected.
