// The conformance suite's triangles: each vertex's position read through the device address
// in the push block, at its vertex number, which counts the draw's base vertex, and every
// fragment of the colour in the push block.

struct Push
{
    float4* positions;
    uint2 unused;
    float4 colour;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("vertex")]
float4 vs_main(uint vertex : SV_VulkanVertexID) : SV_Position
{
    return push.positions[vertex];
}

[shader("fragment")]
float4 fs_main() : SV_Target0
{
    return push.colour;
}
