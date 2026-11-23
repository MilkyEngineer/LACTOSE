// The conformance suite's indirect dispatch: each workgroup writes one to its own cell through
// the device address in the push block, numbered x fastest by the counts `x` and `y` the push
// block gives, the ones the case expects.

struct Push
{
    uint* cells;
    uint x;
    uint y;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("compute")]
[numthreads(1, 1, 1)]
void cs_main(uint3 group : SV_GroupID)
{
    push.cells[group.x + push.x * (group.y + push.y * group.z)] = 1u;
}
