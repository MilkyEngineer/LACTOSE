# LACTOSE

LACTOSE: Lock-free Asynchronous Chunked Task-Oriented Simulation Engine. A from-scratch Rust game engine built around a short critical path, deterministic simulation,
and a work-stealing task system with no coordinator thread. Built by AI agent teams working
against interface crates in parallel.

- [docs/architecture.md](docs/architecture.md): the binding design. Start here: goals, decisions, crate layout, determinism rules and milestones.
- [docs/decisions-log.md](docs/decisions-log.md): why each decision was made, which review finding drove it, and what it replaced.
- [docs/verification.md](docs/verification.md): spikes, third-party claims to re-verify on pinned versions, and open items, each with its gating milestone.
