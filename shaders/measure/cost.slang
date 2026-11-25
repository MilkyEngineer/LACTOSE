// `cargo xtask measure gpu-cost`'s shader: a pipeline drawn ten thousand times, and made 96
// times under a specialisation constant the run salts, so that its SPIR-V, and under MoltenVK
// its MSL, stay the same. Both stages use the whole constant, each half exactly (a float holds
// 16 bits), so a driver that folds it into the code it hands Metal makes a program per value
// and not per residue. The pipelines made from new source are the measurement's own, written
// and cooked by the run with a link-time constant it salts (`tools/xtask`, `measure`).

[[vk::constant_id(0)]] const uint VARIANT = 0;

struct Push
{
    // The vertex pool, read through a device address: a pipeline has no vertex buffer binding.
    float4* positions;
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
    result.position = push.positions[vertex];
    // Far below a pixel's resolution: the draw is the same for every value.
    result.position.x += float(VARIANT & 0xFFFFu) * 1e-20;
    result.position.y += float(VARIANT >> 16) * 1e-20;
    return result;
}

struct Colour
{
    [[vk::location(0)]] float4 colour : SV_Target0;
};

[shader("fragment")]
Colour fs_main()
{
    Colour result;
    result.colour = float4(float(VARIANT & 0xFFFFu) / 65535.0, float(VARIANT >> 16) / 65535.0,
                           0.25, 1.0);
    return result;
}
