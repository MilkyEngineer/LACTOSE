//! The shader kind (`README.md`, "The shader kind"). The tests that run Slang are ignored, so
//! a job without the pinned release does not run them: they run once `cargo xtask slang` has
//! installed it, in the `shaders` CI job and by hand, with `--run-ignored all`. Each mutation
//! is applied to the engine's own shaders, and asserted to have changed them, so a test cannot
//! pass on a mutation that no longer applies.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::common::Env;
use lactose_asset::Store;
use lactose_asset_api::{
    Asset, AssetCatalog, AssetId, AssetKind, AssetRef, ErasedLoader, LoadPriority, Loader,
};
use lactose_cook::{SHADER_KIND, Shaders, Slang, cook};
use lactose_core::{ErrorKind, Result};
use lactose_rhi_api::{
    COOKED_SHADER_VERSION, ConstantSlot, CookedShader, HEAP_SET, HEAP_SLOTS_DEFAULT, HeapArray,
    HeapArrayKind, HeapForm, HeapUse, SAMPLERS_DEFAULT, ShaderStage, ShaderTarget,
};

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The pinned `slangc`, as `cargo xtask slang` installs it under the target directory these
/// tests are built in, which is `CARGO_TARGET_TMPDIR`'s parent.
fn slangc() -> PathBuf {
    #[derive(serde::Deserialize)]
    struct Pin {
        version: String,
    }
    let text = std::fs::read_to_string(root().join("tools/cook/slang.toml")).unwrap();
    let pin: Pin = toml::from_str(&text).unwrap();
    let file = if cfg!(windows) { "slangc.exe" } else { "slangc" };
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).parent().unwrap();
    let slangc = target.join("slang").join(&pin.version).join("bin").join(file);
    assert!(slangc.is_file(), "{} is missing: run `cargo xtask slang`", slangc.display());
    slangc
}

fn slang() -> Slang {
    Slang::new(&slangc()).expect("the pinned slangc")
}

/// The engine's shaders copied to `<dir>/<name>`, with `from` replaced by `to` once in
/// `file`, a path below `shaders/`; `from` must be there.
fn mutated(env: &Env, name: &str, file: &str, from: &str, to: &str) -> PathBuf {
    let sources = env.dir.join(name);
    copy(&root().join("shaders"), &sources);
    edit(&sources, file, from, to);
    sources
}

/// `from` replaced by `to` once in `file`, a path below `sources`; `from` must be there.
fn edit(sources: &Path, file: &str, from: &str, to: &str) {
    let path = sources.join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "the mutation `{from}` no longer applies to {file}");
    std::fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

/// The engine's modules alone copied to `<dir>/<name>/modules`, each of `edits` (a file below
/// `shaders/modules/`, `from` and `to`) applied to them, and beside them each of `shaders`, a
/// file name and its source.
fn probe(env: &Env, name: &str, edits: &[(&str, &str, &str)], shaders: &[(&str, &str)]) -> PathBuf {
    let sources = env.dir.join(name);
    copy(&root().join("shaders").join("modules"), &sources.join("modules"));
    for (file, from, to) in edits {
        edit(&sources.join("modules"), file, from, to);
    }
    for (file, source) in shaders {
        std::fs::write(sources.join(file), source).unwrap();
    }
    sources
}

/// Asserts that `probe.slang`, of `source` and beside the modules `edits` changed, is refused
/// as invalid with a message that contains `says`, and returns the message.
fn assert_probe_refused(
    name: &str,
    edits: &[(&str, &str, &str)],
    source: &str,
    says: &str,
) -> String {
    let env = Env::new(name);
    let sources = probe(&env, "sources", edits, &[("probe.slang", source)]);
    let error = cook_shaders(&env, &sources, None).expect_err("the probe is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(error.message().contains(says), "{error}");
    error.message().to_owned()
}

/// Cooks `probe.slang`, of `source` and beside the modules `edits` changed, and returns the
/// MSL of its entry `entry`.
fn assert_probe_cooks(
    name: &str,
    edits: &[(&str, &str, &str)],
    source: &str,
    entry: &str,
) -> String {
    let env = Env::new(name);
    let sources = probe(&env, "sources", edits, &[("probe.slang", source)]);
    let code = env.dir.join("code");
    cook_shaders(&env, &sources, Some(&code)).unwrap_or_else(|error| panic!("{name}: {error}"));
    std::fs::read_to_string(code.join(format!("probe.{entry}.msl"))).unwrap()
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

/// Cooks `sources` with the pinned Slang, writing each shader's code below `code`.
fn cook_shaders(env: &Env, sources: &Path, code: Option<&Path>) -> Result<Vec<String>> {
    let slang = slang();
    let shaders = Shaders { slang: &slang, code };
    let pack = env.dir.join(format!("{}.pack", sources.file_name().unwrap().to_string_lossy()));
    let published = env.wait(cook(&env.io, sources, &pack, Some(shaders)))?;
    Ok(published.into_iter().map(|asset| asset.name).collect())
}

/// Asserts that the mutation is refused as invalid with a message that contains `says`.
fn assert_refused(name: &str, file: &str, from: &str, to: &str, says: &str) {
    let env = Env::new(name);
    let sources = mutated(&env, "sources", file, from, to);
    let error = cook_shaders(&env, &sources, None).expect_err("the mutation is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(error.message().contains(says), "{error}");
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn the_marker_removed_is_refused() {
    assert_refused(
        "marker-removed",
        "surface.slang",
        "heap.images[nonuniform_slot(slot)]",
        "heap.images[slot]",
        "surface.slang: fs_main (SPIR-V): an access through the heap, %",
    );
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn the_marker_in_a_helper_not_inlined_is_refused() {
    assert_refused(
        "marker-not-inlined",
        "modules/lactose/heap.slang",
        "[ForceInline]",
        "[noinline]",
        "carries no non-uniform decoration",
    );
}

/// The heap's other form: an image reached through a `DescriptorHandle` in the push block.
fn handle(call: &str) -> [(&'static str, &'static str, String); 2] {
    [
        (
            "surface.slang",
            "    uint slot;\n    uint sampler;\n};",
            "    uint slot;\n    uint sampler;\n    DescriptorHandle<Texture2D<float4>> image;\n};"
                .to_owned(),
        ),
        ("surface.slang", "heap.images[nonuniform_slot(slot)].Sample(", format!("{call}.Sample(")),
    ]
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn non_uniform_resource_index_on_a_descriptor_handle_is_refused_and_so_is_the_handle() {
    // Slang only warns of `NonUniformResourceIndex` on a handle, and decorates nothing: the
    // warning is an error.
    let env = Env::new("handle");
    for (name, call, says) in [
        ("nuri", "NonUniformResourceIndex(push.image)", "error[E31200]"),
        // With Slang's own marker the handle still needs a runtime descriptor array.
        ("marked", "nonuniform(push.image)", "capability RuntimeDescriptorArray"),
    ] {
        let [first, second] = handle(call);
        let sources = mutated(&env, name, first.0, first.1, &first.2);
        let text = std::fs::read_to_string(sources.join(second.0)).unwrap();
        assert!(text.contains(second.1));
        std::fs::write(sources.join(second.0), text.replacen(second.1, &second.2, 1)).unwrap();
        let error = cook_shaders(&env, &sources, None).expect_err("the handle is refused");
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        assert!(error.message().contains(says), "{name}: {error}");
    }
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_lone_sv_target1_is_refused() {
    assert_refused(
        "lone-target1",
        "surface.slang",
        "[[vk::location(1)]] ",
        "",
        "surface.slang: fs_resolve: the fragment output `colour` is `SV_Target1` at location 0",
    );
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn outputs_off_their_target_index_are_refused() {
    // Each is at the same set of locations in the SPIR-V as of colours in the MSL, and at
    // another location than its colour, for each output or for one.
    for (name, source, says) in [
        (
            "reversed-order",
            "struct Two { float4 a : SV_Target1; float4 b : SV_Target0; };\n\
             [shader(\"fragment\")] Two fs_main() { Two r; r.a = 1; r.b = 0; return r; }\n",
            "probe.slang: fs_main: the fragment output `a` is `SV_Target1` at location 0",
        ),
        (
            "swapped-locations",
            "struct Two {\n\
             [[vk::location(1)]] float4 a : SV_Target0;\n\
             [[vk::location(0)]] float4 b : SV_Target1;\n\
             };\n\
             [shader(\"fragment\")] Two fs_main() { Two r; r.a = 1; r.b = 0; return r; }\n",
            "probe.slang: fs_main: the fragment output `a` is `SV_Target0` at location 1",
        ),
        (
            "value-target1",
            "[shader(\"fragment\")] float4 fs_main() : SV_Target1 { return 1; }\n",
            "probe.slang: fs_main: the returned fragment output is `SV_Target1` at location 0, \
             where each is at its semantic's index: declare it with `[[vk::location(1)]]`, since \
             Slang places a returned value after any `out` outputs",
        ),
    ] {
        assert_probe_refused(name, &[], source, says);
    }
    // An `out` output, which Slang places before the returned value: its hint is the location
    // alone.
    let source = "[shader(\"fragment\")] float4 fs_main(float4 p : SV_Position, \
                  out float4 b : SV_Target1) : SV_Target0 { b = 0; return 1; }\n";
    let says = "probe.slang: fs_main: the fragment output `b` is `SV_Target1` at location 0, \
                where each is at its semantic's index: declare it with `[[vk::location(1)]]`";
    let message = assert_probe_refused("return-and-out", &[], source, says);
    assert!(message.ends_with("`[[vk::location(1)]]`"), "{message}");
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn varyings_off_their_texcoord_index_are_refused() {
    // Metal links the two stages by semantic and Vulkan by location: each pair would be given
    // other values on each target.
    for (name, source, says) in [
        (
            "varyings-mismatch",
            "struct V { float4 p : SV_Position; float2 uv : TEXCOORD0; };\n\
             [shader(\"vertex\")] V vs_p() { V v; v.p = 0; v.uv = 0; return v; }\n\
             struct F { float4 p : SV_Position; float4 extra : TEXCOORD1; };\n\
             [shader(\"fragment\")] float4 fs_p(F f) : SV_Target0 { return f.extra; }\n",
            "probe.slang: fs_p: the varying `extra` is `TEXCOORD1` at location 0",
        ),
        (
            "varyings-subset",
            "struct V { float4 p : SV_Position; float4 a : TEXCOORD0; float4 b : TEXCOORD1; };\n\
             [shader(\"vertex\")] V vs_p() { V v; v.p = 0; v.a = 0; v.b = 1; return v; }\n\
             struct F { float4 p : SV_Position; float4 b : TEXCOORD1; };\n\
             [shader(\"fragment\")] float4 fs_p(F f) : SV_Target0 { return f.b; }\n",
            "probe.slang: fs_p: the varying `b` is `TEXCOORD1` at location 0",
        ),
        (
            "varying-colour",
            "struct V { float4 p : SV_Position; float4 c : COLOR0; };\n\
             [shader(\"vertex\")] V vs_p() { V v; v.p = 0; v.c = 1; return v; }\n",
            "probe.slang: vs_p: the varying `c` is `COLOR`, where each varying is `TEXCOORD<i>`",
        ),
    ] {
        assert_probe_refused(name, &[], source, says);
    }
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_vertex_input_that_is_not_a_built_in_is_refused() {
    let vertex = "struct V { float4 p : SV_Position; };\n\
                  [shader(\"vertex\")] V vs_p(float3 pos : POSITION) \
                  { V v; v.p = float4(pos, 1); return v; }\n";
    let says = "probe.slang: vs_p (SPIR-V): a vertex entry's input at location 0, %";
    assert_probe_refused("vertex-input", &[], vertex, says);
    // Beside a fragment entry, which the vertex entry's input is not a varying of.
    let pair = format!(
        "{vertex}[shader(\"fragment\")] float4 fs_p(float4 p : SV_Position) : SV_Target0 \
         {{ return 1; }}\n"
    );
    assert_probe_refused("vertex-input-pair", &[], &pair, says);
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_push_block_float3_is_refused() {
    // The SPIR-V puts `b` at offset 12 and the MSL at 16.
    let source = "struct Push { float3 a; float b; uint64_t address; uint c; };\n\
                  [[vk::push_constant]] ConstantBuffer<Push> push;\n\
                  [shader(\"fragment\")] float4 fs_p() : SV_Target0 \
                  { return float4(push.a, push.b) + float(push.c) + float(push.address); }\n";
    let says = "probe.slang: fs_p (SPIR-V): the push-constant block holds a 3-component vector";
    assert_probe_refused("push-float3", &[], source, says);
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_push_block_matrix_of_3_rows_or_columns_cooks() {
    // Its stride is 16 on both targets, so the member after it is at one offset on each.
    for matrix in ["float3x4", "float4x3", "float3x3"] {
        for layout in ["row_major", "column_major"] {
            let source = format!(
                "struct Push {{ {layout} {matrix} m; float4 after; }};\n\
                 [[vk::push_constant]] ConstantBuffer<Push> push;\n\
                 [shader(\"fragment\")] float4 fs_p() : SV_Target0 \
                 {{ return float4(push.m[0][0] + push.after.x); }}\n"
            );
            let name = format!("push-{layout}-{matrix}");
            assert_probe_cooks(&name, &[], &source, "fs_p");
        }
    }
}

const HEAP: &str = "lactose/heap.slang";

/// A fragment entry that reads the heap's image at `push.slot` with `read`.
fn heap_reader(read: &str) -> String {
    format!(
        "import lactose.heap;\n\
         struct Push {{ uint slot; uint sampler; }};\n\
         [[vk::push_constant]] ConstantBuffer<Push> push;\n\
         [shader(\"fragment\")] float4 fs_main(float4 position : SV_Position, \
         float2 uv : TEXCOORD0) : SV_Target0\n\
         {{\n    let image = heap.images[nonuniform_slot(push.slot)];\n    return {read};\n}}\n"
    )
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_heap_image_of_another_form_is_refused() {
    let sampler = "heap.samplers[nonuniform_slot(push.sampler)]";
    // Each is refused by the SPIR-V's reader, so on every host, Metal's or not.
    for (name, form, read, says) in [
        (
            "array-image",
            "Texture2DArray<float4> images",
            format!("image.Sample({sampler}, float3(uv, 0))"),
            "is arrayed",
        ),
        (
            "uint-image",
            "Texture2D<uint4> images",
            "float4(image.Load(int3(int2(position.xy), 0)))".to_owned(),
            "has texels that are not 32-bit floats",
        ),
        (
            "multisampled-image",
            "Texture2DMS<float4> images",
            "image.Load(int2(position.xy), 0)".to_owned(),
            "is multisampled",
        ),
    ] {
        let says = format!(
            "probe.slang: fs_main (SPIR-V): set 1, binding 0 is the heap's array of 2D images of \
             32-bit floats, each read through a sampler, and the module's image there {says}"
        );
        let edit = (HEAP, "Texture2D<float4> images", form);
        assert_probe_refused(name, &[edit], &heap_reader(&read), &says);
    }
}

/// The heap as `lactose.heap` declares it, which the tests below move.
const HEAP_BLOCK: &str = "public ParameterBlock<Heap> heap : register(b1);";

/// A fragment entry with no push block that samples the heap at a varying's slot.
const HEAP_WITHOUT_PUSH: &str = "import lactose.heap;\n\
     [shader(\"fragment\")] float4 fs_p(float2 uv : TEXCOORD0, \
     nointerpolation uint slot : TEXCOORD1) : SV_Target0\n\
     { return heap.images[nonuniform_slot(slot)].Sample(heap.samplers[nonuniform_slot(slot)], \
     uv); }\n";

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn the_heap_off_its_metal_buffer_or_bound_alone_is_refused() {
    // Without the register, a shader with no push block takes the heap at index 0.
    let edit = (HEAP, HEAP_BLOCK, "public ParameterBlock<Heap> heap;");
    let says = "probe.slang: fs_p (MSL): the entry takes the heap's argument buffer at \
                [[buffer(0)]], where it is at [[buffer(1)]] once";
    assert_probe_refused("heap-at-buffer0", &[edit], HEAP_WITHOUT_PUSH, says);
    // As globals, which Metal binds a texture and a sampler at a time.
    let globals = "public Texture2D<float4> images[16];\n\
                   [[vk::binding(1, 1)]] public SamplerState samplers[4];";
    let source = HEAP_WITHOUT_PUSH.replace("heap.", "");
    let says =
        "probe.slang: fs_p (MSL): the entry takes a texture or a sampler bound alone, `array<";
    assert_probe_refused("heap-globals", &[(HEAP, HEAP_BLOCK, globals)], &source, says);
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn the_heap_is_at_buffer_1_with_or_without_a_push_block_and_a_pointee_float3_is_read() {
    let env = Env::new("heap-at-buffer1");
    let with_push = "import lactose.heap;\n\
                     struct Push { uint slot; uint sampler; };\n\
                     [[vk::push_constant]] ConstantBuffer<Push> push;\n";
    let with_push =
        format!("{with_push}{}", HEAP_WITHOUT_PUSH.replace("import lactose.heap;\n", ""));
    let pointee = "struct Vertex { float3 p; float w; };\n\
                   struct Push { Vertex* vertices; };\n\
                   [[vk::push_constant]] ConstantBuffer<Push> push;\n\
                   [shader(\"vertex\")] float4 vs_p(uint i : SV_VulkanVertexID) : SV_Position \
                   { Vertex v = push.vertices[i]; return float4(v.p, v.w); }\n";
    let shaders = [
        ("without.slang", HEAP_WITHOUT_PUSH),
        ("with.slang", &with_push),
        ("pointee.slang", pointee),
    ];
    let sources = probe(&env, "sources", &[], &shaders);
    let code = env.dir.join("code");
    cook_shaders(&env, &sources, Some(&code)).unwrap();
    for shader in ["without", "with"] {
        let msl = std::fs::read_to_string(code.join(format!("{shader}.fs_p.msl"))).unwrap();
        // The heap's parameter, up to its attribute's end.
        let at = msl.find("Heap_default_0 constant* ").expect("the heap's parameter");
        let parameter = msl[at..].split_once("]]").unwrap().0;
        assert!(parameter.ends_with("[[buffer(1)"), "{shader}: {parameter}");
    }
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_size_query_of_a_heap_image_cooks() {
    // `GetDimensions` declares `ImageQuery` on SPIR-V and is `get_width` on Metal.
    let source = "import lactose.heap;\n\
                  struct Push { uint slot; };\n\
                  [[vk::push_constant]] ConstantBuffer<Push> push;\n\
                  [shader(\"fragment\")] float4 fs_p() : SV_Target0\n\
                  {\n    uint w, h;\n    \
                  heap.images[nonuniform_slot(push.slot)].GetDimensions(w, h);\n    \
                  return float4(w, h, 0, 1);\n}\n";
    let msl = assert_probe_cooks("size-query", &[], source, "fs_p");
    assert!(msl.contains("get_width"), "{msl}");
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn non_uniform_resource_index_for_metal_cooks_on_the_pin_only_through_a_generic() {
    // Why the marker is the index alone on Metal: with `NonUniformResourceIndex` on every
    // target, the pin refuses it for Metal, inlined from `lactose.heap` or not, unless a
    // generic over an interface reaches it, as the engine shader's surface does.
    let switch = "    __target_switch\n    {\n    case spirv:\n        \
                  return NonUniformResourceIndex(slot);\n    default:\n        return slot;\n    }";
    let edit = (HEAP, switch, "    return NonUniformResourceIndex(slot);");
    let says = "uses features that are not available in 'fragment' stage for 'metal'";
    assert_probe_refused("nuri-direct", &[edit], HEAP_WITHOUT_PUSH, says);
    let generic = "import lactose.heap;\n\
                   interface ISurface { float4 shade(float2 uv); }\n\
                   struct Textured : ISurface\n{\n    uint slot;\n    \
                   float4 shade(float2 uv) { return heap.images[nonuniform_slot(slot)]\
                   .Sample(heap.samplers[nonuniform_slot(slot)], uv); }\n}\n\
                   float4 evaluate<S : ISurface>(S surface, float2 uv) \
                   { return surface.shade(uv); }\n\
                   [shader(\"fragment\")] float4 fs_p(float2 uv : TEXCOORD0, \
                   nointerpolation uint slot : TEXCOORD1) : SV_Target0\n\
                   { Textured surface; surface.slot = slot; return evaluate(surface, uv); }\n";
    assert_probe_cooks("nuri-generic", &[edit], generic, "fs_p");
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn sv_vertex_id_is_refused() {
    assert_refused(
        "vertex-id",
        "surface.slang",
        "SV_VulkanVertexID)",
        "SV_VertexID)",
        "surface.slang: vs_main (SPIR-V): the module declares capability DrawParameters",
    );
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn a_warning_is_an_error() {
    // An implicit conversion from float to uint is a warning of Slang's.
    assert_refused(
        "warning",
        "surface.slang",
        "    Resolved result;\n",
        "    Resolved result;\n    uint truncated = varyings.uv.x;\n",
        "slangc failed:\nerror[E30081]",
    );
}

#[test]
fn a_shader_without_slangc_and_a_lone_link_module_are_refused() {
    let env = Env::new("no-slangc");
    let sources = env.dir.join("sources");
    env.write_tree(&sources, &[("a.slang", b"")]);
    let error = env.wait(cook(&env.io, &sources, &env.dir.join("a.pack"), None)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
    assert!(
        error.message().starts_with("a.slang: a shader source, and the cook was given no slangc")
    );

    let sources = env.dir.join("lone");
    env.write_tree(&sources, &[("a.link.slang", b""), ("b.slang", b"")]);
    let error = env.wait(cook(&env.io, &sources, &env.dir.join("b.pack"), None)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(error.message().starts_with("a.link.slang: a link module, and there is no shader"));

    // A module below `modules/` alone is no source.
    let sources = env.dir.join("modules-only");
    env.write_tree(&sources, &[("modules/lactose/heap.slang", b"")]);
    let error = env.wait(cook(&env.io, &sources, &env.dir.join("c.pack"), None)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[cfg(unix)]
#[test]
fn a_slangc_of_another_version_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = Env::new("other-version");
    // The SDK's copy on the Mac reports this.
    let fake = env.dir.join("slangc");
    std::fs::write(&fake, "#!/bin/sh\necho 2026.17-25-gc7954ebd1 >&2\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = Slang::new(&fake).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");
    assert!(error.message().contains("reports `2026.17-25-gc7954ebd1`"), "{error}");
    let error = Slang::new(&env.dir.join("missing")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[test]
fn the_binary_refuses_code_without_slangc() {
    let env = Env::new("code-alone");
    let output = Command::new(env!("CARGO_BIN_EXE_ricotta"))
        .args(["--code", "out", "sources", "x.pack"])
        .current_dir(&env.dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, "ricotta: InvalidData: --code is given without --slangc\n");
    let output = Command::new(env!("CARGO_BIN_EXE_ricotta"))
        .args(["--code", "a", "--code", "b", "sources", "x.pack"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "an option twice is a usage error");
}

/// Every cooked file below `dir`, by its path below it.
fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            found.extend(
                files(&path).into_iter().map(|(below, bytes)| (format!("{name}/{below}"), bytes)),
            );
        } else {
            found.push((name, std::fs::read(&path).unwrap()));
        }
    }
    found.sort();
    found
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn two_checkouts_at_different_paths_cook_the_same_spirv_and_msl_and_name_no_path() {
    let env = Env::new("two-paths");
    let near_root = env.dir.join("a");
    let far_root = env.dir.join("a much").join("longer path").join("to the second checkout");
    copy(&root().join("shaders"), &near_root);
    copy(&root().join("shaders"), &far_root);
    let (near_code, far_code) = (env.dir.join("near-code"), env.dir.join("far-code"));
    let names = cook_shaders(&env, &near_root, Some(&near_code)).unwrap();
    assert_eq!(cook_shaders(&env, &far_root, Some(&far_code)).unwrap(), names);
    // No path of the machine's: neither checkout's, as given or resolved, nor the temporary
    // directory's, nor the repository's.
    let machine = [
        near_root.clone(),
        near_root.canonicalize().unwrap(),
        far_root.canonicalize().unwrap(),
        std::env::temp_dir(),
        root().canonicalize().unwrap(),
    ];
    let (near_files, far_files) = (files(&near_code), files(&far_code));
    assert_eq!(near_files.len(), names.len());
    let mut libraries = Vec::new();
    for ((name, near), (far_name, far)) in near_files.iter().zip(&far_files) {
        assert_eq!(name, far_name);
        if name.ends_with(".metallib") {
            libraries.push(format!("{name}: {}", if near == far { "equal" } else { "differs" }));
        } else {
            assert!(near == far, "{name} differs between the two checkouts");
        }
        for path in &machine {
            let text = path.to_string_lossy();
            let found = [near, far]
                .iter()
                .any(|bytes| bytes.windows(text.len()).any(|w| w == text.as_bytes()));
            assert!(!found, "{name} holds the path {text}");
        }
    }
    // Recorded, and nothing rests on it (decision 5.8).
    println!("metallib equality across the two checkouts: {libraries:?}");
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn the_link_time_constant_folds_into_both_targets() {
    let env = Env::new("link-constant");
    let on = env.dir.join("on-code");
    let off_sources =
        mutated(&env, "off", "surface.link.slang", "ALPHA_TESTED = true", "ALPHA_TESTED = false");
    let off = env.dir.join("off-code");
    cook_shaders(&env, &root().join("shaders"), Some(&on)).unwrap();
    cook_shaders(&env, &off_sources, Some(&off)).unwrap();
    let msl = |dir: &Path| std::fs::read_to_string(dir.join("surface.fs_main.msl")).unwrap();
    assert!(msl(&on).contains("discard_fragment()"));
    assert!(!msl(&off).contains("discard_fragment()"));
    // OpKill is opcode 252, one word: the word is its count (1) and its opcode.
    let kills = |dir: &Path| {
        let code = std::fs::read(dir.join("surface.fs_main.spirv")).unwrap();
        code.as_chunks::<4>()
            .0
            .iter()
            .filter(|w| u32::from_le_bytes(**w) == (1 << 16 | 252))
            .count()
    };
    assert_eq!((kills(&on) > 0, kills(&off)), (true, 0));
    // Folded, not a specialisation constant: the one constant either declares is TINT's.
    for dir in [&on, &off] {
        let msl = msl(dir);
        assert_eq!(msl.matches("[[function_constant(").count(), 1, "{msl}");
    }
}

/// A cooked shader as a store loads it: the cook's kind, decoded.
struct Shader(CookedShader);

impl Asset for Shader {
    const KIND: AssetKind = SHADER_KIND;
    const FORMAT_VERSION: u32 = COOKED_SHADER_VERSION;
}

struct ShaderLoader;

impl Loader for ShaderLoader {
    type Asset = Shader;
    fn load(&self, bytes: &[u8], _: &[AssetId]) -> Result<Shader> {
        CookedShader::decode(bytes).map(Shader)
    }
}

fn encode(shader: &CookedShader) -> Vec<u8> {
    shader.encode()
}

/// The engine's shaders cooked into a pack, mounted alone in a store, and each loaded, resolved
/// and pinned through [`ShaderLoader`]: what the cook published, and each shader in that order.
fn cook_and_load(env: &Env) -> (Vec<lactose_cook::Published>, Vec<CookedShader>) {
    let pack = env.dir.join("shaders.pack");
    let slang = slang();
    let shaders = Shaders { slang: &slang, code: None };
    let published = env.wait(cook(&env.io, &root().join("shaders"), &pack, Some(shaders))).unwrap();
    let loaders = vec![ErasedLoader::new(ShaderLoader)];
    let store = Store::new(env.executor.clone(), env.io.clone(), loaders).unwrap();
    env.wait(store.mount(&pack)).expect("the pack mounts");
    for asset in &published {
        store.request_newest(asset.id, LoadPriority(0));
    }
    env.wait(store.settled());
    let catalog: &dyn AssetCatalog = &store;
    let loaded = published
        .iter()
        .map(|asset| {
            let handle = catalog.newest_typed(AssetRef::<Shader>::new(asset.id));
            handle.ready().expect("the shader loads").0.clone()
        })
        .collect();
    (published, loaded)
}

#[test]
#[ignore = "runs the pinned Slang: `cargo xtask slang` installs it"]
fn the_engine_shader_cooks_to_what_its_code_declares_and_loads_from_the_pack() {
    let env = Env::new("engine");
    let (published, loaded) = cook_and_load(&env);
    let targets: &[&str] =
        if cfg!(target_os = "macos") { &["metallib", "msl", "spirv"] } else { &["msl", "spirv"] };
    let mut expected = Vec::new();
    for entry in ["fs_main", "fs_resolve", "vs_main"] {
        expected.extend(targets.iter().map(|target| format!("surface.{entry}.{target}")));
    }
    let names: Vec<&str> = published.iter().map(|asset| asset.name.as_str()).collect();
    assert_eq!(names, expected);
    let heap = HeapUse {
        form: HeapForm::FixedArray,
        set: HEAP_SET,
        arrays: vec![
            HeapArray {
                kind: HeapArrayKind::SampledImage2d,
                binding: 0,
                capacity: HEAP_SLOTS_DEFAULT,
            },
            HeapArray { kind: HeapArrayKind::Sampler, binding: 1, capacity: SAMPLERS_DEFAULT },
        ],
    };
    for (asset, shader) in published.iter().zip(&loaded) {
        let target = match asset.name.rsplit('.').next().unwrap() {
            "spirv" => ShaderTarget::SpirV,
            "msl" => ShaderTarget::Msl,
            _ => ShaderTarget::MetalLibrary,
        };
        assert_eq!(shader.target, target, "{}", asset.name);
        let entry = asset.name.split('.').nth(1).unwrap();
        assert_eq!(shader.entry, entry);
        let (stage, heap, constants, push, inputs, outputs) = match entry {
            "vs_main" => (ShaderStage::Vertex, None, vec![], 16, 0, 0),
            "fs_main" => (
                ShaderStage::Fragment,
                Some(heap.clone()),
                vec![ConstantSlot { id: 0, bytes: 4 }],
                16,
                0,
                0b1,
            ),
            _ => (ShaderStage::Fragment, None, vec![], 0, 0b1, 0b10),
        };
        assert_eq!(
            (
                shader.stage,
                &shader.heap,
                &shader.constants,
                shader.push_bytes,
                shader.inputs,
                shader.outputs
            ),
            (stage, &heap, &constants, push, inputs, outputs),
            "{}",
            asset.name
        );
    }
}

#[test]
#[ignore = "a measurement: `--run-ignored all --release --no-capture`, after `cargo xtask slang`"]
fn the_shader_kind_loads_against_the_slice() {
    // Row 136: the store's one synchronous call per load, the version hash and the decode,
    // against section 5.1's 0.5 ms slice, for each cooked shader; the median of 101 calls.
    let env = Env::new("shader-load");
    let (published, loaded) = cook_and_load(&env);
    let loader = ErasedLoader::new(ShaderLoader);
    for (asset, shader) in published.iter().zip(&loaded) {
        // The bytes the pack holds: the format's encoding is a function of the shader.
        let bytes = encode(shader);
        let header = lactose_asset_api::CookedHeader::new(SHADER_KIND, COOKED_SHADER_VERSION, []);
        let version = header.version_of(&bytes);
        assert_eq!(version, asset.version);
        let mut times: Vec<f64> = (0..101)
            .map(|_| {
                let start = Instant::now();
                loader.load(asset.id, version, &header, &bytes).unwrap();
                start.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(f64::total_cmp);
        println!("measure: shader_load_us.{} {:.2} ({} bytes)", asset.name, times[50], bytes.len());
    }
}
