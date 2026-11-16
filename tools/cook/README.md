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

**Status: the milestone 1 skeleton.** One kind, one pack per run. This file is the
contract of the command line and the source schema; the cooked pack set, the rule by which
a process mounts what the cooker writes, is `lactose-asset`'s ("The cooked pack set"
below).

## Surface

| Item | What it is |
|---|---|
| `ricotta SOURCES PACK` | The binary: "The command line" below |
| `cook(io, sources, pack)` | The cook as a future over the pool, returning a `Published` per asset written; the binary drives it with `Executor::block_on` (D130) |
| `Published` | One asset written: its name, its `AssetId` and its `AssetVersion` |
| `SkeletonProcessor` | The `Processor` of `Skeleton`: a `.gltf` file's bytes in, the skeleton byte form out |

## The command line

`ricotta SOURCES PACK`, both required, nothing else accepted (exit 2 and the usage
otherwise):

- **Sources.** Every regular file under the directory `SOURCES`, at any depth, whose name
  ends in `.gltf` is a source, hidden files included (a `._knight.gltf` a copy left behind
  is one, and fails the cook if it is not a glTF document), and each cooks to one
  `anim::skeleton`. Other files are ignored. A name that is only an ending, `.gltf` or
  `.glb` in any case, is refused (`InvalidData`): its asset's file name would be empty. A
  file whose name ends in `.glb` is refused (`Unsupported`), as is an entry that is
  neither a file nor a directory, a symbolic link among them, and a name that is not UTF-8
  (`InvalidData`). So is a name ending in `.gltf` or `.glb` in any other case, `.GLTF` or
  `.Glb` (`InvalidData`, naming the file renamed): the endings are read in lowercase
  alone, so that a source is a source, and its asset's name the same, on every file
  system. A tree with no source is refused (`NotFound`).
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

Every object the reader reads is read only when written as a JSON object: the document,
`asset`, each node and each skin, and every object of the mark ("The source schema"). An
object written as an array, its fields by position, which serde would otherwise accept, is
refused. Every other value keeps its JSON type: a list is an array, a name a string.

Refused, each naming the node by index and name:

- a child index past the nodes, a node listed as a child twice, and a node whose parents
  form a cycle, or lead into one (the first such node by index);
- a node with both `matrix` and any of `translation`, `rotation`, `scale`;
- a rotation that is not a unit quaternion (its squared length more than
  `UNIT_TOLERANCE` from one);
- a scale other than one on any axis, uniform or not (each factor positive and its square
  within `UNIT_TOLERANCE` of one);
- a matrix whose last row is not `(0, 0, 0, 1)`, whose columns are not unit length or not
  orthogonal (within `UNIT_TOLERANCE`), or whose determinant is negative.

The transform rules hold only for the nodes the skeleton composes: each bone and socket,
and every node between it and its parent bone or, for the root bone, the top of the tree.
A mesh node with a scale elsewhere in the tree is no refusal. A scale is refused rather
than baked, uniform or not: `BonePose` holds none, and while baking a uniform scale into
the rests is exact, a body's values and a clip's samples are authored in the same scaled
unit and would need the same factor. The refusal says so: apply the scale to the
armature before export.

The reader's cost is linear in the nodes: one walk of the tree finds a cycle, each node's
pose below its bone is composed once from its parent's, and the socket keys are a set.

## The source schema

A node is marked by an object under the one key `lactose` of its `extras`; nothing else of
`extras` is read. The mark is exactly one of:

```json
{ "bone": { "role": "hips", "body": { ... } } }
{ "socket": {} }
```

Objects are the only supported form: every level of the mark is a JSON object, and one
written as an array is refused. Unknown keys are refused at every level, and so is a mark
with both. A key written twice in one object, `lactose` itself or any key below it, is
refused, naming its path; a repeated key elsewhere in `extras` is not the cooker's and is
not read. A marked node needs a `name`: a bone's key and a socket's key are
`BoneKey::from_name` of it.

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

Which packs a fresh process mounts, and in which order, is `lactose-asset`'s rule,
`lactose_asset::pack_set` (`crates/asset/README.md`, "Mounting"; D147): the regular files
directly in a cooked directory whose names end in `.pack`, in byte order of the names, a
later name naming the version where two packs hold one asset. So a team orders the packs
it cooks by naming them (`00-base.pack`, `10-dlc.pack`, `90-patch.pack`), and a re-cook
that writes its pack at the same path keeps its name's rank in the order. Milestone 3's
directory watch must leave the store naming what a fresh process mounting the set would
name, with no version the set shadows named in between; how is milestone 3's to decide (D153).

## Determinism

The same sources and the same build write the same bytes: the pack and every asset
version (`tests/skeleton.rs`).

- **Directory order.** The sources are listed through `Io::read_dir`, then sorted by their
  paths below `SOURCES` as UTF-8 bytes, and read, cooked and added in that order. No
  `HashMap` or other unordered collection is iterated: the ID claims are a `BTreeMap`, and
  `PackBuilder` keeps its index sorted by `AssetId` and its payloads in the order added.
- **Map order.** The reader deserialises into typed structs and reads a node's `extras` by
  its one key, and refuses a key written twice: no output but an error's wording (which
  unknown or repeated key is named first) depends on a JSON object's key order, so neither
  the order of a source's keys nor whether a build's serde_json keeps that order changes a
  byte. Lists (`children`, `joints`, `locked_axes`) are read in their written order, which
  is the source's content.
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
- Incremental cooks, a cache, and the directory watch of milestone 3 (D153).
- Dependencies between assets: a skeleton has none, so the cycle check of
  `CookedHeader::dependencies` is the clip processor's to add.

## Proof for milestone 1

| Property | Where |
|---|---|
| A pack `ricotta` wrote from the committed fixture mounts in a `Store`; its skeleton loads through `SkeletonLoader`, resolves and pins, and equals the skeleton the source describes, worked out by hand: nine nodes, four bones, a socket, three bodies and two joints | `tests/skeleton.rs`, `the_knight_cooks_mounts_loads_pins_and_is_the_skeleton_its_nodes_describe` |
| The half turn: the root at `+Z` in the source is at `-Z`, and a bone at `-X` is at `+X` in the root's space | the same test |
| Cooking twice writes the same pack and prints the same versions; three sources written in opposite orders, one directory and one file sorting differently by path and by name, cook to the same bytes | `tests/skeleton.rs`, `cooking_twice_writes_the_same_bytes`, `sources_written_in_another_order_cook_to_the_same_bytes` |
| The encoder writes anim's golden skeleton bytes, copied from `crates/anim/tests/format.rs`; swapping its linear and angular damping fails the test | `src/codec.rs`, `the_encoder_writes_anims_golden_skeleton_bytes` |
| Every role, shape, body type and joint kind reads back through anim's decoder as itself; exchanging two role codes, or two joint-kind codes, in the encoder fails the test | `src/codec.rs`, `every_variant_of_every_enum_reads_back_as_itself` |
| Every role, body type, locked axis and joint kind written as its name in a source is read as its value; exchanging two foot roles or two translation axes in the reader fails the test | `src/skeleton.rs`, `every_name_in_a_source_is_read_as_its_value` |
| Every refusal, each naming the file and the node: a `.glb` by name and by magic, an ending in another case with its rename, a name that is only an ending, a non-uniform scale, a uniform scale other than one above a bone with its remedy, a negative factor, a scaling or mirroring matrix, a cycle, a second root, a 17th bone (16 cook), an object written as an array at every level of the mark and of the document, a key written twice in the mark, and the reader's, the schema's and `validate`'s other rules; nothing is written | `tests/refusals.rs` |
