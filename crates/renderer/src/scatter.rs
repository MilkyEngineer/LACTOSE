//! What the scatter is given and what it leaves: the staging ring's row, a batch's entry
//! and the persistent instance buffer's record, each in the bytes the shaders read
//! (`README.md`, "Formats"), and the scatter's rule as a function of the host.
//!
//! The rule is stated here once, in Rust, so that the shader `slang` writes, the batch
//! `renderer-impl` builds and the reference a device test compares against are three
//! readings of one text.

use lactose_present_api::{InstanceRow, RenderTransform};

use crate::slots::Slot;

/// One row of the staging ring: the placement an instance has now, in the cell-plus-offset
/// format of D41. The ring's final format (the milestone 2 plan, decision 5.4): a later
/// milestone adds buffers beside the ring, not fields to its row.
///
/// 48 bytes, little-endian: the cell as three `i32` (0), the offset from the cell's centre
/// as three `f32` in metres (12), the rotation as four `f32`, `x y z w` (24), the flags
/// (40), and four reserved bytes written as zero (44).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StagingRow {
    /// The placement.
    pub transform: RenderTransform,
    /// [`NO_INTERPOLATE`](Self::NO_INTERPOLATE), or zero.
    pub flags: u32,
}

impl StagingRow {
    /// The bytes of a row, and the ring's stride.
    pub const BYTES: u32 = 48;

    /// The row's placement is a jump and not a movement: the scatter sets the previous
    /// placement to the row's as well.
    pub const NO_INTERPOLATE: u32 = 1;

    /// The row a writer puts in the ring for `row`: its transform, and
    /// [`NO_INTERPOLATE`](Self::NO_INTERPOLATE) when `row.no_interpolate` is set. The key
    /// and the mesh are not in the ring: a writer hands them to the slot table beside it.
    #[must_use]
    pub fn of(row: &InstanceRow) -> Self {
        todo!()
    }

    /// The row's bytes, as the ring holds them.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 48] {
        todo!()
    }
}

/// One entry of a batch: what the scatter does to one slot.
///
/// 12 bytes, little-endian: the slot (0), the row (4) and the flags (8), each a `u32`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScatterEntry {
    /// The slot whose record the entry writes. No other entry of its dispatch names it.
    pub slot: Slot,
    /// The index of the row the entry reads, counted from the first row of the frame's
    /// region. Zero, and not read, for a [`FREE`](Self::FREE) entry.
    pub row: u32,
    /// [`FREE`](Self::FREE) alone, or any of [`TAKE_BASE`](Self::TAKE_BASE) and
    /// [`HAS_BASE`](Self::HAS_BASE).
    pub flags: u32,
}

impl ScatterEntry {
    /// The bytes of an entry.
    pub const BYTES: u32 = 12;

    /// The slot was freed: the scatter clears the record's flags and reads no row.
    pub const FREE: u32 = 1;

    /// The record's base is taken first, from its current placement: the entry is the
    /// instance's first write in a tick.
    pub const TAKE_BASE: u32 = 2;

    /// The record's base counts for this write: the previous placement becomes the base,
    /// unless the row says [`StagingRow::NO_INTERPOLATE`]. Clear for a new instance, for a
    /// remote instance, and in a tick whose first pass dropped the bases.
    pub const HAS_BASE: u32 = 4;

    /// The entry's bytes, as the batch holds them.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 12] {
        todo!()
    }
}

/// One record of the persistent instance buffer: the two placements a frame draws an
/// instance between, and the base the next tick's first write is drawn from
/// (`crates/present-api/README.md`, "What the sink keeps").
///
/// 128 bytes, little-endian: the previous placement (0), the current one (40) and the
/// base (80), each 40 bytes laid out as a [`StagingRow`]'s first 40; the flags (120); and
/// four reserved bytes (124). The bytes of a record no entry has written are undefined,
/// and so is the base of a record whose instance has had none taken.
///
/// The record is milestone 2's layout of the instance buffer, and milestone 3's GPU scene
/// may change it, with the scatter's rule, the shaders and this type in one change.
/// [`StagingRow`], the ring's row, it does not change.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InstanceRecord {
    /// Where the instance is drawn at fraction zero.
    pub previous: RenderTransform,
    /// Where the instance is drawn at fraction one.
    pub current: RenderTransform,
    /// The placement the record held as current when its instance's first write of the
    /// current tick was scattered.
    pub base: RenderTransform,
    /// [`LIVE`](Self::LIVE), or zero.
    pub flags: u32,
}

impl InstanceRecord {
    /// The bytes of a record, and the instance buffer's stride.
    pub const BYTES: u32 = 128;

    /// An instance holds the slot. The triangle pass draws no triangle for a record
    /// without it.
    pub const LIVE: u32 = 1;

    /// Reads a record from the bytes a readback of the instance buffer returned.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 128]) -> Self {
        todo!()
    }

    /// What the scatter does to this record for an entry whose flags are `entry` and
    /// whose row is `row`, when `entry` is not [`ScatterEntry::FREE`]:
    ///
    /// 1. with [`ScatterEntry::TAKE_BASE`], the base becomes the current placement;
    /// 2. the previous placement becomes the base when `entry` has
    ///    [`ScatterEntry::HAS_BASE`] and `row` has no [`StagingRow::NO_INTERPOLATE`], and
    ///    the row's otherwise;
    /// 3. the current placement becomes the row's, and the flags become
    ///    [`LIVE`](Self::LIVE).
    ///
    /// The base is never set to the row's. The scatter shader is this function, run once
    /// for each entry of a dispatch, in no order: no two entries of a dispatch name one
    /// slot.
    pub fn scatter(&mut self, entry: u32, row: &StagingRow) {
        todo!()
    }

    /// What the scatter does to this record for a [`ScatterEntry::FREE`] entry: the flags
    /// become zero, and the placements stay as they were.
    pub fn free(&mut self) {
        todo!()
    }

    /// Where the triangle pass draws the instance at `fraction` for a camera in `cell`,
    /// as the vertex shader computes it, in 32-bit floats:
    ///
    /// - each placement's position relative to the cell's centre,
    ///   `RenderPosition::relative_to(cell)`, which subtracts the cells as integers,
    ///   wrapping, before anything becomes a float, then multiplies by the cell size and
    ///   adds the offset. The shader subtracts them as `uint` and reinterprets the result
    ///   as `int`, `int(uint(a) - uint(b))`, which wraps on every target: a signed
    ///   overflow is undefined in MSL;
    /// - the position `p + (c - p) * fraction` of those two;
    /// - the rotation `normalize(p + (c' - p) * fraction)`, where `c'` is the current
    ///   rotation negated when its dot product with the previous one is negative.
    ///
    /// `None` for a record without [`LIVE`](Self::LIVE).
    #[must_use]
    pub fn drawn(&self, fraction: f32, cell: [i32; 3]) -> Option<CellTransform> {
        todo!()
    }
}

/// A placement relative to the centre of a camera's cell: what a vertex is moved by.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CellTransform {
    /// The instance's origin, in metres from the cell's centre.
    pub position: [f32; 3],
    /// The rotation, a unit quaternion `x y z w`.
    pub rotation: [f32; 4],
}

impl CellTransform {
    /// A vertex of the instance's mesh, moved: `position + rotate(rotation, vertex)`,
    /// with the rotation applied as `v + 2 * cross(q.xyz, cross(q.xyz, v) + q.w * v)`.
    #[must_use]
    pub fn apply(&self, vertex: [f32; 3]) -> [f32; 3] {
        todo!()
    }
}

/// The entries one dispatch of the scatter runs over: a run of a batch's entries, none of
/// which names a slot another names.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BatchRange {
    /// The index of the run's first entry in [`ScatterBatch::entries`].
    pub first: u32,
    /// How many entries the run holds, at least one.
    pub count: u32,
}

/// The batch of the frame [`Renderer::prepare`](crate::Renderer::prepare) last prepared:
/// every entry the frame's scatter runs, and the dispatches it runs them in.
///
/// The entries are the bytes the renderer uploaded, kept on the host so that a test on a
/// device that runs no shader can hold the batch to its rule (`README.md`, "The batch").
#[derive(Clone, Copy, Debug)]
pub struct ScatterBatch<'a> {
    /// Every entry, the dispatches' runs one after another.
    entries: &'a [ScatterEntry],
    /// The dispatches, in the order the scatter pass records them.
    dispatches: &'a [BatchRange],
}

impl<'a> ScatterBatch<'a> {
    /// The batch of `entries` run as `dispatches`. The renderer's plumbing.
    pub(crate) const fn new(entries: &'a [ScatterEntry], dispatches: &'a [BatchRange]) -> Self {
        Self { entries, dispatches }
    }

    /// Every entry of the batch: the dispatches' runs, one after another.
    #[must_use]
    pub const fn entries(&self) -> &'a [ScatterEntry] {
        self.entries
    }

    /// The dispatches, in the order the scatter pass records them. Their runs do not
    /// overlap and together cover [`entries`](Self::entries).
    #[must_use]
    pub const fn dispatches(&self) -> &'a [BatchRange] {
        self.dispatches
    }

    /// The entries of one dispatch.
    ///
    /// # Panics
    ///
    /// If `range` reaches past the batch's entries, which no range of
    /// [`dispatches`](Self::dispatches) does.
    #[must_use]
    pub fn run(&self, range: BatchRange) -> &'a [ScatterEntry] {
        &self.entries[range.first as usize..(range.first + range.count) as usize]
    }
}
