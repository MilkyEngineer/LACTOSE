# lactose-cook

The asset cooker, binary `ricotta` (architecture sections 2 and 14): a directory of
authored sources cooked into one pack of SILO's format. It cooks two kinds:
`lactose-anim`'s sim `Skeleton`, from glTF sources read by the cooker's own reader over
serde_json, through a `Processor` of `asset-api`; and from milestone 2 `rhi-api`'s
`CookedShader`, one for each entry point of a Slang source and each target, compiled by the
pinned Slang release and on macOS by Apple's `metal`, and checked on what they emitted
("The shader kind"). Both go into one pack through `lactose-asset`'s `PackBuilder`.

| | |
|---|---|
| Tier | leaf |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` in the library and the binary; architecture section 21 lists `tools/cook` for the FFI to Slang and compression libraries, and Slang runs as a subprocess instead: `SAFETY.md` |

**Status: two kinds, one pack per run.** The skeleton of milestone 1, and the shader kind
of milestone 2's `slang` row. This file is the contract of the command line, the source
schema and the shader checks; the cooked pack set, the rule by which a process mounts what
the cooker writes, is `lactose-asset`'s ("The cooked pack set" below).

## Surface

| Item | What it is |
|---|---|
| `ricotta [--slangc SLANGC] [--code DIR] SOURCES PACK` | The binary: "The command line" below |
| `cook(io, sources, pack, shaders)` | The cook as a future over the pool, returning a `Published` per asset written; the binary drives it with `Executor::block_on` (D130) |
| `Published` | One asset written: its name, its `AssetId` and its `AssetVersion` |
| `SkeletonProcessor` | The `Processor` of `Skeleton`: a `.gltf` file's bytes in, the skeleton byte form out |
| `Slang` | The pinned `slangc`, checked by the version it reports, and on macOS `metal` found |
| `Shaders` | What a cook of shader sources is given: the `Slang`, and an optional directory that also receives each shader's code as a file |
| `SHADER_KIND` | The shader's `AssetKind`, `AssetKind::from_name("rhi::shader")`; its format version is `COOKED_SHADER_VERSION` |
| `slang.toml` | The pin: the Slang version and each host's archive and SHA-256, which `cargo xtask slang` installs from (`tools/xtask/README.md`, "slang") |

## The command line

`ricotta [--slangc SLANGC] [--code DIR] SOURCES PACK`, the two paths required and each
option at most once, before them; nothing else is accepted (exit 2 and the usage otherwise).
`--slangc` names the pinned `slangc`, which a tree with a shader source needs
(`NotFound` without it), and `--code` a directory that also receives each cooked shader's
code ("The shader kind"); `--code` without `--slangc` is refused (`InvalidData`).

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
  system. `.slang` is held to the same rules. A tree with no source is refused
  (`NotFound`).
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

## The shader kind

Each Slang source cooks to one `CookedShader` for each of its entry points and each target:
SPIR-V and MSL on every host, and a metallib on macOS (decision 5.8 of the milestone 2
plan). The format is `rhi-api`'s ("Pipelines and the cooked shader format"); its bytes are
`CookedShader::encode`'s, under `SHADER_KIND` and `COOKED_SHADER_VERSION`, with no
dependency.

- **The layout of the sources.** A `.slang` file below `SOURCES` is a shader source, except
  below `modules/`, which is the import root (`-I modules`) and whose files are cooked only
  as a source imports them: the engine's heap is `modules/lactose/heap.slang`, imported as
  `lactose.heap`. `<stem>.link.slang` beside `<stem>.slang` is that shader's link module: it
  is compiled to a Slang module and linked into every entry point, which is how a link-time
  constant (`extern static const`) gets its value. A link module with no shader beside it
  is refused (`InvalidData`).
- **Asset names.** `<stem>.<entry>.<target>`, the target `spirv`, `msl` or `metallib`:
  `SOURCES/surface.slang`'s fragment entry `fs_main` is `surface.fs_main.spirv` and so on.
  The entries are the module's, in byte order of their names. A source with none is refused.
- **The toolchain.** `slangc` is the one `--slangc` names, never one on the path, and it
  must report the version `slang.toml` pins (`VersionMismatch` otherwise, which is what the
  SDK's copy gives). Every call passes `-warnings-as-errors all`, so a warning fails the
  cook, and `-line-directive-mode none`, so no path reaches the output. SPIR-V is compiled
  with `-fvk-use-entrypoint-name`. On macOS the MSL is compiled by
  `xcrun -sdk macosx metal -std=metal3.1 -mmacosx-version-min=14.0 -Werror`, MSL 3.1 for
  macOS 14, the oldest client.
- **Where the toolchain writes.** It runs as subprocesses on the thread that polls the
  cook, in the sources directory, and reads the sources itself, outside the pool. A link
  module's compiled form and each metallib's MSL are written to a scratch directory of the
  cook's under the system's temporary directory, removed when the source is cooked: the MSL
  under the stable name `<file>.<entry>.metal`, since a metallib records its source's file
  name.
- **What a shader is cooked from.** The SPIR-V is read back for each entry
  (`src/spirv.rs`): the stage, the heap's arrays and capacities, the specialisation
  constants and their lengths, the push-constant bytes, the input attachments and the
  fragment outputs. The same fields are written for each target of the entry. The MSL is
  read for its colours and function constants, and the two must agree.
- **`--code DIR`.** Each shader's code alone, the SPIR-V module, the MSL text or the
  metallib, is also written to `DIR/<asset name>` once every source has cooked, before the
  pack, each file replaced by rename. It is what `cargo xtask gpu` and the `shaders` CI job
  load through Metal.

## The checks

What the format cannot hold is checked on what Slang emitted, not on the source (decision
5.8; `rhi-api`'s "Pipelines and the cooked shader format"). Each refusal is `InvalidData`
and names the source, the entry and, where one alone is read, the target; nothing is written.

- **Capabilities.** The SPIR-V declares only `rhi-api`'s list: `Shader`,
  `PhysicalStorageBufferAddresses`, `Int64`, `ShaderNonUniform`,
  `SampledImageArrayNonUniformIndexing`, `InputAttachment` and
  `StorageImageReadWithoutFormat`, and no extension but `SPV_KHR_physical_storage_buffer`.
  The refusal of `DrawParameters` says it comes from `SV_VertexID` or `SV_InstanceID`, and
  that `SV_VulkanVertexID` and `SV_VulkanInstanceID` are the engine's; that of
  `RuntimeDescriptorArray`, that a runtime descriptor array declares it, such as the one
  behind `DescriptorHandle`.
- **One entry point** in each module the cook reads back.
- **The heap.** Set `HEAP_SET` holds an array of 2D sampled images at binding 0 and an array
  of samplers at binding 1, each of a constant length, which is its capacity in `HeapUse`
  (`HeapForm::FixedArray`); nothing else is in that set, and an unsized array is refused.
  Set 0 holds the fragment stage's input attachments alone, input `i` at binding `i`. Any
  other resource, a uniform or storage buffer among them, is refused: a buffer is reached
  through its device address in the push block.
- **Non-uniform access.** Every access chain into the heap, and every load through one,
  carries the `NonUniform` decoration. So does an access whose index is uniform: the cook
  cannot tell, and the heap's index is data. An array of the heap loaded whole, stored,
  copied, or passed through a call, a copy, a conversion, a select or a phi is refused,
  since the check could not follow it. The marker is `nonuniform_slot` of
  `modules/lactose/heap.slang`, which is `NonUniformResourceIndex` on SPIR-V and the index
  itself elsewhere, and is `[ForceInline]`: a helper that is not inlined takes the index
  as a parameter and the decoration is lost.
- **Fragment outputs.** Each output has exactly one location, is neither an array nor a
  struct, and no two share one; the MSL's `[[color(i)]]` must be the same set. Slang gives an
  output without `[[vk::location(i)]]` location 0 on SPIR-V whatever its `SV_Target<i>`, so a
  lone `SV_Target1` is refused by the comparison, and the message says to write both.
- **Inputs and constants.** The input attachments the SPIR-V declares are the MSL's
  `[[color(i)]]` parameters, and its `SpecId`s, each of one byte for a `bool` or four for a
  32-bit scalar, are the MSL's `[[function_constant(i)]]`s.
- **Push bytes.** At most one push-constant block, whose size, a device address counting
  eight bytes, is at most `PUSH_BYTES_MAX`.

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
version (`tests/skeleton.rs`). For shaders, the same sources, the same pinned Slang and the
same host write the same SPIR-V and MSL from checkouts at different paths, and no path of
the machine's reaches a cooked file (`tests/shader.rs`). Whether two cooks write the same
metallib is recorded by that test and nothing rests on it: the `shaders` CI job compares
the SPIR-V and MSL of two hosts, and loads the macOS host's metallibs.

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
  source, meshes, textures, materials, scenes and collision (architecture section 2's
  list), and `.glb`.
- For shaders: a compute shader, which the reader takes and `xtask gpu` makes a pipeline
  of, but which no engine shader is and no test cooks; the Metal buffer indices of the
  push block and the heap's argument buffer, which the format does not carry (Slang gives
  them in declaration order, the push block at `[[buffer(0)]]` and the heap at
  `[[buffer(1)]]` in the engine shader); and the constants' types, which the format
  carries as lengths.
- A processor registry and a choice of processor by anything but the `.gltf` ending.
- Incremental cooks, a cache, and the directory watch of milestone 3 (D153).
- Dependencies between assets: a skeleton has none, so the cycle check of
  `CookedHeader::dependencies` is the clip processor's to add.

## Proof for milestone 2

The tests that run Slang are ignored, and run once `cargo xtask slang` has installed it:
`cargo nextest run -p lactose-cook --run-ignored all`, in the `shaders` CI job and by hand.

| Property | Where |
|---|---|
| Each refusal decision 5.8 names, on the engine shader mutated: the marker removed; the marker in a helper that is not inlined; `NonUniformResourceIndex` on a `DescriptorHandle`, which Slang only warns of, and the handle with Slang's own marker; a lone `SV_Target1`; `SV_VertexID`; a warning | `tests/shader.rs`, `the_marker_removed_is_refused`, `the_marker_in_a_helper_not_inlined_is_refused`, `non_uniform_resource_index_on_a_descriptor_handle_is_refused_and_so_is_the_handle`, `a_lone_sv_target1_is_refused`, `sv_vertex_id_is_refused`, `a_warning_is_an_error` |
| A `slangc` of another version, the SDK's among them, and a missing one, are refused; a shader source with no `slangc`, a link module alone and `--code` without `--slangc` are refused | `tests/shader.rs`, `a_slangc_of_another_version_is_refused`, `a_shader_without_slangc_and_a_lone_link_module_are_refused`, `the_binary_refuses_code_without_slangc` |
| Two checkouts at different paths cook the same SPIR-V and MSL, no cooked file names a path of either checkout or of the temporary directory, and the metallibs' equality is printed | `tests/shader.rs`, `two_checkouts_at_different_paths_cook_the_same_spirv_and_msl_and_name_no_path` |
| The link-time constant folds into both targets: `discard_fragment` and `OpKill` with it true, neither with it false, and no specialisation constant for it | `tests/shader.rs`, `the_link_time_constant_folds_into_both_targets` |
| The engine shader cooks to what its code declares, loaded back from the pack through a store: the heap's two arrays and capacities, the constant, the push bytes, the input attachment and each output | `tests/shader.rs`, `the_engine_shader_cooks_to_what_its_code_declares_and_loads_from_the_pack` |
| The shader kind's load against section 5.1's slice (row 136): the store's synchronous call, the version hash and the decode, the median of 101 calls per shader | `tests/shader.rs`, `the_shader_kind_loads_against_the_slice`, with `--release --no-capture` |
| The SPIR-V reader refuses what is not a module, a capability or an extension off the list, and a module of two entry points | `src/spirv.rs`'s tests |
| Every cooked MSL and metallib makes a pipeline state with its function constants under Metal API and shader validation, each shown listening by its canary | `cargo xtask gpu --driver system` (`tools/xtask/README.md`, "gpu") |

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
