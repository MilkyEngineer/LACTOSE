// The conformance suite's compute writer: thread `i` below `count` writes `value + i` through
// the device address in the push block, after `lag` rounds of arithmetic that delay the
// write and cannot be folded away.

struct Push
{
    uint* out;
    uint count;
    uint lag;
    uint value;
    uint unused;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("compute")]
[numthreads(64, 1, 1)]
void cs_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.count)
        return;
    uint x = id.x;
    for (uint round = 0; round < push.lag; round++)
        x = x * 1664525u + 1013904223u;
    uint value = push.value + id.x;
    // Never true for the lags the suite gives; the write depends on every round.
    if (x == 0xffffffffu && push.lag == 1u)
        value = 0u;
    push.out[id.x] = value;
}
