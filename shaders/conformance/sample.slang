// The conformance suite's heap read: the image of one heap slot sampled at each fragment's
// coordinate across the viewport, by one of two samplers of the table, the left half's or
// the right half's: an index that is not uniform across the draw. The coordinate is the
// pixel's centre over the square target's side, exact where the side is a power of two, not
// an interpolated varying, whose last bits a device may round either way.
import lactose.heap;

struct Push
{
    uint slot;
    uint left;
    uint right;
    uint side;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

struct Varyings
{
    float4 position : SV_Position;
};

[shader("vertex")]
Varyings vs_main(uint vertex : SV_VulkanVertexID)
{
    float2 corner = float2(float((vertex << 1) & 2), float(vertex & 2));
    Varyings result;
    result.position = float4(corner * 2.0 - 1.0, 0.0, 1.0);
    return result;
}

[shader("fragment")]
float4 fs_main(Varyings varyings) : SV_Target0
{
    float2 uv = varyings.position.xy / float(push.side);
    uint sampler = uv.x < 0.5 ? push.left : push.right;
    return heap.images[nonuniform_slot(push.slot)].Sample(heap.samplers[nonuniform_slot(sampler)], uv);
}
