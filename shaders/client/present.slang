// `lactose-app-client`'s present pass: the offscreen target the frame rendered, sampled
// through the heap slot the pass declares and drawn over the whole swapchain image, scaled to
// it. One triangle covers the viewport; the coordinate is the pixel's centre over the image's
// extent, not an interpolated varying.
import lactose.heap;

struct Push
{
    // The heap slot that holds the target.
    uint slot;
    // The sampler table's entry to sample it with.
    uint sampler;
    // The swapchain image's extent, in pixels.
    uint width;
    uint height;
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
    float2 uv = varyings.position.xy / float2(float(push.width), float(push.height));
    return heap.images[nonuniform_slot(push.slot)].Sample(heap.samplers[nonuniform_slot(push.sampler)], uv);
}
