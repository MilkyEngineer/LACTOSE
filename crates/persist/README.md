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

**Status: implemented** (milestone 1). The tests under "Proof for milestone 1" hold this
file's contract, and every sentence under "Behaviour the signatures cannot express" is
one a dependent crate may rely on. Architecture sections 4, 6, 7, 8.2, 9.3, 14, 15.1, 15.2, 15.3, 15.4, 15.5, 19
and 22 are the design; where this file and the architecture disagree, the architecture
wins and this file is wrong. Decision D129 records the choices those sections left open and
what the design review changed. `crates/ecs/README.md` ("Snapshots and bulk copy":
`export_region`, `load_region`, transient presence), `crates/sim/README.md` ("Saves and
loads", "Regions": `save_point`, `RegionExport`, `RegionBundle`, the load path) and
`crates/bundle/README.md` (the documents, their canonical form, `check_against` and
`rebase`) are the contracts this crate builds on and never restates; `crates/net-api/
README.md` ("The relevance message") is the one consumer contract it meets.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, the game crates (their migrations), the
journal tooling and the section 15.5 tests are written against this surface, and `net`
sends what `app` builds here. `tests/sketch.rs` is a compile-only sketch of what each of
them writes: a change that breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `SchemaTable`, `Schemas`, `LayoutDigest`, `SchemaHash` | The policy as a value: built once from a `Registry`, which types a save carries, the `ComponentSchema` every document carries for each, the digest of what the schema cannot see, and the hash a container records |
| `diff` | The one diff of a region's export against its cooked baseline into an `Overlay` |
| `RegionRecords` | A region's part of a save, and what the relevance message carries: the overlay (or none) against the baseline named by its hash, with the seed, the persistent resources with their schemas, the transient resource keys and the layout digests beside it; `from_export`, `check`, `to_relevance`, `rebase`, `encode_section` |
| `Container`, `ContainerHeader`, `Sections`, `EngineVersion`, `CONTAINER_FORMAT_VERSION` | The save: a header (format, engine, schema hash, tick, `NetId` counter) over one section per region; `insert`, `encode`, `decode` |
| `EncodedSection`, `Container::assemble` | One region's section made on a worker, and the file assembled from sections |
| `ContainerIndex`, `RegionIndexEntry`, `PREAMBLE_BYTES`, `Container::{index_len, index, decode_section, decode_region}` | The per-region index of architecture section 15.4, read from the front of the file without any section, and one region's section read alone |
| `MAX_CONTAINER_BYTES`, `MAX_SECTION_BYTES` | The size caps a reader enforces before it allocates |
| `Migrations`, `Migration`, `MigrateFn`, `FieldValues`, `FieldIds` | One registered step per (type, version) and the value a step edits by field ID; `migrate` for one value, `migrate_overlay` for a document |
| `Loader` | The load side: records migrated to the registry's schemas, validated, and built into the `RegionBundle` the input sync point loads |

Hidden plumbing: none. The private fields of every type are the implementation's to
reshape.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (server, the save) | One table per registry; the save copy at a sync point; one diff and one section per region on worker tasks; the bytes to write through `io` | `SchemaTable::from_registry(simulation.world().registry())` once; `Simulation::save_point` (`lactose-sim`); per `SavePoint::regions` entry, on a background task, `RegionRecords::from_export(export, &baseline, &schemas)?.encode_section()`; then `Container::assemble(&ContainerHeader::of(&save, &schemas), sections)`; the slot write is `app`'s ("Saving and loading" below). In memory: `Container::new`, `insert`, `encode` |
| `app` (server, the relevance message) | The message for a region that became relevant to a client, through the same records a save holds | `RegionRecords::from_export(export, &baseline, &schemas)?.to_relevance(table)`, on the export `app` takes for the region between runs; `net` sends it. One rule for `Cooked` and one canonicalisation, both `from_export`'s |
| `app` (a fresh process) | The tick and the counter into the builder; one bundle per region into the first frame | `Container::decode` (a whole file in memory), `ContainerHeader::{tick, next_net_id}` into `SimulationBuilder::{tick, next_net_id}`; `Loader::new(&schemas, &migrations)`, `Loader::bundle(records, baseline, mode, assets)` per section into `TickInput::loaded`; every player joins again through `TickInput::sessions` |
| `app` (a reload in place, one frame or two) | The same bundles, from the save or from the unload's exports | One frame: `unloaded` every loaded region and `loaded` the bundles of the save; two frames: `unloaded`, then `RegionLifecycle::take_exports` through `RegionRecords::from_export` and `Loader::bundle` into the next frame's `loaded` |
| `app` (a save-slot browser; a streaming load; the corrupt-save path) | What a save holds without reading a section; one region alone; what is readable of a damaged file | `Container::index_len` over the first `PREAMBLE_BYTES`, `Container::index` over that prefix; `ContainerIndex::entry`, the section's bytes at its `offset`, `Container::decode_section`, then `Loader::bundle`; a damaged section fails alone and the others decode |
| `app` (a patched baseline) | Carry a save over to re-cooked baselines | `Loader::migrate`, then `RegionRecords::rebase(&patched, &table)` per section before `Loader::bundle`, which refuses a mismatched hash; the dropped records go to the re-homing pass ("Not in this crate yet") |
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
a time, in version order, and never sees a world, a registry or another value. A nested
`#[value]` struct is one field of `FieldValues`, so a step over a nested layout change
reads the old nested offsets it hard-codes and writes the new value whole. Every layout
change to a saved persistent type is a version bump with a step, and so is renaming an
enum variant ("Schemas" below); a change to a type that stays transient is neither, since
its value is never saved ("Migrations" below).

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
  (D126), so the next step after a load reads exactly the state the save holds; the
  physics solver's state, which a journal checkpoint carries and a save does not (D124: a
  reload gets a fresh solver equal to a fresh process's); and a region's asset table and
  mode, which the loader's caller supplies (`app` resolves a table from its store;
  `Directed` control or the policy decides the mode). The world hash covers the table and
  the mode (section 8.3), so section 15.5's hash equality between a save and its load
  holds under the condition that the load resolves the same table (one store, no publish
  between the save and the load) and the mode the policy gives; a save taken after a
  tick-stamped publish and loaded against a later one hashes apart from the original, as
  it should, since the regions then see other assets. Carrying an optional table and mode
  per section is deferred ("Not in this crate yet").
- `SchemaTable::from_registry` is the policy as a value: a type is saved when its
  `Persistence` is `Persistent` or `Transient`, component or resource; `derived` types are
  known to the table (so the diff can ignore a baseline column of one) and never given a
  schema. It refuses, with `Unsupported`, a saved type whose layout holds at any depth a
  `FieldKind::Float` or a `FieldKind::Scalar` whose type tag is not
  `lactose_math::Scalar::NAME`, exactly the types `lactose-sim`'s `TypeTable::register`
  refuses, so no such type reaches a world.
- A saved value is at most `lactose_bundle::MAX_COMPONENT_BYTES`, 16 KiB, the one size a
  document carries. The ECS caps components there (a row fits a chunk) and does not cap
  resources, so a persistent region resource over 16 KiB is legal to the world, held,
  exported and hashed, and refused by `from_registry` with `InvalidData` for the whole
  table: no region of that world can be saved, found the first time the table is built.
  Until registration refuses it (D129 names that cross-crate addition), a game keeps every
  persistent resource under the cap or splits it.

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
  registry; `tests/roundtrip.rs` holds the two derivations equal, and D129 names the ECS
  exporting its derivation so there is one.
- What that schema cannot see, `LayoutDigest` does: xxh64, seed zero, over the layout
  part of `TypeInfo::write_schema`, the bytes it appends after its first fifteen (key,
  version, role, persistence, tracking), that is, from the layout's size on: size,
  alignment, each non-padding field's ID, offset, size, type tag and kind, nested layouts,
  enum sizes and discriminants, array lengths and element kinds; with two changes. A field
  or array element of kind `Struct` or `Enum` contributes an empty type tag (a zero
  length, no bytes) where `write_schema` writes its Rust name, since its structure follows
  and a name is not a layout; a primitive's tag stays, since it is what tells `u32` from
  `RegionId`. And an enum's variants are written, in declaration order, as each variant's
  value (`u32`) followed by its name (a `u32` length and the UTF-8 bytes), where
  `write_schema` writes the values alone: a saved discriminant means the variant of that
  name, so the name is part of the layout, and a permutation of discriminants among the
  variants, or of their names, is seen. A reorder inside a nested `#[value]` struct,
  renumbered discriminants and a same-size type change (`i32` to `u32`; `u64` to `Scalar`
  on a 64-bit scalar build, `u32` on a 32-bit one) leave the document schema equal and
  change the digest. The tracking bits are left out: marking a type replicated changes no
  saved byte. Free, because no saved byte depends on them: renaming a field (its ID stays)
  or a `#[value]` struct or enum type; neither is a layout change and neither needs a
  bump. Not free: renaming an enum variant changes the digest, so it is a version bump
  with a step that changes nothing (the saved value means the same variant under its new
  name), and so is reordering the variants' declarations, since the pairs are written in
  declaration order; the bump is the price of seeing a permutation. A save carries the
  digest of every type its sections hold bytes of (`RegionRecords::layouts`), and
  `Migrations::migrate` refuses equal versions with different digests. What no digest
  sees is a change of meaning under an unchanged layout (a field's unit; one `#[value]`
  struct replaced by another of the same layout; one enum by another of the same size
  with the same (name, value) pairs in the same order; one primitive by another of the
  same tag): only a version bump and a step carry that, and the build guard D129 names (a
  committed record of each saved type's version and `LayoutDigest`, compared per type by
  a test that fails when the digest changed and the version did not) is what keeps the
  bump from being forgotten. A baseline and an overlay carry no digest, so the ECS
  loader's own comparison stays blind to those three changes in a cooked column until
  `lactose-bundle`'s `ComponentSchema` carries the digest (D129, cross-crate).
- `SchemaHash::of(registry)` is xxh64, seed zero, over one stream holding, for every saved
  type ascending by key, its key (`u64`), version (`u32`), role (`u8`: 0 component, 1
  resource), persistence (`u8`: 0 persistent, 1 transient) and `LayoutDigest` (`u64`),
  little-endian. Registration order, `derived` types and the tracking bits change nothing;
  a version, a layout (through the digest), a role or a persistence change does. It is the
  container's one-comparison shortcut and what a tool prints; the loader never relies on
  it, since every section carries its own schemas and digests and `Migrations::migrate`
  compares per type. Nor would a shortcut save anything worth a second path: `Loader::
  migrate` over a section of 100k records whose types are all current costs about
  0.05 ms (Apple M1, release), against 10 to 26 ms to decode the section.

**The diff** (architecture section 15.2; `crates/bundle/README.md`, "Overlay")

`diff(rows, baseline, schemas)` compares the exported rows with the baseline's rows by
`NetId` and produces, against `baseline.content_hash()`, exactly these records and no
other, so a region state has one record set and the set is the same whatever produced the
export:

| Entity | Records |
|---|---|
| A baseline row the export lacks (destroyed, or moved to another region) | `Destroy` (a row that stayed and is owned by it keeps naming it) |
| An exported row the baseline lacks | `Spawn` (runtime ID) or `SpawnFromBaseline` (cooked ID) with every persistent component, canonical, and every transient component as zeroed bytes, ascending by key; `OwnershipChange { owner }` if the row has an owner |
| A row of both: a persistent component the export has and the baseline row lacks, or one both have whose canonical bytes differ | `AddComponent` with the whole component. Never `SetField`: a changed component is written whole (D129) |
| A row of both: a component the baseline row has and the export lacks | `RemoveComponent` |
| A row of both: a transient component | By presence alone: `AddComponent` with zeroed bytes when the export has it and the baseline row does not, `RemoveComponent` the other way, nothing when both have it, whatever the baseline's bytes |
| A row of both: the owner | `OwnershipChange { owner }` with the export's owner when it differs from the baseline row's `OwnedBy` (empty counts as none; a baseline `OwnedBy` whose reference is not a `Net` key is `InvalidData`). `ecs::owned_by` is never a component record |
| A row of both: a `derived` column of the baseline | Nothing: never compared, since the export does not carry it and the loader restores the baseline's value |

- Records within an entity are in the `Record` order `lactose-bundle` canonicalises, and
  the overlay is `Overlay::new(baseline.content_hash(), schemas.schemas(), records)`, so
  every check `Overlay::new` makes holds of the result or the diff fails. An export the
  ECS made never fails them, and a document made from one always loads: the world accepts
  no ownership state the format refuses, the format refuses no state the world accepts, and
  a cycle two regions' documents close between them is broken by the load
  (`crates/ecs/README.md`, Regions, "Ownership is a forest", and the invariant as stated
  under "Writing `OwnedBy` moves nothing"); the other checks hold of the diff by
  construction. A failing `from_export` is therefore a defect, never a state of the
  world, which is what `app`'s `server: None` on a failed unload export rests on.
- An empty record set is still an overlay; `RegionRecords::from_export` stores it as
  `None` ("cooked state"), and `to_relevance` sends it as `Cooked`.
- A component key the table does not know, on either side, is `NotFound` (the diff never
  drops data silently); a resource type among a row's components, or a value whose bytes
  are not its schema's size, is `InvalidData`.
- Whole components cost two things, stated here because D129 accepts them for the
  migration rule's sake: the unchanged fields of a changed component travel with the
  changed one (a 1 KiB component with one 8-byte field changed, over 1,000 rows, is a 1.04
  MB overlay against 27 KB with `SetField`, a factor of 38; the save is compressed, the
  relevance message is not, since `net` sends `Overlay::encode` as it is), and a content
  patch to a field the save never touched does not reach an entity whose component the
  save touched, because `Overlay::rebase` keeps an `AddComponent` whole and it overwrites
  the patched row (`SetField` would let the patch through). Both are measured and
  re-decided by the verification row D129 adds, at milestone 3.
- The cost is one pass over the exported rows and one over the baseline's index with a
  byte comparison per shared component; nothing is decoded through a `Reflect` impl. The
  implementation measures a 100k-row region (`docs/verification.md`, the save-cost item).

**Records and the relevance message** (architecture sections 9.3 and 15.2)

- `RegionRecords` is one region's part of a save and the content of a relevance message:
  the overlay carries the rows, and the seed, the persistent resources (canonical, with
  the schema each was written with), the transient resource keys and the layout digest of
  every type the records carry bytes of travel beside it, since `Overlay` has a field for
  none of them (`docs/verification.md`). `to_relevance(table)` is the message: the same
  fields copied, the overlay as `Full` or `Cooked`, the digests and resource schemas left
  behind since both ends share a build by the handshake's protocol hash. So the message's
  resources are the save's, canonical, and the one rule for `Cooked` is `from_export`'s.
- The canonical form is `RegionRecords::check`: `overlay` is `None` or holds a record,
  never `Some` of an empty overlay; `baseline_hash` equals the overlay's; `resources` is
  strictly ascending by key, with `resource_schemas` one per resource in the same order,
  each naming its resource and stating its size; `transient_resources` is strictly
  ascending and names no type `resources` holds (a resource is saved by value or by
  presence, never both); `layouts` has exactly one entry per component the overlay carries
  bytes of and per resource. `from_export`, `Loader::migrate` and `decode_section` produce
  it; `Container::insert`, `encode_section`, `Loader::bundle` and `decode_section` refuse
  records that fail it with `InvalidData` naming the first failure; so "equal state gives
  equal records" holds for every `RegionRecords` this crate handles, except that records
  migrated from an older build keep the saved bytes of a now-transient component, which no
  loader reads; a caller that assembles records by hand runs `check` first.
- An unload, a save and a relevance message are one path: the ECS's export, `from_export`,
  and the bundle or the message built from the records. The server's `app` runs it on the
  export of a region that became relevant and `net` sends the message; co-located clients
  share one export and one overlay (`crates/net-api/README.md`). The client never diffs
  and caches overlays as bytes by digest.
- `rebase(&patched, &table)` carries records over to a re-cooked baseline whose hash
  differs, and its result is the record set `from_export` would give the same state
  against `patched`: `Overlay::rebase` decides what is kept and what is dropped
  (architecture section 15.2: every runtime-spawned and migrated entity kept, every record
  of a cooked entity that is no longer a row dropped, a `RemoveComponent` or `SetField` of
  a column the row lacks dropped); the kept records are applied to the patched rows into
  the export's form (the owner from `OwnershipChange`, else the row's `ecs::owned_by`
  column; persistent components; transient components by presence; a derived column is
  the baseline's) and diffed by the rule of "The diff" against `patched`. So an
  `AddComponent` equal to the patched value, a transient the row already has, a derived
  column and `ecs::owned_by` leave no record, a different `AddComponent` stays whole, a
  component the save never touched takes the patched value, and a changed owner is an
  `OwnershipChange`: one state has one record set whichever path made it, and a rebased
  section's digest equals a fresh export's. The records must pass `check` (`InvalidData`)
  and be at the table's schemas, as `Loader::migrate` leaves them (`VersionMismatch` for a
  type the overlay carries at another schema or digest, `NotFound` for one the table
  lacks); the records are unchanged on failure.
  Afterwards `baseline_hash` is `patched.content_hash()`, the overlay passes
  `check_against(patched)` or is `None` if nothing is left, the dropped records are
  returned in order, and `layouts` holds the table's digest of every type the new overlay
  carries and the resources' entries as they were; a region with no overlay only takes the
  new hash. A patch whose cooked ownership closes a cycle with a saved `OwnershipChange`
  within one region is refused at `load_region`, since the cycle lies within the document
  (the cook and the save contradict); one that crosses regions is broken by the load like
  any other (`crates/ecs/README.md`, Regions, "Ownership is a forest"). Neither is repaired
  by `rebase`; the re-homing pass (milestone 3) is the place to decide it. The re-homing of dropped records to the region whose patched
  baseline now has the entity is not in this crate yet.

**Migrations** (architecture section 15.4)

- A transient type is taken by presence, never by value: `Migrations::migrate` returns the
  all-zero value of the registry's size for a type whose persistence is `Transient`,
  whatever the document's schema, digest and bytes, with no version check and no step; a
  layout change to a type that stays transient therefore breaks no save and needs no bump
  (one made persistent since the save is read from its saved version and digest, next
  bullet). `migrate_overlay` never reads such a component's bytes: when the overlay's
  schema of it equals the table's they are kept as they are, whether the zeroes this
  crate's diff wrote or a value saved while the type was persistent, since
  `World::load_region` zeroes a transient component whatever a document says, so the
  overlay is returned as the same `Arc`; when the schema differs, the component is
  rewritten as zeroes of the registry's size under the table's schema, since the loader
  requires every schema of a document to match the registry's. `Loader::migrate` moves a
  resource the registry now says is transient from `resources` to `transient_resources`.
- A type that became persistent is restored from a zero value, by two rules, since a
  document records which resources were transient (`transient_resources`) and not which
  components were: a transient key whose type is now persistent leaves
  `transient_resources` and joins `resources` as the all-zero value under the registry's
  schema and digest, and no step runs (a key has no saved version for one to start from);
  a component is read by the persistent rule below from the schema and digest it was saved
  under, like any component, so a type made persistent loads only if it kept the version
  and layout it was saved under, or has the steps from that version, which run over the
  all-zero old value (a bump or a layout change made while the type was transient breaks
  the older saves once it is persistent). `Loader::bundle` then validates the restored
  value like any other, so one that is invalid refuses the region with `InvalidData`. A
  type the registry now says is `derived`, component or resource, saved by value or by
  presence, is dropped: a derived value is rebuilt on load and never comes from a
  document, and the build declared it so; the component leaves its `Spawn` or
  `SpawnFromBaseline` record, which stays, an `AddComponent`, `RemoveComponent` or
  `SetField` of it is dropped whole, and an overlay left with no record becomes `None`. A
  transient type the registry lacks is `NotFound` like any removed type, since the
  document does not say it was transient.
- For a persistent type, a section's schema and digest of it are compared with the
  registry's (the table's). `Migrations::migrate` decides, in this order: a document
  version above the registry's is `VersionMismatch` (a newer save than the build); equal
  versions with an equal `LayoutDigest` are the fast path (the value is used as it is, the
  bulk-copy-plus-remap path of the ECS; no step, no validation here); equal versions with
  different digests are `VersionMismatch`, a layout changed without a bump, which the
  macros cannot see, so it is caught here; else every step from the document's version up
  to the registry's version minus one must be registered (`VersionMismatch` naming the
  first missing one), the value is split into a `FieldValues` by the document schema's
  fields, the steps run in version order, and the result is assembled by the registry's
  layout: every non-padding field of the type takes the bytes of its ID, which must be
  present and of its size (`VersionMismatch` naming the field), a field left over that the
  type lacks is `VersionMismatch` naming it (a step takes what it retires), padding is
  zeroed and the value validated through `TypeInfo::validate` (`InvalidData`). The
  document's digest is not compared on the step path: the step is written against the old
  layout and the assembly is by ID. A step's own error fails the load as returned.
- `migrate_overlay` migrates each component of every `Spawn`, `SpawnFromBaseline` and
  `AddComponent` record under the overlay's schema of that component and the digest the
  records carry for it, and rebuilds the overlay against the same baseline hash with the
  table's schemas; `Destroy`, `RemoveComponent` and `OwnershipChange` carry no component
  bytes and pass through. An overlay whose every carried type is at the registry's version
  and digest (a transient type whose overlay schema equals the table's counting as such)
  is returned as the same `Arc`, equal by content and by digest, so a client's cached copy
  still matches; a migrated overlay is a new document with a new digest. A `SetField`
  record of a persistent component whose version or digest differs is `Unsupported`: one
  field cannot be migrated without the rest of its value, and this crate's diff never
  writes one, so none is in a save it wrote. A transient type's `SetField` is dropped when
  its schema differs and kept when it is equal; either way its value loads zeroed. A
  component the registry does not know is `NotFound`, never
  dropped: deleting a type is a migration this crate does not have yet. Resources migrate
  the same way under the schema and digest the records carry for each.
- A baseline is never migrated: it is cooked by this build's cooker, so its schemas match
  by construction, and a save against an older cook is rebased onto the new baseline. A
  baseline whose schema the loader refuses is a cook that is out of date.

**The container** (architecture sections 15.3, 15.4 and 19)

- The shape is the type: a `ContainerHeader` (format, engine version, schema hash, tick,
  `NetId` counter) over one `RegionRecords` per region, ascending by region. `insert`
  replaces a region's section; `decode(encode(c)) == c`; containers compare by content.
- A region is a section of its own, made and read on its own. `RegionRecords::encode_section`
  is one zstd frame at level 3 (measured, D129), without zstd's own checksum since the
  entry's xxh64 covers the frame, carrying its content size, whose content is the region,
  the seed and the baseline hash, the overlay as `Overlay::encode` behind a presence
  byte, the resource schema table as
  `lactose-bundle` writes one (its encoder is crate-private, so this crate writes the
  same bytes and `tests/format.rs` holds the two equal), the resources as key
  and bytes of the schema's size, the transient keys ascending, and the layout digests as
  key and digest ascending. It returns an `EncodedSection`, which only it builds: the
  bytes, and a `RegionIndexEntry` that states the region, seed, baseline hash and overlay
  digest the content states too, the compressed and decompressed sizes and the xxh64 of
  the compressed bytes, with `offset` zero until placed; the fields are read through
  `entry()` and `bytes()`, so an entry and its bytes agree by construction. It is pure and
  independent per region, so `app` runs it beside `from_export` on a background worker.
  `Container::assemble(header, sections)` is the file: the preamble, the header-and-index
  block, and the sections' bytes back to back in index order, offsets assigned; `encode`
  is `encode_section` of every section then `assemble`.
- The encoding is the implementation's within these invariants, pinned by golden bytes in
  `tests/format.rs`, and `CONTAINER_FORMAT_VERSION` moves with any change
  (before the first shipped save freely; after it, with a converter): every integer
  little-endian, counts `u32`, sizes and offsets `u64`; the preamble is `PREAMBLE_BYTES`
  (16): a magic, the format version, a zero `u16` and the byte length of the
  header-and-index block; the block is the header, one index entry per section strictly
  ascending by region, and the xxh64 (seed zero) of the block's preceding bytes; the
  sections follow at the offsets the index states. There is no whole-file hash: the block
  is checked on its own and each section on its own, so a reader reads the preamble, then
  the block, then only the sections it wants, and damage to one section leaves the others
  readable.
- Reading untrusted bytes: `index_len(preamble)` needs the first `PREAMBLE_BYTES` and
  refuses fewer, a wrong magic, a non-zero reserved word or a block longer than
  `MAX_CONTAINER_BYTES` (`InvalidData`) and another format version (`VersionMismatch`);
  `index(prefix)` reads the block from a prefix of at least `index_len` bytes (a whole
  file will do) and refuses with `InvalidData` a block whose hash does not match, an index
  that is not strictly ascending by region, an entry whose section lies within the block,
  beyond `MAX_CONTAINER_BYTES` or over another's, or whose decompressed size exceeds
  `MAX_SECTION_BYTES`; `decode_section(entry, section)` takes exactly the entry's
  `compressed_bytes` at its `offset`, read by the caller, and refuses with `InvalidData` a
  slice of another length or hash (before decompressing), a frame that does not decompress
  to exactly `entry.bytes`, content that is not the canonical encodings `lactose-bundle`
  and this crate accept, content whose region, seed, baseline hash, or overlay presence
  and digest differ from the entry's (an index that disagrees with its section is never
  shown as the section), or records that fail `check`; a failure says nothing about any
  other section. Nothing is allocated on the word of a count alone, and decoding one
  section costs at most its stated size plus `lactose-bundle`'s bound on its documents.
  `decode_region` and `decode` are the in-memory shorthands over a whole file: the index,
  then one section (`NotFound` for a region the index lacks) or every section, failing on
  the first that does; the corrupt-save path of section 19 reads the index and each
  section itself and decides what to do with the regions that fail (`tests/sketch.rs`,
  `recover`).
- `assemble` is deterministic within one build (equal inputs give equal bytes) and not
  across builds (a zstd bump may change the bytes); a save's identity is the content of
  its sections (each overlay's digest, each baseline hash), never the file's bytes.
  `assemble` refuses two sections for one region (`InvalidData`) and a whole over
  `MAX_CONTAINER_BYTES` (`ResourceExhausted`); `encode_section` refuses records that fail
  `check` and a content over `MAX_SECTION_BYTES`.
- `EngineVersion::CURRENT` is the workspace version at compile time, recorded for tools;
  a load never refuses on it.

**Saving and loading: who calls what, on which thread** (architecture sections 5.1, 15.3
and 15.5; D124, D126)

- The save, in order: at a sync point `Simulation::save_point` copies persistent columns
  of every loaded region on frame-critical workers (the save pass, one child per region)
  and hands the timer root a `SavePoint`; `app` then spawns background tasks
  (`Executor::spawn`, the background class), one per region, each `from_export` then
  `encode_section` over its export, the region's `Arc<Baseline>` from the store and the
  shared `SchemaTable`; then one `assemble`; then the bytes are written to a temporary
  file beside the slot, synced, and renamed over the slot, through `lactose-io` when it
  exists and `std::fs` on a background task until then. Nothing in this crate touches a
  world, an executor or a file: every function is pure over its arguments, and every type
  crosses threads (`lib.rs` asserts it), which is what lets `app` place the work. A failed
  `from_export`, `encode_section` or `assemble` loses nothing: each is a pure function
  that returns an error and no partial value, the simulation runs on, nothing has been
  written, and the slot keeps its previous save. Measured at 100k rows (Apple M1,
  release; `docs/verification.md`): `from_export` and `encode_section` together take
  about 19 ms per region with a tenth of its rows changed and 41 to 63 ms when every
  row is new to the baseline.
- The load: on a background task `index_len` and `index` over the front of the file, then
  `decode_section` over each wanted region's bytes (or `decode` over a whole file), then
  `Loader::bundle` per region; the bundles cross into the simulation through
  `TickInput::loaded` at an input sync point, the one path a region load has. A fresh
  process builds with `SimulationBuilder::tick(header.tick).next_net_id(header.next_net_id)`
  and loads every section in its first frame, every player joining again in the same
  frame; `Loader::bundle` validates everything before anything loads, and the sync point
  validates again before `create_region`, so a refused region changes nothing.
- `Loader::bundle(records, baseline, mode, assets)`, in order: `records` passes `check`;
  `baseline.content_hash()` must equal `records.baseline_hash` (`VersionMismatch`: rebase
  first); the records are migrated (above), the overlay kept as the same `Arc` when
  nothing differs; the overlay passes `Overlay::check_against(baseline)`; every value a
  record or a resource carries is validated against the registry, a transient component's
  by presence alone since the loader zeroes it; a baseline column whose schema is not the
  table's is `VersionMismatch` here rather than at the sync point. The bundle is then the
  seed, `mode`, the baseline, the migrated overlay and resources, the transient keys and
  `assets`. `mode` is honoured under `RegionControl::Directed` and ignored under
  `Anchored`; `assets` is the table `app` resolved from its store and pinned, or `None`,
  in which case the region waits `Frozen` for a table through `TickInput::assets`.
- The reload paths are all this one path. One frame in place: `unloaded` every loaded
  region and `loaded` the bundles of the save, or of the `RegionExport`s the unload left
  in the lifecycle (both are `from_export`). Two frames: the unload, then
  `take_exports` through `from_export` and `bundle` into the next frame. A fresh process:
  above. Section 15.5's first test holds through them because the save holds everything
  the next step reads (no channel entry is in flight between ticks, D126) and the
  physics sync rebuilds a fresh solver on a reload (D124), under the table-and-mode
  condition stated under "Policy"; its fourth test is hash equality without physics and
  D124's tolerance with it.
- The relevance message is `from_export` then `to_relevance`, run by the server's `app`
  between runs on the export it takes for the region; `net` sends it and the client's
  `app` builds the bundle with `RegionRelevance::to_bundle`, never through `Loader` (the
  two sides share a build by the handshake's protocol hash, so nothing migrates on the
  wire).

## Not in this crate yet

- Field-level records: the diff writes a changed component whole and never `SetField`,
  and migration refuses a persistent component's `SetField` under a changed schema (a
  transient type's is dropped, its value loading zeroed), at the two costs stated under
  "The diff" (an uncompressed relevance message a large factor bigger for a large
  component with one changed field; a content patch not reaching the untouched fields of
  a component the save touched). The verification row D129 adds measures both on the
  milestone 3 scenes and decides whether the diff gains a per-component rule and the
  migration a rule for partial values; the record kind and the loader already accept
  them.
- The re-homing pass after `rebase`: dropped records of a cooked entity the patch moved
  are applied to the section of the region whose patched baseline has it, a `Destroy`
  staying a `Destroy` there and an `AddComponent` an `AddComponent`, and a region the save
  lacks gets a new section; records of an entity no patched baseline has are discarded.
  Milestone 3, with content patches.
- Migrations with no `MigrateFn` expression, each refused today and additive to
  `Migrations` when a game needs it: a removed type (`NotFound`; a declared drop,
  `Migrations::drop_type`); a renamed type, whose new name is a new key (the old key is
  then a removed type; a declared rename); a component split into two or merged from two
  (a declared cross-type step). Fields within one component, a renamed field or `#[value]`
  type, the layout of a type that stays transient, and a persistence change (restored
  from a zero value or dropped, "Migrations"; a component made persistent is read from
  its saved version, so a bump since then needs its steps) need none of these; a renamed
  enum variant or a reorder of its variants' declarations needs only a bump and a step
  that changes nothing ("Schemas").
- Carrying a region's asset table and mode per section, so a save pins what the hash
  covers: deferred until a save must load against a store that has published since
  (milestone 3, with `lactose-asset`); until then the condition under "Policy" holds.
- The slot write through `lactose-io` (temporary file, sync, rename), and the disk-full
  and corrupt-save policies of architecture section 19 (milestone 4): `app` holds the
  policy; this crate's part is that a damaged section is refused on its own, before
  anything is read into a world, and the other sections stay readable.
- A region's asset set beyond its baseline's (the assets of migrated residents,
  architecture section 14) in the section, and the overlay against a baseline streamed
  from `lactose-asset` rather than handed in by the caller: milestone 3, through the
  `[exceptions]` edge; `Loader::bundle` and `decode_section` are the named places.
- Player portability between hosts (architecture section 22.1, milestone 4).
- A serde bridge in `lactose-reflect`: not needed by this crate, whose documents are
  `lactose-bundle`'s canonical encodings and the container's (D129 proposes the
  amendment).

## Proof for milestone 1

The crate is listed under `[harness]` in `tools/xtask/ci-policy.toml`: its reload tests
step a simulation, so they record under SOUR (`tests/sour/mod.rs` is the glue). The tests:

| What | How |
|---|---|
| The diff | `tests/diff.rs`: a world with every change kind (a destroyed, a moved-in, a moved-out and a spawned entity; a component added, removed and changed; a transient component present and absent; an owner set, changed and cleared; a NaN written to a scalar of a changed, a spawned, a moved-in and a gained component) exported and diffed gives exactly the records of the table above, in canonical order, with the transient's bytes zeroed and the NaN canonical; the digest is equal under every `layout_mode` capacity, order and seed and with one and four workers; the empty diff is `None`; a key the table lacks and a resource among components are refused as stated and nothing is produced; a baseline column under another schema is `InvalidData` from the diff and `VersionMismatch` from `bundle` when the type is persistent, and accepted by the diff but `VersionMismatch` from `bundle` when it is derived; a persistent resource's odd NaN is canonical in the save and in the bytes `Loader::migrate` returns, its negative zero kept; the relevance message's resources equal the save's and `Cooked` exactly when the overlay is `None`; an entity owned by a cooked owner that moved to the other region and one owned by a runtime entity there save with the owner's `Destroy` and their `OwnershipChange`, the same with the owners' region unloaded, and reload to the same records in either order; a self-owner and the write that closes a two-entity cycle are refused by the world and the region saves as before; 200 seeded histories of spawns, owner writes, moves, despawns, unloads and reloads over two regions keep the world a forest with `from_export` never failing, every unload's records reload (a reload changes nothing but owner edges it breaks, each a row's edge to a live entity of the other region whose chain of owners returns to the row, and some history reaches one), and a restart in either order loads and round-trips exactly |
| Round trip | `tests/roundtrip.rs`: the diff's overlay loads back through `World::load_region` and `insert_region_resource_erased` into a world that exports the same `RegionRows` (the ECS's `bulk.rs` does this with a hand-built overlay); `SchemaTable`'s schemas are accepted by the loader for every type of a registry with nested structs, arrays, enums and padding; `decode(encode(c)) == c`; `index` and `decode_section` agree with `decode` section by section, and `assemble` of sections made in any order equals `encode`; golden container bytes in `tests/format.rs` |
| Canonical records | `tests/records.rs`: `check` refuses `Some` of an empty overlay, a `baseline_hash` other than the overlay's, unsorted or duplicate resources, schemas or keys, a schema that is not its resource's, a transient key that is also a resource's type, and `layouts` with an entry missing or extra, each naming the failure; `insert`, `encode_section` and `Loader::bundle` refuse the same and change nothing |
| Rebase | `tests/rebase.rs`: records rebased onto a patch that re-homes a moved-in entity with an equal component, a changed one, a derived column, a transient it has and the owner it has equal `from_export` of the same state against the patch, digest included, with no record naming `ecs::owned_by` as a component, where `Overlay::rebase` alone gives more; a re-homed entity with no owner, another owner and the cooked owner gives `OwnershipChange { None }`, `{ owner }` and nothing; a patch to a component the save never touched loads, a patch to the saved value leaves no record and a patch to another value keeps the saved one whole, each equal to the diff of the state the rebased records load to; what the rebase drops (a `Destroy`, an orphaned `RemoveComponent` and `SetField`, the records of a row the patch removed) is `Overlay::rebase`'s dropped list and none of it is kept; another NaN in the patch where the save holds the canonical one leaves no record, and negative zero where the save holds zero keeps it; a result with no record is `None` with the resources' digests alone and a region with no overlay only takes the hash; rebased records bundled, loaded and exported diff back to themselves; records at another digest than the table's are `VersionMismatch` and unchanged, and records that fail `check` are `InvalidData` and unchanged; a persistent `SetField` over a component the patch changed is applied to the patched value, its digest equal to that of the records `Overlay::rebase` keeps, loaded and saved |
| Section 15.5 and D124 | `tests/reload.rs`, with a small game as `crates/sim/tests/common` writes one: the original reloads in place at tick T through this crate's bundles, a fresh process loads the container's bytes, an untouched twin never reloads, all K ticks hash-equal with one store and the policy's modes (test 1 and 4; `tests/physics.rs` does the same with `lactose-physics-api`'s components, a joint and a contact, over its test backend included by path, where the reload, the fresh process and the twin are bit-equal in the hash, the backend's bytes and the handle rows at every tick, since that backend keeps no solver state the rows lack; Rapier's twin within D124's tolerance is `lactose-physics-rapier`'s own test); the two-frame path likewise; an unload and reload through `from_export` and `bundle` exports the same `RegionExport` (test 2); the save taken while a left player's row is kept |
| Migrations | `tests/migrate.rs`: a container written under `Health` version 1 (bytes pinned) loads through the step of the README's example and the world hashes as one saved at version 2 would (test 3); a newer version, a missing step, a field left over, a field unset, an invalid value and a persistent component's `SetField` under a changed schema are each refused as stated and nothing loads; a transient type's `SetField` is dropped under a changed schema and kept under an equal one, its value loading zeroed either way; the two steps of a chain run in version order, the second reading a field only the first's output has; equal versions are refused with `VersionMismatch` by the digest where the document schema is equal, one change per case: a nested reorder, discriminants renumbered in declaration order, the variants' names swapped over unchanged values (`{A = 0, B = 1}` to `{B = 0, A = 1}`), at the top level and as an array element, which only the names catch, the variants' declarations reordered, each name keeping its value (`{A = 0, B = 1}` to `{B = 1, A = 0}`), a variant renamed, a same-size type change; a transient component and a transient resource with a changed layout load zeroed with no step and no bump, a transient component with an unchanged layout leaves the overlay as the same `Arc`, and a persistent resource that became transient moves to the transient keys; a transient key whose type became persistent loads as the zeroed value with no step; a transient component whose type became persistent loads from a zero value under its saved version and digest, through its steps when the version moved since, and is refused with `VersionMismatch` when its layout moved without a bump; a restored zero that fails validation is `InvalidData`; a type that became `derived` is dropped, an overlay it emptied becoming `None`, and the world rebuilds it; a resource migrates under the schema and digest the section carries; an overlay whose types match comes back as the same `Arc` with the same digest; a second step for one (type, version) is refused |
| The schema hash and the digest | `tests/schema.rs`: a golden hash and golden digests for a small registry; equal across registration order and with a derived type added or a tracking flag set; the hash changed by a version bump, a field ID, offset, size, primitive tag or kind, a type's size, a role and a persistence change, the digest by every layout change and by none of the rest, one change per case against one base type: a nested reorder, discriminants renumbered in declaration order, the variants' names swapped over unchanged values (`{A = 0, B = 1}` to `{B = 0, A = 1}`), at the top level and as an array element, which only the names catch, the variants' declarations reordered, each name keeping its value (`{A = 0, B = 1}` to `{B = 1, A = 0}`), a variant renamed, a same-size primitive change (`u64` to `i64`, `u64` to `Scalar`), and an array element's primitive, nested order and enum changed, each changing the digest; a renamed field, `#[value]` struct or enum type leaving both equal; `from_registry` refuses a raw float, a foreign scalar tag and a 20 KiB persistent resource |
| The container's caps and the prefix read | `tests/container.rs`: a preamble too short, a wrong magic, a wrong version, a block over the cap, a damaged block hash, a non-ascending index, an entry inside the block, past the cap or overlapping another, an entry over the section cap, a section of another length, another hash or another decompressed size, and a section whose content states another region, seed, baseline hash or overlay digest than its entry are each refused as stated, with an allocation bound checked by a counting allocator in the test; `index` over exactly `index_len` bytes equals `index` over the file; one damaged section fails alone and every other section decodes |
| The consumers | `tests/sketch.rs`, type-checked: `app`'s save with per-region sections, the relevance message, the loads (fresh, in place, two frames, one region from a prefix and a section, the corrupt-save read), a game crate's migration, the shape of the tests above |
