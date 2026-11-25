//! The shown frame (architecture sections 5.1 and 5.3; the milestone 2 plan's decisions
//! 5.1, 5.6 and 5.11; D162): a world that is drawn, the render side `lactose-app` drives
//! beside it through two `dyn` objects the binary builds, and the protocol by which a
//! finished target reaches the present thread. Nothing here names a backend crate: the
//! traits name `lactose-rhi-api`, `lactose-present-api`, `lactose-ecs` and `lactose-task`
//! alone, which is what keeps the server's denylist satisfiable.
//!
//! `README.md`, "The shown frame", is the contract, and the steps named below are its.

use core::any::Any;
use core::fmt;
use core::future::poll_fn;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_ecs::{Executor, WorldKind};
use lactose_net_api::StagingFrame;
use lactose_present_api::{
    ExtractState, InstanceSink, PresentFrame, SnapshotReader, extract_state, snapshot_channel,
};
use lactose_rhi_api::{Extent, ImageHandle, OFFSCREEN_TARGET_IMAGES_MIN, QueuePath, TimelinePoint};
use lactose_sim::TickInput;
use lactose_task::{JobContext, Lane, LaneCall, Sleep, Task, WorkerClass};
use tracing::warn;

use crate::game::Game;
use crate::policy::{ErrorPolicy, LOG_TARGET, Response, Source};
use crate::runtime::{CATCH_UP_CAP, TICK_PERIOD};
use crate::save::Cooked;
use crate::server::{Failed, FrameOutput, Server, StepOutput, build_world, pacer_scale};
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

/// How long before the next tick's deadline a paced frame stops waiting for the frame
/// before it. [`Shown::frame`] awaits the render task in flight, and on a shared queue
/// the present call before a job that asks `open`, until the pacer's next deadline less
/// this, and is [busy](Rendered::Busy) if neither has resolved by then. It is what the
/// frame may still have to do before that deadline: its passes, which criterion 4 of the
/// milestone 2 plan holds to a millisecond for 50,000 dirty rows, behind a timed wake
/// that D144 holds to a millisecond on Windows where a worker is idle to keep it. It does
/// not cover a wake kept behind the record's jobs: the frame's job leaves one
/// frame-critical worker out for that (`Executor::run_job_width`, which `lactose-task` is
/// asked for). Two milliseconds until `frame-pace` reports the lateness of the ticks that
/// follow such a wait; a follow-up commit changes this constant and nothing else if the
/// file shows it wrong (`README.md`, "The shown frame", step 2).
pub const JOIN_MARGIN: Duration = Duration::from_millis(2);

/// The count of finished pictures still on their way to the display, taken as a present
/// call begins, at which the frame after that call is shed: submitted, and not queued for
/// a present ([`PresentReport::waiting`], [`Rendered::Drew::shed`]). A frame is shed
/// where the count rose to this: the last report reads it or more, and the one before
/// read less; the next `SHED_DEPTH` reports that carry a count are then passed over,
/// after a shed and after any other frame that queued no present, one dropped for a call
/// in flight or one that drew nothing, since they count that frame's gap. Three.
/// `reviewer-gpu` measured the count on the M1, under MoltenVK and native Metal at 60 Hz
/// with three images, in the review of D162's fourth draft: it reads one at the
/// swapchain's floor, two in the floor's top band, about 9% of a sweep of the frames'
/// phase against the refresh, and never none while the GPU keeps up; two with one image
/// waiting behind another; and three where the present call waits for an image, in
/// whichever of its driver calls that driver blocks. So three sheds what a drift owes and
/// nothing at the floor, and its steady state is one image waiting. `frame-pace` reads
/// each other device that reports display times; a follow-up commit changes this constant
/// and nothing else (`README.md`, "Pacing", *The shedding rule*).
pub const SHED_DEPTH: u32 = 3;

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
    /// later and from then on, and one while the world is paused. It is the weight of the
    /// pair's newer state over the whole pair, whatever ticks the pair's two snapshots
    /// are: `lactose-renderer` draws `p + (c - p) * fraction` (`InstanceRecord::drawn`),
    /// and nothing rescales it for a pair that spans several ticks, whose picture then
    /// covers them in one period.
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
    /// at the fraction of the [`present`](Self::present) that preceded it in the same
    /// job. The image is declared left as a colour attachment on the graphics role and
    /// handed off to the present role for a sampled read. A region is open when this is
    /// called, and no pass of extract runs from before it until the frame's
    /// [`submit`](Self::submit) or a later [`open`](Self::open) has answered `Ready`.
    /// What it forks from `ctx` runs on `Executor::run_job_width` frame-critical workers
    /// at most, the job's own among them, so an object over `lactose-render-graph` is
    /// made with `GraphDesc::jobs` no greater than that. It makes no submit.
    /// [`submit`](Self::submit) follows every `draw` that returned `Ok`, unless the
    /// executor shut down between the two.
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
    /// How many of the presents this side made before this call were, as this call
    /// began, finished on the device and not yet displayed: their `read` was reached,
    /// and the swapchain's display-time report had named neither them nor a later one.
    /// It is taken before the call's own waits and its acquire. `None` where the
    /// swapchain reports no display time, which is every swapchain until `rhi-api`'s
    /// gains the report (`README.md`, "What D162 asks of other crates"). The frame sheds
    /// where it rises to [`SHED_DEPTH`] and reads it for nothing else.
    pub waiting: Option<u32>,
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
    /// Where its swapchain reports display times it counts [`PresentReport::waiting`]
    /// as it begins, with one read of its timeline's value and one of the swapchain's
    /// report, neither of which waits; where it reports none, `waiting` is `None`.
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
        /// shares the graphics queue, a frame is dropped where its submit was held. On
        /// either, a frame is dropped where it was shed. So `queued` is `!(held || shed)`
        /// on a shared queue.
        queued: bool,
        /// Whether the frame's submit was held for a present call that was still in flight
        /// when the frame's job returned. Such a frame is submitted once that call returns
        /// and queues no call of its own, whatever the call reported. `false` wherever the
        /// present role has a queue of its own.
        held: bool,
        /// Whether the frame was shed: no earlier call was in flight at its decision, and
        /// the count of the last call's report had risen to [`SHED_DEPTH`]
        /// ([`PresentReport::waiting`]): that report read [`SHED_DEPTH`] or more, and the
        /// one before it read less. A rise sheds one frame, the next [`SHED_DEPTH`]
        /// reports that carry a count are passed over, and a frame that was held is not
        /// counted here. `false` wherever the swapchain reports no display time.
        shed: bool,
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
    /// The frame before was still on the render side at this frame's limit, the next
    /// tick's deadline less [`JOIN_MARGIN`]: its render task had not resolved by then,
    /// or, where the present role shares the graphics queue, a present call was still in
    /// flight and this frame had to ask for a region. The frame ran its ticks, waited
    /// until its limit, deferred its passes and published nothing, as one that waits for
    /// a region does, and started no render task. So the frame waits for a record that
    /// outlasts the tick period, or for a submit held for a present call, no later than
    /// its limit. A paced frame alone reports it, in [`ShownOutput::deferred`]:
    /// [`Shown::step`] and [`Shown::flush`] await with no limit where [`Shown::frame`]
    /// awaits up to its own.
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
    /// Present calls queued. `drawn` less this is the frames dropped, on either queue
    /// layout. A queued call may still show nothing, so the frames never shown are
    /// `drawn` less `shown`.
    pub queued: u64,
    /// Frames whose submit was held for a present call in flight, each of them dropped
    /// after its submit: nonzero only where the present role shares the graphics queue,
    /// where it is `drawn` less `queued` less `shed`.
    pub held: u64,
    /// Frames shed ([`Rendered::Drew::shed`]), each of them dropped after its submit:
    /// nonzero only where the swapchain reports display times.
    pub shed: u64,
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
    /// by this call before its passes. `None` if none was in flight, or if its render
    /// task had not resolved by this frame's limit: `deferred` is then
    /// `Some(Rendered::Busy)`, and a later call reports that frame.
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
    driver: Driver,
}

/// What the task that drives the frame keeps beside the world and the flight: what it
/// reads and counts while a render task owns `Side`.
struct Driver {
    policy: ErrorPolicy,
    stats: ShownStats,
    /// Whether a frame was busy behind the render task in `flight`
    /// ([`Rendered::Busy`], at step 2). Such a frame queued no present and could not
    /// reach `Side`, which the task owns. Where the task is taken back,
    /// `Side::pass_over` is set to [`SHED_DEPTH`] unless `Side::gap` is set, and this is
    /// cleared: the task made its decision at its end, which had not come at the busy
    /// frame's limit, so the report of a call it queued is the first passed over.
    busy_gap: bool,
    /// Ticks the world has run since its last passes. It is here and not in `Side`, since
    /// a busy frame runs its ticks while the render task owns `Side`.
    unpassed: u32,
    /// `WorldConfig::log_retention` less [`CATCH_UP_CAP`]: a frame defers its passes only
    /// while `unpassed` is at most this (`README.md`, "The frame's three states").
    bound: u32,
}

/// The render side between frames, or the frame in flight that owns it.
enum Flight {
    Idle(Box<Side>),
    Running(Task<Joined>),
}

/// What a render task resolves to. It decides its own errors with a copy of the policy:
/// the inner `Err` is an error the policy stops the run for, and one it logged is
/// `Ok(Rendered::Torn)`, or a present that was skipped. The outer `Err` is `Cancelled`:
/// the executor shut down before the frame's job started, and the job's closure, which
/// held `Side`, was dropped unrun.
type Joined = Result<(Box<Side>, Result<Rendered>)>;

/// Everything a frame's render task owns: moved into the task, by the task into each job
/// it runs (`Executor::run_job`) and handed back when each resolves, so nothing here is
/// shared and nothing is locked.
struct Side {
    /// The render side.
    render: Box<dyn RenderSide>,
    /// The snapshot's reading half: one `acquire` in every frame that published.
    reader: SnapshotReader,
    /// The present role's queue, as `render` said at the build. On a queue of its own the
    /// render task looks at the call in flight after the frame's submit. On a shared one
    /// it awaits the call before the submit, and the driving task awaits it, up to the
    /// frame's limit, before a job that asks `open`.
    queue: QueuePath,
    /// The lane to the present thread.
    present: PresentLane,
    /// The one present call in flight, and the target it names.
    call: Option<(TargetIndex, LaneCall<Result<PresentReport>>)>,
    /// The target the last queued call named, `None` before the first: a frame draws into
    /// the other.
    named: Option<TargetIndex>,
    /// For each target, the point of the present thread's last read of it.
    read: [Option<TimelinePoint>; TARGETS],
    /// Reports taken that said `shown`, not yet added to `ShownStats::shown`.
    shown: u64,
    /// Whether the last report taken read below [`SHED_DEPTH`]
    /// ([`PresentReport::waiting`]): `false` before the first report, and after a call
    /// that returned an error or was cancelled, which brings no report. A report whose
    /// `waiting` is `None` leaves it as it was, and so does a report that is passed over
    /// (`pass_over`).
    below: bool,
    /// Whether the report taken since the last decision rose to [`SHED_DEPTH`]: it read
    /// that or more, and the one before it read below. The next decision reads it and
    /// clears it, so a rise sheds one frame; a frame that queues no present for another
    /// reason clears it as well, being the frame left out. The report of a call whose
    /// return was still to be taken when a frame queued no present never sets it (`gap`),
    /// and nor does a report that is passed over.
    rose: bool,
    /// How many of the reports after the last frame that queued no present are still to
    /// be passed over: a frame shed, a frame dropped for a call in flight, or a frame
    /// that drew nothing, having waited for a region or been busy, hidden or torn. It is
    /// set to [`SHED_DEPTH`], and never added to: where a frame is shed; where a frame
    /// queues none with no call's return still to be taken; where the return of a call
    /// that was still out at such a frame is taken, after its report is read (`gap`); and
    /// where a render task is taken back that a frame was busy behind
    /// (`Driver::busy_gap`). It is counted down by each report taken that carries a count,
    /// which then changes neither `below` nor `rose`. A frame that queued no present left
    /// the reports of the calls queued after it one present fewer to count, and they
    /// would read as the count falling and then rising again. A report whose `waiting` is
    /// `None`, and a call that brought no report, leave it as it was.
    pass_over: u32,
    /// Whether a frame queued no present while the return of the call in `call` was still
    /// to be taken: a frame dropped for that call, its submit held for it on a shared
    /// queue or not queued at its submit on a queue of its own; a frame busy behind it at
    /// step 3; or a frame that waited for a region, was hidden or was torn while it was
    /// out. The count of such a call was taken before the gap: its report never sets
    /// `rose`, and once the call's return is taken, whatever it brought, `pass_over` is
    /// set to [`SHED_DEPTH`] and this is cleared.
    gap: bool,
    /// Whether a region is open, as the render side last said; `None` before the first
    /// frame and after a torn one, when the frame asks.
    region: Option<Region>,
    /// Whether the world's sink is detached: the last frame that ran its passes was
    /// hidden.
    detached: bool,
    /// Whether the world is owed a new sink before its next pass that writes one: a
    /// frame's counts were stale for it, or a frame was torn.
    owed: bool,
    /// Whether a sink was installed that no pass has written: the next pass that writes
    /// one is full.
    fresh: bool,
    /// Whether the last pass that wrote the sink was a full one: its frame's dropped and
    /// unplaced counts then owe no second sink. A pass that extract made full for a
    /// reason of its own (a restore, a trimmed log) is not known here and reads as not
    /// full, which costs at most one reset more.
    full: bool,
    /// Whether the last frame that ran its passes hid for want of a region.
    starved: bool,
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
    let bound = config.log_retention.saturating_sub(CATCH_UP_CAP);
    let mut server = build_world(executor, game, config, start, cooked, true)?;
    let mut render = render;
    let queue = render.present_queue();
    let (writer, reader) = snapshot_channel();
    let extract = extract_of(&mut server);
    extract.set_snapshot(Some(writer));
    extract.set_sink(Some(render.sink()));
    let side = Side {
        render,
        reader,
        queue,
        present,
        call: None,
        named: None,
        read: [None; TARGETS],
        shown: 0,
        below: false,
        rose: false,
        pass_over: 0,
        gap: false,
        region: None,
        detached: false,
        owed: false,
        fresh: true,
        full: false,
        starved: false,
    };
    let driver = Driver {
        policy: ErrorPolicy::MILESTONE_2,
        stats: ShownStats::default(),
        busy_gap: false,
        unpassed: 0,
        bound,
    };
    Ok(Shown { server, flight: Flight::Idle(Box::new(side)), driver })
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
        self.driver.stats
    }

    /// One tick and its frame, joined: the step-driven form, for the fixed-frame mode of
    /// decision 5.1, a recorded test and a replayed log. [`Server::step`] with `frame`;
    /// then the frame's passes, publication and render side as [`frame`](Self::frame)
    /// runs them, at `time` and `window`; then the render task joined before this
    /// resolves, so nothing of the frame overlaps the next step. `time` is the caller's
    /// whole: this call derives nothing from a clock or from the world's pacer. The
    /// gathered frame is taken and dropped after the passes, as the headless binary drops
    /// it, so it does not grow across steps. It awaits with no limit wherever a paced
    /// frame awaits up to its own, so it never reports [`Rendered::Busy`]. Resolves with
    /// the world, the step's output and what the frame did; or with [`ShownError`] as
    /// [`frame`](Self::frame) does.
    pub fn step(
        self,
        executor: &Executor,
        frame: TickInput,
        time: FrameTime,
        window: Option<Extent>,
    ) -> ShownStep<'_> {
        ShownStep { run: Box::pin(self.run_step(executor, frame, time, window)) }
    }

    /// One paced frame, by value: [`Server::frame`]'s ticks at `now` with `frames`; then
    /// the wait for the frame in flight, which ends at the frame's limit, the pacer's
    /// next deadline less [`JOIN_MARGIN`], through `executor`'s timer; then, unless no
    /// region is open, the passes, the publication and a render task started and left in
    /// flight, which a later call takes back. Where the render task in flight has not
    /// resolved by the limit, the frame defers its passes and reports [`Rendered::Busy`]:
    /// the frame waits for the render side no later than its limit, short of the bound
    /// `README.md` gives a frame that defers, past which the frame awaits with no limit.
    /// That the wake at the limit comes on time is the executor's: the frame's job leaves
    /// one frame-critical worker out to keep it (`Executor::run_job_width`, which
    /// `lactose-task` is asked for). An executor of one frame-critical worker has none to
    /// leave, and there every frame looks once and does not wait for the task, as a frame
    /// does whose limit has passed when its ticks are done, which that of a frame that is
    /// behind has. `now` is `executor`'s clock, `Instant::now()` read just before the
    /// call: the limit is the pacer's, in the time base of `now`, and the executor's
    /// timer compares it with that clock, so a `now` that runs behind the clock makes
    /// every frame look, and one that runs ahead waits past the next deadline. The call
    /// reads no clock of its own. A frame suspended in its wait when the executor shuts
    /// down is dropped with its `Server`, unpolled, on the thread that called
    /// `Executor::shutdown`, and resolves nothing. Only a frame polled after the shutdown
    /// began, as one driven through `Executor::block_on` is, resolves `Cancelled` with
    /// [`ShownKept::Nothing`], whether the render task's join or the timer reports it.
    /// `window` is the window's extent in pixels, or `None` while it has no area; an
    /// extent with a zero dimension is read as `None`. `README.md`, "The shown frame",
    /// states each step. The caller sleeps until `world.next_deadline` and calls again:
    /// the timer root paces the frame and nothing else does. Resolves with the world and
    /// the output, or with [`ShownError`].
    pub fn frame<'a>(
        self,
        executor: &'a Executor,
        now: Instant,
        frames: &'a mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
        window: Option<Extent>,
    ) -> ShownFrame<'a> {
        ShownFrame { run: Box::pin(self.run_frame(executor, now, frames, window)) }
    }

    /// Joins the frame in flight, if any: it awaits, and resolves when the frame's render
    /// task has, with what it did. A present call it queued may still be in flight.
    pub fn flush(self) -> ShownFlush {
        ShownFlush { run: Box::pin(self.run_flush()) }
    }

    /// Ends the render side: [`flush`](Self::flush); then the present call in flight
    /// awaited; then [`PresentSide::close`] as the last call on the present thread,
    /// awaited; then the lane dropped. An error of the render side met on the way is
    /// logged and ends nothing. Resolves with the world and the render side, which the
    /// binary drops off the frame-critical workers after the present thread has stopped
    /// (`README.md`, "Shutdown").
    pub fn stop(self) -> ShownStop {
        ShownStop {
            run: Box::pin(async move {
                let Self { server, flight, .. } = self;
                // With a frame in flight after a shutdown, the server is dropped here.
                let render = end(flight).await?;
                Ok(Stopped { server, render })
            }),
        }
    }
}

/// How a shown run resolves.
type Outcome<T> = core::result::Result<T, ShownError>;

/// How long a wait of the frame may last.
enum Limit<'s> {
    /// No limit: `Shown::step`, and a frame past the bound on deferring.
    None,
    /// One poll: step 2 on an executor of one frame-critical worker, and a render task's
    /// look at the call in flight on a queue of its own.
    Look,
    /// The frame's limit, kept by the frame's one `Sleep`.
    Until(&'s mut Sleep),
}

/// `wait` up to `limit`, polled first at each wake: `Some` with what it resolved to, or
/// `None` where the limit came first. `Err` is the `Sleep`'s `Cancelled`.
async fn wait_for<W: Future + Unpin>(wait: &mut W, limit: Limit<'_>) -> Result<Option<W::Output>> {
    let mut limit = limit;
    poll_fn(move |cx| {
        if let Poll::Ready(value) = Pin::new(&mut *wait).poll(cx) {
            return Poll::Ready(Ok(Some(value)));
        }
        match &mut limit {
            Limit::None => Poll::Pending,
            Limit::Look => Poll::Ready(Ok(None)),
            Limit::Until(sleep) => Pin::new(&mut **sleep).poll(cx).map_ok(|()| None),
        }
    })
    .await
}

/// The world's extract state, which `lactose_present_api::setup` installs in every world.
fn extract_of(server: &mut Server) -> &mut ExtractState {
    extract_state(server.simulation_mut())
        .expect("`lactose_present_api::setup` is in every world kind's engine list")
}

/// The fraction of a paced frame (`README.md`, "The shown frame", the frame's time).
fn fraction(server: &Server, next_deadline: Instant, now: Instant) -> f32 {
    let scale = server.time_scale();
    if scale.is_paused() {
        return 1.0;
    }
    let period = TICK_PERIOD.as_secs_f64() / pacer_scale(scale);
    let left = next_deadline.saturating_duration_since(now).as_secs_f64();
    (1.0 - left / period).clamp(0.0, 1.0) as f32
}

/// What a render task taken back left.
enum Back {
    /// The render side, and what the frame did.
    Side(Box<Side>, Rendered),
    /// The render side, and an error the policy stopped the run for.
    Stopped(Box<Side>, Error),
    /// `Cancelled`: the render side is gone.
    Lost(Error),
}

impl Driver {
    /// Takes a render task's output back: its counts, and the pass-over a frame busy
    /// behind it owes.
    fn take_back(&mut self, joined: Result<Joined>) -> Back {
        let (mut side, rendered) = match joined.and_then(|joined| joined) {
            Ok(joined) => joined,
            Err(error) => return Back::Lost(error),
        };
        self.stats.shown += core::mem::take(&mut side.shown);
        if core::mem::take(&mut self.busy_gap) && !side.gap {
            side.pass_over = SHED_DEPTH;
            side.rose = false;
        }
        match rendered {
            Ok(rendered) => {
                let stats = &mut self.stats;
                match rendered {
                    Rendered::Drew { queued, held, shed, .. } => {
                        stats.drawn += 1;
                        stats.queued += u64::from(queued);
                        stats.held += u64::from(held);
                        stats.shed += u64::from(shed);
                    }
                    Rendered::Hidden => stats.hidden += 1,
                    Rendered::Torn => stats.torn += 1,
                    Rendered::Waited(_) | Rendered::Busy => {}
                }
                Back::Side(side, rendered)
            }
            Err(error) => Back::Stopped(side, error),
        }
    }
}

impl Shown {
    /// The failure of a run that kept the world and the render side.
    fn failed(self, error: Error) -> ShownError {
        ShownError { error, kept: ShownKept::Shown(Box::new(self)) }
    }

    /// [`flush`](Self::flush).
    async fn run_flush(self) -> Outcome<(Self, Option<Rendered>)> {
        let Self { server, flight, mut driver } = self;
        let task = match flight {
            Flight::Idle(side) => {
                return Ok((Self { server, flight: Flight::Idle(side), driver }, None));
            }
            Flight::Running(task) => task,
        };
        match driver.take_back(task.await) {
            Back::Side(side, rendered) => {
                Ok((Self { server, flight: Flight::Idle(side), driver }, Some(rendered)))
            }
            Back::Stopped(side, error) => {
                Err(Self { server, flight: Flight::Idle(side), driver }.failed(error))
            }
            Back::Lost(error) => Err(ShownError { error, kept: ShownKept::Nothing }),
        }
    }

    /// [`step`](Self::step).
    async fn run_step(
        self,
        executor: &Executor,
        frame: TickInput,
        time: FrameTime,
        window: Option<Extent>,
    ) -> Outcome<(Self, StepOutput, Rendered)> {
        let (shown, _) = self.run_flush().await?;
        let Self { server, flight, mut driver } = shown;
        let (server, output, _) = match server.run_step(executor, frame).await {
            Ok(stepped) => stepped,
            Err(failed) => return Err(world_failed(failed, flight, driver).await),
        };
        driver.unpassed = driver.unpassed.saturating_add(1);
        let Flight::Idle(side) = flight else { unreachable!("the flush took the task back") };
        let drawn = draw_frame(executor, server, side, driver, Limit::None, time, window).await?;
        let Drawn { shown, deferred, gathered } = drawn;
        // Taken and dropped, as the headless binary drops it.
        drop(gathered);
        if let Some(deferred) = deferred {
            return Ok((shown, output, deferred));
        }
        let (shown, rendered) = shown.run_flush().await?;
        Ok((shown, output, rendered.expect("a frame that did not defer started a render task")))
    }

    /// [`frame`](Self::frame).
    async fn run_frame(
        self,
        executor: &Executor,
        now: Instant,
        frames: &mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
        window: Option<Extent>,
    ) -> Outcome<(Self, ShownOutput)> {
        let Self { server, flight, mut driver } = self;
        // Step 1: the ticks, beside the render task in flight.
        let (server, ticks) = match server.run_ticks(executor, now, frames).await {
            Ok(ticked) => ticked,
            Err(failed) => return Err(world_failed(failed, flight, driver).await),
        };
        driver.unpassed = driver.unpassed.saturating_add(ticks.ticks);
        let deferring = driver.unpassed <= driver.bound;
        let limit = ticks.next_deadline.checked_sub(JOIN_MARGIN).unwrap_or(now);
        // One `Sleep` serves both waits. It registers no timer before its first poll, and
        // answers a passed limit at that poll without registering one.
        let mut sleep = executor.sleep_until(limit);
        let time = FrameTime { now, fraction: fraction(&server, ticks.next_deadline, now) };

        // Step 2: the wait for the frame before.
        let (side, joined) = match flight {
            Flight::Idle(side) => (side, None),
            Flight::Running(mut task) => {
                let wait = if !deferring {
                    Limit::None
                } else if executor.frame_worker_count() == 1 {
                    Limit::Look
                } else {
                    Limit::Until(&mut sleep)
                };
                let joined = match wait_for(&mut task, wait).await {
                    Ok(Some(joined)) => joined,
                    Ok(None) => {
                        driver.stats.busy += 1;
                        driver.busy_gap = true;
                        let world = ticks.output(StagingFrame::new(server.tick()));
                        let output =
                            ShownOutput { world, joined: None, deferred: Some(Rendered::Busy) };
                        return Ok((
                            Self { server, flight: Flight::Running(task), driver },
                            output,
                        ));
                    }
                    Err(error) => return Err(ShownError { error, kept: ShownKept::Nothing }),
                };
                match driver.take_back(joined) {
                    Back::Side(side, rendered) => (side, Some(rendered)),
                    Back::Stopped(side, error) => {
                        return Err(
                            Self { server, flight: Flight::Idle(side), driver }.failed(error)
                        );
                    }
                    Back::Lost(error) => {
                        return Err(ShownError { error, kept: ShownKept::Nothing });
                    }
                }
            }
        };

        // Steps 3 to 5.
        let wait = if deferring { Limit::Until(&mut sleep) } else { Limit::None };
        let Drawn { shown, deferred, gathered } =
            draw_frame(executor, server, side, driver, wait, time, window).await?;
        let tick = shown.server.tick();
        let world = ticks.output(gathered.unwrap_or_else(|| StagingFrame::new(tick)));
        Ok((shown, ShownOutput { world, joined, deferred }))
    }
}

/// What steps 3 to 5 left: the world with the render task started, or with the render
/// side idle where the frame deferred.
struct Drawn {
    shown: Shown,
    /// `Some` where the frame deferred its passes and started no render task.
    deferred: Option<Rendered>,
    /// The gather of the passes, where they ran.
    gathered: Option<StagingFrame>,
}

/// What step 3 decided.
enum State {
    /// Draw at this extent.
    Draw(Extent),
    /// Run the passes and the job with no draw: no window, or `starved`.
    Hidden { starved: bool },
    /// Run nothing more.
    Defer(Rendered),
}

/// Steps 3 to 5 of a frame (`README.md`, "The shown frame"), with the render side taken
/// back: the state, the passes and the publication, and the render task, spawned and left
/// in flight. `wait` is how long step 3 awaits a present call on a shared queue.
async fn draw_frame(
    executor: &Executor,
    server: Server,
    mut side: Box<Side>,
    mut driver: Driver,
    wait: Limit<'_>,
    time: FrameTime,
    window: Option<Extent>,
) -> Outcome<Drawn> {
    let deferring = driver.unpassed <= driver.bound;
    let mut server = server;
    let window = window.filter(|extent| extent.width != 0 && extent.height != 0);

    // Step 3: the state.
    let state = match window {
        // A frame with no window asks nothing, so it leaves a starvation as it found it.
        None => State::Hidden { starved: side.starved },
        Some(extent) if side.region == Some(Region::Ready) => State::Draw(extent),
        Some(extent) => {
            if side.queue == QueuePath::Shared
                && let Some((target, mut call)) = side.call.take()
            {
                match wait_for(&mut call, wait).await {
                    Ok(Some(outcome)) => {
                        if let Err(error) = side.take_report(driver.policy, target, outcome) {
                            let shown = Shown { server, flight: Flight::Idle(side), driver };
                            return Err(shown.failed(error));
                        }
                    }
                    Ok(None) => {
                        // Busy behind the call: this frame queues none with its return
                        // still to be taken.
                        side.call = Some((target, call));
                        side.queued_none();
                        driver.stats.busy += 1;
                        let shown = Shown { server, flight: Flight::Idle(side), driver };
                        let deferred = Some(Rendered::Busy);
                        return Ok(Drawn { shown, deferred, gathered: None });
                    }
                    Err(error) => return Err(ShownError { error, kept: ShownKept::Nothing }),
                }
            }
            let asked = executor.run_job(move |ctx| {
                let opened = side.render.open(ctx);
                (side, opened)
            });
            let (asked, opened) = match asked.await {
                Ok(asked) => asked,
                Err(error) => return Err(ShownError { error, kept: ShownKept::Nothing }),
            };
            side = asked;
            let starved = !deferring || side.starved;
            match opened {
                Ok(Region::Ready) => {
                    side.region = Some(Region::Ready);
                    State::Draw(extent)
                }
                Ok(Region::Wait(point)) => {
                    side.region = Some(Region::Wait(point));
                    if starved {
                        State::Hidden { starved }
                    } else {
                        driver.stats.waited += 1;
                        State::Defer(Rendered::Waited(point))
                    }
                }
                Err(error) => match side.torn(driver.policy, error) {
                    Ok(_) => {
                        driver.stats.torn += 1;
                        if starved {
                            State::Hidden { starved }
                        } else {
                            State::Defer(Rendered::Torn)
                        }
                    }
                    Err(error) => {
                        let shown = Shown { server, flight: Flight::Idle(side), driver };
                        return Err(shown.failed(error));
                    }
                },
            }
        }
    };
    driver.stats.shown += core::mem::take(&mut side.shown);
    let extent = match state {
        State::Defer(deferred) => {
            side.queued_none();
            let shown = Shown { server, flight: Flight::Idle(side), driver };
            return Ok(Drawn { shown, deferred: Some(deferred), gathered: None });
        }
        State::Hidden { starved } => {
            if !side.detached {
                extract_of(&mut server).set_sink(None);
                side.detached = true;
            }
            side.starved = starved;
            None
        }
        State::Draw(extent) => {
            // The four cases of a new sink (`README.md`, "Counts, torn frames and resets").
            if side.detached || side.owed {
                extract_of(&mut server).set_sink(Some(side.render.sink()));
                driver.stats.resets += 1;
                side.detached = false;
                side.owed = false;
                side.fresh = true;
            }
            side.full = core::mem::take(&mut side.fresh);
            side.starved = false;
            Some(extent)
        }
    };

    // Step 4: the passes and the publication.
    let (mut server, gathered) = match server.run_gather(executor).await {
        Ok(gathered) => gathered,
        Err(failed) => return Err(world_failed(failed, Flight::Idle(side), driver).await),
    };
    let events = server.simulation_mut().world_mut().take_present_events();
    extract_of(&mut server).publish(events);
    driver.unpassed = 0;

    // Step 5: the render task.
    let task = executor.spawn(
        WorkerClass::FrameCritical,
        render(executor.clone(), side, driver.policy, time, extent),
    );
    let shown = Shown { server, flight: Flight::Running(task), driver };
    Ok(Drawn { shown, deferred: None, gathered: Some(gathered) })
}

/// A frame's render task: the frame's job, then where it drew the hold, the submit and the
/// decision (`README.md`, "The shown frame", step 5).
async fn render(
    executor: Executor,
    side: Box<Side>,
    policy: ErrorPolicy,
    time: FrameTime,
    extent: Option<Extent>,
) -> Joined {
    let mut side = side;
    let draw = extent.map(|extent| {
        let target = side.named.map_or(TargetIndex(0), other);
        Draw { target, extent, ready: side.read[target.index()] }
    });
    let job = executor.run_job(move |ctx| {
        let job = frame_job(&mut side, ctx, time, draw);
        (side, job)
    });
    drop(executor);
    let (mut side, job) = job.await?;
    let rendered = match (job, draw) {
        (Err(error), _) => side.torn(policy, error),
        (Ok(Some(counts)), Some(draw)) => side.finish(policy, draw, counts).await,
        (Ok(_), _) => {
            side.queued_none();
            Ok(Rendered::Hidden)
        }
    };
    Ok((side, rendered))
}

/// The frame's job: one acquire, `present`, and `draw` where the frame draws.
fn frame_job(
    side: &mut Side,
    ctx: &mut JobContext<'_>,
    time: FrameTime,
    draw: Option<Draw>,
) -> Result<Option<FrameCounts>> {
    let frame = side.reader.acquire().expect("the frame published before its job");
    side.render.present(ctx, &frame, time)?;
    draw.map(|draw| side.render.draw(ctx, draw)).transpose()
}

/// The target after `target`: the one a frame draws into where the last call named
/// `target`.
fn other(target: TargetIndex) -> TargetIndex {
    TargetIndex(((target.index() + 1) % TARGETS) as u8)
}

impl Side {
    /// A frame queued no present: the gap it leaves in what the calls after it count
    /// (`README.md`, "The targets and the present thread", the shedding rule).
    fn queued_none(&mut self) {
        if self.call.is_some() {
            self.gap = true;
        } else {
            self.pass_over = SHED_DEPTH;
        }
        self.rose = false;
    }

    /// A method of the render side failed: the frame is torn, or the run stops.
    fn torn(&mut self, policy: ErrorPolicy, error: Error) -> Result<Rendered> {
        self.region = None;
        self.owed = true;
        self.queued_none();
        match policy.decide(Source::Render, &error) {
            Response::Stop => Err(error),
            Response::Log | Response::Resync => Ok(Rendered::Torn),
        }
    }

    /// Takes the return of the call that named `target`. `Err` is an error the policy
    /// stops the run for.
    fn take_report(
        &mut self,
        policy: ErrorPolicy,
        target: TargetIndex,
        outcome: Result<Result<PresentReport>>,
    ) -> Result<()> {
        let gap = core::mem::take(&mut self.gap);
        let taken = match outcome.and_then(|report| report) {
            Ok(report) => {
                self.read_report(target, report, gap);
                Ok(())
            }
            // A present skipped: the target's `read` is unchanged and the call brings no
            // count, so the report after it has none to rise from.
            Err(error) => {
                self.below = false;
                match policy.decide(Source::Render, &error) {
                    Response::Stop => Err(error),
                    Response::Log | Response::Resync => Ok(()),
                }
            }
        };
        if gap {
            self.pass_over = SHED_DEPTH;
        }
        taken
    }

    /// What `Side` keeps of a report. One taken after a gap is read as the report before
    /// the next, and never as a rise.
    fn read_report(&mut self, target: TargetIndex, report: PresentReport, gap: bool) {
        if let Some(read) = report.read {
            self.read[target.index()] = Some(read);
        }
        self.shown += u64::from(report.shown);
        let Some(waiting) = report.waiting else { return };
        if self.pass_over > 0 {
            self.pass_over -= 1;
            return;
        }
        let at = waiting >= SHED_DEPTH;
        self.rose = !gap && at && self.below;
        self.below = !at;
    }

    /// What a frame's counts owe (`README.md`, "Counts, torn frames and resets").
    fn note_counts(&mut self, counts: FrameCounts) {
        let counts = counts.of(WorldKind::Server);
        if counts.lost || (counts.stale() && !self.full) {
            self.owed = true;
        } else if counts.stale() {
            warn!(
                target: LOG_TARGET,
                dropped = counts.dropped,
                unplaced = counts.unplaced,
                "a full pass left rows out: the renderer is not sized for the world"
            );
        }
    }

    /// After a `draw` that succeeded: the hold on a shared queue, the submit and the
    /// decision.
    async fn finish(
        &mut self,
        policy: ErrorPolicy,
        draw: Draw,
        counts: FrameCounts,
    ) -> Result<Rendered> {
        self.note_counts(counts);
        // The hold. An error the call reports is decided here and the submit is made
        // whatever the response.
        let mut held = false;
        let mut stop = None;
        if self.queue == QueuePath::Shared
            && let Some((target, mut call)) = self.call.take()
        {
            let outcome = match wait_for(&mut call, Limit::Look).await? {
                Some(outcome) => outcome,
                None => {
                    held = true;
                    // Dropped for this call: its report is taken after the gap.
                    self.gap = true;
                    call.await
                }
            };
            stop = self.take_report(policy, target, outcome).err();
        }
        let submitted = match self.render.submit() {
            Ok(submitted) => submitted,
            Err(error) => return stop.map_or_else(|| self.torn(policy, error), Err),
        };
        self.region = Some(submitted.region);
        if let Some(error) = stop {
            return Err(error);
        }
        // The decision: on a queue of its own, one look at the call in flight.
        let free = match self.call.take() {
            None => !held,
            Some((target, mut call)) => match wait_for(&mut call, Limit::Look).await? {
                Some(outcome) => {
                    self.take_report(policy, target, outcome)?;
                    true
                }
                None => {
                    self.call = Some((target, call));
                    false
                }
            },
        };
        let shed = free && self.rose;
        let queued = free && !shed;
        if queued {
            let finished = Finished {
                target: draw.target,
                image: submitted.image,
                extent: draw.extent,
                point: submitted.point,
            };
            let call = self
                .present
                .run(move |present: &mut Box<dyn PresentSide>| present.present(finished));
            self.call = Some((draw.target, call));
            self.named = Some(draw.target);
            self.rose = false;
        } else {
            self.queued_none();
        }
        Ok(Rendered::Drew {
            target: draw.target,
            point: submitted.point,
            counts,
            queued,
            held,
            shed,
        })
    }
}

/// A world's run failed: what survived it (`README.md`, "Errors").
async fn world_failed(failed: Box<Failed>, flight: Flight, driver: Driver) -> ShownError {
    let Failed { error, server, .. } = *failed;
    let kept = match server {
        Some(server) => ShownKept::Shown(Box::new(Shown { server, flight, driver })),
        None if error.kind() == ErrorKind::Cancelled => ShownKept::Nothing,
        None => match end(flight).await {
            Ok(render) => ShownKept::Render(render),
            Err(_) => ShownKept::Nothing,
        },
    };
    ShownError { error, kept }
}

/// Ends the render side as [`Shown::stop`] does: the frame in flight joined, the call in
/// flight awaited, `close` queued last and awaited, the lane dropped. An error of the
/// render side is logged and ends nothing; `Err` is `Cancelled` from the join.
async fn end(flight: Flight) -> Result<Box<dyn RenderSide>> {
    let mut side = match flight {
        Flight::Idle(side) => side,
        Flight::Running(task) => {
            let (side, rendered) = task.await.and_then(|joined| joined)?;
            if let Err(error) = rendered {
                warn!(target: LOG_TARGET, %error, "the frame in flight at the stop failed");
            }
            side
        }
    };
    if let Some((_, call)) = side.call.take()
        && let Err(error) = call.await.and_then(|report| report)
    {
        warn!(target: LOG_TARGET, %error, "the present call in flight at the stop failed");
    }
    let Side { render, present, .. } = *side;
    if let Err(error) = present.run(|present: &mut Box<dyn PresentSide>| present.close()).await {
        warn!(target: LOG_TARGET, %error, "the present side's close did not run");
    }
    drop(present);
    Ok(render)
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
