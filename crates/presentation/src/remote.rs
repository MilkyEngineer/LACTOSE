//! The remote entities of a client's replicated world (architecture section 5.3): every
//! row with a `RemoteSlot` and a `MeshInstance` sampled from the interpolation buffer in one
//! batch call, converted to render space, and written through the renderer's ring from the
//! frame's job.

use lactose_core::StableKey;
use lactose_net_api::{InterpolationSlot, RemoteSlot};
use lactose_net_host::InterpolationReader;
use lactose_net_host::interpolation::{PresentationTime, RemoteSample, SampleStatus};
use lactose_physics_api::Transform;
use lactose_present_api::{InstanceRow, MeshInstance, PresentSnapshot, RenderTransform};
use lactose_task::JobContext;

/// The rows one forked write takes: as `lactose-renderer`'s consumer sketch forks them.
const WRITE_ROWS: usize = 256;

/// What the remote sample keeps between frames, so that a steady frame allocates nothing: the
/// slots and the samples of the batch call, and the rows of this frame and the last.
#[derive(Debug, Default)]
pub(crate) struct RemoteRows {
    slots: Vec<InterpolationSlot>,
    samples: Vec<RemoteSample>,
    /// This frame's rows, ascending by key, each key once.
    rows: Vec<InstanceRow>,
    /// The last frame's, ascending by key: a slot that samples as missing is shown where it
    /// was.
    last: Vec<InstanceRow>,
    /// This frame's rows whose slot held no state and that had no placement to keep.
    missing: usize,
}

impl RemoteRows {
    /// Samples every row of `snapshot` that has a `RemoteSlot` and a `MeshInstance` at `at`,
    /// in one `sample_batch` call, and makes this frame's rows of them: the sample's
    /// placement, or where the key was shown last frame if its slot holds no state. A key
    /// that has neither is left out and counted in [`missing`](Self::missing).
    pub(crate) fn sample(
        &mut self,
        snapshot: &PresentSnapshot,
        buffer: &InterpolationReader,
        at: PresentationTime,
    ) {
        core::mem::swap(&mut self.rows, &mut self.last);
        self.rows.clear();
        self.slots.clear();
        // Both iterators are in ascending key order: a merge, in one pass.
        let mut meshes = snapshot.rows::<MeshInstance>().peekable();
        for (key, remote) in snapshot.rows::<RemoteSlot>() {
            while meshes.next_if(|&(mesh_key, _)| mesh_key < key).is_some() {}
            if let Some((_, mesh)) = meshes.next_if(|&(mesh_key, _)| mesh_key == key) {
                self.slots.push(remote.slot);
                self.rows.push(InstanceRow {
                    key,
                    mesh: mesh.mesh,
                    transform: RenderTransform::from_sim(&Transform::IDENTITY),
                    no_interpolate: true,
                });
            }
        }
        self.samples.clear();
        self.samples.resize(self.slots.len(), RemoteSample::MISSING);
        buffer.sample_batch(&self.slots, at, &mut self.samples);

        let last = &self.last;
        let mut index = 0;
        self.rows.retain_mut(|row| {
            let sample = self.samples[index];
            index += 1;
            if sample.status != SampleStatus::Missing {
                let placed = Transform::new(sample.position, sample.rotation);
                row.transform = RenderTransform::from_sim(&placed);
                return true;
            }
            match shown_last(last, row.key) {
                Some(transform) => {
                    row.transform = transform;
                    true
                }
                None => false,
            }
        });
        self.missing = self.slots.len() - self.rows.len();
    }

    /// This frame's rows.
    pub(crate) fn rows(&self) -> &[InstanceRow] {
        &self.rows
    }

    /// How many of this frame's remote rows were left out: their slot held no state, and they
    /// were not shown last frame.
    pub(crate) const fn missing(&self) -> usize {
        self.missing
    }

    /// No row this frame, and none to keep: for a frame that samples nothing.
    pub(crate) fn clear(&mut self) {
        self.rows.clear();
        self.last.clear();
        self.missing = 0;
    }
}

/// Where `key` was shown in `last`, which is ascending by key.
fn shown_last(last: &[InstanceRow], key: StableKey) -> Option<RenderTransform> {
    let index = last.binary_search_by_key(&key, |row| row.key).ok()?;
    Some(last[index].transform)
}

/// Writes `rows` through `write` from the job `ctx` runs: in forked jobs of up to 256 rows
/// each, or in the calling job when they fit in one, and returns once every write has
/// returned. Each row is written once, in exactly one call, and no call is empty.
///
/// `write` is the renderer's ring: `RemoteWriter::write`, which may be called from several
/// jobs at once and waits for nothing (`lactose-renderer`, "Concurrency"). The writer is made
/// by `Renderer::remote` once the frame's `Client` pass has ended, and its drop closes the
/// frame's one remote batch; both are the frame's to order.
///
/// The calls come in no stated order and from no stated thread, the calling job's among
/// them, so `write` must not need anything the calling job holds across `upload`. A panic in
/// `write` aborts the process, as any job's does.
///
/// ```no_run
/// use lactose_present_api::InstanceRow;
/// use lactose_renderer::Renderer;
/// use lactose_task::JobContext;
///
/// fn remote_batch(renderer: &mut Renderer, ctx: &mut JobContext<'_>, rows: &[InstanceRow]) {
///     let writer = renderer.remote();
///     lactose_presentation::upload(ctx, rows, |batch| writer.write(batch));
/// }
/// ```
pub fn upload<W>(ctx: &mut JobContext<'_>, rows: &[InstanceRow], write: W)
where
    W: Fn(&[InstanceRow]) + Sync,
{
    if rows.len() <= WRITE_ROWS {
        if !rows.is_empty() {
            write(rows);
        }
        return;
    }
    let write = &write;
    ctx.scope(|scope| {
        for batch in rows.chunks(WRITE_ROWS) {
            scope.fork(move |_| write(batch));
        }
    });
}
