// The conformance suite's draw parameters: each triangle written with the vertex number of
// its first vertex, which counts the base vertex, and its instance number, which counts the
// first instance, as fractions of 255. Every vertex of a triangle carries the same pair, so
// no provoking-vertex convention enters the output.

struct Push
{
    float4* positions;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

struct Varyings
{
    float4 position : SV_Position;
    nointerpolation uint2 ids : TEXCOORD0;
};

[shader("vertex")]
Varyings vs_main(uint vertex : SV_VulkanVertexID, uint instance : SV_VulkanInstanceID)
{
    Varyings result;
    result.position = push.positions[vertex];
    result.ids = uint2(vertex - vertex % 3, instance);
    return result;
}

[shader("fragment")]
float4 fs_main(Varyings varyings) : SV_Target0
{
    return float4(float(varyings.ids.x) / 255.0, float(varyings.ids.y) / 255.0, 0.0, 1.0);
}
