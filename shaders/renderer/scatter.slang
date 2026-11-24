// `lactose-renderer`'s scatter: each invocation applies one entry of a dispatch to the record
// of its slot, as `InstanceRecord::free` and `InstanceRecord::scatter` state the rule; this is
// a transcription of them. No two entries of a dispatch name one slot, so the invocations
// write disjoint records in any order.
import lactose.renderer;

// `SCATTER_PUSH_BYTES`, 32 bytes. An entry-point parameter, never a global, so that Metal
// takes it at `[[buffer(0)]]` (the README, "Metal's buffer index").
struct ScatterPush
{
    // The region's first row; zero where the dispatch's entries are all `FREE`, and then
    // never read.
    StagingRow* rows;
    // The dispatch's first entry.
    ScatterEntry* entries;
    // The instance buffer's first record.
    InstanceRecord* instances;
    uint count;
    uint zero;
};

// `SCATTER_GROUP`.
[shader("compute")]
[numthreads(64, 1, 1)]
void cs_main(uint3 id : SV_DispatchThreadID, uniform ScatterPush push)
{
    uint i = id.x;
    if (i >= push.count)
        return;
    ScatterEntry entry = push.entries[i];
    InstanceRecord* record = push.instances + entry.slot;
    if ((entry.flags & FREE) != 0)
    {
        record.flags = 0;
        return;
    }
    StagingRow row = push.rows[entry.row];
    if ((entry.flags & TAKE_BASE) != 0)
        record.base = record.current;
    bool from_base = (entry.flags & HAS_BASE) != 0 && (row.flags & NO_INTERPOLATE) == 0;
    record.previous = from_base ? record.base : row.placement;
    record.current = row.placement;
    record.flags = LIVE;
    record.reserved = 0;
}
