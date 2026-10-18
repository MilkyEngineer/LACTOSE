# SKIM (`lactose-persist`)

The save system: the persistence policy as this crate reads it from the registry, the
world overlay diffed from a region's export against its cooked baseline into the typed
records of `lactose-bundle`, the save container that holds one such record set per region
(CARTON, its name on the tools that read it), and the migrations that bring a document
written under an older schema to the registry's.

| | |
|---|---|
| Tier | backend; `lactose-asset` is the one backend crate it may name (`crate-tiers.toml`, for the streamed baseline at milestone 3), and it names no backend crate yet |
| Determinism boundary | outside. It reads what the boundary exported and builds what the boundary loads, so it computes no float, reads no clock and orders everything by key, and its outputs are functions of its inputs on every machine ("Determinism" below); no boundary crate may depend on it |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every body that decides anything
is `todo!()`; nothing here runs. This file is the contract the implementation lane meets,
and every sentence under "Behaviour the signatures cannot express" is one a dependent crate
may rely on. Architecture sections 4, 6, 7, 8.2, 9.3, 14, 15.1, 15.2, 15.3, 15.4, 15.5, 19
and 22 are the design; where this file and the architecture disagree, the architecture
wins and this file is wrong. Decision D129 records the choices those sections left open.
`crates/ecs/README.md` ("Snapshots and bulk copy": `export_region`, `load_region`, transient
presence), `crates/sim/README.md` ("Saves and loads", "Regions": `save_point`, `RegionExport`,
`RegionBundle`, the load path) and `crates/bundle/README.md` (the documents, their canonical
form, `check_against` and `rebase`) are the contracts this crate builds on and never
restates; `crates/net-api/README.md` ("The relevance message") is the one consumer contract
it meets.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `lactose-net`, the game crates (their
migrations), the journal tooling and the section 15.5 tests are written against this
surface. `tests/sketch.rs` is a compile-only sketch of what each of them writes: a change
that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `SchemaTable`, `Schemas`, `SchemaHash` | The policy as a value: built once from a `Registry`, which types a save carries, the `ComponentSchema` every document carries for each, and the hash a container records |
| `diff` | The one diff of a region's export against its cooked baseline into an `Overlay`, shared by the save and the relevance message |
| `RegionRecords` | A region's part of a save, and what the relevance message carries: the overlay (or none) against the baseline named by its hash, with the seed, the persistent resources and their schemas, and the transient resource keys beside it; `from_export` and `rebase` |
| `Container`, `ContainerHeader`, `Sections`, `EngineVersion`, `CONTAINER_FORMAT_VERSION` | The save: a header (format, engine, schema hash, tick, `NetId` counter) over one section per region; `encode` and `decode` |
| `ContainerIndex`, `RegionIndexEntry`, `Container::index`, `Container::decode_region` | The per-region index of architecture section 15.4, read without decompressing a section, and one region's section read alone |
| `MAX_CONTAINER_BYTES`, `MAX_SECTION_BYTES` | The size caps a reader enforces before it allocates |
| `Migrations`, `Migration`, `MigrateFn`, `FieldValues`, `FieldIds` | One registered step per (type, version) and the value a step edits by field ID; `migrate` for one value, `migrate_overlay` for a document |
| `Loader` | The load side: records migrated to the registry's schemas, validated, and built into the `RegionBundle` the input sync point loads |

Hidden plumbing: none. The private fields of every type are the implementation's to
reshape.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (server, the save) | One table per registry; the save copy at a sync point; one diff per region on worker tasks; the container's bytes to write through `io` | `SchemaTable::from_registry(simulation.world().registry())` once; `Simulation::save_point` (`lactose-sim`); `RegionRecords::from_export(export, &baseline, &schemas)` per `SavePoint::regions` entry, each a background task; `Container::new(ContainerHeader::of(&save, &schemas))`, `insert`, `encode`; the slot write is `app`'s ("Saving and loading" below) |
| `app` (a fresh process) | The tick and the counter into the builder; one bundle per region into the first frame | `Container::decode`, `ContainerHeader::{tick, next_net_id}` into `SimulationBuilder::{tick, next_net_id}`; `Loader::new(&schemas, &migrations)`, `Loader::bundle(records, baseline, mode, assets)` per section into `TickInput::loaded`; every player joins again through `TickInput::sessions` |
| `app` (a reload in place, one frame or two) | The same bundles, from the save or from the unload's exports | One frame: `unloaded` every loaded region and `loaded` the bundles of the save; two frames: `unloaded`, then `RegionLifecycle::take_exports` through `RegionRecords::from_export` and `Loader::bundle` into the next frame's `loaded` |
| `app` (a save-slot browser; a streaming load) | What a save holds without reading it all; one region alone | `Container::index`; `Container::decode_region` then `Loader::bundle` |
| `app` (a patched baseline) | Carry a save over to re-cooked baselines | `RegionRecords::rebase(&patched)` per section before `Loader::bundle`, which refuses a mismatched hash; the dropped records go to the re-homing pass ("Not in this crate yet") |
| `lactose-net` (server) | The relevance message built through the same diff a save uses | `diff(&export.rows, &baseline, &schemas)` into `lactose_net_api::RegionRelevance::from_export(export, baseline.content_hash(), overlay, table)`, `None` when `records()` is empty |
| `lactose-net` (client) | Nothing: the client never diffs | It caches overlays by digest as the bytes of `Overlay::encode` (`lactose-bundle`) and builds its bundle with `RegionRelevance::to_bundle` |
| `lactose-sim` | Nothing new | `RegionExport` is the input, `RegionBundle` the output, `SavePoint::{tick, next_net_id}` the header's |
| Game crates | A migration per schema version bump, as plain functions | `register_migrations(&mut Migrations)` beside `register_types`; `Migration { component, from, run }`; `FieldValues::{read, write, take, get, set}` ("The registration a game crate writes" below) |
| The journal tooling (milestone 1) | A checkpoint's regions in the save's format | `Journal::checkpoint` is a `SavePoint`: its regions are a `Container` under the checkpoint's header; the journal's own fields (tables, modes, players, held joins, requests, services, frames) are the tooling's, beside it |
| The section 15.5 tests and D124's reload paths | A real save and load behind every path | "Proof for milestone 1" below; `tests/sketch.rs` sections 4 and 5 |

## The registration a game crate writes

A schema version bump (`version = N` on the type, the old field IDs under `retired`) ships
with the step that reads version `N - 1` and writes `N`, in a function `app` calls on the
one `Migrations` it hands the loader, the way `register_types` fills the `TypeTable`:

```rust
use lactose_core::{FieldId, Result};
use lactose_persist::{FieldValues, Migration, Migrations};
use lactose_reflect::{Reflect, component};

/// Version 1 held `i32` points under field 1; version 2 holds `u16` points under 2 and a
/// maximum under 3.
#[component(key = "game::health", persistent, replicated, version = 2, retired = [1])]
pub struct Health {
    #[field(id = 2)]
    pub points: u16,
    #[field(id = 3)]
    pub max: u16,
}

fn health_v1_to_v2(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.take(FieldId(1)); // a step takes what it retires
    let points = u16::try_from(points.max(0)).unwrap_or(u16::MAX);
    value.write(FieldId(2), points);
    value.write(FieldId(3), points);
    Ok(())
}

pub fn register_migrations(migrations: &mut Migrations) -> Result<()> {
    migrations.register(Migration { component: Health::KEY, from: 1, run: health_v1_to_v2 })
}
```

A step is a plain `fn`, so the set of migrations is a function of the build; two crates
cannot register a step for one (type, version) (`InvalidData`). A step runs on one value at
a time, in version order, and never sees a world, a registry or another value.

## Behaviour the signatures cannot express

**Policy** (architecture sections 15.1, 15.3 and 8.3)

- A save carries, per loaded region: every `persistent` component of every entity, with
  reference fields as the ECS exported them (origin keys, so a reference to a predicted
  entity round-trips through `PredictedOrigin`); the presence of every `transient`
  component, never its value; the owner of every owned entity; every `persistent` region
  resource; the presence of every `transient` region resource; the region seed; and the
  content hash of the cooked baseline the region was loaded from. Beyond the regions it
  carries the tick and the runtime `NetId` counter, so RNG streams and ordering survive a
  load (section 15.5). That is exactly what `World::export_region` yields plus the two
  header fields of `SavePoint`, and nothing is added here.
- It does not carry: `derived` columns and resources (rebuilt on load, so a `derived`
  column of a baseline keeps its cooked value after a reload); world resources
  (configuration or session state, `derived` by `lactose-sim`'s build rule); session state
  (`ClaimedBy`, `StreamingAnchor`, pending joins, the lifecycle's requests), so on a load
  every player joins again; `InRegion` (the section is the region); the structural log,
  change stamps and the entity table; events and channels, which are empty between ticks
  (D126), so the next step after a load reads exactly the state the save holds; asset
  tables and region modes, which the loader's caller supplies (`app` resolves a table from
  its store, `Directed` control or the policy decides the mode); and the physics solver's
  state, which a journal checkpoint carries and a save does not (D124: a reload gets a
  fresh solver equal to a fresh process's).
- `SchemaTable::from_registry` is the policy as a value: a type is saved when its
  `Persistence` is `Persistent` or `Transient`, component or resource; `derived` types are
  known to the table (so the diff can ignore a baseline column of one) and never given a
  schema. It refuses, with `Unsupported`, a saved type whose layout holds at any depth a
  `FieldKind::Float` or a `FieldKind::Scalar` whose type tag is not
  `lactose_math::Scalar::NAME`, exactly the types `lactose-sim`'s `TypeTable::register`
  refuses, so no such type reaches a world.

**Determinism** (architecture sections 8.1, 8.2 and 9.3)

- This crate is outside the boundary and no boundary crate names it, so nothing it does
  can change a simulation result. What it may assume of its inputs: an export is a pure
  function of region state (rows ascending by `NetId`, resources ascending by key,
  independent of chunk layout and worker count, `crates/ecs/README.md`), and a baseline
  and an overlay are canonical documents (`crates/bundle/README.md`). What it must give:
  every output is a function of its inputs, the same on every machine, under every
  layout and in every registration order. The diff's digest is what a client compares
  its cached overlay against (section 9.3) and what makes one record set per region state
  (D101), so the diff performs no float arithmetic (it copies bytes and rewrites scalar
  bits by `lactose_math`'s canonical rule), reads no clock, holds no global or thread-local
  state, and orders by key alone (`BTreeMap` and sorted vectors; never a hash map's
  iteration order). It is not held to the boundary lint file; the reviewer and
  `tests/diff.rs` (equal digests under SOUR's layouts and worker counts) hold it to this
  paragraph.
- Every value this crate writes is canonical: for each component or resource value in a
  record or beside it, every leaf at any depth whose kind is `Scalar` (tagged
  `lactose_math::Scalar::NAME`) has its little-endian bits replaced by
  `Scalar::from_canonical_bits(bits).to_canonical_bits()` (one pattern for every NaN,
  negative zero kept, D105), and every other byte is as exported. The ECS exports memory
  bytes uncanonicalised and says this crate canonicalises (`ExportedRow`); the world hash
  canonicalises the same way, so a save and a hash agree on what a value is.

**Schemas** (architecture section 15.4)

- The schema the table derives for a type, and that every document carries for it, is:
  the component key, `TypeInfo::version`, the layout's size, and one `FieldLayout` per
  non-padding top-level field with its ID, offset and size, ascending by ID; padding
  fields and nested structure are not in it, a nested `#[value]` struct or an array being
  one field of its outer size. This is byte for byte what `World::load_region` compares a
  document's schema against and refuses with `VersionMismatch` when it differs, so a
  document this crate builds from the table is accepted by the loader of the same
  registry; `tests/roundtrip.rs` holds the two derivations equal, and D129 proposes the
  ECS export its derivation so there is one.
- `SchemaHash::of(registry)` is xxh64, seed zero, over the concatenation of
  `TypeInfo::write_schema` (the encoding `lactose-reflect` pins) of every saved type,
  components and resources, in ascending component key order. Registration order and
  `derived` types change nothing; a version, a field's ID, offset, size, tag or kind, a
  type's size or its persistence does. It is the container's one-comparison shortcut and
  what a tool prints; the loader never relies on it, since every document carries its own
  schema table and `Migrations::migrate` compares per type.

**The diff** (architecture section 15.2; `crates/bundle/README.md`, "Overlay")

`diff(rows, baseline, schemas)` compares the exported rows with the baseline's rows by
`NetId` and produces, against `baseline.content_hash()`, exactly these records and no
other, so a region state has one record set and the set is the same whatever produced the
export:

| Entity | Records |
|---|---|
| A baseline row the export lacks (destroyed, or moved to another region) | `Destroy` |
| An exported row the baseline lacks | `Spawn` (runtime ID) or `SpawnFromBaseline` (cooked ID) with every persistent component, canonical, and every transient component as zeroed bytes, ascending by key; `OwnershipChange { owner }` if the row has an owner |
| A row of both: a persistent component the export has and the baseline row lacks, or one both have whose canonical bytes differ | `AddComponent` with the whole component. Never `SetField`: a changed component is written whole (D129) |
| A row of both: a component the baseline row has and the export lacks | `RemoveComponent` |
| A row of both: a transient component | By presence alone: `AddComponent` with zeroed bytes when the export has it and the baseline row does not, `RemoveComponent` the other way, nothing when both have it, whatever the baseline's bytes |
| A row of both: the owner | `OwnershipChange { owner }` with the export's owner when it differs from the baseline row's `OwnedBy` (empty counts as none; a baseline `OwnedBy` whose reference is not a `Net` key is `InvalidData`). `ecs::owned_by` is never a component record |
| A row of both: a `derived` column of the baseline | Nothing: never compared, since the export does not carry it and the loader restores the baseline's value |

- Records within an entity are in the `Record` order `lactose-bundle` canonicalises, and
  the overlay is `Overlay::new(baseline.content_hash(), schemas.schemas(), records)`, so
  every check `Overlay::new` makes (no spawn of a cooked row, no `Destroy` beside another
  record, ownership without a cycle through the records) holds of the result or the diff
  fails; an export the ECS made never fails them.
- An empty record set is still an overlay; `RegionRecords::from_export` stores it as
  `None` ("cooked state"), and the server's `app` passes `None` to
  `RegionRelevance::from_export`, which sends `Cooked`.
- A component key the table does not know, on either side, is `NotFound` (the diff never
  drops data silently); a resource type among a row's components, or a value whose bytes
  are not its schema's size, is `InvalidData`.
- The cost is one pass over the exported rows and one over the baseline's index with a
  byte comparison per shared component; nothing is decoded through a `Reflect` impl. The
  implementation measures a 100k-row region (`docs/verification.md`, the save-cost item).

**Records and the relevance message** (architecture sections 9.3 and 15.2)

- `RegionRecords` is one region's part of a save and the content of a relevance message:
  the overlay carries the rows, and the seed, the persistent resources (canonical, with
  the schema each was written with, since `Overlay` has no field for resources,
  `docs/verification.md`) and the transient resource keys travel beside it, as
  `RegionRelevance` carries them. `resources` and `resource_schemas` are ascending by key
  with one schema per resource, and `transient_resources` ascending: anything else is
  `InvalidData` wherever records are built or decoded.
- An unload, a save and a relevance message are one path: the ECS's export, this diff,
  and the bundle built from the result. The server's `app` runs `diff` on the export of a
  region that became relevant and `net` sends `RegionRelevance`; co-located clients share
  one export and one overlay (`crates/net-api/README.md`). The client never diffs and
  caches overlays as bytes by digest.
- `rebase(&patched)` carries records over to a re-cooked baseline whose hash differs:
  `Overlay::rebase`, after which `baseline_hash` is `patched.content_hash()` and the
  overlay passes `check_against(patched)`; the dropped records are returned in order. A
  region with no overlay only takes the new hash. The re-homing of dropped records to the
  region whose patched baseline now has the entity is not in this crate yet.

**Migrations** (architecture section 15.4)

- A document's schema of a type is compared with the registry's (the table's). Equal:
  the value is used as it is, the bulk-copy-plus-remap fast path of the ECS. Otherwise
  `Migrations::migrate` decides, in this order: a document version above the registry's
  is `VersionMismatch` (a newer save than the build); equal versions with different
  layouts are `VersionMismatch` (a layout changed without a bump, which the macros cannot
  see, so it is caught here); else every step from the document's version up to the
  registry's version minus one must be registered (`VersionMismatch` naming the first
  missing one), the value is split into a `FieldValues` by the document schema's fields,
  the steps run in version order, and the result is assembled by the registry's layout:
  every non-padding field of the type takes the bytes of its ID, which must be present and
  of its size (`VersionMismatch` naming the field), a field left over that the type lacks
  is `VersionMismatch` naming it (a step takes what it retires), padding is zeroed and
  the value validated through `TypeInfo::validate` (`InvalidData`). A step's own error
  fails the load as returned.
- `migrate_overlay` migrates each component of every `Spawn`, `SpawnFromBaseline` and
  `AddComponent` record under the overlay's schema of that component and rebuilds the
  overlay against the same baseline hash with the table's schemas; `Destroy`,
  `RemoveComponent` and `OwnershipChange` carry no component bytes and pass through. An
  overlay whose every schema matches is returned as the same `Arc`, equal by content and
  by digest, so a client's cached copy still matches; a migrated overlay is a new document
  with a new digest. A `SetField` record of a component whose schema differs is
  `Unsupported`: one field cannot be migrated without the rest of its value, and this
  crate's diff never writes one, so none is in a save it wrote. A component the registry
  does not know is `NotFound`, never dropped: deleting a type is a migration this crate
  does not have yet. Resources migrate the same way under the schema the records carry
  for each.
- A baseline is never migrated: it is cooked by this build's cooker, so its schemas match
  by construction, and a save against an older cook is rebased onto the new baseline. A
  baseline whose schema the loader refuses is a cook that is out of date.

**The container** (architecture sections 15.3 and 15.4)

- The shape is the type: a `ContainerHeader` (format, engine version, schema hash, tick,
  `NetId` counter) over one `RegionRecords` per region, ascending by region. `insert`
  replaces a region's section; `decode(encode(c)) == c`; containers compare by content.
- The encoding is the implementation's within these invariants, pinned by golden bytes in
  `tests/format.rs` once written, and `CONTAINER_FORMAT_VERSION` moves with any change
  (before the first shipped save freely; after it, with a converter): every integer
  little-endian, counts `u32`, sizes `u64`; a magic, the format version and a zero `u16`
  in front, as `lactose-bundle` frames its documents, and the xxh64 (seed zero) of every
  preceding byte at the end, so a reader refuses a damaged file before reading it; the
  header, then the index (one `RegionIndexEntry` per region, strictly ascending, with
  each section's offset, compressed and decompressed sizes and the xxh64 of its
  compressed bytes), then the sections back to back; a section is one zstd frame, level
  and parameters pinned by the implementation, that carries its content size, whose
  content is the overlay as `Overlay::encode` behind a presence byte, the resource schema
  table as `lactose-bundle` writes one, the resources as key and bytes of the schema's
  size, and the transient keys ascending. Sections are compressed on their own and hashed
  on their own so one region is read without touching the others, and the index is read
  without decompressing anything.
- Reading untrusted bytes: `index` refuses more than `MAX_CONTAINER_BYTES` before reading,
  another format with `VersionMismatch`, and with `InvalidData` a wrong magic, a trailing
  hash that does not match, an index that is not strictly ascending or whose entries do
  not fit the bytes, or an entry whose decompressed size exceeds `MAX_SECTION_BYTES`;
  `decode_region` refuses a section whose compressed hash differs, that does not
  decompress to exactly the size its entry states, or whose documents are not the
  canonical encodings `lactose-bundle` accepts, and `NotFound` for a region the index
  lacks; `decode` is the index then every section. Nothing is allocated on the word of a
  count alone, and decoding one section costs at most its stated size plus
  `lactose-bundle`'s bound on its documents.
- `encode` is deterministic within one build (equal containers give equal bytes) and not
  across builds (a zstd bump may change the bytes); a save's identity is the content of
  its sections (each overlay's digest, each baseline hash), never the file's hash.
- `EngineVersion::CURRENT` is the workspace version at compile time, recorded for tools;
  a load never refuses on it.

**Saving and loading: who calls what, on which thread** (architecture sections 5.1, 15.3
and 15.5; D124, D126)

- The save, in order: at a sync point `Simulation::save_point` copies persistent columns
  of every loaded region on frame-critical workers (the save pass, one child per region)
  and hands the timer root a `SavePoint`; `app` then spawns background tasks
  (`Executor::spawn`, the background class), one `RegionRecords::from_export` per region,
  each needing only its export, the region's `Arc<Baseline>` from the store and the shared
  `SchemaTable`, then one that inserts them into a `Container` and calls `encode`; then
  the bytes are written to a temporary file beside the slot, synced, and renamed over the
  slot, through `lactose-io` when it exists and `std::fs` on a background task until then.
  Nothing in this crate touches a world, an executor or a file: every function is pure
  over its arguments, and every type crosses threads (`lib.rs` asserts it), which is what
  lets `app` place the work. A failed diff or encode loses nothing: the simulation runs
  on, and the slot keeps its previous save.
- The load: on a background task `Container::decode` (or `index` and `decode_region`),
  then `Loader::bundle` per region; the bundles cross into the simulation through
  `TickInput::loaded` at an input sync point, the one path a region load has. A fresh
  process builds with `SimulationBuilder::tick(header.tick).next_net_id(header.next_net_id)`
  and loads every section in its first frame, every player joining again in the same
  frame; `Loader::bundle` validates everything before anything loads, and the sync point
  validates again before `create_region`, so a refused region changes nothing.
- `Loader::bundle(records, baseline, mode, assets)`, in order: `baseline.content_hash()`
  must equal `records.baseline_hash` (`VersionMismatch`: rebase first); the records are
  migrated (above), the overlay kept as the same `Arc` when nothing differs; the overlay
  passes `Overlay::check_against(baseline)`; every value a record or a resource carries is
  validated against the registry, a transient component's by presence alone since the
  loader zeroes it; a baseline column whose schema is not the table's is `VersionMismatch`
  here rather than at the sync point. The bundle is then the seed, `mode`, the baseline,
  the migrated overlay and resources, the transient keys and `assets`. `mode` is honoured
  under `RegionControl::Directed` and ignored under `Anchored`; `assets` is the table
  `app` resolved from its store and pinned, or `None`, in which case the region waits
  `Frozen` for a table through `TickInput::assets`.
- The reload paths are all this one path. One frame in place: `unloaded` every loaded
  region and `loaded` the bundles of the save, or of the `RegionExport`s the unload left
  in the lifecycle (both are `from_export`). Two frames: the unload, then
  `take_exports` through `from_export` and `bundle` into the next frame. A fresh process:
  above. Section 15.5's first test holds through them because the save holds everything
  the next step reads (no channel entry is in flight between ticks, D126) and the
  physics sync rebuilds a fresh solver on a reload (D124); its fourth test is hash
  equality without physics and D124's tolerance with it.
- The relevance message is the diff alone, run by the server's `app` between runs on the
  export it takes for the region; `net` sends it and the client's `app` builds the bundle
  with `RegionRelevance::to_bundle`, never through `Loader` (the two sides share a build
  by the handshake's protocol hash, so nothing migrates on the wire).

## Not in this crate yet

- Field-level records: the diff writes a changed component whole and never `SetField`,
  and migration refuses a `SetField` under a changed schema. When the relevance message
  or the save size asks for field deltas (a large component with one changed scalar), the
  diff gains a per-component rule and the migration a rule for partial values; the record
  kind and the loader already accept them.
- The re-homing pass after `rebase`: dropped records of a cooked entity the patch moved
  are applied to the section of the region whose patched baseline has it, a `Destroy`
  staying a `Destroy` there and an `AddComponent` an `AddComponent`, and a region the save
  lacks gets a new section; records of an entity no patched baseline has are discarded.
  Milestone 3, with content patches.
- The slot write through `lactose-io` (temporary file, sync, rename), and the disk-full
  and corrupt-save paths of architecture section 19 (milestone 4): `app` holds the policy;
  this crate's part is that a damaged file is refused before anything is read into a
  world.
- Deleting a type: a document carrying a type the registry lacks is `NotFound`. A
  declared drop (`Migrations::drop_type`), if a game needs one.
- A region's asset set beyond its baseline's (the assets of migrated residents,
  architecture section 14) in the section, and the overlay against a baseline streamed
  from `lactose-asset` rather than handed in by the caller: milestone 3, through the
  `[exceptions]` edge; `Loader::bundle` and `Container::decode_region` are the named
  places.
- The compression level, the save's size and time at 100k rows, and the schema-hash
  shortcut's use by the loader: measured at implementation (`docs/verification.md`).
- Player portability between hosts (architecture section 22.1, milestone 4).
- A serde bridge in `lactose-reflect`: not needed by this crate, whose documents are
  `lactose-bundle`'s canonical encodings and the container's (D129 proposes the
  amendment).

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()`, lists this crate under
`[harness]` in `tools/xtask/ci-policy.toml` (its reload tests step a simulation, so they
record under SOUR) and proves:

| What | How |
|---|---|
| The diff | `tests/diff.rs`: a world with every change kind (a destroyed, a moved-in, a moved-out and a spawned entity; a component added, removed and changed; a transient component present and absent; an owner set, changed and cleared; a NaN written to a scalar) exported and diffed gives exactly the records of the table above, in canonical order, with the transient's bytes zeroed and the NaN canonical; the digest is equal under every `layout_mode` capacity, order and seed and with one and four workers; the empty diff is `None`; a key the table lacks and a resource among components are refused as stated and nothing is produced |
| Round trip | `tests/roundtrip.rs`: the diff's overlay loads back through `World::load_region` and `insert_region_resource_erased` into a world that exports the same `RegionRows` (the ECS's `bulk.rs` does this with a hand-built overlay); `SchemaTable`'s schemas are accepted by the loader for every type of a registry with nested structs, arrays, enums and padding; `decode(encode(c)) == c`; `index` and `decode_region` agree with `decode` section by section; golden container bytes in `tests/format.rs` |
| Section 15.5 and D124 | `tests/reload.rs`, with a small game as `crates/sim/tests/common` writes one: the original reloads in place at tick T through this crate's bundles, a fresh process loads the container's bytes, an untouched twin never reloads, all K ticks hash-equal (test 1 and 4; with `lactose-physics-api`'s components, the reload and the fresh process equal each other and the twin within D124's tolerance); the two-frame path likewise; an unload and reload through `from_export` and `bundle` exports the same `RegionExport` (test 2); the save taken while a left player's row is kept |
| Migrations | `tests/migrate.rs`: a container written under `Health` version 1 (bytes pinned) loads through the step of the README's example and the world hashes as one saved at version 2 would (test 3); a newer version, an equal version with another layout, a missing step, a field left over, a field unset, an invalid value and a `SetField` under a changed schema are each refused as stated and nothing loads; a resource migrates under the schema the section carries; an overlay whose schemas match comes back as the same `Arc` with the same digest; a second step for one (type, version) is refused |
| The schema hash | `tests/schema.rs`: a golden hash for a small registry; equal across registration order and with a derived type added; changed by a version bump, a field ID, offset, size, tag or kind, a type's size and a persistence change; `from_registry` refuses a raw float and a foreign scalar tag |
| The container's caps | `tests/container.rs`: a file over the cap, a wrong magic, a wrong version, a damaged trailing hash, a non-ascending index, an entry past the bytes, an entry over the section cap, a section whose hash differs and one that decompresses to another size are each refused as stated, with an allocation bound checked by a counting allocator in the test |
| The consumers | `tests/sketch.rs`, type-checked: `app`'s save and loads, `net`'s message, a game crate's migration, the shape of the tests above |
