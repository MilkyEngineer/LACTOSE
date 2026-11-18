// The engine's first shader, the mechanism the artist-surfaces proposal rests on and nothing
// more (milestone 2 plan, decision 5.14): a surface written against an interface that is
// specialised statically, and a link-time constant whose value surface.link.slang gives, both
// folded into SPIR-V and MSL alike.
import lactose.heap;

// A surface: what a fragment's colour is at a texture coordinate.
interface ISurface
{
    float4 shade(float2 uv);
}

// A surface that samples one heap slot with one sampler of the table.
struct Textured : ISurface
{
    uint slot;
    uint sampler;

    float4 shade(float2 uv)
    {
        return heap.images[nonuniform_slot(slot)].Sample(heap.samplers[nonuniform_slot(sampler)], uv);
    }
}

// Specialised for each surface type at compile time: no dynamic dispatch reaches the output.
float4 evaluate<S : ISurface>(S surface, float2 uv)
{
    return surface.shade(uv);
}

// The link-time constant: declared here, given by the link module.
extern static const bool ALPHA_TESTED;

// A specialisation constant, set when the pipeline is made.
[[vk::constant_id(0)]] const uint TINT = 0;

struct Push
{
    // The vertices, read through a device address: a pipeline has no vertex buffer binding.
    float4* positions;
    uint slot;
    uint sampler;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

struct Varyings
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

// `SV_VulkanVertexID`, never `SV_VertexID`: the cook refuses the capability the latter needs.
[shader("vertex")]
Varyings vs_main(uint vertex : SV_VulkanVertexID)
{
    Varyings result;
    result.position = push.positions[vertex];
    result.uv = result.position.xy * 0.5 + 0.5;
    return result;
}

struct Colour
{
    [[vk::location(0)]] float4 colour : SV_Target0;
};

[shader("fragment")]
Colour fs_main(Varyings varyings)
{
    Textured surface;
    surface.slot = push.slot;
    surface.sampler = push.sampler;
    float4 colour = evaluate(surface, varyings.uv);
    if (ALPHA_TESTED && colour.a < 0.5)
        discard;
    Colour result;
    result.colour = colour + float4(float(TINT) / 255.0);
    return result;
}

// Input 0 is colour attachment 0 of the pass: the local read.
[[vk::input_attachment_index(0)]] SubpassInput<float4> lit;

struct Resolved
{
    // One location on both targets: without `vk::location`, SPIR-V would put a lone
    // `SV_Target1` at location 0 and MSL at colour 1, and the cook refuses that.
    [[vk::location(1)]] float4 colour : SV_Target1;
};

[shader("fragment")]
Resolved fs_resolve(Varyings varyings)
{
    Resolved result;
    result.colour = float4(1.0) - lit.SubpassLoad();
    return result;
}
