# lactose-cook

The asset cooker, binary `ricotta` (architecture sections 2 and 14): a directory of
authored sources cooked into one pack of SILO's format. At milestone 1 it cooks one kind,
`lactose-anim`'s sim `Skeleton`, from glTF sources read by the cooker's own reader over
serde_json, through a `Processor` of `asset-api` and `lactose-asset`'s `PackBuilder`.

| | |
|---|---|
| Tier | leaf |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` in the library and the binary; architecture section 21 lists `tools/cook` for the FFI to Slang and compression libraries, none of which is here yet: `SAFETY.md` |

**Status: the milestone 1 skeleton.** One kind, one pack per run, the cooked pack set's
rule. This file is the contract of the command line, the source schema and the pack set.

## Surface

| Item | What it is |
|---|---|
| `ricotta SOURCES PACK` | The binary: "The command line" below |
| `cook(io, sources, pack)` | The cook as a future over the pool, returning a `Published` per asset written; the binary drives it with `Executor::block_on` (D130) |
| `Published` | One asset written: its name, its `AssetId` and its `AssetVersion` |
| `pack_set(io, directory)` | The cooked pack set of a fresh process, in mount order: "The cooked pack set" below |
| `SkeletonProcessor` | The `Processor` of `Skeleton`: a `.gltf` file's bytes in, the skeleton byte form out |

## The command line

`ricotta SOURCES PACK`, both required, nothing else accepted (exit 2 and the usage
otherwise):

- **Sources.** Every regular file under the directory `SOURCES`, at any depth, whose name
  ends in `.gltf` is a source, and each cooks to one `anim::skeleton`. Other files are
  ignored. A file whose name ends in `.glb` is refused (`Unsupported`), as is an entry that
  is neither a file nor a directory, a symbolic link among them, and a name that is not
  UTF-8 (`InvalidData`). A tree with no source is refused (`NotFound`).
- **Asset names.** An asset's name is its source's path below `SOURCES`, its components
  joined by `/`, with `.gltf` replaced by `.skeleton`: `SOURCES/characters/knight.gltf` is
  `characters/knight.skeleton`, and its ID is `AssetId::from_name` of that name. Where the
  sources directory is does not enter a name. Two names with one ID, or a name whose ID is
  `AssetId::NONE`, are refused.
- **Output.** One pack at `PACK`, whose file name must end in `.pack`; its directory is
  created. The pack is written whole by `Io::replace`, so a reader sees the previous pack or
  the new one, and nothing is written unless every source cooks. Every asset is stored as
  `Storage::Zstd { level: 3 }`.
- **Report.** One line per asset on stdout, in byte order of the names: the name, the ID
  and the version, each hexadecimal as `0x` and sixteen digits. A failure is one line on
  stderr, `ricotta: ` then the error (its kind, then its message, which begins with the
  path concerned, for a source its path below `SOURCES`, and then names the node), and
  exit status 1. A panic prints a crash report and aborts
  (architecture section 5.1).

## The glTF reader

The reader reads the JSON form, `.gltf`, of glTF 2.0 (`asset.version` of major version 2),
and of it only: each node's `name`, `children`, `translation`, `rotation`, `scale`,
`matrix` and `extras`, and each skin's `joints`. Every other property is ignored, scenes
included: every node without a parent is a root of the walk. A source's rest poses are its
nodes' transforms, so the reader needs no buffer data: inverse bind matrices, meshes and
animation data are not read, and a source whose `.bin` is missing cooks (the committed
fixture's is). A binary glTF is refused by its name and, under a `.gltf` name, by its
magic.

Refused, each naming the node by index and name:

- a child index past the nodes, a node listed as a child twice, and a cycle;
- a node with both `matrix` and any of `translation`, `rotation`, `scale`;
- a rotation that is not a unit quaternion (its squared length more than
  `UNIT_TOLERANCE` from one);
- a scale other than one on any axis, uniform or not (each factor's square within
  `UNIT_TOLERANCE` of one);
- a matrix whose last row is not `(0, 0, 0, 1)`, whose columns are not unit length or not
  orthogonal (within `UNIT_TOLERANCE`), or whose determinant is negative.

The transform rules hold only for the nodes the skeleton composes: each bone and socket,
and every node between it and its parent bone or, for the root bone, the top of the tree.
A mesh node with a scale elsewhere in the tree is no refusal.

## The source schema

A node is marked by an object under the one key `lactose` of its `extras`; nothing else of
`extras` is read. The mark is exactly one of:

```json
{ "bone": { "role": "hips", "body": { ... } } }
{ "socket": {} }
```

Unknown keys are refused at every level, and so is a mark with both. A marked node needs a
`name`: a bone's key and a socket's key are `BoneKey::from_name` of it.

- **`bone`**: `role` is one of `hips`, `spine`, `head`, `hand_left`, `hand_right`,
  `foot_left`, `foot_right`; without it the bone's role is `Other`. Without `body` the bone
  is evaluated only.
- **`body`**: `body_type` (`dynamic`, the default, `kinematic` or `fixed`) picks the
  `RigidBody` constructor, and `mass`, `linear_damping`, `angular_damping`,
  `gravity_scale`, `locked_axes` (a list of `translation_x`, `translation_y`,
  `translation_z`, `rotation_x`, `rotation_y`, `rotation_z`) and `can_sleep` override its
  fields. `collider` is required; `joint` is optional.
- **`collider`**, in the bone's space: `shape` is one of `{"sphere": {"radius"}}`,
  `{"capsule": {"half_height", "radius"}}` and `{"cuboid": {"half_extents": [x, y, z]}}`;
  `membership` and `filter` (`u32`) are required, since `Collider::new`'s default, every
  group, is one the bone collider rule always refuses; `offset` (`[x, y, z]`), `rotation`
  (`[x, y, z, w]`), `friction`, `restitution`, `density`, `impact_threshold`, `sensor`,
  `events` and `material` (`u16`) override `Collider::new`'s values.
- **`joint`**, to the nearest ancestor bone with a body: `kind` (`fixed`, `spherical`,
  `revolute`, `prismatic`) is required; `anchor_parent` (in that ancestor's space),
  `anchor_bone` (in the bone's), `axis` (in the ancestor's) default to `Joint::new`'s, the
  origins and local y; `limits` is `[min, max]`, unlimited without it.
- **`socket`**: hangs off the nearest bone above it.

Numbers are read as `f64` and rounded once to the build's `Scalar`. Whatever
`Skeleton::validate` refuses (a repeated role, the bones' groups meeting, a joint with no
ancestor body, a physics value no backend accepts) is refused naming the node of the first
bone, or else socket, that makes the skeleton invalid, followed by `validate`'s message.

## Choosing the sim bones

A full rig has more nodes than a sim skeleton has bones (`MAX_SIM_BONES`, 16): the marked
nodes are the sim bones, and the rest are composed through.

- The bones are the nodes marked `bone`, in preorder of the node tree: the roots in index
  order, each node before its children, children in the order their parent lists them.
  That order is the bone index, so every parent precedes its children.
- A bone's parent is its nearest ancestor marked `bone`. The first bone is the root bone;
  a later bone with no bone above it is a second root and refused, naming both.
- A bone's rest pose is the composition, parent first, of the transforms of every node
  below its parent bone down to the bone itself: the unmarked nodes between them are
  composed in, so the rest is parent-local. The root bone's composes from the top of the
  tree, so an armature node's transform is in it.
- A socket's offset is the composition of the nodes below its bone down to the socket.
  A socket under no bone is refused, as is a second socket with one key.
- Where the document has skins, every bone must be a joint of one of them; a document
  with none (a hand-written rig) has no such rule. Skin joints left unmarked are not bones.
- More than 16 bones is refused at the 17th in that order, naming its node; no bone at
  all is refused.

Composition is in `f64`, and each rest rotation is normalised once, when it is rounded to
the build's `Scalar`.

## The engine's frame

glTF's assets face `+Z`; the engine's yaw zero faces `-Z` (architecture section 5.3). The
cooker turns a source half a turn about `+Y` in one function, `space::to_engine_frame`,
which the clip processor will share, applied to what is expressed in the asset's root
space and to nothing parent-local. Of a skeleton that is the root bone's rest pose alone:
every other rest, every socket offset, every collider's offset and rotation and every
joint's anchors and axis are in a bone's space, which turns with the root. Of a clip it will
be the root bone's samples and the root track. The turn negates and exchanges components
and rounds nothing.

## The byte form

`SkeletonProcessor` writes the skeleton byte form of `crates/anim/README.md`, "Assets",
with the cooker's own encoder, at the build's scalar width, and declares no dependency.
Before it returns, it loads its bytes through `SkeletonLoader` and checks that they decode
to the skeleton it encoded, so what it cooks is what the runtime accepts.

## The cooked pack set

**The set** of a fresh process in a cooked directory is every regular file directly in
that directory whose name ends in `.pack`, mounted in ascending byte order of the names
(`Io::read_dir`'s order, `OsStr`'s: the names' bytes on Unix, their WTF-8 bytes on
Windows, which agree for ASCII names; `pack_set` returns it). A later name takes a later place in the
store's mount order, so where two packs hold one asset, the store names the version of the
pack whose name sorts last. Subdirectories, symbolic links, other files and the
temporaries of a replace in flight (`<name>.<hex>.tmp`) are not in the set.

A team orders packs by naming them (`00-base.pack`, `10-dlc.pack`, `90-patch.pack`); a
re-cook replaces its pack at the same path, so its place does not change. Milestone 2's
directory watch keeps this order: when a pack in the set is replaced or added, it mounts
that pack and every pack whose name sorts after it, again, in name order, so the last name
still names the version (D146).

The store's contract is unchanged (D136): it mounts the list it is given, in the order of
the calls. The rule is the cooker's and the caller's, so it is stated here.

## Determinism

The same sources and the same build write the same bytes: the pack and every asset
version (`tests/skeleton.rs`).

- **Directory order.** The sources are listed through `Io::read_dir`, then sorted by their
  paths below `SOURCES` as UTF-8 bytes, and read, cooked and added in that order. No
  `HashMap` or other unordered collection is iterated: the ID claims are a `BTreeMap`, and
  `PackBuilder` keeps its index sorted by `AssetId` and its payloads in the order added.
- **Map order.** The reader deserialises into typed structs and reads a node's `extras` by
  its one key: no JSON object is iterated, so neither the order of a source's keys nor
  whether a build's serde_json keeps that order changes a byte. Lists (`children`,
  `joints`, `locked_axes`) are read in their written order, which is the source's content.
- **Time.** No clock is read, and the pack format holds no timestamp, cook sequence or
  host name.
- **Numbers.** serde_json's `float_roundtrip` feature makes every number parse to its
  correctly rounded `f64`. The transform arithmetic uses IEEE addition, subtraction,
  multiplication, division and square root only, each correctly rounded, and Rust contracts
  none of them into a fused multiply-add, so a source gives the same bits on every host.
- **Compression.** zstd at a fixed level on one thread is a function of its input for the
  `zstd` version `Cargo.lock` pins.

## Not in this crate yet

- The other kinds: clips and state machines from glTF animations and the machine's
  source, meshes, textures, materials, shaders, scenes and collision (architecture
  section 2's list), and `.glb`.
- A processor registry and a choice of processor by anything but the `.gltf` ending.
- Incremental cooks, a cache, and the directory watch of milestone 2.
- Dependencies between assets: a skeleton has none, so the cycle check of
  `CookedHeader::dependencies` is the clip processor's to add.

## Proof for milestone 1

| Property | Where |
|---|---|
| A pack `ricotta` wrote from the committed fixture mounts in a `Store`; its skeleton loads through `SkeletonLoader`, resolves and pins, and equals the skeleton the source describes, worked out by hand: nine nodes, four bones, a socket, three bodies and two joints | `tests/skeleton.rs`, `the_knight_cooks_mounts_loads_pins_and_is_the_skeleton_its_nodes_describe` |
| The half turn: the root at `+Z` in the source is at `-Z`, and a bone at `-X` is at `+X` in the root's space | the same test |
| Cooking twice writes the same pack and prints the same versions; three sources written in opposite orders, one directory and one file sorting differently by path and by name, cook to the same bytes | `tests/skeleton.rs`, `cooking_twice_writes_the_same_bytes`, `sources_written_in_another_order_cook_to_the_same_bytes` |
| The encoder writes anim's golden skeleton bytes, copied from `crates/anim/tests/format.rs`; swapping its linear and angular damping fails the test | `src/codec.rs`, `the_encoder_writes_anims_golden_skeleton_bytes` |
| Every refusal, each naming the file and the node: a `.glb` by name and by magic, a non-uniform scale, a uniform scale other than one above a bone, a scaling or mirroring matrix, a cycle, a second root, a 17th bone (16 cook), and the reader's, the schema's and `validate`'s other rules; nothing is written | `tests/refusals.rs` |
| A fresh process mounts the cooked set in name order and names the last name's versions, though the later name was cooked first and the mounts end in the other order | `tests/pack_set.rs` |
