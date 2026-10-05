# lactose-bundle

The region bundle format shared by RICOTTA, SILO, SKIM and MILKMAN: how a cooked entity gets
its identity, what a region's cooked baseline is, and what an overlay against it is.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

It depends on `lactose-core` and, for the `AssetId` and `AssetSet` types only, on
`lactose-asset-api`. It does not depend on `lactose-reflect` or `lactose-ecs`: component and
field bytes are opaque here, stored under their `ComponentKey` and `FieldId`.

## What it holds

| Item | Purpose |
|---|---|
| `PlacementGuid`, `cooked_net_id` | The one derivation of a cooked entity's `NetId` (architecture section 7) |
| `Baseline`, `Archetype`, `Column`, `BaselineRow` | A region's cooked entities as columns of raw component bytes per archetype, with the region's asset set |
| `Overlay`, `Record` | The seven record kinds of architecture section 15.2, keyed by `NetId` |
| `ComponentData`, `MAX_COMPONENT_BYTES` | One component's bytes with its schema version, and the size limit |
| `Baseline::encode`, `Baseline::decode`, `Baseline::content_hash` | Canonical bytes and the baseline content hash |
| `Overlay::encode`, `Overlay::decode`, `Overlay::digest` | Canonical bytes and the overlay digest |
| `Overlay::rebase` | The patched-baseline rule |

## Rules a caller should know

**Cooked identity.** `cooked_net_id(scene, placement, child_path)` hashes the authoring
scene's `AssetId`, the placement's GUID, and the GUIDs of the nested placements that lead to
the entity. Path elements are GUIDs, never indices, so reordering or adding siblings does not
change an ID. RICOTTA checks the IDs of a whole world for collisions.

**Documents are immutable and canonical.** `Baseline::from_rows` and `Overlay::new` accept
their input in any order, sort it, and reject contradictions. Equal content means equal
values and equal bytes; a decoder accepts only the canonical bytes.

**Baseline.** Rows are grouped by exact component set. Each column is the component's bytes
for every row, back to back, with its schema version and element size, so loading an
archetype is one copy per column followed by the reference remap pass. Rows are in ascending
`NetId` order.

**Overlay.** Records are ordered by entity and, within an entity, in the order they apply:
`Spawn` or `SpawnFromBaseline` or `Destroy`, then `AddComponent`, `RemoveComponent`,
`SetField`, `OwnershipChange`.

- `Spawn` (a runtime `NetId`) and `SpawnFromBaseline` (a cooked `NetId`) carry every
  persistent component whole.
- A cooked entity that moves from region A to region B is `Destroy` in A's overlay and
  `SpawnFromBaseline` in B's, so B loads without A. A runtime entity that moves leaves A's
  overlay and becomes a `Spawn` in B's.
- `Destroy` names a cooked entity of this region's baseline and is the only record with that
  ID. A destroyed runtime entity simply has no records.
- An overlay records the content hash of the baseline it was made against. If that differs
  from the baseline at hand, the baseline was patched and `Overlay::rebase` carries the
  overlay over: it keeps everything except `SetField` records whose entity or component is
  gone, and returns what it dropped.

**Encoding.** Little-endian, `u32` lengths, a magic and format version in front and the
xxh64 of everything before it at the end. That trailing hash is the content hash. The layout
tables are in the crate documentation and on `Baseline` and `Overlay`; golden
bytes for both documents are pinned in `tests/format.rs`. Compression is not part of the
format: `io` and `persist` wrap these bytes.

**Untrusted input.** `decode` returns `lactose_core::Error` and never panics. Every count is
checked against the bytes that remain before anything is allocated or looped over, and
component data is capped at `MAX_COMPONENT_BYTES`. The hash detects damage; it is not a
signature, so the structural checks do not rely on it.
