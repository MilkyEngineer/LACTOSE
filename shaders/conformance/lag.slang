// `lactose-rhi-vulkan`'s device test of timestamps around a render pass: a full-screen
// triangle whose fragment stage runs `lag` rounds of arithmetic that cannot be folded away,
// so that one render pass lasts long enough to tell whether two timestamps bracket it.

struct Push
{
    uint lag;
    uint unused;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

struct Varyings
{
    float4 position : SV_Position;
};

[shader("vertex")]
Varyings vs_main(uint vertex : SV_VulkanVertexID)
{
    Varyings result;
    float2 uv = float2(float((vertex << 1) & 2u), float(vertex & 2u));
    result.position = float4(uv * 2.0 - 1.0, 0.5, 1.0);
    return result;
}

struct Colour
{
    [[vk::location(0)]] float4 colour : SV_Target0;
};

[shader("fragment")]
Colour fs_main(Varyings input)
{
    uint x = uint(input.position.x) ^ (uint(input.position.y) << 16);
    for (uint round = 0; round < push.lag; round++)
        x = x * 1664525u + 1013904223u;
    Colour result;
    result.colour = float4(float(x & 255u) / 255.0, 0.0, 0.0, 1.0);
    return result;
}
