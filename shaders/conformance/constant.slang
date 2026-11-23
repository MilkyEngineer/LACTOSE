// The conformance suite's specialisation: every fragment of a colour two constants set when
// the pipeline is made, a 32-bit red as a fraction of 255 and a `bool` green.

[[vk::constant_id(0)]] const uint RED = 0;
[[vk::constant_id(1)]] const bool GREEN = false;

[shader("vertex")]
float4 vs_main(uint vertex : SV_VulkanVertexID) : SV_Position
{
    float2 corner = float2(float((vertex << 1) & 2), float(vertex & 2));
    return float4(corner * 2.0 - 1.0, 0.0, 1.0);
}

[shader("fragment")]
float4 fs_main() : SV_Target0
{
    return float4(float(RED) / 255.0, GREEN ? 1.0 : 0.0, 0.0, 1.0);
}
