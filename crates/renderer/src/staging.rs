//! The staging ring's writers (architecture section 5.3): the sink extract writes a
//! world's instances through, and the handle the presentation schedule writes remote
//! instances through. Both write the region the renderer has open, and neither offers a
//! read: the ring is the device's mapped memory, write-combined on a discrete GPU, where
//! a read is very slow (the milestone 2 plan, decision 5.2).
//!
//! What the writers share with the renderer is atomics and the device, and nothing
//! behind a lock of this crate's (`README.md`, "Concurrency").

use std::marker::PhantomData;
use std::sync::Arc;

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, MeshAsset};
use lactose_rhi_api::{BufferHandle, Device};

use crate::key::KeyBits;
use crate::scatter::StagingRow;
use crate::sync::{AtomicU64, Ordering};

/// The open word's region index while no region is open.
const NO_REGION: u32 = 0xffff;

/// The most passes the open word counts.
const COUNT_MAX: u32 = 0xffff;

/// The most regions a renderer has: every index below [`NO_REGION`].
pub(crate) const REGIONS_MAX: u32 = NO_REGION;

/// The open word of `generation`, `region` and `count`.
const fn word(generation: u32, region: u32, count: u32) -> u64 {
    (generation as u64) << 32 | (region as u64) << 16 | count as u64
}

/// The generation, the region and the count of an open word.
const fn fields(word: u64) -> (u32, u32, u32) {
    ((word >> 32) as u32, (word >> 16) as u32 & 0xffff, word as u32 & 0xffff)
}

/// The generation after `generation`, which is never zero.
pub(crate) const fn next_generation(generation: u32) -> u32 {
    match generation.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

/// The index of `kind` among the counts by kind: `WorldKind`'s variants in order.
pub(crate) const fn kind_index(kind: WorldKind) -> usize {
    match kind {
        WorldKind::Server => 0,
        WorldKind::Client => 1,
        WorldKind::Prediction => 2,
    }
}

/// The kinds in the order of their variants, which is [`kind_index`]'s.
pub(crate) const KINDS: [WorldKind; 3] =
    [WorldKind::Server, WorldKind::Client, WorldKind::Prediction];

// A shadow entry's note, the low byte of its `note` word.
const BEGIN: u64 = 1;
const END: u64 = 2;
const REMOVED: u64 = 3;
const ROW: u64 = 4;
const REMOTE_OPEN: u64 = 5;
const REMOTE_END: u64 = 6;

/// What the writers and the renderer share: the regions' buffers, which the writers reach
/// through `Device::write_buffer`; the open word; each region's 64-bit cursor and counts;
/// what a writer leaves beside each row for [`Renderer::prepare`](crate::Renderer::prepare)
/// to read, the row's key, its mesh, the pass or batch it belongs to and the open
/// generation it was written under; and the renderer-wide record of kinds lost and rows
/// dropped while no region is open, which `prepare` takes with a swap and `open` never
/// resets.
///
/// Every field a writer and the renderer both touch is an atomic, and the orderings are
/// `README.md`'s, "Concurrency": `open`'s store of the word is `Release`; a `begin`'s
/// compare-exchange of it is `Acquire` on success; a pass's `end`, and a sink's drop with
/// a pass open, give back the count with a `Release` subtraction; the sealing's
/// compare-exchange is `Acquire`; and a reservation is a `Relaxed` addition to the cursor.
/// Every change of the word between `open`'s store and the sealing is a read-modify-write,
/// so that the sealing's acquire reads every give-back's release. The implementation adds
/// no `Mutex`, `RwLock`, condition variable or channel here.
pub(crate) struct Staging {
    /// The device the writers write through.
    device: Arc<dyn Device>,
    /// Which region is open, its open generation, and how many passes are open on it, in
    /// one word, so that a pass's beginning and the renderer's sealing cannot both
    /// succeed: the generation in the high 32 bits, the region's index in the next 16,
    /// all ones while no region is open, and the count in the low 16. The generation is
    /// never zero, which the shadow entries start at. A `begin` that would take the count
    /// past its width is lost and takes no count, so its lost mark goes in the
    /// renderer-wide record, not the region's: the sealing cannot miss it and `open`
    /// cannot reset it. The remote batch takes no count.
    open: AtomicU64,
    /// The rows of each region.
    rows: u32,
    /// The regions, by index.
    regions: Box<[Region]>,
    /// What was lost and dropped while no region was open, or by a pass that held no
    /// count of the word.
    wide: Record,
}

/// One region of the ring, as the writers see it.
struct Region {
    /// Its rows: `rows * StagingRow::BYTES` bytes of `Upload` memory.
    buffer: BufferHandle,
    /// The next row to reserve. 64 bits, so it never wraps, and a region left open for
    /// good keeps refusing what does not fit.
    cursor: AtomicU64,
    /// What its passes lost and dropped.
    counts: Record,
    /// What the writers left beside each row.
    shadow: Box<[Shadow]>,
}

/// Kinds lost and rows dropped, in all and by kind. Each count is 64 bits, so it never
/// wraps, and saturates at `u32::MAX` where `prepare` reports it.
struct Record {
    /// The lost kinds, as bits of [`kind_index`].
    lost: AtomicU64,
    /// The rows dropped.
    dropped: AtomicU64,
    /// The rows of extract's passes dropped, by [`kind_index`].
    dropped_by_kind: [AtomicU64; 3],
}

impl Record {
    fn new() -> Self {
        Self {
            lost: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            dropped_by_kind: [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)],
        }
    }

    fn lose(&self, kind: WorldKind) {
        self.lost.fetch_or(1 << kind_index(kind), Ordering::Relaxed);
    }

    /// Counts `rows` dropped, of `kind`'s extract passes where it is given.
    fn drop_rows(&self, rows: u64, kind: Option<WorldKind>) {
        if rows == 0 {
            return;
        }
        self.dropped.fetch_add(rows, Ordering::Relaxed);
        if let Some(kind) = kind {
            self.dropped_by_kind[kind_index(kind)].fetch_add(rows, Ordering::Relaxed);
        }
    }

    /// The counts, which the caller reads after the sealing.
    fn read(&self) -> Counts {
        Counts {
            lost: self.lost.load(Ordering::Relaxed) as u8,
            dropped: self.dropped.load(Ordering::Relaxed),
            dropped_by_kind: self.dropped_by_kind.each_ref().map(|c| c.load(Ordering::Relaxed)),
        }
    }

    /// The counts, reset to zero.
    fn take(&self) -> Counts {
        Counts {
            lost: self.lost.swap(0, Ordering::Relaxed) as u8,
            dropped: self.dropped.swap(0, Ordering::Relaxed),
            dropped_by_kind: self.dropped_by_kind.each_ref().map(|c| c.swap(0, Ordering::Relaxed)),
        }
    }

    fn reset(&self) {
        self.lost.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
        for count in &self.dropped_by_kind {
            count.store(0, Ordering::Relaxed);
        }
    }
}

/// A [`Record`]'s values.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Counts {
    /// The lost kinds, as bits of [`kind_index`].
    pub(crate) lost: u8,
    /// The rows dropped.
    pub(crate) dropped: u64,
    /// The rows of extract's passes dropped, by [`kind_index`].
    pub(crate) dropped_by_kind: [u64; 3],
}

impl Counts {
    fn add(self, other: Self) -> Self {
        Self {
            lost: self.lost | other.lost,
            dropped: self.dropped.saturating_add(other.dropped),
            dropped_by_kind: core::array::from_fn(|k| {
                self.dropped_by_kind[k].saturating_add(other.dropped_by_kind[k])
            }),
        }
    }
}

/// What a writer leaves beside one row of a region: a note, or the key and mesh of a row.
/// Its words start at zero, a generation no use of a region has.
struct Shadow {
    /// The open generation in the high 32 bits, and the index of the row holding its
    /// pass's `begin` or its batch's opening in the low 32.
    mark: AtomicU64,
    /// The note in the low byte; for a `begin`, the kind (bits 8 and 9), whether the pass
    /// is full (bit 10) and the tick (the high 32 bits).
    note: AtomicU64,
    /// A row's or a removed key's [`KeyBits`], low word first.
    key: [AtomicU64; 2],
    /// A row's mesh, as `AssetId` bits.
    mesh: AtomicU64,
}

impl Shadow {
    fn new() -> Self {
        Self {
            mark: AtomicU64::new(0),
            note: AtomicU64::new(0),
            key: [AtomicU64::new(0), AtomicU64::new(0)],
            mesh: AtomicU64::new(0),
        }
    }
}

/// A use of a region: its index and the open generation it was opened under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Use {
    pub(crate) region: u32,
    pub(crate) generation: u32,
}

/// Why a sealing was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Unsealed {
    /// No region is open.
    NotOpen,
    /// A sink's pass is open on the region.
    PassOpen,
}

/// A region sealed by [`Staging::seal`]: the use, the entries a writer may have left, and
/// the counts of the region and of the renderer-wide record together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Sealed {
    pub(crate) at: Use,
    /// `min(cursor, rows)`: the entries below it are read.
    pub(crate) len: u32,
    pub(crate) counts: Counts,
}

/// What a writer left at one index of a sealed region.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Note {
    /// A pass began.
    Begin { kind: WorldKind, tick: Tick, full: bool },
    /// The pass ended.
    End,
    /// The pass removed a key.
    Removed(KeyBits),
    /// A row of a pass or a remote batch.
    Row { key: KeyBits, mesh: AssetRef<MeshAsset> },
    /// A remote batch opened.
    RemoteOpen,
    /// The remote batch was closed.
    RemoteEnd,
}

/// One entry of a sealed region: its index, the index of its pass's `begin` or its
/// batch's opening, and what it holds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Entry {
    pub(crate) index: u32,
    pub(crate) pass: u32,
    pub(crate) note: Note,
}

impl Staging {
    /// The shared state of a ring whose regions' rows are `buffers`, each of `rows` rows,
    /// with region zero open under generation one.
    pub(crate) fn new(device: Arc<dyn Device>, rows: u32, buffers: &[BufferHandle]) -> Self {
        let regions = buffers
            .iter()
            .map(|&buffer| Region {
                buffer,
                cursor: AtomicU64::new(0),
                counts: Record::new(),
                shadow: (0..rows).map(|_| Shadow::new()).collect(),
            })
            .collect();
        Self { device, open: AtomicU64::new(word(1, 0, 0)), rows, regions, wide: Record::new() }
    }

    /// The rows buffer of `region`.
    pub(crate) fn buffer(&self, region: u32) -> BufferHandle {
        self.regions[region as usize].buffer
    }

    /// Opens `at` to the writers: resets its cursor and counts, then publishes it in the
    /// open word with `Release`. No region is open when it is called.
    pub(crate) fn open(&self, at: Use) {
        let region = &self.regions[at.region as usize];
        region.cursor.store(0, Ordering::Relaxed);
        region.counts.reset();
        debug_assert_eq!(fields(self.open.load(Ordering::Relaxed)).1, NO_REGION);
        self.open.store(word(at.generation, at.region, 0), Ordering::Release);
    }

    /// Seals the open region, which succeeds only while no pass is open on it, with
    /// `Acquire`, so that every note, row and lost mark of every pass that ended or was
    /// dropped is read after it; then takes the renderer-wide record with a swap.
    pub(crate) fn seal(&self) -> Result<Sealed, Unsealed> {
        let mut current = self.open.load(Ordering::Relaxed);
        let at = loop {
            let (generation, region, count) = fields(current);
            if region == NO_REGION {
                return Err(Unsealed::NotOpen);
            }
            if count != 0 {
                return Err(Unsealed::PassOpen);
            }
            let sealed = word(generation, NO_REGION, 0);
            match self.open.compare_exchange(current, sealed, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => break Use { region, generation },
                Err(now) => current = now,
            }
        };
        let region = &self.regions[at.region as usize];
        let cursor = region.cursor.load(Ordering::Relaxed);
        // Below `rows`, which is a `u32`.
        let len = cursor.min(u64::from(self.rows)) as u32;
        let counts = region.counts.read().add(self.wide.take());
        Ok(Sealed { at, len, counts })
    }

    /// The entries of the sealed region that its use's writers left, in index order: the
    /// entries below `sealed.len` of `sealed`'s generation.
    pub(crate) fn entries(&self, sealed: &Sealed) -> impl Iterator<Item = Entry> + '_ {
        let generation = u64::from(sealed.at.generation);
        let shadow = &self.regions[sealed.at.region as usize].shadow[..sealed.len as usize];
        shadow.iter().enumerate().filter_map(move |(index, entry)| {
            let mark = entry.mark.load(Ordering::Relaxed);
            if mark >> 32 != generation {
                return None;
            }
            let note = entry.note.load(Ordering::Relaxed);
            let key =
                || KeyBits::from_words(entry.key.each_ref().map(|w| w.load(Ordering::Relaxed)));
            let note = match note & 0xff {
                BEGIN => Note::Begin {
                    kind: KINDS[(note >> 8 & 3) as usize],
                    tick: Tick((note >> 32) as u32),
                    full: note >> 10 & 1 != 0,
                },
                END => Note::End,
                REMOVED => Note::Removed(key()),
                ROW => Note::Row {
                    key: key(),
                    mesh: AssetRef::new(AssetId::from_bits(entry.mesh.load(Ordering::Relaxed))),
                },
                REMOTE_OPEN => Note::RemoteOpen,
                REMOTE_END => Note::RemoteEnd,
                other => unreachable!("a shadow entry of note {other}"),
            };
            // A region holds at most `u32::MAX` rows.
            Some(Entry { index: index as u32, pass: mark as u32, note })
        })
    }

    /// Reserves `count` rows of `at` with one `Relaxed` addition to its cursor: the
    /// first reserved, and how many of them lie in the region.
    fn reserve(&self, at: Use, count: u64) -> (u32, u32) {
        let start = self.regions[at.region as usize].cursor.fetch_add(count, Ordering::Relaxed);
        let rows = u64::from(self.rows);
        if start >= rows {
            return (self.rows, 0);
        }
        // Both below `rows`.
        (start as u32, (rows - start).min(count) as u32)
    }

    /// Stores a shadow entry of `at` at `index`: `Relaxed`, since what makes it visible to
    /// `prepare` is the release of its pass's `end`, or the borrow a remote batch holds.
    fn store(&self, at: Use, index: u32, pass: u32, note: u64, key: [u64; 2], mesh: u64) {
        let entry = &self.regions[at.region as usize].shadow[index as usize];
        entry.mark.store(u64::from(at.generation) << 32 | u64::from(pass), Ordering::Relaxed);
        entry.note.store(note, Ordering::Relaxed);
        entry.key[0].store(key[0], Ordering::Relaxed);
        entry.key[1].store(key[1], Ordering::Relaxed);
        entry.mesh.store(mesh, Ordering::Relaxed);
    }

    /// Leaves one note of `pass` in `at`: whether it found room.
    fn note(&self, at: Use, pass: Option<u32>, note: u64) -> Option<u32> {
        let (index, fit) = self.reserve(at, 1);
        if fit == 0 {
            return None;
        }
        self.store(at, index, pass.unwrap_or(index), note, [0; 2], 0);
        Some(index)
    }

    /// Writes `rows` of `pass` into `at`: one reservation, one `write_buffer` over the part
    /// that lies in the region, then each written row's shadow entry. Returns how many
    /// rows it dropped.
    fn write(&self, at: Use, pass: u32, rows: &[InstanceRow]) -> u64 {
        let count = rows.len() as u64;
        let (start, fit) = self.reserve(at, count);
        if fit == 0 {
            return count;
        }
        let written = &rows[..fit as usize];
        let stride = u64::from(StagingRow::BYTES);
        let mut fill = |bytes: &mut [u8]| {
            let (chunks, _) = bytes.as_chunks_mut::<{ StagingRow::BYTES as usize }>();
            for (out, row) in chunks.iter_mut().zip(written) {
                *out = StagingRow::of(row).to_bytes();
            }
        };
        let buffer = self.regions[at.region as usize].buffer;
        let len = u64::from(fit) * stride;
        if self.device.write_buffer(buffer, u64::from(start) * stride, len, &mut fill).is_err() {
            return count;
        }
        for (index, row) in (start..).zip(written) {
            let key = KeyBits::of(row.key).words();
            self.store(at, index, pass, ROW, key, row.mesh.id().to_bits());
        }
        count - u64::from(fit)
    }

    /// Gives back the count a `begin` took, after every entry and lost mark of its pass.
    fn give_back(&self) {
        self.open.fetch_sub(1, Ordering::Release);
    }
}

/// The pass a sink has open.
#[derive(Clone, Copy, Debug)]
struct OpenPass {
    /// The pass's kind.
    kind: WorldKind,
    /// The use its `begin` took a count on, or `None` for a pass lost at its `begin`,
    /// which took none and never touches a region.
    at: Option<Use>,
    /// The index of its `begin` note.
    id: u32,
    /// Whether the pass is lost, and its kind with it.
    lost: bool,
}

/// The renderer's `InstanceSink`: what extract writes one world's instances through.
///
/// Made by [`Renderer::sink`](crate::Renderer::sink) and installed with
/// `ExtractState::set_sink`. A pass's kind is its `ExtractPass::kind`, so one renderer's
/// sinks serve every world a client shows, each world holding its own. The sink meets
/// `crates/present-api/README.md`, "What a pass gives the sink" and "What the sink keeps":
/// what it keeps is kept on the render side, in the slot table and in the instance
/// buffer, from what each pass leaves in the region (`README.md`, "Writers").
pub struct ExtractSink {
    /// What the sink shares with the renderer.
    staging: Arc<Staging>,
    /// The pass that is open, if one is.
    pass: Option<OpenPass>,
}

impl ExtractSink {
    /// A sink that writes through `staging`. The renderer's plumbing.
    pub(crate) const fn new(staging: Arc<Staging>) -> Self {
        Self { staging, pass: None }
    }

    /// Leaves the open pass lost, giving back its count where it took one: what a drop
    /// with a pass open does, and a `begin` over an open pass, which present-api's
    /// bracketing rules out.
    fn abandon(&mut self) {
        let Some(pass) = self.pass.take() else { return };
        if let Some(at) = pass.at {
            self.staging.regions[at.region as usize].counts.lose(pass.kind);
            self.staging.give_back();
        }
    }
}

impl InstanceSink for ExtractSink {
    /// Opens a pass on the open region and leaves the pass's kind, tick and fullness in
    /// it. Waits for nothing. With no region open, or no room in it, the pass is *lost*
    /// (`README.md`, "Writers").
    fn begin(&mut self, pass: ExtractPass) {
        self.abandon();
        let staging = &*self.staging;
        let kind = pass.kind;
        let lost = Some(OpenPass { kind, at: None, id: 0, lost: true });
        let mut current = staging.open.load(Ordering::Relaxed);
        let at = loop {
            let (generation, region, count) = fields(current);
            if region == NO_REGION || count == COUNT_MAX {
                staging.wide.lose(kind);
                self.pass = lost;
                return;
            }
            let taken = current + 1;
            match staging.open.compare_exchange(
                current,
                taken,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => break Use { region, generation },
                Err(now) => current = now,
            }
        };
        let note = BEGIN
            | (kind_index(kind) as u64) << 8
            | u64::from(pass.full) << 10
            | u64::from(pass.tick.0) << 32;
        match staging.note(at, None, note) {
            Some(id) => self.pass = Some(OpenPass { kind, at: Some(at), id, lost: false }),
            None => {
                staging.regions[at.region as usize].counts.lose(kind);
                staging.give_back();
                self.pass = lost;
            }
        }
    }

    /// Leaves each key in the region, for the slot table to free. Each key takes the room
    /// of one row; a key that does not fit makes the pass lost.
    fn removed(&mut self, keys: &[StableKey]) {
        let staging = &*self.staging;
        let Some(pass) = &mut self.pass else { return };
        let Some(at) = pass.at.filter(|_| !pass.lost && !keys.is_empty()) else { return };
        let (start, fit) = staging.reserve(at, keys.len() as u64);
        for (index, key) in (start..).zip(&keys[..fit as usize]) {
            staging.store(at, index, pass.id, REMOVED, KeyBits::of(*key).words(), 0);
        }
        if (fit as usize) < keys.len() {
            staging.regions[at.region as usize].counts.lose(pass.kind);
            pass.lost = true;
        }
    }

    /// Writes `rows` into the region: reserves their rows with one atomic addition,
    /// writes them as [`StagingRow`](crate::StagingRow)s through one
    /// `Device::write_buffer` call over the part of the range it reserved that lies in
    /// the region, and leaves each row's key and mesh beside it, in atomics, under the
    /// region and generation the pass's `begin` read. Called from extract's forked jobs,
    /// several at once, so it reads the sink's pass and never changes it. It waits for
    /// nothing and takes no lock but the device backend's on its records, which
    /// architecture section 3 permits (D158). Rows that do not fit are dropped and
    /// counted.
    fn write(&self, rows: &[InstanceRow]) {
        let staging = &*self.staging;
        let count = rows.len() as u64;
        match self.pass {
            None => staging.wide.drop_rows(count, None),
            Some(OpenPass { kind, at: None, .. }) => staging.wide.drop_rows(count, Some(kind)),
            Some(OpenPass { kind, at: Some(at), id, lost }) => {
                let dropped = if lost { count } else { staging.write(at, id, rows) };
                staging.regions[at.region as usize].counts.drop_rows(dropped, Some(kind));
            }
        }
    }

    /// Closes the pass, and gives back the count its `begin` took, where it took one.
    /// Nothing of the pass reaches the slot table or the device before the frame's
    /// [`Renderer::prepare`](crate::Renderer::prepare).
    fn end(&mut self) {
        let staging = &*self.staging;
        let Some(pass) = self.pass.take() else { return };
        let Some(at) = pass.at else { return };
        if !pass.lost && staging.note(at, Some(pass.id), END).is_none() {
            staging.regions[at.region as usize].counts.lose(pass.kind);
        }
        staging.give_back();
    }
}

impl Drop for ExtractSink {
    /// With a pass open, leaves it lost and, where its `begin` took a count, gives the
    /// count back with `end`'s `Release` subtraction, after the lost mark, so that the
    /// region can be sealed and the `prepare` that seals it sees the loss: it applies
    /// nothing of the pass's kind and frees the kind's slots, as for any lost pass
    /// (`README.md`, "Writers"). A pass lost at its `begin` took no count and gives none
    /// back. With none open, does nothing.
    fn drop(&mut self) {
        self.abandon();
    }
}

/// The remote upload's writer (architecture section 5.3): what the presentation schedule
/// writes a frame's remote instances through, after it has sampled the interpolation
/// buffer and converted to render space.
///
/// Made by [`Renderer::remote`](crate::Renderer::remote), once a frame, after the frame's
/// `Client` pass has ended. It borrows the renderer for as long as it lives, so no
/// [`prepare`](crate::Renderer::prepare) runs beside it, and the batch takes no count of
/// the open word; dropping it closes the batch with its end note. A batch whose end note
/// is missing, as after `mem::forget`, is left out whole. Of the batches a region holds,
/// `prepare` applies the last alone, and only where no note of a `Client` pass lies after
/// its opening and the `Client` kind is not lost. A batch that is not applied, whichever
/// the reason, counts its written rows in `FrameStats::remote_rows` alone (`README.md`,
/// "Slots").
pub struct RemoteWriter<'r> {
    /// What the writer shares with the sinks.
    staging: &'r Staging,
    /// The region open when the writer was made, if one was.
    at: Option<Use>,
    /// The index of the batch's opening note, if it found room.
    opening: Option<u32>,
    /// The renderer is borrowed mutably, though only the shared state is reached.
    exclusive: PhantomData<&'r mut ()>,
}

impl<'r> RemoteWriter<'r> {
    /// A writer on `staging`'s open region, with its batch opened. The renderer's
    /// plumbing.
    pub(crate) fn new(staging: &'r Staging) -> Self {
        let (generation, region, _) = fields(staging.open.load(Ordering::Acquire));
        let at = (region != NO_REGION).then_some(Use { region, generation });
        let opening = at.and_then(|at| staging.note(at, None, REMOTE_OPEN));
        Self { staging, at, opening, exclusive: PhantomData }
    }

    /// Writes `rows` into the open region, as [`ExtractSink`]'s `write` does and under
    /// the same rules: callable from several jobs at once through `&self`, waiting for
    /// nothing, dropping and counting what does not fit.
    ///
    /// Each row is an instance of the client's replicated world, `WorldKind::Client`,
    /// under its key. The scatter sets both of its placements to the row's, whatever
    /// `no_interpolate` says: the presentation schedule sampled the row at the frame's
    /// time, so nothing is drawn between two placements of it.
    pub fn write(&self, rows: &[InstanceRow]) {
        let count = rows.len() as u64;
        match (self.at, self.opening) {
            (None, _) => self.staging.wide.drop_rows(count, None),
            (Some(at), None) => {
                self.staging.regions[at.region as usize].counts.drop_rows(count, None)
            }
            (Some(at), Some(opening)) => {
                let dropped = self.staging.write(at, opening, rows);
                self.staging.regions[at.region as usize].counts.drop_rows(dropped, None);
            }
        }
    }
}

impl Drop for RemoteWriter<'_> {
    /// Closes the batch: leaves its end note in the region, or, with no room for it,
    /// leaves the batch out whole.
    fn drop(&mut self) {
        if let (Some(at), Some(opening)) = (self.at, self.opening) {
            self.staging.note(at, Some(opening), REMOTE_END);
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use std::sync::Arc;

    use lactose_core::NetId;
    use lactose_present_api::RenderTransform;
    use lactose_rhi_api::{
        BufferDesc, BufferUsage, Capabilities, DeviceRequest, MemoryClass, Message, MessageSink,
        Sharing,
    };
    use lactose_rhi_null::NullDevice;

    use super::*;

    struct Silent;

    impl MessageSink for Silent {
        fn message(&self, _: &Message<'_>) {}
    }

    /// A ring of `regions` regions of `rows` rows on a null device.
    fn ring(regions: u32, rows: u32) -> Arc<Staging> {
        let request = DeviceRequest {
            adapter: None,
            surface: None,
            heap_slots: 0,
            samplers: 0,
            pipeline_cache: Vec::new(),
            messages: Arc::new(Silent),
        };
        let device: Arc<dyn Device> =
            Arc::new(NullDevice::new(request, Capabilities::LEAST).expect("a null device"));
        let buffers: Vec<BufferHandle> = (0..regions)
            .map(|_| {
                let desc = BufferDesc {
                    label: "rows",
                    size: u64::from(rows) * u64::from(StagingRow::BYTES),
                    usage: BufferUsage::DEVICE_ADDRESS,
                    memory: MemoryClass::Upload,
                    sharing: Sharing::Exclusive,
                };
                device.create_buffer(&desc).expect("a buffer")
            })
            .collect();
        Arc::new(Staging::new(device, rows, &buffers))
    }

    fn key(n: u64) -> StableKey {
        StableKey::Net(NetId::from_bits(n).expect("a nonzero id"))
    }

    fn row(n: u64) -> InstanceRow {
        InstanceRow {
            key: key(n),
            mesh: AssetRef::new(AssetId::from_bits(n * 3)),
            transform: RenderTransform {
                position: lactose_math::render::RenderPosition::default(),
                rotation: [0.0, 0.0, 0.0, 1.0],
            },
            no_interpolate: false,
        }
    }

    fn pass(kind: WorldKind, tick: u32, full: bool) -> ExtractPass {
        ExtractPass { kind, tick: Tick(tick), full }
    }

    fn notes(staging: &Staging, sealed: &Sealed) -> Vec<(u32, u32, Note)> {
        staging.entries(sealed).map(|entry| (entry.index, entry.pass, entry.note)).collect()
    }

    fn row_note(n: u64) -> Note {
        Note::Row { key: KeyBits::of(key(n)), mesh: AssetRef::new(AssetId::from_bits(n * 3)) }
    }

    const fn lost(kind: WorldKind) -> u8 {
        1 << kind_index(kind)
    }

    #[test]
    fn the_open_word_holds_its_three_fields() {
        for (generation, region, count) in [(1, 0, 0), (u32::MAX, NO_REGION, COUNT_MAX), (7, 3, 9)]
        {
            assert_eq!(fields(word(generation, region, count)), (generation, region, count));
        }
        assert_eq!(next_generation(1), 2);
        assert_eq!(next_generation(u32::MAX), 1);
    }

    #[test]
    fn a_pass_is_read_back_whole_in_index_order() {
        let staging = ring(2, 16);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Prediction, 9, true));
        sink.removed(&[key(5)]);
        sink.write(&[row(1), row(2)]);
        sink.end();
        let sealed = staging.seal().expect("no pass is open");
        assert_eq!((sealed.at, sealed.len), (Use { region: 0, generation: 1 }, 5));
        assert_eq!(sealed.counts, Counts::default());
        let begin = Note::Begin { kind: WorldKind::Prediction, tick: Tick(9), full: true };
        assert_eq!(
            notes(&staging, &sealed),
            [
                (0, 0, begin),
                (1, 0, Note::Removed(KeyBits::of(key(5)))),
                (2, 0, row_note(1)),
                (3, 0, row_note(2)),
                (4, 0, Note::End),
            ]
        );
    }

    #[test]
    fn a_sealing_is_refused_while_a_pass_is_open_and_with_no_region_open() {
        let staging = ring(2, 16);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Server, 1, false));
        assert_eq!(staging.seal(), Err(Unsealed::PassOpen));
        sink.end();
        assert!(staging.seal().is_ok());
        assert_eq!(staging.seal(), Err(Unsealed::NotOpen));
    }

    #[test]
    fn a_pass_with_no_region_open_is_lost_and_reported_by_the_next_sealing() {
        let staging = ring(2, 16);
        staging.seal().expect("region zero is open");
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Client, 1, false));
        sink.write(&[row(1), row(2)]);
        sink.end();
        // Opening the next region resets its own counts, not the renderer-wide record.
        staging.open(Use { region: 1, generation: 2 });
        let sealed = staging.seal().expect("region one is open");
        assert_eq!(sealed.counts.lost, lost(WorldKind::Client));
        assert_eq!((sealed.counts.dropped, sealed.counts.dropped_by_kind), (2, [0, 2, 0]));
        assert!(notes(&staging, &sealed).is_empty());
        // The sealing took the record.
        staging.open(Use { region: 0, generation: 3 });
        assert_eq!(staging.seal().expect("open").counts, Counts::default());
    }

    #[test]
    fn rows_past_the_region_are_dropped_and_counted_by_kind() {
        let staging = ring(2, 4);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Server, 1, false));
        sink.write(&[row(1), row(2), row(3), row(4)]);
        sink.end();
        let sealed = staging.seal().expect("open");
        // The begin and three rows fit; the fourth row and the end do not.
        assert_eq!(sealed.len, 4);
        assert_eq!(sealed.counts.dropped_by_kind, [1, 0, 0]);
        assert_eq!(sealed.counts.lost, lost(WorldKind::Server));
        assert_eq!(notes(&staging, &sealed).len(), 4);
    }

    #[test]
    fn removed_keys_that_do_not_fit_lose_the_pass() {
        let staging = ring(2, 2);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Prediction, 1, false));
        sink.removed(&[key(1), key(2)]);
        // A lost pass writes nothing more.
        sink.write(&[row(3)]);
        sink.end();
        let sealed = staging.seal().expect("open");
        assert_eq!(sealed.counts.lost, lost(WorldKind::Prediction));
        assert_eq!(sealed.counts.dropped_by_kind, [0, 0, 1]);
        let begin = Note::Begin { kind: WorldKind::Prediction, tick: Tick(1), full: false };
        assert_eq!(
            notes(&staging, &sealed),
            [(0, 0, begin), (1, 0, Note::Removed(KeyBits::of(key(1))))]
        );
    }

    #[test]
    fn a_sink_dropped_with_a_pass_open_leaves_it_lost_and_the_region_sealable() {
        let staging = ring(2, 16);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Client, 1, false));
        sink.write(&[row(1)]);
        drop(sink);
        let sealed = staging.seal().expect("the drop gave the count back");
        assert_eq!(sealed.counts.lost, lost(WorldKind::Client));
    }

    #[test]
    fn a_begin_over_an_open_pass_leaves_the_first_lost() {
        let staging = ring(2, 16);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Server, 1, false));
        sink.begin(pass(WorldKind::Prediction, 1, false));
        sink.end();
        let sealed = staging.seal().expect("both counts were given back");
        assert_eq!(sealed.counts.lost, lost(WorldKind::Server));
    }

    #[test]
    fn an_earlier_use_of_a_region_is_never_read_as_a_later_one() {
        let staging = ring(2, 16);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Server, 1, false));
        sink.write(&[row(1), row(2), row(3)]);
        sink.end();
        staging.seal().expect("open");
        staging.open(Use { region: 0, generation: 2 });
        // A row index the later use reserved and never wrote, as a failed device write
        // leaves, holds the earlier use's entry, which the generation leaves out.
        let (start, fit) = staging.reserve(Use { region: 0, generation: 2 }, 2);
        assert_eq!((start, fit), (0, 2));
        sink.begin(pass(WorldKind::Server, 2, false));
        sink.end();
        let sealed = staging.seal().expect("open");
        let begin = Note::Begin { kind: WorldKind::Server, tick: Tick(2), full: false };
        assert_eq!(sealed.len, 4);
        assert_eq!(notes(&staging, &sealed), [(2, 2, begin), (3, 2, Note::End)]);
    }

    #[test]
    fn a_remote_batch_is_opened_written_and_closed() {
        let staging = ring(2, 16);
        {
            let writer = RemoteWriter::new(&staging);
            writer.write(&[row(1), row(2)]);
        }
        let sealed = staging.seal().expect("the batch takes no count");
        assert_eq!(
            notes(&staging, &sealed),
            [
                (0, 0, Note::RemoteOpen),
                (1, 0, row_note(1)),
                (2, 0, row_note(2)),
                (3, 0, Note::RemoteEnd),
            ]
        );
    }

    #[test]
    fn a_remote_batch_drops_what_finds_no_room_and_loses_no_kind() {
        let staging = ring(2, 2);
        staging.seal().expect("open");
        // With no region open, the rows go to the renderer-wide record.
        RemoteWriter::new(&staging).write(&[row(1)]);
        staging.open(Use { region: 1, generation: 2 });
        RemoteWriter::new(&staging).write(&[row(2), row(3)]);
        let sealed = staging.seal().expect("open");
        assert_eq!((sealed.counts.dropped, sealed.counts.dropped_by_kind), (2, [0; 3]));
        assert_eq!(sealed.counts.lost, 0);
        // The opening and one row; the second row and the end found no room.
        assert_eq!(notes(&staging, &sealed), [(0, 0, Note::RemoteOpen), (1, 0, row_note(2))]);
    }

    #[test]
    fn a_count_at_its_width_loses_the_next_begin_in_the_wide_record() {
        let staging = ring(2, 16);
        let (generation, region, _) = fields(staging.open.load(Ordering::Relaxed));
        staging.open.store(word(generation, region, COUNT_MAX), Ordering::Relaxed);
        let mut sink = ExtractSink::new(Arc::clone(&staging));
        sink.begin(pass(WorldKind::Prediction, 1, false));
        sink.write(&[row(1)]);
        sink.end();
        staging.open.store(word(generation, region, 0), Ordering::Relaxed);
        let sealed = staging.seal().expect("open");
        assert_eq!(sealed.counts.lost, lost(WorldKind::Prediction));
        assert_eq!(sealed.counts.dropped_by_kind, [0, 0, 1]);
        assert_eq!(sealed.len, 0);
    }
}
