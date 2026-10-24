# lactose-asset-api

The asset interface: what every crate that defines, cooks, loads or reads an asset agrees on.
The store behind it is SILO (`lactose-asset`).

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; boundary crates may use its types (architecture section 8.2), so its `clippy.toml` carries the boundary's bans on files, the environment, subprocesses, the network and the standard library's thread creation; the conformance suite's one threaded check lifts the thread ban on its one item, and the `conformance` feature that compiles the suite is library code that `[dev_only_features]` in `crate-tiers.toml` holds to dev-dependency edges, which `cargo xtask tiers` checks |
| Unsafe | none, `#![forbid(unsafe_code)]`. `AssetRef<T>` is plain data through `lactose_core::typed_plain_data!`, whose `unsafe impl`s are justified in `crates/core/SAFETY.md` |

## What it holds

| Item | Purpose |
|---|---|
| `AssetId` | Stable identity: xxh64 of the declared asset name. Zero is `AssetId::NONE` |
| `AssetRef<T>` | The typed reference field components hold. An `AssetId` in memory |
| `Asset`, `AssetKind` | An asset type, its stable kind key and its cooked format version |
| `SimulationAsset` | Marks the kinds the simulation may read |
| `AssetVersion` | One published version of an asset: a hash of its cooked header and bytes |
| `VersionTable` | The versions one region sees. Sorted, comparable, hashable |
| `AssetSet` | The assets a region declares. Sorted. `lactose-bundle` embeds it in a baseline |
| `AssetHandle<T>`, `UntypedHandle` | Shared ownership of one published version |
| `PinnedTable` | A version table plus a handle to everything in it: how boundary code reads assets |
| `Processor`, `Loader`, `ErasedLoader` | Source to cooked bytes, and cooked bytes to a runtime asset |
| `CookedAsset`, `CookedHeader` | A processor's output, and the part of it a pack index records |
| `PublishedAssets` | The `dyn` resource a boundary world receives: request and lookup by exact version |
| `AssetCatalog` | The `dyn` resource code outside the boundary receives: adds newest, `resolve` and `generation` |
| `Readiness<T>`, `AssetFailure` | The values that cross the store |
| `LoadPriority`, `MemoryBudget` | Inputs for the streaming system |
| `conformance` (feature) | The suite every store must pass, and `ReferenceStore` |

## Rules a caller should know

**Identity.** An `AssetId` is the hash of a name, like `ComponentKey`. RICOTTA checks every
name it publishes for collisions, in the same pass that builds the pack index, and treats zero
as taken. Kind names are checked where loaders are registered.

**Versions are content hashes.** `CookedHeader::version_of` hashes the cooked header and
bytes, so `(AssetId, AssetVersion)` names the same content on the server, on every client, in
a replay and after any number of cooks, and no cook keeps a ledger. A store verifies the hash
when it loads (`ErasedLoader::load` does it); a mismatch is a failure value. Versions have no
order. "Newest" and "current" mean the version a store's pack index names at the moment,
which may be one that was current before.

**Dependencies are by `AssetId`, not by version.** A loaded asset keeps `AssetRef`s to its
dependencies and the reader resolves them through its own table. A loader is therefore a
plain function over decompressed bytes and never receives another asset.

**Two traits, one rule.** `PublishedAssets` answers only what `(AssetId, AssetVersion)`
fixes. `AssetCatalog` adds what depends on load timing. `app` hands a boundary world the
first and everything else the second; a `&dyn AssetCatalog` coerces to `&dyn PublishedAssets`.

**Handle policy.**

| Holder | Holds | Sees |
|---|---|---|
| A component | `AssetRef<T>` | nothing by itself; it is an ID |
| A region | `VersionTable`, and the `PinnedTable` pinned from it | exactly the table's versions, until the table is replaced at the input sync point |
| A system inside the boundary | `&T` borrowed from its region's `PinnedTable`, for `T: SimulationAsset` | the pinned version; it has no way to name "newest" |
| Code outside the boundary | `AssetHandle<T>` from `AssetCatalog::newest_typed` | the version that was current when it asked; it asks again to see a reload |

- `AssetHandle<T>` is the loaded-asset handle. It is `Arc`-backed: the version stays in memory
  while any handle or pinned table refers to it.
- `lactose_core::Handle<T>` is not used for assets. It is an index that needs a borrow of the
  table that issued it, and a `dyn` store shared between threads cannot lend `&T` out of a
  table it is concurrently publishing into. A store is free to use `HandleTable` internally.
- An `AssetHandle` holds a pointer, so it cannot be a component field. The compiler enforces
  that half of the policy.

**Simulation assets.** `PinnedTable::get` is bounded on `SimulationAsset`. Implementing the
marker promises that the kind's loader is a pure decoder of its bytes, bit-identical on every
host, or lives in a boundary crate: loaders run outside the boundary, and what such a loader
returns is simulation input. Nothing checks that promise yet.

**Loading.** Lookups never start a load and never block. `request` and `request_newest` ask
for a load; the answer to a later lookup is `Ready`, `Pending` or `Failed`.

**Region readiness.** Outside the boundary, on the server: request a region's `AssetSet`,
then call `AssetCatalog::resolve`. `Ready` carries a table that holds the current version of
every asset in the set and of everything those versions depend on. That table is journaled
and applied at the input sync point, where the region pins it with `PinnedTable::pin` and
may enter `Simulated`. `AssetCatalog::generation` tells the caller whether anything was
published since it last resolved, so it need not resolve every region every tick. A client
or a replay is given the table, requests each entry by exact version and pins it.

**Failure.** A missing or failed asset is an `AssetFailure { id, version, kind }`: plain
comparable data that reuses `lactose_core::ErrorKind`, with a fixed 17-byte encoding
(`to_bytes`, `from_bytes`) for the journal and the wire. `sim` stamps it with a tick to make
the event of architecture section 14. It converts into `lactose_core::Error`.

**Residency is not expressed yet.** Nothing here unloads an asset or applies a budget.
Milestone 3 amends the store traits with residency and budget methods; `LoadPriority` and
`MemoryBudget` are their inputs. An IO budget type is left to that work, because nothing yet
determines its shape.

## Conformance

A backend lists `lactose-asset-api` with the `conformance` feature under
`[dev-dependencies]`, implements `conformance::Harness` around its store and calls
`conformance::run`. `ReferenceStore` is the in-memory store the suite is tested against, and
a stand-in for tests of crates that need a store. It is not a backend: no IO, no unloading,
no budgets, one lock.

The feature is for dev-dependencies only. No normal or build dependency edge may enable it,
so that nothing which ships can name `ReferenceStore`.
