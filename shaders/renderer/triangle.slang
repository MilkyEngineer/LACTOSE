// `lactose-renderer`'s triangle pass: every held slot drawn as `shader::TRIANGLE`, at
// `CameraData::clip(InstanceRecord::drawn(fraction, camera.cell).apply(vertex))`, in the
// colour `shader::PALETTE[slot % 7]`. The vertex stage is a transcription of those three
// functions, in 32-bit floats.
import lactose.renderer;

// `TRIANGLE_PUSH_BYTES`, 32 bytes, the vertex stage's alone: the fragment stage reads nothing
// of it and declares no block. An entry-point parameter, never a global, so that Metal takes
// it at `[[buffer(0)]]` (the README, "Metal's buffer index").
struct TrianglePush
{
    // The instance buffer's first record.
    InstanceRecord* instances;
    // The frame's camera.
    CameraData* camera;
    // The triangle's first vertex, three `float` each.
    float* vertices;
    float fraction;
    uint zero;
};

// `CELL_SIZE_CONSTANT`: the cell size in metres, which the renderer sets to
// `RenderPosition::CELL_SIZE`. Zero unless set, so a pipeline made without it puts every cell
// at the camera's.
[[vk::constant_id(0)]] const float cell_size = 0.0;

struct Varyings
{
    float4 position : SV_Position;
    // The slot, which the fragment stage colours by.
    nointerpolation uint slot : TEXCOORD0;
};

// `RenderPosition::relative_to(camera.cell)`: the cells subtracted as `uint` and the result
// reinterpreted as `int` before anything becomes a float, which wraps on every target where a
// signed overflow is undefined in MSL.
float3 relative(Placement placement, int3 cell)
{
    float3 result;
    for (int axis = 0; axis < 3; axis++)
    {
        int cells = int(uint(placement.cell[axis]) - uint(cell[axis]));
        result[axis] = float(cells) * cell_size + placement.offset[axis];
    }
    return result;
}

float4 rotation(Placement placement)
{
    return float4(
        placement.rotation[0], placement.rotation[1], placement.rotation[2], placement.rotation[3]);
}

[shader("vertex")]
Varyings vs_main(
    uint vertex : SV_VulkanVertexID,
    uint slot : SV_VulkanInstanceID,
    uniform TrianglePush push)
{
    Varyings result;
    result.slot = slot;
    InstanceRecord* record = push.instances + slot;
    if ((record.flags & LIVE) == 0)
    {
        // Outside the clip volume's `x <= w` plane: clipping discards the triangle, with depth
        // clamp or without.
        result.position = float4(2.0, 2.0, 2.0, 1.0);
        return result;
    }
    CameraData* camera = push.camera;
    float t = push.fraction;
    // `InstanceRecord::drawn`.
    float3 from = relative(record.previous, camera.cell);
    float3 to = relative(record.current, camera.cell);
    float3 position = from + (to - from) * t;
    float4 p = rotation(record.previous);
    float4 c = rotation(record.current);
    if (dot(p, c) < 0.0)
        c = -c;
    float4 q = p + (c - p) * t;
    q = q / sqrt(dot(q, q));
    // `CellTransform::apply`.
    float3 v = float3(
        push.vertices[3 * vertex], push.vertices[3 * vertex + 1], push.vertices[3 * vertex + 2]);
    float3 inner = cross(q.xyz, v) + q.w * v;
    float3 moved = position + (v + 2.0 * cross(q.xyz, inner));
    // `CameraData::clip`.
    result.position = mul(camera.clip_from_cell, float4(moved, 1.0));
    return result;
}

// `shader::PALETTE`, each channel over 255.
static const uint3 PALETTE[7] = {
    uint3(255, 0, 0), uint3(0, 255, 0), uint3(0, 0, 255), uint3(255, 255, 0),
    uint3(255, 0, 255), uint3(0, 255, 255), uint3(255, 255, 255),
};

struct Colour
{
    [[vk::location(0)]] float4 colour : SV_Target0;
};

[shader("fragment")]
Colour fs_main(nointerpolation uint slot : TEXCOORD0)
{
    Colour result;
    result.colour = float4(float3(PALETTE[slot % 7]) / 255.0, 1.0);
    return result;
}
