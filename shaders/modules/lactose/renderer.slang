// The records `lactose-renderer`'s shaders read and write, as its README's "The shader
// interface", "Records", declares them: valid under Vulkan's standard layout rules for a
// physical storage buffer with no `scalarBlockLayout`, so a placement is arrays of 4-byte
// scalars and never vectors. Imported as `lactose.renderer` by `renderer/scatter.slang` and
// `renderer/triangle.slang`. The host's statement of each is the crate's type of the same
// name; a change here is a change there.

// 40 bytes: the cell (0), the offset from its centre in metres (12), the rotation `x y z w`
// (24).
public struct Placement
{
    public int cell[3];
    public float offset[3];
    public float rotation[4];
};

// 48 bytes.
public struct StagingRow
{
    public Placement placement;
    public uint flags;
    public uint reserved;
};

// 12 bytes.
public struct ScatterEntry
{
    public uint slot;
    public uint row;
    public uint flags;
};

// 128 bytes.
public struct InstanceRecord
{
    public Placement previous;
    public Placement current;
    public Placement base;
    public uint flags;
    public uint reserved;
};

// 80 bytes: the matrix from metres about the cell's centre to clip space, column `c` and row
// `r` at index `4 * c + r` (0); the camera's cell (64).
public struct CameraData
{
    public column_major float4x4 clip_from_cell;
    public int3 cell;
    public uint reserved;
};

// `StagingRow::NO_INTERPOLATE`.
public static const uint NO_INTERPOLATE = 1;
// `ScatterEntry::FREE`, `TAKE_BASE` and `HAS_BASE`.
public static const uint FREE = 1;
public static const uint TAKE_BASE = 2;
public static const uint HAS_BASE = 4;
// `InstanceRecord::LIVE`.
public static const uint LIVE = 1;
