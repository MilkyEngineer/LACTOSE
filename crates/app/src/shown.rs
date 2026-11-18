//! The shown frame (architecture sections 5.1 and 5.3; the milestone 2 plan's decisions
//! 5.1, 5.6 and 5.11; D162): a world that is drawn, the render side `lactose-app` drives
//! beside it through two `dyn` objects the binary builds, and the protocol by which a
//! finished target reaches the present thread. Nothing here names a backend crate: the
//! traits name `lactose-rhi-api`, `lactose-present-api`, `lactose-ecs` and `lactose-task`
//! alone, which is what keeps the server's denylist satisfiable.
//!
//! **Status: facade.** Every signature is final and every body that decides anything is
//! `todo!()`; `frame-impl` writes them. `README.md`, "The shown frame", is the contract.

#![allow(
    dead_code,
    unused_variables,
    reason = "facade: `frame-impl` writes every body and removes this with the last `todo!()`"
)]

use core::any::Any;
use core::fmt;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::time::Instant;

use lactose_core::{Error, Result, Tick};
use lactose_ecs::{Executor, WorldKind};
use lactose_present_api::{InstanceSink, PresentFrame, SnapshotReader};
use lactose_rhi_api::{Extent, ImageHandle, OFFSCREEN_TARGET_IMAGES_MIN, QueuePath, TimelinePoint};
use lactose_sim::TickInput;
use lactose_task::{JobContext, Lane, LaneCall, Task};

use crate::game::Game;
use crate::policy::ErrorPolicy;
use crate::save::Cooked;
use crate::server::{FrameOutput, Server, StepOutput};
use crate::world::{Start, WorldConfig};

/// How many offscreen targets a shown frame draws into: `OFFSCREEN_TARGET_IMAGES_MIN`, and
/// no more, because the protocol can use no third. A finished target reaches the present
/// thread only in the call queued at the submit that finished it, so a target that no call
/// names is never read again, and one target beside the one a call in flight names is all
/// a frame can need (`README.md`, "The targets and the present thread").
pub const TARGETS: usize = OFFSCREEN_TARGET_IMAGES_MIN as usize;

/// The physics reservation of a shown world: how many frame-critical workers stay awake
/// while the tick's physics step blocks on the solver pool, which is what a frame's record
/// has left during the overlap (architecture sections 5.3 and 16). One until `record-stall`
/// reports at one and at two (the milestone 2 plan, decision 5.16); a follow-up commit
/// changes this constant and nothing else if the file shows it wrong.
pub const RECORD_RESERVATION: u32 = 1;

/// One of the frame's offscreen targets, by its index below [`TARGETS`]. The render side
/// makes the image of each index and makes it again when the frame's extent changes; the
/// present side learns the image from the call that names it ([`Finished::image`]). The
/// index is what `lactose-app` keeps its protocol by, and what either side may key a cache
/// by.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TargetIndex(u8);

impl TargetIndex {
    /// The target of `index`, or `None` at [`TARGETS`] and above.
    #[must_use]
    pub const fn new(index: usize) -> Option<Self> {
        if index < TARGETS { Some(Self(index as u8)) } else { None }
    }

    /// The index, below [`TARGETS`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Whether the render side has a region open to extract's writers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    /// A region is open: the passes of the next frame may write.
    Ready,
    /// None is free until this point is reached. The frame does not wait for it: it asks
    /// again, in a job, at its next frame.
    Wait(TimelinePoint),
}

/// When a frame presents: the times `lactose-app` computes and nothing else does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameTime {
    /// The `now` of the frame's call. Presentation runs on this, whatever the world's
    /// time scale (architecture section 11); the frame reads no clock of its own.
    pub now: Instant,
    /// How far `now` is past the deadline of the newest tick, as a share of the tick
    /// period at the world's time scale, in `[0, 1]`: zero at the deadline, one a period
    /// later and from then on, and one while the world is paused. How far that is between
    /// the two ticks of a pair that spans several is presentation's to compute from the
    /// snapshots' ticks (`crates/present-api/README.md`, "The snapshot").
    pub fraction: f32,
}

/// What a frame draws into.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Draw {
    /// The target: one no present call in flight names.
    pub target: TargetIndex,
    /// The frame's one extent, in pixels and never zero in either dimension: the window's
    /// when the frame began. The target's image has this extent: where the image the
    /// render side last drew at this index has another, it makes a new one and keeps the
    /// old until `ready` and the point of the frame that last drew it are both reached
    /// and the device takes its destroy, which it refuses while the present side's heap
    /// slot still holds the image (`README.md`, "The targets and the present thread").
    pub extent: Extent,
    /// The point the present thread signalled when its last read of the image last drawn
    /// at this index was done, or `None` if the present thread never read it. Every submit
    /// of the frame that uses that image waits for it on the device: the render side
    /// imports the image from `Undefined` with this as the import's `ready`.
    pub ready: Option<TimelinePoint>,
}

/// What a frame's prepare left of one world's instances undrawn, as `lactose-renderer`
/// counts them (`FrameStats::lost`, `dropped_in`, `unplaced_in`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct KindCounts {
    /// A pass of this world began with no region open, and its rows were lost.
    pub lost: bool,
    /// Rows the open region had no room for.
    pub dropped: u32,
    /// Instances the slot table had no slot for.
    pub unplaced: u32,
}

impl KindCounts {
    /// Whether instances of this world are missing until a full pass of it.
    #[must_use]
    pub const fn stale(&self) -> bool {
        self.lost || self.dropped != 0 || self.unplaced != 0
    }
}

/// A drawn frame's counts, by the kind of the world whose passes they are of.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FrameCounts {
    /// Of a `Server` world's passes: milestone 2's local world.
    pub server: KindCounts,
    /// Of a `Client` world's passes.
    pub client: KindCounts,
    /// Of a prediction world's passes.
    pub prediction: KindCounts,
}

impl FrameCounts {
    /// The counts of `kind`'s passes.
    #[must_use]
    pub const fn of(&self, kind: WorldKind) -> KindCounts {
        match kind {
            WorldKind::Server => self.server,
            WorldKind::Client => self.client,
            WorldKind::Prediction => self.prediction,
        }
    }
}

/// The render side of a shown world, as `lactose-app` drives it: one object the binary
/// builds over the device, the render graph, the renderer and the presentation schedule
/// (decision 5.11). `lactose-app` owns it from [`build_shown`] to [`Shown::stop`] and moves
/// it by value between the one owner it has at a time: the task that drives the frame
/// between frames, a frame's render task from its spawn to its join, and inside either the
/// one job that task is running (`README.md`, "The render side"). No method is called
/// while another is and the object is never shared, so `Send` is all that is asked of it:
/// an implementor may hold a `Cell`, and needs no lock.
/// `Any` is how the binary takes its own object back as its own type from
/// [`Stopped::render`]: the fixed-frame mode reads the last target and its dump from it.
///
/// A device queue is reached from [`open`](Self::open) and [`submit`](Self::submit) and
/// from no other method. Where the present role shares the graphics queue
/// ([`present_queue`](Self::present_queue)) neither is called while a present call is in
/// flight, so no frame-critical worker takes the queue's lock behind a present's driver
/// call (`README.md`, "The targets and the present thread").
///
/// An error from any method is the policy's `Source::Render`. After an error from
/// [`draw`](Self::draw) or [`submit`](Self::submit) the frame is torn: the object has
/// ended it as `Renderer::torn` and `RenderGraph`'s torn frame do, and every world shown
/// needs a full pass, which `lactose-app` brings about with a new [`sink`](Self::sink).
pub trait RenderSide: Any + Send {
    /// Which queue the present role has on the device this object submits to:
    /// `Capabilities::queues.present`. A constant of the object, which [`build_shown`]
    /// asks once.
    fn present_queue(&self) -> QueuePath;

    /// Opens a region to extract's writers if one is free, and never waits. Called in a
    /// job of its own, by the task that drives the frame, at the start of a frame that
    /// begins with none open: the first frame, a frame after one that was torn, and each
    /// frame after a `Wait`. Called with a region already open it answers `Ready`. It
    /// never answers with no region and no point: an implementor over `lactose-renderer`
    /// meets `Opened::Unsubmitted` itself, by the empty submit that crate's "Regions"
    /// names, which is why it is handed the job's context, and answers with what
    /// `Renderer::open` says after it.
    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Region>;

    /// A sink for the passes of one shown world, whether or not a region is open: a pass
    /// writes the region open when it begins. Installing it (`ExtractState::set_sink`)
    /// makes that world's next pass a full one. A pass names its world's kind itself
    /// (`ExtractPass::kind`).
    fn sink(&mut self) -> Box<dyn InstanceSink>;

    /// The presentation schedule on one acquired pair: interpolation, the camera rig, the
    /// remote sample, and the delivery of `frame`'s events (decision 5.5). Called once in
    /// every frame that ran its passes, from the frame's job, whether or not the frame
    /// draws. It records nothing, submits nothing and reaches no device queue.
    fn present(
        &mut self,
        ctx: &mut JobContext<'_>,
        frame: &PresentFrame<'_>,
        time: FrameTime,
    ) -> Result<()>;

    /// The frame's draw up to its recorded passes: the remote batch, `prepare`, the
    /// declaration into the image of `draw.target` at `draw.extent`, compile and record,
    /// at the fraction of the [`present`](Self::present) that preceded it in the same job.
    /// The image is declared left as a colour attachment on the graphics role and handed
    /// off to the present role for a sampled read. A region is open when this is called,
    /// and no pass of extract runs from before it until the frame's
    /// [`submit`](Self::submit) or a later [`open`](Self::open) has answered `Ready`. It
    /// makes no submit. [`submit`](Self::submit) follows every `draw` that returned `Ok`,
    /// unless the executor shut down between the two.
    fn draw(&mut self, ctx: &mut JobContext<'_>, draw: Draw) -> Result<FrameCounts>;

    /// The latch and the submit of the frame [`draw`](Self::draw) recorded, as late as
    /// the frame allows: nothing of `lactose-app`'s runs between them. Then a region opened
    /// for the next frame's writers, without waiting; after a submit that succeeded
    /// `lactose-renderer` answers `Ready` or `Wait` and never `Unsubmitted`, so no job is
    /// needed for it. It gates no submit on a host signal (architecture section 5.3).
    /// Called from the render task's own poll, on a frame-critical worker, with no job's
    /// context. Where the present role shares the graphics queue it is called only once
    /// the present call in flight has returned, so it takes the queue's lock behind no
    /// present, and the sample it latches is the newest at the end of that hold. A frame
    /// so held is submitted and never shown ([`Rendered::Drew::held`]).
    fn submit(&mut self) -> Result<Submitted>;
}

/// What a frame's submit left.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Submitted {
    /// The image the frame drew: the one of the target its [`Draw`] named.
    pub image: ImageHandle,
    /// The frame's point: reached once every submit of the frame has completed.
    pub point: TimelinePoint,
    /// Whether a region is open for the next frame's writers. An implementor whose open
    /// failed here answers `Wait` with `point`, and the next frame's
    /// [`open`](RenderSide::open) reports the failure.
    pub region: Region,
}

/// A target a frame finished: what one present call reads.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Finished {
    /// The target.
    pub target: TargetIndex,
    /// Its image, as the frame's submit left it: released toward the present role for a
    /// sampled read where that is a transfer, and a colour attachment on the graphics role
    /// otherwise.
    pub image: ImageHandle,
    /// The extent the frame drew at, which is the window's when the frame began and the
    /// extent the present side keeps its swapchain at.
    pub extent: Extent,
    /// The point of the frame that drew it. The present's submit waits for it on the
    /// device.
    pub point: TimelinePoint,
}

/// What one present call did.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PresentReport {
    /// The point that is reached when the call's read of the image is done, or `None` if
    /// no submit of the call names the image: no swapchain image was acquired, or the
    /// swapchain was stale. The frame draws into the image again only in submits that
    /// wait for it, and the render side destroys the image only once it is reached.
    pub read: Option<TimelinePoint>,
    /// Whether the image reached the display's queue (`Presented::Shown`).
    pub shown: bool,
}

/// The present thread's side, as `lactose-app` drives it through the present thread's
/// lane: the state `S` of `lactose_platform::present_thread`, which the binary builds over
/// the device's `Swapchain` and the pass that draws a target into its image (decision
/// 5.6). It is called on the present thread alone, one call at a time, and between calls
/// no command of its own names a target. A heap slot it wrote may go on holding the last
/// target's image, which keeps the device from taking that image's destroy and does
/// nothing else: it puts another image there at the first call that names one for the same
/// [`TargetIndex`], and empties its slots in [`close`](Self::close). Before it puts
/// another image in a slot it waits, within its bound, for the last `read` it reported,
/// and reports `read: None` if that is not reached: a pass of an earlier call that
/// declared the slot may still be pending, and the device refuses a slot's write while one
/// is. A new image can arrive with no resize before it, when the frame's extent changed
/// and came back with the frame between dropped or torn, so the resize's wait does not
/// stand in for this one.
pub trait PresentSide: Send + 'static {
    /// One present: resize the swapchain when it is stale or `finished.extent` is not the
    /// extent it was last resized to; acquire an image; record on an encoder of the
    /// present role the acquire half of the target's hand-off and the draw of
    /// `finished.image` into the acquired image; and present in a submit that waits for
    /// `finished.point` and signals the report's `read`. Every wait it makes is bounded.
    /// It names no target's image but `finished.image`, and none in any command recorded
    /// after it returns. A call that could not present (no image acquired, a stale
    /// swapchain, a resize or a present refused as pending) is no error: it reports
    /// `read: None`. An error is the policy's `Source::Render`. A call that reports
    /// `read: None`, or returns an error of any kind but `DeviceLost`, made no submit
    /// that names `finished.image` and leaves none recorded: what it recorded of the
    /// image is discarded before it returns, so no later call's submit acquires a release
    /// this one skipped (`crates/render-graph/README.md`, "What each consumer needs").
    fn present(&mut self, finished: Finished) -> Result<PresentReport>;

    /// Empties the heap slots this side wrote, after a wait within its bound for the last
    /// `read` it reported, then drops the swapchain and everything of the device this side
    /// holds: the last call the frame makes on the present thread
    /// (`crates/platform/README.md`, "Shutdown"). Its waits are bounded, save the
    /// swapchain's drop, which waits for the submits that use its images. Where the wait
    /// for the read runs past its bound, the device refuses to empty the slot and the slot
    /// is left: the device then refuses the destroy of the target it holds, and that image
    /// is freed with the device. On a lost device every wait returns at once and nothing
    /// is left.
    fn close(&mut self);
}

/// The lane to the present thread, over the state `lactose-app` declares for it.
pub type PresentLane = Lane<Box<dyn PresentSide>>;

/// What a frame did on the render side.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Rendered {
    /// It drew and submitted.
    Drew {
        /// The target it drew into.
        target: TargetIndex,
        /// The frame's point.
        point: TimelinePoint,
        /// What its prepare left undrawn.
        counts: FrameCounts,
        /// Whether a present call was queued for the target after the submit. `false`
        /// where the frame was dropped: the target is then never shown, and the next frame
        /// draws into it again. Where the present role has a queue of its own, a frame is
        /// dropped while an earlier call was still in flight at its submit. Where it
        /// shares the graphics queue, a frame is dropped where its submit was held, so
        /// `queued` is `!held`.
        queued: bool,
        /// Whether the frame's submit was held for a present call that was still in flight
        /// when the frame's job returned. Such a frame is submitted once that call returns
        /// and queues no call of its own, whatever the call reported. `false` wherever the
        /// present role has a queue of its own.
        held: bool,
    },
    /// The window had no area, or the frame was starved: no region was open and the
    /// world's structural log could wait no longer for one. The frame ran its passes with
    /// no sink, presented from the snapshot and drew nothing. A starved frame is hidden
    /// whatever [`RenderSide::open`] answered, an error the policy logged included, and
    /// such a frame is counted torn as well.
    Hidden,
    /// No region was free: the frame ran its ticks, deferred its passes and published
    /// nothing. The point frees a region. [`Shown::step`] reports it; a paced frame that
    /// waits starts no render task, and says so in [`ShownOutput::deferred`].
    Waited(TimelinePoint),
    /// A method of the render side failed and the policy logged it: the frame is torn.
    /// Where it was the frame's first, `open`, the frame deferred its passes as one that
    /// waits does, unless it was starved, when it is [`Hidden`](Self::Hidden).
    Torn,
    /// The frame before was still on the render side: its render task had not resolved
    /// when this frame looked, or, where the present role shares the graphics queue, a
    /// present call was in flight and this frame had to ask for a region. The frame ran
    /// its ticks, deferred its passes and published nothing, as one that waits for a
    /// region does, and started no render task. So a record that outlasts the tick period,
    /// or a submit held for a present call, delays no tick. A paced frame alone reports
    /// it, in [`ShownOutput::deferred`]: [`Shown::step`] and [`Shown::flush`] await where
    /// [`Shown::frame`] looks.
    Busy,
}

/// Counters of a shown world's frames since it was built, for a test and a result file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ShownStats {
    /// Frames that drew and submitted.
    pub drawn: u64,
    /// Frames that presented from the snapshot and drew nothing.
    pub hidden: u64,
    /// Frames that deferred their passes for want of a region.
    pub waited: u64,
    /// Frames that deferred their passes because the frame before was still on the render
    /// side ([`Rendered::Busy`]).
    pub busy: u64,
    /// Frames torn. A starved frame whose `open` failed is counted here and in `hidden`.
    pub torn: u64,
    /// Present calls queued. `drawn` less this is the frames finished and never shown, on
    /// either queue layout.
    pub queued: u64,
    /// Frames whose submit was held for a present call in flight, each of them dropped
    /// after its submit: nonzero only where the present role shares the graphics queue,
    /// where it is `drawn` less `queued`.
    pub held: u64,
    /// Present calls that reported `shown`.
    pub shown: u64,
    /// Sinks installed after the first: full passes the render side's counts, a torn
    /// frame or a return from hiding asked for.
    pub resets: u64,
}

/// What one paced frame produced.
pub struct ShownOutput {
    /// The world's half, as [`Server::frame`] reports it. `gathered` is empty in a frame
    /// that deferred its passes.
    pub world: FrameOutput,
    /// What the frame in flight when this call began did on the render side, taken back
    /// by this call at its one look, before its passes. `None` if none was in flight, or
    /// if its render task had not resolved: `deferred` is then `Some(Rendered::Busy)`, and
    /// a later call reports that frame.
    pub joined: Option<Rendered>,
    /// `Some` if this frame deferred its passes and started no render task: `Waited` for
    /// want of a region, `Busy` while the frame before was still on the render side, or
    /// `Torn` where asking for a region failed and the policy logged it. `None` for a
    /// starved frame, which runs its passes hidden whatever `open` answered.
    pub deferred: Option<Rendered>,
}

/// A world that is shown: milestone 2's local world, a [`Server`] built by the one
/// assembly path, and the render side driven beside it. Owned by the task that drives the
/// frame; every run takes it by value and hands it back, as [`Server`]'s do.
///
/// A `Shown` is ended with [`stop`](Self::stop) and by nothing else. Dropping one, or a
/// run future that owns one, drops the render side where the drop lands: on the dropping
/// thread while no frame is in flight or its render task is suspended, and otherwise on
/// the frame-critical worker that finishes the frame's job or the render task's poll.
/// That drop waits for the device for as long as it has submits pending, and
/// [`PresentSide::close`] is never called (`README.md`, "Shutdown").
pub struct Shown {
    server: Server,
    flight: Flight,
    policy: ErrorPolicy,
    stats: ShownStats,
}

/// The render side between frames, or the frame in flight that owns it. A render task
/// decides its own errors with a copy of the policy: `Err` is an error the policy stops
/// the run for, and one it logged is `Ok(Rendered::Torn)`, or a present that was skipped.
enum Flight {
    Idle(Box<Side>),
    Running(Task<(Box<Side>, Result<Rendered>)>),
}

/// Everything a frame's render task owns: moved into the task, by the task into each job
/// it runs (`Executor::run_job`, which `lactose-task` is asked for) and handed back when
/// each resolves, so nothing here is shared and nothing is locked.
struct Side {
    /// The render side.
    render: Box<dyn RenderSide>,
    /// The snapshot's reading half: one `acquire` in every frame that published.
    reader: SnapshotReader,
    /// The present role's queue, as `render` said at the build. On a queue of its own the
    /// render task looks at the call in flight after the frame's submit. On a shared one
    /// it awaits the call before the submit, and the driving task looks at it before a
    /// job that asks `open`.
    queue: QueuePath,
    /// The lane to the present thread.
    present: PresentLane,
    /// The one present call in flight, and the target it names.
    call: Option<(TargetIndex, LaneCall<Result<PresentReport>>)>,
    /// For each target, the point of the present thread's last read of it.
    read: [Option<TimelinePoint>; TARGETS],
    /// Whether a region is open, as the render side last said; `None` before the first
    /// frame and after a torn one, when the frame asks.
    region: Option<Region>,
    /// Whether the world's sink is detached: the last frame that ran its passes was
    /// hidden.
    detached: bool,
    /// Whether the world is owed a new sink before its next pass that writes one: a
    /// frame's counts were stale for it, or a frame was torn.
    owed: bool,
    /// Whether the last pass that wrote the sink was a full one: its frame's dropped and
    /// unplaced counts then owe no second sink.
    full: bool,
    /// Whether the last frame that ran its passes hid for want of a region.
    starved: bool,
    /// Ticks the world has run since its last passes.
    unpassed: u32,
}

/// Builds a shown world: [`build_server`](crate::build_server)'s path with `config`,
/// `start` and `cooked`, and on the builder the `lactose-sim` option that leaves a shown
/// `Server` world's presentation events for the frame; then
/// `lactose_present_api::snapshot_channel`, the writing half installed with
/// `ExtractState::set_snapshot` and `render.sink()` with `ExtractState::set_sink`. It asks
/// `render.present_queue()`, calls no other method of `render` and makes no call on
/// `present`: the first frame asks for a region. The world it builds is the world
/// `build_server` builds from the same arguments, and its hash is that world's: no second
/// assembly path exists. What the render side returns is decided by
/// [`ErrorPolicy::MILESTONE_2`](crate::ErrorPolicy), whose rows for every other source are
/// milestone 1's. Fails as `build_server` fails, before anything runs, with `render` and
/// `present` dropped on the calling thread. That thread may be a frame-critical worker,
/// since a binary may build inside its root task: a render side through which nothing has
/// been submitted waits for nothing when it drops (`crates/rhi-api/README.md`, "Handles
/// and lifetimes"), and a caller whose render side has submits pending builds where a
/// drop may wait.
pub fn build_shown(
    executor: &Executor,
    game: &Game,
    config: WorldConfig,
    start: Start,
    cooked: Arc<dyn Cooked>,
    render: Box<dyn RenderSide>,
    present: PresentLane,
) -> Result<Shown> {
    todo!("frame-impl: `README.md`, \"The shown frame\"")
}

impl Shown {
    /// The world.
    #[must_use]
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// The world, for what [`Server`] offers between runs: the time scale, a resync of
    /// the pacer, the hash. Extract's outputs are the frame's: a caller that sets a sink
    /// or a snapshot writer itself breaks the frame's count of full passes.
    pub fn server_mut(&mut self) -> &mut Server {
        &mut self.server
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> ShownStats {
        self.stats
    }

    /// One tick and its frame, joined: the step-driven form, for the fixed-frame mode of
    /// decision 5.1, a recorded test and a replayed log. [`Server::step`] with `frame`;
    /// then the frame's passes, publication and render side as [`frame`](Self::frame)
    /// runs them, at `time` and `window`; then the render task joined before this
    /// resolves, so nothing of the frame overlaps the next step. `time` is the caller's
    /// whole: this call derives nothing from a clock or from the world's pacer. The
    /// gathered frame is taken and dropped after the passes, as the headless binary drops
    /// it, so it does not grow across steps. It awaits wherever a paced frame looks, so it
    /// never reports [`Rendered::Busy`]. Resolves with the world, the step's output and
    /// what the frame did; or with [`ShownError`] as [`frame`](Self::frame) does.
    pub fn step(
        self,
        executor: &Executor,
        frame: TickInput,
        time: FrameTime,
        window: Option<Extent>,
    ) -> ShownStep<'_> {
        todo!("frame-impl: `README.md`, \"The shown frame\"")
    }

    /// One paced frame, by value: [`Server::frame`]'s ticks at `now` with `frames`; then
    /// one look at the frame in flight, which waits for nothing; then, unless no region is
    /// open, the passes, the publication and a render task started and left in flight,
    /// which a later call takes back. While the render task in flight has not resolved,
    /// the frame defers its passes and reports [`Rendered::Busy`]: no tick waits for the
    /// render side, short of the bound `README.md` gives a frame that defers, past which
    /// the frame awaits. `window` is the window's extent in pixels, or `None` while it has
    /// no area; an extent with a zero dimension is read as `None`.
    /// `README.md`, "The shown frame", states each step. The caller sleeps until
    /// `world.next_deadline` and calls again: the timer root paces the frame and nothing
    /// else does. Resolves with the world and the output, or with [`ShownError`].
    pub fn frame<'a>(
        self,
        executor: &'a Executor,
        now: Instant,
        frames: &'a mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
        window: Option<Extent>,
    ) -> ShownFrame<'a> {
        todo!("frame-impl: `README.md`, \"The shown frame\"")
    }

    /// Joins the frame in flight, if any: it awaits, and resolves when the frame's render
    /// task has, with what it did. A present call it queued may still be in flight.
    pub fn flush(self) -> ShownFlush {
        todo!("frame-impl: `README.md`, \"The shown frame\"")
    }

    /// Ends the render side: [`flush`](Self::flush); then the present call in flight
    /// awaited; then [`PresentSide::close`] as the last call on the present thread,
    /// awaited; then the lane dropped. An error of the render side met on the way is
    /// logged and ends nothing. Resolves with the world and the render side, which the
    /// binary drops off the frame-critical workers after the present thread has stopped
    /// (`README.md`, "Shutdown").
    pub fn stop(self) -> ShownStop {
        todo!("frame-impl: `README.md`, \"The shown frame\"")
    }
}

/// What [`Shown::stop`] hands back.
pub struct Stopped {
    /// The world, for [`Server::shutdown`].
    pub server: Server,
    /// The render side, with no frame of its own in flight on a worker: its drop waits for
    /// the device, so it is the binary's to drop on a thread that may block. It is the
    /// object [`build_shown`] was given, so the binary may take it back as its own type
    /// (`Box<dyn Any>::downcast`).
    pub render: Box<dyn RenderSide>,
}

/// What a failed run of a shown world leaves its caller.
pub enum ShownKept {
    /// The world and the render side, as they were when the world's run handed its
    /// simulation back, or when the policy's response to a render side's error was `Stop`
    /// (a device loss). The binary calls [`Shown::stop`].
    Shown(Box<Shown>),
    /// The render side alone, ended as [`Shown::stop`] ends it: the world did not survive
    /// (a step whose unload export could not be diffed into the store, as `ServerError`
    /// says) and the executor did. The binary drops it as it drops [`Stopped::render`].
    Render(Box<dyn RenderSide>),
    /// Nothing: after `Cancelled`, when the executor shut down during the run, and the
    /// world and the render side dropped with it, wherever the executor dropped them:
    /// a frame-critical worker, the thread that shut the executor down, or the present
    /// thread, where a present call's reply woke a task of the frame after the shutdown
    /// began (`README.md`, "Shutdown").
    Nothing,
}

/// How a shown run fails: the error, and what survived it. It does not convert into
/// `lactose_core::Error`, as `ServerError` does: `?` into a plain `Result` would drop
/// `kept`, and with it the render side, on the caller's thread, which for a root task is a
/// frame-critical worker. A caller matches `kept` and ends what it holds in order.
pub struct ShownError {
    /// What failed.
    pub error: Error,
    /// What the caller still holds.
    pub kept: ShownKept,
}

impl fmt::Debug for ShownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kept = match self.kept {
            ShownKept::Shown(_) => "Shown",
            ShownKept::Render(_) => "Render",
            ShownKept::Nothing => "Nothing",
        };
        f.debug_struct("ShownError").field("error", &self.error).field("kept", &kept).finish()
    }
}

impl fmt::Display for ShownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for ShownError {}

/// A run in flight, owning the shown world until it resolves.
type Run<'a, T> = Pin<Box<dyn Future<Output = core::result::Result<T, ShownError>> + Send + 'a>>;

/// One step and its frame in flight. See [`Shown::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct ShownStep<'a> {
    run: Run<'a, (Shown, StepOutput, Rendered)>,
}

impl Future for ShownStep<'_> {
    type Output = core::result::Result<(Shown, StepOutput, Rendered), ShownError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// One paced frame in flight. See [`Shown::frame`].
#[must_use = "futures do nothing unless awaited"]
pub struct ShownFrame<'a> {
    run: Run<'a, (Shown, ShownOutput)>,
}

impl Future for ShownFrame<'_> {
    type Output = core::result::Result<(Shown, ShownOutput), ShownError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// A join in flight. See [`Shown::flush`].
#[must_use = "futures do nothing unless awaited"]
pub struct ShownFlush {
    run: Run<'static, (Shown, Option<Rendered>)>,
}

impl Future for ShownFlush {
    type Output = core::result::Result<(Shown, Option<Rendered>), ShownError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// The render side's end in flight. See [`Shown::stop`]. Resolves with an error of kind
/// `Cancelled`, and nothing handed back, when the executor shut down first with a frame
/// in flight: the join of that frame's render task is what the executor cancels. With no
/// frame in flight `stop` awaits the present thread alone and resolves as it would have.
#[must_use = "futures do nothing unless awaited"]
pub struct ShownStop {
    run: Pin<Box<dyn Future<Output = Result<Stopped>> + Send>>,
}

impl Future for ShownStop {
    type Output = Result<Stopped>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
