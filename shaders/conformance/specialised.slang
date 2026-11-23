// The conformance suite's typed specialisation: one invocation writes each constant as it reads
// it, through the device address in the push block, a `bool` as 0 or 1 and an `int` or a
// `float` as its bits. The case gives the first five a value other than their defaults and the
// last two none.

[[vk::constant_id(0)]] const bool SET_TRUE = false;
[[vk::constant_id(1)]] const bool SET_FALSE = true;
[[vk::constant_id(2)]] const int SIGNED = 5;
[[vk::constant_id(3)]] const uint UNSIGNED = 2;
[[vk::constant_id(4)]] const float REAL = 0.25;
[[vk::constant_id(5)]] const bool KEPT_TRUE = true;
[[vk::constant_id(6)]] const uint KEPT = 9;

struct Push
{
    uint* out;
};

[[vk::push_constant]] ConstantBuffer<Push> push;

[shader("compute")]
[numthreads(1, 1, 1)]
void cs_main()
{
    push.out[0] = SET_TRUE ? 1u : 0u;
    push.out[1] = SET_FALSE ? 1u : 0u;
    push.out[2] = asuint(SIGNED);
    push.out[3] = UNSIGNED;
    push.out[4] = asuint(REAL);
    push.out[5] = KEPT_TRUE ? 1u : 0u;
    push.out[6] = KEPT;
}
