# Proposal: artist-defined surfaces behind fixed pipelines

Status: proposal, not applied; `docs/architecture.md` section 12.5 ("No free-form artist
shaders") stands until it is decided. It is decided with the milestone 2 Slang toolchain
spike, whose brief takes this file as input and whose measurements are the verification
items below: adopted as an amendment to sections 12.2, 12.5 and 14 with a numbered decision
entry in the merge commit, or declined with the reason recorded in the decisions log. Nothing
in the sim track changes meanwhile.

## Architecture amendments (exact text)

### §12.2 Shader toolchain, append after "Bindless access is abstracted at the Slang level."

The renderer owns a small set of Slang interfaces that every shader is written against, and
they are the shader architecture: `IVertexDeform` (the vertex-deformed permutation axis),
`IMaterialSurface` (a material's surface evaluation, returning one `SurfaceSample` struct),
and `ILightingModel` (how a `SurfaceSample` and the lights become radiance). The fixed
permutation axes are static specialisation through Slang generics and link-time constants;
no preprocessor permutation. The cooker links every shader module against these interfaces and
refuses one that does not implement them exactly.

### §12.5 Lighting and materials, replace the paragraph beginning "Uber-shader with a material ID"

Uber-shader with a material ID and a bindless parameter block, with fixed permutation axes:
material (alpha-tested, two-sided, skinned, vertex-deformed) by pass type (G-buffer,
depth-only, forward transparent). Those axes give 48 graphics pipelines; the budget is 96 to
leave room for post-processing, compute, UI and debug pipelines. All are created at load.
Artists and tools may add surfaces, never pipelines: a material is an implementation of
`IMaterialSurface` compiled by the cooker into the uber-shader, selected at runtime by the
material ID inside the one pipeline its permutation names. A surface may not add a pass,
change blend or depth state, the vertex layout or the G-buffer, and may `discard` only under
the alpha-tested axis; the cooker enforces each. Dispatch on the material ID is a cooked
switch over the game's whole surface set, not a GPU function table, so adding a surface is a
cook-time recompile of the uber-shader (no runtime source compilation) and the pipeline count
never changes; the cull pass sorts a pipeline's compacted triangles by surface so that
divergence inside a wave stays rare. The G-buffer layout is fixed and carries a shading-model
ID; the lighting pass dispatches on it over a small fixed set of `ILightingModel`
implementations (standard PBR first; cel or toon second), so a lighting model is not a
pipeline either. Passes from `ui`, `terrain` and `debug-ui`, and anything that needs its own
passes or a different G-buffer, reach the graph through the renderer's `RenderFeature`
pass-provider trait, registered by the client binary.

### §12.5, append a paragraph

`SurfaceSample` is the contract between surfaces and lighting on both paths: deferred writes
it to the G-buffer and lights from the G-buffer; forward transparent evaluates surface and
lighting in one shader. A forward opaque path, if one is ever wanted, is one more pass type
over the same surfaces, culling and cluster compaction (sixteen more pipelines, inside the
budget), not a second renderer.

### §14 Assets and streaming, append to the cooker bullet

Surfaces are cooked assets: the cooker links each against `IMaterialSurface`, checks its
resource budget and its axis constraints, and emits the uber-shader's dispatch table; a
surface that fails to link is a cook error, never a runtime fallback.

## Decision entry (exact text; number at merge)

**D12x. Artist surfaces behind fixed pipelines** (§12.2, §12.5, §14)
- Why: revision 3.2 said "no free-form artist shaders" to protect the two hard constraints of
  GPU-driven submission (one indexed indirect draw per pipeline) and MoltenVK's per-pipeline
  compile cost (D47). Those constraints bind the pipeline count, not the surface math, so the
  rule becomes "free-form surfaces, fixed pipelines": a material is an `IMaterialSurface`
  implementation dispatched by material ID inside the pipeline its permutation names, and a
  lighting model is an `ILightingModel` implementation dispatched by a shading-model ID in the
  G-buffer. Slang interfaces are the enforcement, the cooker the gate.
  - *Dispatch is a cooked switch, not a function table.* Slang's dynamic dispatch lowers to
    function tables, which Metal supports on M1 but MoltenVK does not carry well; a switch over
    the game's whole surface set, cooked when any surface changes, is portable and keeps the
    count fixed. The cost is a cook-time recompile per surface change, acceptable under "no
    runtime source compilation".
  - *Divergence is managed by sorting, not by pipelines.* The cull pass already bins surviving
    clusters per pipeline; it sorts a pipeline's compacted triangles by surface before the
    indirect draw. The residual divergence is measured in milestone 3.
  - *The G-buffer layout and the shading-model set are the renderer's, not the game's.* A game
    that needs another G-buffer or another pass writes a `RenderFeature` provider; that is the
    extension point for passes, as surfaces are the extension point for materials.
  - *`SurfaceSample` is shared by deferred and forward.* So a forward opaque pass type is a
    cheap later addition and the renderer is not abstracted behind a trait.
  - Rules out: artist-authored pipelines, blend or depth state, vertex layouts or G-buffer
    targets; GPU function-table dispatch; a per-game lighting architecture; a renderer trait.
- Driven by: the material pipeline question (2026-10-10); lands with the milestone 2 Slang
  spike.
- Replaced: §12.5's "No free-form artist shaders."

## Verification items (exact text)

| Item | Gates milestone |
|---|---|
| Surface dispatch: the cooked switch over N surfaces against a Slang function-table build on native Metal and on MoltenVK, at N = 1, 16 and 128, measuring G-buffer pass time and pipeline creation time; the switch is the design unless the table wins on both backends | 2 |
| Divergence: G-buffer pass cost with triangles sorted by surface within a pipeline versus unsorted, on a scene with 64 surfaces, on RTX 3060-class and M1-class | 3 |
| `SurfaceSample` layout: the struct the G-buffer packs and the forward path consumes is decided with the Slang spike and frozen before the first cooked surface; a change afterwards is a cook break | 2 |
