// The conformance suite's workgroup: workgroups of (8, 4, 2), each invocation whose global id
// lies inside (24, 8, 2) writing seven words through the device address in the push block, at
// the record its global id flattens to, x fastest: its local id, its global id and its local
// index. One outside writes nothing, so a dispatch of another size cannot write past the
// records.

static const uint3 BOUNDS = uint3(24, 8, 2);
static const uint RECORD = 7;

struct Push
{
    uint* out;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("compute")]
[numthreads(8, 4, 2)]
void cs_main(uint3 global : SV_DispatchThreadID, uint3 local : SV_GroupThreadID,
             uint index : SV_GroupIndex)
{
    if (any(global >= BOUNDS))
        return;
    uint at = (global.x + BOUNDS.x * (global.y + BOUNDS.y * global.z)) * RECORD;
    push.out[at + 0] = local.x;
    push.out[at + 1] = local.y;
    push.out[at + 2] = local.z;
    push.out[at + 3] = global.x;
    push.out[at + 4] = global.y;
    push.out[at + 5] = global.z;
    push.out[at + 6] = index;
}
