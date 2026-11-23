// The conformance suite's compute reader: thread `i` below `count` writes `from[i] + add` to
// `to[i]`, both through device addresses in the push block.

struct Push
{
    uint* from;
    uint* to;
    uint count;
    uint add;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("compute")]
[numthreads(64, 1, 1)]
void cs_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.count)
        return;
    push.to[id.x] = push.from[id.x] + push.add;
}
