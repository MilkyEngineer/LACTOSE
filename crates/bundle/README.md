# lactose-bundle

The region bundle format shared by RICOTTA, SILO, SKIM and MILKMAN: how a cooked entity gets
its identity, what a region's cooked baseline is, and what an overlay against it is.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

It depends on `lactose-core` and, for the `AssetSet` type only, on `lactose-asset-api`. It does
not depend on `lactose-reflect` or `lactose-ecs`: component and field bytes are opaque here,
stored under their `ComponentKey` and `FieldId`.

## What it holds

| Item | Purpose |
|---|---|
| `AuthoredGuid`, `cooked_net_id` | The one derivation of a cooked entity's `NetId` (architecture section 7) |
| `ComponentSchema`, `FieldLayout` | A component's schema version, size and field layout, carried by every document that holds its bytes |
| `Baseline`, `Archetype`, `Column`, `BaselineRow` | A region's cooked entities as columns of raw component bytes per archetype, with the region's asset set |
| `Overlay`, `Record` | The seven record kinds of architecture section 15.2, keyed by `NetId` |
| `ComponentData` | One component's bytes |
| `Baseline::encode`, `Baseline::decode`, `Baseline::content_hash` | Canonical bytes and the baseline content hash |
| `Overlay::encode`, `Overlay::decode`, `Overlay::digest` | Canonical bytes and the overlay digest |
| `Overlay::check_against`, `Overlay::rebase` | Whether an overlay fits a baseline, and the patched-baseline rule |
| `MAX_COMPONENT_BYTES`, `MAX_DOCUMENT_BYTES` | The size limits |

## Rules a caller should know

**Cooked identity.** `cooked_net_id(scene, placement, child_path)` hashes three kinds of
authored GUID: the scene's own, the placement's, and the chain leading to the entity inside
what was placed. Nothing in it is a name, a path or an index, so renaming a scene file or
reordering siblings changes no ID. Every entity inside a multi-entity prefab needs an authored
GUID. RICOTTA checks the IDs of a whole world for collisions.

**Documents are immutable and canonical.** `Baseline::from_rows` and `Overlay::new` accept
their input in any order, sort it, and reject contradictions. Equal content means equal
values and equal bytes; a decoder accepts only the canonical bytes.

**Documents describe themselves.** Each carries a `ComponentSchema` for every component it
holds bytes of: schema version, element size, and the offset and size of each field by
`FieldId`. The caller supplies schemas as plain data from the reflection registry, and may
pass the whole registry. This crate checks that each schema is well-formed and that the bytes
stored under it have its size. A save written under an old schema can therefore be decoded
field by field by a build that no longer knows that layout.

**Baseline.** Rows are grouped by exact component set. Each column is the component's bytes
for every row, back to back, so loading an archetype is one copy per column followed by the
reference remap pass. Rows are in ascending `NetId` order.

**Overlay.** Each state of a region has one record set:

| Entity | Records |
|---|---|
| Runtime-spawned, residing here | `Spawn` with every persistent component, optionally `OwnershipChange` |
| Cooked in another region, residing here | `SpawnFromBaseline` with every persistent component, optionally `OwnershipChange` |
| Row of this region's baseline, changed | `AddComponent` (a whole component, also over one the row has), `RemoveComponent`, `SetField`, `OwnershipChange` |
| Row of this region's baseline, destroyed or moved away | `Destroy`, alone |

A cooked entity that moves from region A to region B is `Destroy` in A's overlay and
`SpawnFromBaseline` in B's, so B loads without A. `Overlay::new` and `Overlay::decode` enforce
what can be judged from the records alone, including that no entity is its own owner and that
the ownership records form no cycle among themselves. A `Destroy` says only that the entity is
no longer a row of this region, which a move to another region or an unload also is, so an
owner this overlay destroys is accepted; whether an owner exists is the applier's. These are
the format's only judgements of ownership, and a new one must be one the world refuses too
(`crates/ecs/README.md`, "Ownership"), or an export could fail to be an overlay.
`Overlay::check_against` judges what needs the baseline. Left to whoever applies an overlay:
that each owner exists, ownership cycles that pass through the baseline's own ownership or
through other regions, schema agreement or migration, and cross-region references.

**Patched baselines.** An overlay records the content hash of the baseline it was made
against. If that differs from the baseline at hand, `Overlay::rebase` produces an overlay that
fits the patched one and returns the records it dropped, so `persist` can re-home those of an
entity the patch moved elsewhere. It keeps every runtime-spawned and migrated entity; drops
every record of a cooked entity that is no longer a row; drops `RemoveComponent` and
`SetField` of a component the row no longer has; and keeps `AddComponent`, which overwrites.
A migrated entity that the patch made a local row is rewritten as component records.

**Encoding.** Little-endian, `u32` counts, a magic and format version in front and the xxh64
of everything before it at the end. That trailing hash is the content hash, and a document
keeps it, so `content_hash` and `digest` cost nothing. The layout tables are in the crate
documentation and on `Baseline` and `Overlay`; golden bytes for both documents are pinned in
`tests/format.rs`. Compression is not part of the format: `io` and `persist` wrap these bytes.

**Untrusted input.** `decode` returns `lactose_core::Error` and never panics. It refuses more
than `MAX_DOCUMENT_BYTES` (256 MiB) before reading anything, checks every count against the
bytes that remain before allocating or looping, and takes data sizes from the schema table.
The hash detects damage; it is not a signature, so the structural checks do not rely on it.

**What a size cap buys.** On a 64-bit host decoding allocates at most eight times the encoded
length, and runs in time proportional to the length times its logarithm. The densest inputs
are an overlay of `Destroy` records (9 bytes each, held as 48-byte records) and a baseline of
one-row archetypes without components (16 bytes each, held in 96 bytes). A caller that
caps the byte length it passes to `decode` at N therefore caps the memory at 8 N; `net` should
choose N for overlays well below `MAX_DOCUMENT_BYTES`.
