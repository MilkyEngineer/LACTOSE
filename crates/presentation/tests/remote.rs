//! The remote sample and upload (architecture section 5.3): every remote row of a client's
//! replicated world, a row with a `RemoteSlot` and a `MeshInstance`, is written once a frame,
//! shown by counts (that it is sampled in one `sample_batch` call is by inspection); a slot
//! with no state shows the row where it was last shown, and one never shown is left out and
//! counted; the upload writes each row in exactly one call from the frame's job, and calls
//! from forked jobs at once.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::common;

use common::{Ignored, Scene, at, in_job, key, mesh, view};
use lactose_core::{StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::{InterpolationSlot, RemoteSlot};
use lactose_net_host::interpolation::{PresentationTime, ReceivedState};
use lactose_net_host::{InterpolationBuffer, InterpolationReader, InterpolationWriter};
use lactose_present_api::{InstanceRow, RenderTransform};
use lactose_presentation::{PresentView, RemoteView, upload};

/// How many remote rows the scene holds: three forked writes' worth.
const REMOTE: u64 = 600;

/// A client world with `REMOTE` remote rows, row `n` in slot `n` moving from x `n` at tick 8
/// to `n + 2` at tick 10; a row with a slot and no mesh, a row with a mesh and no slot, and a
/// remote row whose slot never received a state.
struct Remote {
    scene: Scene,
    writer: InterpolationWriter,
    reader: InterpolationReader,
    slots: Vec<InterpolationSlot>,
}

const NO_MESH: u64 = REMOTE + 1;
const NO_SLOT: u64 = REMOTE + 2;
const NO_STATE: u64 = REMOTE + 3;

fn state(tick: u32, x: f64) -> ReceivedState {
    let position = Vec3::new(Scalar::from_f64(x), Scalar::ZERO, Scalar::ZERO);
    ReceivedState { tick: Tick(tick), teleported: Tick(0), position, rotation: Quat::IDENTITY }
}

fn remote() -> Remote {
    let mut scene = Scene::new(WorldKind::Client, Tick(10), 4);
    let (mut writer, reader) = InterpolationBuffer::split(1024);
    let mut slots = Vec::new();
    for n in 1..=NO_STATE {
        let slot = writer.allocate().unwrap();
        slots.push(slot);
        let world = scene.world_mut();
        match n {
            NO_MESH => world.spawn(key(n), common::REGION, (RemoteSlot { slot },)).unwrap(),
            NO_SLOT => world.spawn(key(n), common::REGION, (mesh(n),)).unwrap(),
            NO_STATE => {
                world.spawn(key(n), common::REGION, (RemoteSlot { slot }, mesh(n))).unwrap()
            }
            _ => {
                world.spawn(key(n), common::REGION, (RemoteSlot { slot }, mesh(n))).unwrap();
                writer.push(slot, state(8, n as f64)).unwrap();
                writer.push(slot, state(10, n as f64 + 2.0)).unwrap();
            }
        }
    }
    scene.publish();
    Remote { scene, writer, reader, slots }
}

impl Remote {
    /// The schedule's frame at `tick` and `fraction` of server time: its remote rows and how
    /// many it left out.
    fn frame(&mut self, tick: u32, fraction: u16) -> (Vec<InstanceRow>, usize) {
        let at = PresentationTime { tick: Tick(tick), fraction };
        let remote = RemoteView { buffer: &self.reader, at };
        let view = PresentView { remote: Some(remote), ..view(0.5) };
        let shown = self.scene.frame(&view, &mut Ignored).unwrap();
        (shown.remote.to_vec(), shown.missing)
    }

    /// A frame with no remote view.
    fn frame_without_remote(&mut self) -> (Vec<InstanceRow>, usize) {
        let shown = self.scene.frame(&view(0.5), &mut Ignored).unwrap();
        (shown.remote.to_vec(), shown.missing)
    }
}

fn placed(x: f64) -> RenderTransform {
    RenderTransform::from_sim(&at(x))
}

/// Every key once, ascending, each with its mesh and the remote rows alone.
fn assert_each_once(rows: &[InstanceRow], keys: &[StableKey]) {
    let written: Vec<StableKey> = rows.iter().map(|row| row.key).collect();
    assert_eq!(written, keys);
    for row in rows {
        let n = (1..=REMOTE).find(|&n| key(n) == row.key).expect("a remote row");
        assert_eq!(row.mesh, mesh(n).mesh);
        assert!(row.no_interpolate, "nothing is drawn between two samples");
    }
}

/// Row by row: each remote row is sampled at the frame's time once a frame, whether or not a
/// snapshot was published since; a released slot keeps the row where it was shown; a despawned
/// row is gone; a frame with no remote view has no remote row.
#[test]
fn each_remote_row_is_sampled_once_a_frame() {
    let mut remote = remote();
    let keys: Vec<StableKey> = (1..=REMOTE).map(key).collect();

    // Tick 9: halfway from tick 8's state to tick 10's.
    let (rows, missing) = remote.frame(9, 0);
    assert_each_once(&rows, &keys);
    assert_eq!(missing, 1, "the slot that never received a state");
    for (n, row) in (1..=REMOTE).zip(&rows) {
        assert_eq!(row.transform, placed(n as f64 + 1.0));
    }

    // No publication since: the same pair, sampled again at a later time.
    remote.writer.release(remote.slots[1]).unwrap();
    let (rows, missing) = remote.frame(9, 1 << 15);
    assert_each_once(&rows, &keys);
    assert_eq!(missing, 1);
    assert_eq!(rows[0].transform, placed(2.5));
    assert_eq!(rows[1].transform, placed(3.0), "released: shown where it was");
    assert_eq!(rows[2].transform, placed(4.5));

    // Past the newest state: held there. The released slot stays where it was shown.
    let (rows, _) = remote.frame(30, 0);
    assert_eq!(rows[0].transform, placed(3.0));
    assert_eq!(rows[1].transform, placed(3.0));

    // A despawned row is not written.
    remote.scene.world_mut().despawn(key(3)).unwrap();
    remote.scene.publish();
    let (rows, missing) = remote.frame(30, 0);
    let mut kept = keys.clone();
    kept.remove(2);
    assert_each_once(&rows, &kept);
    assert_eq!(missing, 1);

    let (rows, missing) = remote.frame_without_remote();
    assert!(rows.is_empty());
    assert_eq!(missing, 0);
    // With the placements dropped, a released slot has nothing to keep.
    let (rows, missing) = remote.frame(30, 0);
    assert_eq!(rows.len(), kept.len() - 1);
    assert!(rows.iter().all(|row| row.key != key(2)));
    assert_eq!(missing, 2);
}

/// What an upload wrote: each call's rows.
#[derive(Default)]
struct Calls(Mutex<Vec<Vec<StableKey>>>);

/// The upload of a frame's rows writes each in exactly one call, no call empty and none over
/// 256 rows, from forked jobs where they do not fit one; the calls together are the frame's
/// rows.
#[test]
fn the_upload_writes_each_row_in_one_call() {
    let mut remote = remote();
    let (rows, _) = remote.frame(9, 0);
    let executor = remote.scene.executor.clone();
    for count in [0, 1, 256, 257, REMOTE as usize] {
        let rows: Arc<[InstanceRow]> = rows[..count].into();
        let calls = Arc::new(Calls::default());
        let (job_rows, job_calls) = (Arc::clone(&rows), Arc::clone(&calls));
        in_job(&executor, move |ctx| {
            upload(ctx, &job_rows, |batch| {
                let keys = batch.iter().map(|row| row.key).collect();
                job_calls.0.lock().expect("unpoisoned").push(keys);
            });
        });
        let calls = calls.0.lock().expect("unpoisoned");
        assert_eq!(calls.len(), count.div_ceil(256), "{count} rows");
        assert!(calls.iter().all(|call| !call.is_empty() && call.len() <= 256));
        let mut written: Vec<StableKey> = calls.iter().flatten().copied().collect();
        written.sort_unstable();
        let expected: Vec<StableKey> = rows.iter().map(|row| row.key).collect();
        assert_eq!(written, expected, "{count} rows");
    }
}

/// How long a call of the overlap test waits for a second call to begin.
const OVERLAP_WAIT: Duration = Duration::from_secs(10);

/// More rows than two calls take are written from forked jobs at once: on an executor of four
/// frame workers, each call of 600 rows' upload counts itself in, then waits, for at most
/// `OVERLAP_WAIT`, until two calls have been in progress at once. They have been, which
/// an upload that writes its batches one after another never has; every row is still written
/// once.
#[test]
fn the_upload_writes_from_jobs_at_once() {
    let mut remote = remote();
    let (rows, _) = remote.frame(9, 0);
    assert!(rows.len() > 2 * 256);
    let executor = remote.scene.executor.clone();
    let rows: Arc<[InstanceRow]> = rows.into();
    // Calls begun, calls in progress, the most in progress at once, rows written.
    let counts = Arc::new([const { AtomicUsize::new(0) }; 4]);
    let (job_rows, job_counts) = (Arc::clone(&rows), Arc::clone(&counts));
    in_job(&executor, move |ctx| {
        upload(ctx, &job_rows, |batch| {
            let [begun, active, most, written] = &*job_counts;
            begun.fetch_add(1, Ordering::SeqCst);
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            let started = Instant::now();
            // A call stays in progress until two have been at once, so the second to begin
            // counts the first.
            while most.load(Ordering::SeqCst) < 2 && started.elapsed() < OVERLAP_WAIT {
                std::thread::yield_now();
            }
            written.fetch_add(batch.len(), Ordering::SeqCst);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    });
    let [begun, _, most, written] = &*counts;
    assert_eq!(begun.load(Ordering::SeqCst), rows.len().div_ceil(256));
    assert_eq!(written.load(Ordering::SeqCst), rows.len());
    let most = most.load(Ordering::SeqCst);
    assert!(most >= 2, "no two calls were in progress at once within {OVERLAP_WAIT:?}: {most}");
}
