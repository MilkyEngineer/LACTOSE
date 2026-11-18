//! The schedule per frame on one world's snapshot (architecture section 5.3): acquire the
//! pair, deliver the events, place the camera, sample the remote rows.

use lactose_core::StableKey;
use lactose_ecs::{PresentEmission, PresentEventKey};
use lactose_net_host::InterpolationReader;
use lactose_net_host::interpolation::PresentationTime;
use lactose_present_api::{InstanceRow, PresentFrame, SnapshotReader};

use crate::camera::CameraPose;
use crate::remote::RemoteRows;

/// Where a frame's presentation events go: audio, effects and the debug overlay take them
/// from here. The schedule calls it from the frame's job, once per event and frame.
pub trait EventSink {
    /// An event delivered for the first time. A rollback that re-simulates its tick and
    /// emits it again does not deliver it again.
    fn emitted(&mut self, emission: &PresentEmission);

    /// An event delivered by an earlier frame whose tick was re-simulated without it: what
    /// it started stops.
    fn cancelled(&mut self, key: PresentEventKey);
}

/// What one frame of the schedule is shown at.
#[derive(Clone, Copy, Debug)]
pub struct PresentView<'a> {
    /// The frame's fraction, as `lactose-renderer`'s `FrameView::fraction` (see
    /// [`interpolate`](crate::interpolate)).
    pub fraction: f32,
    /// The row whose `CameraRig` the camera follows, if any.
    pub focus: Option<StableKey>,
    /// The remote rows' source, for a client's replicated world; `None` samples none.
    pub remote: Option<RemoteView<'a>>,
}

/// Where and when a frame samples the remote rows.
#[derive(Clone, Copy, Debug)]
pub struct RemoteView<'a> {
    /// The interpolation buffer the network thread fills.
    pub buffer: &'a InterpolationReader,
    /// The server time to sample at, from the clock estimate.
    pub at: PresentationTime,
}

/// What a frame shows.
#[derive(Clone, Copy, Debug)]
pub struct Shown<'p> {
    /// The pair the frame was run on, for what else the frame reads of it (lights, decals,
    /// the UI's components). Its events are already delivered.
    pub frame: PresentFrame<'p>,
    /// The camera, if the view has a focus and the newer snapshot holds its transform and
    /// its `CameraRig`.
    pub camera: Option<CameraPose>,
    /// Every remote row with a placement this frame, ascending by key, each key once: what
    /// [`upload`](crate::upload) writes. Empty without a [`RemoteView`].
    pub remote: &'p [InstanceRow],
    /// The remote rows left out: their slot held no state and they were not shown in the
    /// previous frame.
    pub missing: usize,
}

/// The presentation schedule of one shown world: its snapshot reader, and what the remote
/// sample keeps between frames. A client showing two worlds holds one for each.
#[derive(Debug)]
pub struct Presentation {
    reader: SnapshotReader,
    remote: RemoteRows,
}

impl Presentation {
    /// The schedule on the snapshots `reader` acquires: the reading half of the world's
    /// `snapshot_channel`, whose writer `app` installs with `ExtractState::set_snapshot`.
    #[must_use]
    pub fn new(reader: SnapshotReader) -> Self {
        Self { reader, remote: RemoteRows::default() }
    }

    /// Runs the schedule once: acquires the newest pair, hands its events to `events`
    /// (every emission, then every cancellation, in the batch's order), places the camera on
    /// the focus, and samples the remote rows of the newer snapshot. Returns `None`, and
    /// delivers nothing, before the world's first publication; its events wait for the
    /// first frame.
    ///
    /// Each call acquires once, so each event published is delivered by exactly one call,
    /// whatever the cadence of publications and calls (`lactose-present-api`, "Events and
    /// publication"). The remote rows are sampled once a call, each slot once, and a call
    /// with no publication since the last samples again at the view's time.
    pub fn frame(
        &mut self,
        view: &PresentView<'_>,
        events: &mut dyn EventSink,
    ) -> Option<Shown<'_>> {
        let frame = self.reader.acquire()?;
        let batch = frame.events();
        for emission in &batch.emitted {
            events.emitted(emission);
        }
        for key in &batch.cancelled {
            events.cancelled(*key);
        }
        let camera = view.focus.and_then(|focus| CameraPose::of(&frame, focus, view.fraction));
        match view.remote {
            Some(remote) => self.remote.sample(frame.newer(), remote.buffer, remote.at),
            None => self.remote.clear(),
        }
        Some(Shown { frame, camera, remote: self.remote.rows(), missing: self.remote.missing() })
    }
}
