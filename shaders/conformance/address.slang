// The conformance suite's address read: every fragment of the colour read through the device
// address in the push block.

struct Push
{
    float4* colour;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("vertex")]
float4 vs_main(uint vertex : SV_VulkanVertexID) : SV_Position
{
    float2 corner = float2(float((vertex << 1) & 2), float(vertex & 2));
    return float4(corner * 2.0 - 1.0, 0.0, 1.0);
}

[shader("fragment")]
float4 fs_main() : SV_Target0
{
    return *push.colour;
}
