// The conformance suite's local read: one draw writes attachment 1 with each pixel's own
// coordinates as fractions of 255, and a second, after the local read, reads it as input 1
// and writes attachment 0 with its channels swizzled.

[[vk::input_attachment_index(1)]] [[vk::binding(1, 0)]] SubpassInput<float4> written;

[shader("vertex")]
float4 vs_main(uint vertex : SV_VulkanVertexID) : SV_Position
{
    float2 corner = float2(float((vertex << 1) & 2), float(vertex & 2));
    return float4(corner * 2.0 - 1.0, 0.0, 1.0);
}

struct Written
{
    [[vk::location(1)]] float4 colour : SV_Target1;
};

[shader("fragment")]
Written fs_write(float4 position : SV_Position)
{
    uint2 pixel = uint2(position.xy);
    Written result;
    result.colour = float4(float(pixel.x) / 255.0, float(pixel.y) / 255.0, 0.5, 1.0);
    return result;
}

[shader("fragment")]
float4 fs_light() : SV_Target0
{
    return written.SubpassLoad().bgra;
}
