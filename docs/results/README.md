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

Run on a quiet machine: the numbers are wall-clock. A file records the commit it was measured
at, with `-dirty` appended if the tree had uncommitted changes, and the core topology the
executor detected.
