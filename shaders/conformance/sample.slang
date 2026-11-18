// The conformance suite's heap read: the image of one heap slot sampled at each fragment's
// coordinate across the viewport, by one of two samplers of the table, the left half's or
// the right half's: an index that is not uniform across the draw.
import lactose.heap;

struct Push
{
    uint slot;
    uint left;
    uint right;
    uint unused;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

struct Varyings
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

[shader("vertex")]
Varyings vs_main(uint vertex : SV_VulkanVertexID)
{
    float2 corner = float2(float((vertex << 1) & 2), float(vertex & 2));
    Varyings result;
    result.position = float4(corner * 2.0 - 1.0, 0.0, 1.0);
    result.uv = float2(corner.x, 1.0 - corner.y);
    return result;
}

[shader("fragment")]
float4 fs_main(Varyings varyings) : SV_Target0
{
    uint sampler = varyings.uv.x < 0.5 ? push.left : push.right;
    return heap.images[nonuniform_slot(push.slot)].Sample(heap.samplers[nonuniform_slot(sampler)], varyings.uv);
}
