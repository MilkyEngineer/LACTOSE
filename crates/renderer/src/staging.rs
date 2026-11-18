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
use std::sync::atomic::AtomicU64;

use lactose_core::StableKey;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink};
use lactose_rhi_api::Device;

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
/// compare-exchange of it is `Acquire` on success; a pass's `end` gives back its count
/// with a `Release` subtraction; the sealing's compare-exchange is `Acquire`; and a
/// reservation is a `Relaxed` addition to the cursor. The implementation adds no `Mutex`,
/// `RwLock`, condition variable or channel here.
pub(crate) struct Staging {
    /// The device the writers write through.
    device: Arc<dyn Device>,
    /// Which region is open, its open generation, and how many passes are open on it, in
    /// one word, so that a pass's beginning and the renderer's sealing cannot both
    /// succeed. The remote batch takes no count.
    open: AtomicU64,
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
    pass: Option<ExtractPass>,
}

impl ExtractSink {
    /// A sink that writes through `staging`. The renderer's plumbing.
    pub(crate) const fn new(staging: Arc<Staging>) -> Self {
        Self { staging, pass: None }
    }
}

impl InstanceSink for ExtractSink {
    /// Opens a pass on the open region and leaves the pass's kind, tick and fullness in
    /// it. Waits for nothing. With no region open, or no room in it, the pass is *lost*
    /// (`README.md`, "Writers").
    fn begin(&mut self, pass: ExtractPass) {
        todo!()
    }

    /// Leaves each key in the region, for the slot table to free. Each key takes the room
    /// of one row; a key that does not fit makes the pass lost.
    fn removed(&mut self, keys: &[StableKey]) {
        todo!()
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
        todo!()
    }

    /// Closes the pass. Nothing of the pass reaches the slot table or the device before
    /// the frame's [`Renderer::prepare`](crate::Renderer::prepare).
    fn end(&mut self) {
        todo!()
    }
}

impl Drop for ExtractSink {
    /// With a pass open, leaves it lost and gives back the count its `begin` took, so
    /// that the region can be sealed: the next `prepare` applies nothing of the pass's
    /// kind and frees the kind's slots, as for any lost pass (`README.md`, "Writers").
    /// With none open, does nothing.
    fn drop(&mut self) {
        todo!()
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
/// is missing, as after `mem::forget`, is left out whole and its rows are counted as
/// dropped.
pub struct RemoteWriter<'r> {
    /// What the writer shares with the sinks.
    staging: &'r Staging,
    /// The renderer is borrowed mutably, though only the shared state is reached.
    exclusive: PhantomData<&'r mut ()>,
}

impl<'r> RemoteWriter<'r> {
    /// A writer on `staging`'s open region, with its batch opened. The renderer's
    /// plumbing.
    pub(crate) fn new(staging: &'r Staging) -> Self {
        todo!()
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
        todo!()
    }
}

impl Drop for RemoteWriter<'_> {
    /// Closes the batch: leaves its end note in the region, or, with no room for it,
    /// leaves the batch out whole.
    fn drop(&mut self) {
        todo!()
    }
}
