//! The schedule per frame on one world's snapshot (architecture section 5.3): on the pair the
//! frame acquired, deliver the events, place the camera, sample the remote rows.

use lactose_core::StableKey;
use lactose_ecs::{PresentEmission, PresentEventKey};
use lactose_net_host::InterpolationReader;
use lactose_net_host::interpolation::PresentationTime;
use lactose_present_api::{InstanceRow, PresentFrame};

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

/// What a frame's schedule placed, beside the remote rows [`Presentation::remote`] lends.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Presented {
    /// The camera, if the view has a focus and the newer snapshot holds its transform and
    /// its `CameraRig`.
    pub camera: Option<CameraPose>,
    /// The remote rows left out: their slot held no state and they were not shown in the
    /// previous frame. Zero without a [`RemoteView`].
    pub missing: usize,
}

/// The presentation schedule of one shown world: what the remote sample keeps between
/// frames. It holds no reader: the frame acquires the world's pair and hands it over. A
/// client showing two worlds holds one for each.
#[derive(Debug, Default)]
pub struct Presentation {
    remote: RemoteRows,
}

impl Presentation {
    /// A schedule with no remote rows yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs the schedule once on `frame`, the pair the caller acquired from the world's
    /// `SnapshotReader`: hands its events to `events` (every emission, then every
    /// cancellation, in the batch's order), places the camera on the focus, and samples the
    /// remote rows of the newer snapshot, which [`remote`](Self::remote) then lends. Returns
    /// the camera and the count of remote rows left out.
    ///
    /// The events are those `frame` carries, so the caller gives each pair it acquires to
    /// one call, once: then each event published is delivered once, whatever the cadence of
    /// publications and frames (`lactose-present-api`, "Events and publication"). A pair
    /// given twice delivers its events twice, and a pair acquired and not given drops them.
    /// The remote rows are sampled once a call, each slot once, and a call on the same pair
    /// samples again at the view's time.
    pub fn frame(
        &mut self,
        frame: &PresentFrame<'_>,
        view: &PresentView<'_>,
        events: &mut dyn EventSink,
    ) -> Presented {
        let batch = frame.events();
        for emission in &batch.emitted {
            events.emitted(emission);
        }
        for key in &batch.cancelled {
            events.cancelled(*key);
        }
        let camera = view.focus.and_then(|focus| CameraPose::of(frame, focus, view.fraction));
        match view.remote {
            Some(remote) => self.remote.sample(frame.newer(), remote.buffer, remote.at),
            None => self.remote.clear(),
        }
        Presented { camera, missing: self.remote.missing() }
    }

    /// The remote rows of the last [`frame`](Self::frame): every remote row with a
    /// placement, ascending by key, each key once, what [`upload`](crate::upload) writes.
    /// Empty before the first call and after a call without a [`RemoteView`]. Lent, so the
    /// frame's draw uploads them with no copy.
    #[must_use]
    pub fn remote(&self) -> &[InstanceRow] {
        self.remote.rows()
    }
}
