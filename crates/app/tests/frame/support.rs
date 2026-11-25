//! What the frame's tests share: a game whose rows extract writes, a render side and a
//! present side with no device that log every call into one script and answer what the
//! test scripted, a present thread of the test's own, and a rig that builds a shown world
//! over them and runs it by steps or paced frames, each wait bounded.
//!
//! Every wait of a test is bounded at [`WAIT_BOUND`] by a clock that is no worker's: a
//! condition variable's own timeout, or a thread of the test's (`bounded`). The executor's
//! timers are fired by a frame-critical worker that looks for work, and the failures the
//! bounds are for leave none to fire one.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use game_sim::Orbit;
use lactose_app::{
    Cooked, CrateRegistration, Draw, Finished, FrameCounts, FrameTime, Game, PresentLane,
    PresentReport, PresentSide, Region, RenderSide, Rendered, Shown, ShownError, ShownKept,
    ShownOutput, ShownStats, Start, Submitted, TICK_PERIOD, TargetIndex, WorldConfig, build_shown,
};
use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{Error, ErrorKind, Handle, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Ctx, PresentEvents, RegionId, Res, StableKeyOf, World, system};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_present_api::{
    DebugShape, ExtractPass, InstanceRow, InstanceSink, MeshInstance, PresentFrame,
};
use lactose_rhi_api::{Extent, ImageHandle, QueuePath, TimelineHandle, TimelinePoint};
use lactose_sim::{
    Registration, SessionApplier, SessionEvent, Stage, TickInput, TimeScale, TypeTable,
};
use lactose_task::{ChaosConfig, Executor, JobContext, parking_lane};

use crate::headless::{GAME_SIM, InMemory};

/// How long any wait of a test may take before the test fails with a message naming what
/// never arrived. The bound of a failure: no assertion reads a duration.
pub const WAIT_BOUND: Duration = Duration::from_secs(10);

/// The window's extent in most tests.
pub const E1: Extent = Extent { width: 64, height: 48 };

/// The region the rows enter.
pub const REGION: RegionId = RegionId(1);

/// How many rows the session applier spawns: what extract writes every tick.
pub const ROWS: u64 = 3;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A point of the render side's timeline.
pub fn point(value: u64) -> TimelinePoint {
    let timeline: TimelineHandle = Handle::from_parts(0, core::num::NonZeroU32::MIN);
    TimelinePoint { timeline, value }
}

/// A point of the present side's timeline.
pub fn read_point(value: u64) -> TimelinePoint {
    let timeline: TimelineHandle = Handle::from_parts(1, core::num::NonZeroU32::MIN);
    TimelinePoint { timeline, value }
}

/// The image of a target, as the render side below names it.
pub fn image_of(target: TargetIndex) -> ImageHandle {
    Handle::from_parts(target.index() as u32, core::num::NonZeroU32::MIN)
}

/// The target of `index`.
pub fn target(index: usize) -> TargetIndex {
    TargetIndex::new(index).expect("below `TARGETS`")
}

// ---- The game ----

/// What the tick's system calls, where a test installed it: the tests of criterion 3 log
/// the start of each tick and hold one. Process-wide, as a system's state must be; a test
/// that installs it takes [`SERIAL`] first.
pub static TICK_HOOK: Mutex<Option<Arc<Script>>> = Mutex::new(None);

/// Taken by every test that installs [`TICK_HOOK`], so that tests run in one process by
/// `cargo test` do not share it. Under nextest each test is a process of its own.
pub static SERIAL: Mutex<()> = Mutex::new(());

/// Logs the start of each tick into the installed script, and holds the tick there where
/// the script says. One run a tick: it reads a resource and no column.
#[system(name = "frame_test::log_tick", worlds = [server])]
fn log_tick(ctx: Ctx, _scale: Res<TimeScale>) {
    let hook = lock(&TICK_HOOK).clone();
    if let Some(script) = hook {
        script.tick_started(ctx.tick());
    }
}

/// One presentation event a tick from each row that orbits, so that a test can count what
/// reaches the frame.
#[system(name = "frame_test::mark", worlds = [server])]
fn mark(present: PresentEvents, key: StableKeyOf, _orbit: &Orbit) {
    present.emit(*key, DebugShape::line(Vec3::ZERO, Vec3::ZERO, 0));
}

fn no_types(_: &mut TypeTable) -> Result<()> {
    Ok(())
}

fn register_log_tick(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&LOG_TICK).add(&MARK);
}

static FRAME_TEST: CrateRegistration = CrateRegistration {
    name: "frame-test",
    types: no_types,
    setup: None,
    systems: register_log_tick,
    migrations: None,
};

/// `game-sim` and the tick's log, with an applier that spawns [`ROWS`] rows extract writes:
/// each a `Transform`, a `MeshInstance` and an `Orbit`, so `game_sim::orbit` moves every
/// row every tick.
pub static GAME: Game =
    Game { name: "frame-test", crates: &[GAME_SIM, FRAME_TEST], order: None, sessions: Some(rows) };

fn rows() -> Box<dyn SessionApplier> {
    Box::new(Rows)
}

struct Rows;

impl SessionApplier for Rows {
    fn apply(&mut self, world: &mut World, _: PlayerId, event: SessionEvent) -> Result<()> {
        let SessionEvent::Join { region } = event else { return Ok(()) };
        for n in 0..ROWS {
            let key = StableKey::Net(world.allocate_net_id()?);
            let orbit = Orbit {
                centre: Vec3::ZERO,
                radius: Scalar::from_i32(2),
                period: 60,
                phase: (n * 20) as u32,
            };
            let mesh = MeshInstance::new(AssetRef::new(AssetId::from_bits(n + 1)));
            world.spawn(key, region, (Transform::at(Vec3::ZERO), mesh, orbit))?;
        }
        Ok(())
    }
}

/// The frames a run feeds: the region and player 1's join in the first, nothing after.
pub fn frames(cooked: &InMemory) -> Box<dyn FnMut(Tick) -> Option<TickInput> + Send> {
    let mut first = Some(cooked.fresh().expect("the region's bundle"));
    Box::new(move |_| {
        let mut input = TickInput::default();
        if let Some(bundle) = first.take() {
            input.loaded.insert(REGION, bundle);
            input.sessions.insert(PlayerId(1), SessionEvent::Join { region: REGION });
        }
        Some(input)
    })
}

// ---- The script: what each side logs and answers ----

/// One call, as the sides and the tick's system log it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Call {
    /// A tick's system started.
    Tick(Tick),
    /// `RenderSide::sink`.
    Sink,
    /// A pass began writing the sink.
    Begin(ExtractPass),
    /// The pass ended, having written `rows`.
    End { rows: u32 },
    /// `RenderSide::open`.
    Open,
    /// `RenderSide::present`: the newer snapshot's tick, the frame's time, and how many
    /// presentation events the frame carried.
    Present { newer: Tick, time: FrameTime, events: usize },
    /// `RenderSide::draw` began.
    Draw(Draw),
    /// `RenderSide::draw` returned.
    Drawn,
    /// `RenderSide::submit`, of the frame that drew `target`, at `point`.
    Submit { target: TargetIndex, point: TimelinePoint },
    /// `PresentSide::present` began.
    Called(Finished),
    /// `PresentSide::present` returned this.
    Returned { finished: Finished, report: Option<PresentReport> },
    /// `PresentSide::close`.
    Close,
}

impl Call {
    /// Whether the call is the render side's own, made by `lactose-app` on the frame's side
    /// and not on the present thread or in a tick.
    pub fn is_render(self) -> bool {
        matches!(
            self,
            Self::Sink | Self::Open | Self::Present { .. } | Self::Draw(_) | Self::Submit { .. }
        )
    }
}

/// What `open` answers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Answer {
    Ready,
    Wait,
    Fail(ErrorKind),
}

/// A gate a side's call waits at while the test holds it.
#[derive(Default)]
pub struct Gate {
    /// The test holds the gate.
    pub held: bool,
    /// Calls that have reached the gate since it was made.
    pub reached: u32,
}

/// What the test scripts and reads. Every answer list is popped one call at a time; an
/// empty list answers the default.
#[derive(Default)]
pub struct Plan {
    pub log: Vec<Call>,
    /// `open`'s answers; `Ready` by default.
    pub opens: VecDeque<Answer>,
    /// `draw`'s answers; the default counts by default.
    pub draws: VecDeque<Result<FrameCounts>>,
    /// The region `submit` leaves, or its error; `Ready` by default.
    pub submits: VecDeque<Result<Region>>,
    /// The present side's reports, or its errors; a read, shown, no count by default.
    pub reports: VecDeque<Result<PresentReport>>,
    pub draw_gate: Gate,
    pub submit_gate: Gate,
    pub present_gate: Gate,
    /// The tick whose system meets the next `draw` (criterion 3(b)): that draw waits until
    /// the tick's system has started, and the system until the draw has.
    pub rendezvous: Option<Tick>,
    pub draw_met: bool,
    pub tick_met: bool,
    /// Calls entered on the present thread, `close` among them.
    pub entered: u64,
    /// Calls the present thread's drains have run and replied to.
    pub drained: u64,
    /// Points handed out by `submit` and `open`, and reads by the present side.
    pub points: u64,
    pub reads: u64,
    pub closed: bool,
}

/// The script both sides and the tick's system share with the test.
#[derive(Default)]
pub struct Script {
    plan: Mutex<Plan>,
    changed: Condvar,
}

impl Script {
    /// Changes the plan and wakes every waiter.
    pub fn with<R>(&self, change: impl FnOnce(&mut Plan) -> R) -> R {
        let value = change(&mut lock(&self.plan));
        self.changed.notify_all();
        value
    }

    /// Waits within [`WAIT_BOUND`] until `done` holds, and fails naming `what` past it.
    pub fn until(&self, what: &str, mut done: impl FnMut(&Plan) -> bool) {
        let plan = lock(&self.plan);
        let (_plan, waited) = self
            .changed
            .wait_timeout_while(plan, WAIT_BOUND, |plan| !done(plan))
            .unwrap_or_else(PoisonError::into_inner);
        assert!(!waited.timed_out(), "{what} did not arrive within its bound");
    }

    fn log(&self, call: Call) {
        self.with(|plan| plan.log.push(call));
    }

    /// Reaches `gate` and waits while the test holds it.
    fn pass(&self, gate: fn(&mut Plan) -> &mut Gate, what: &str) {
        self.with(|plan| gate(plan).reached += 1);
        let plan = lock(&self.plan);
        let (_plan, waited) = self
            .changed
            .wait_timeout_while(plan, WAIT_BOUND, |plan| gate(plan).held)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(!waited.timed_out(), "the test never released {what}");
    }

    /// Waits within the bound until `met` holds, and fails naming `side` past it: the
    /// bound is the condition variable's own timeout, which no worker keeps.
    fn meet(&self, side: &str, met: fn(&Plan) -> bool) {
        let plan = lock(&self.plan);
        let (_plan, waited) = self
            .changed
            .wait_timeout_while(plan, WAIT_BOUND, |plan| !met(plan))
            .unwrap_or_else(PoisonError::into_inner);
        assert!(!waited.timed_out(), "{side} never arrived at the rendezvous");
    }

    fn tick_started(&self, tick: Tick) {
        self.log(Call::Tick(tick));
        let meets = self.with(|plan| {
            let meets = plan.rendezvous == Some(tick);
            plan.tick_met |= meets;
            meets
        });
        if meets {
            self.meet("the frame's draw", |plan| plan.draw_met);
        }
    }

    fn draw_started(&self) {
        let meets = self.with(|plan| {
            let meets = plan.rendezvous.is_some() && !plan.draw_met;
            plan.draw_met |= meets;
            meets
        });
        if meets {
            self.meet("a system of the next tick", |plan| plan.tick_met);
        }
    }

    /// The log so far.
    pub fn calls(&self) -> Vec<Call> {
        lock(&self.plan).log.clone()
    }

    /// Takes the log so far.
    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut lock(&self.plan).log)
    }

    pub fn hold_draws(&self, held: bool) {
        self.with(|plan| plan.draw_gate.held = held);
    }

    pub fn hold_submits(&self, held: bool) {
        self.with(|plan| plan.submit_gate.held = held);
    }

    pub fn hold_presents(&self, held: bool) {
        self.with(|plan| plan.present_gate.held = held);
    }

    /// Waits until `count` draws have reached the draw gate.
    pub fn await_draws(&self, count: u32) {
        self.until("a draw at the gate", |plan| plan.draw_gate.reached >= count);
    }

    /// Waits until `count` present calls have reached their gate.
    pub fn await_presents(&self, count: u32) {
        self.until("a present call at the gate", |plan| plan.present_gate.reached >= count);
    }

    /// Waits until `count` submits have reached their gate.
    pub fn await_submits(&self, count: u32) {
        self.until("a submit at the gate", |plan| plan.submit_gate.reached >= count);
    }

    pub fn open_answers(&self, answers: &[Answer]) {
        self.with(|plan| plan.opens.extend(answers.iter().copied()));
    }

    pub fn draw_answers(&self, answers: impl IntoIterator<Item = Result<FrameCounts>>) {
        self.with(|plan| plan.draws.extend(answers));
    }

    pub fn submit_answers(&self, answers: impl IntoIterator<Item = Result<Region>>) {
        self.with(|plan| plan.submits.extend(answers));
    }

    pub fn report_answers(&self, answers: impl IntoIterator<Item = Result<PresentReport>>) {
        self.with(|plan| plan.reports.extend(answers));
    }
}

// ---- The render side ----

/// A render side with no device: it logs each call and answers what the script says.
pub struct ScriptedRender {
    script: Arc<Script>,
    shared: bool,
    drawn: Option<Draw>,
}

impl ScriptedRender {
    pub fn new(script: Arc<Script>, shared: bool) -> Self {
        Self { script, shared, drawn: None }
    }
}

struct LoggingSink {
    script: Arc<Script>,
    rows: AtomicU32,
}

impl InstanceSink for LoggingSink {
    fn begin(&mut self, pass: ExtractPass) {
        self.rows.store(0, Ordering::Relaxed);
        self.script.log(Call::Begin(pass));
    }

    fn removed(&mut self, _: &[StableKey]) {}

    fn write(&self, rows: &[InstanceRow]) {
        self.rows.fetch_add(rows.len() as u32, Ordering::Relaxed);
    }

    fn end(&mut self) {
        self.script.log(Call::End { rows: self.rows.load(Ordering::Relaxed) });
    }
}

impl RenderSide for ScriptedRender {
    fn present_queue(&self) -> QueuePath {
        if self.shared { QueuePath::Shared } else { QueuePath::Own }
    }

    fn open(&mut self, _: &mut JobContext<'_>) -> Result<Region> {
        self.script.log(Call::Open);
        self.script.with(|plan| match plan.opens.pop_front().unwrap_or(Answer::Ready) {
            Answer::Ready => Ok(Region::Ready),
            Answer::Wait => {
                plan.points += 1;
                Ok(Region::Wait(point(plan.points)))
            }
            Answer::Fail(kind) => Err(Error::new(kind, "the scripted open failed")),
        })
    }

    fn sink(&mut self) -> Box<dyn InstanceSink> {
        self.script.log(Call::Sink);
        Box::new(LoggingSink { script: Arc::clone(&self.script), rows: AtomicU32::new(0) })
    }

    fn present(
        &mut self,
        _: &mut JobContext<'_>,
        frame: &PresentFrame<'_>,
        time: FrameTime,
    ) -> Result<()> {
        let events = frame.events().emitted.len();
        self.script.log(Call::Present { newer: frame.newer().tick(), time, events });
        Ok(())
    }

    fn draw(&mut self, _: &mut JobContext<'_>, draw: Draw) -> Result<FrameCounts> {
        self.script.log(Call::Draw(draw));
        self.script.draw_started();
        self.script.pass(|plan| &mut plan.draw_gate, "a held draw");
        self.drawn = Some(draw);
        let counts =
            self.script.with(|plan| plan.draws.pop_front().unwrap_or(Ok(FrameCounts::default())));
        self.script.log(Call::Drawn);
        counts
    }

    fn submit(&mut self) -> Result<Submitted> {
        self.script.pass(|plan| &mut plan.submit_gate, "a held submit");
        let draw = self.drawn.take().expect("`submit` follows a `draw`");
        let (point, region) = self.script.with(|plan| {
            plan.points += 1;
            (point(plan.points), plan.submits.pop_front().unwrap_or(Ok(Region::Ready)))
        });
        self.script.log(Call::Submit { target: draw.target, point });
        let region = region?;
        Ok(Submitted { image: image_of(draw.target), point, region })
    }
}

// ---- The present side and its thread ----

struct ScriptedPresent {
    script: Arc<Script>,
}

impl PresentSide for ScriptedPresent {
    fn present(&mut self, finished: Finished) -> Result<PresentReport> {
        self.script.with(|plan| plan.entered += 1);
        self.script.log(Call::Called(finished));
        self.script.pass(|plan| &mut plan.present_gate, "a held present call");
        let answer = self.script.with(|plan| {
            plan.reads += 1;
            let read = read_point(plan.reads);
            plan.reports.pop_front().unwrap_or(Ok(PresentReport {
                read: Some(read),
                shown: true,
                waiting: None,
            }))
        });
        self.script.log(Call::Returned { finished, report: answer.as_ref().ok().copied() });
        answer
    }

    fn close(&mut self) {
        self.script.with(|plan| {
            plan.entered += 1;
            plan.closed = true;
        });
        self.script.log(Call::Close);
    }
}

/// A present thread of the test's own, which drains its lane until `close` has run.
fn present_thread(script: Arc<Script>) -> (PresentLane, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("present".to_owned())
        .spawn(move || {
            let (lane, mut pump) = parking_lane::<Box<dyn PresentSide>>("present");
            sender.send(lane).expect("the rig waits for the lane");
            let mut state: Box<dyn PresentSide> =
                Box::new(ScriptedPresent { script: Arc::clone(&script) });
            loop {
                pump.wait();
                let ran = pump.drain(&mut state) as u64;
                let closed = script.with(|plan| {
                    plan.drained += ran;
                    plan.closed
                });
                if closed {
                    break;
                }
            }
        })
        .expect("the present thread");
    (receiver.recv().expect("the present thread's lane"), thread)
}

// ---- Bounded waits ----

/// What a bounded wait shares with the thread that keeps its bound.
struct Expiry {
    passed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// A wait raced against a clock that is no worker's: `None` once the bound passed first.
struct Bounded<W> {
    wait: Pin<Box<W>>,
    expiry: Arc<Expiry>,
}

impl<W: Future> Future for Bounded<W> {
    type Output = Option<W::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Read before the wait is polled: a wait that is ready only when the keeper's wake
        // polls it, its own waker having woken nothing, fails at the bound.
        if self.expiry.passed.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        if let Poll::Ready(value) = self.wait.as_mut().poll(cx) {
            return Poll::Ready(Some(value));
        }
        // The waker is left before the flag is read, and the keeping thread raises the
        // flag before it takes the waker: a bound that passes between the two is seen here
        // or woken there.
        *lock(&self.expiry.waker) = Some(cx.waker().clone());
        if self.expiry.passed.load(Ordering::Acquire) { Poll::Ready(None) } else { Poll::Pending }
    }
}

/// `wait`, driven by `Executor::block_on` on the calling thread, within [`WAIT_BOUND`],
/// kept by a thread of the test's own; past the bound the test fails naming `what`.
pub fn bounded<W: Future>(executor: &Executor, wait: W, what: &str) -> W::Output {
    let expiry = Arc::new(Expiry { passed: AtomicBool::new(false), waker: Mutex::new(None) });
    let (resolved, ended) = mpsc::channel::<()>();
    let keeper = {
        let expiry = Arc::clone(&expiry);
        std::thread::Builder::new()
            .name("wait-bound".to_owned())
            .spawn(move || {
                if ended.recv_timeout(WAIT_BOUND) == Err(mpsc::RecvTimeoutError::Timeout) {
                    expiry.passed.store(true, Ordering::Release);
                    if let Some(waker) = lock(&expiry.waker).take() {
                        waker.wake();
                    }
                }
            })
            .expect("the thread that keeps the bound")
    };
    let value = executor.block_on(Bounded { wait: Box::pin(wait), expiry });
    drop(resolved);
    keeper.join().expect("the thread that keeps the bound");
    value.unwrap_or_else(|| panic!("{what} did not resolve within its bound"))
}

/// Runs `release` on a thread of the test's own after `delay`.
pub fn later(delay: Duration, release: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("release".to_owned())
        .spawn(move || {
            std::thread::sleep(delay);
            release();
        })
        .expect("the releasing thread")
}

// ---- The rig ----

/// Where a paced frame's `now` lies against the executor's clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum When {
    /// An hour ahead: the frame's limit is an hour off, and every wait of step 2 and 3
    /// lasts until what it waits for resolves.
    Ahead,
    /// The clock as it reads: the limit of a frame that is behind has passed.
    Now,
    /// A second behind: the frame's limit has passed, and every wait looks once.
    Passed,
}

/// How a rig is built.
#[derive(Clone, Copy)]
pub struct Setup {
    pub frame_workers: u32,
    /// Whether the present role shares the graphics queue.
    pub shared: bool,
    pub log_retention: Option<u32>,
    pub chaos: Option<ChaosConfig>,
}

impl Setup {
    pub const fn new(frame_workers: u32, shared: bool) -> Self {
        Self { frame_workers, shared, log_retention: None, chaos: None }
    }
}

/// A shown world over the scripted sides, with its executor and its present thread.
pub struct Rig {
    pub executor: Executor,
    pub script: Arc<Script>,
    shown: Option<Shown>,
    frames: Box<dyn FnMut(Tick) -> Option<TickInput> + Send>,
    present: Option<JoinHandle<()>>,
    /// Present calls queued, as the frames reported them.
    queued: u64,
}

impl Rig {
    pub fn new(frame_workers: u32, shared: bool) -> Self {
        Self::with(Setup::new(frame_workers, shared))
    }

    pub fn with(setup: Setup) -> Self {
        let mut builder =
            Executor::builder().frame_workers(setup.frame_workers).background_workers(1);
        if let Some(chaos) = setup.chaos {
            builder = builder.chaos(chaos);
        }
        let executor = builder.build().expect("the executor");
        let script = Arc::new(Script::default());
        Self::over(executor, script, setup)
    }

    /// A rig over `executor`, logging into `script`.
    pub fn over(executor: Executor, script: Arc<Script>, setup: Setup) -> Self {
        let (lane, present) = present_thread(Arc::clone(&script));
        let cooked = Arc::new(InMemory::new(REGION).expect("the content"));
        let frames = frames(&cooked);
        let mut config = WorldConfig::shown();
        if let Some(retention) = setup.log_retention {
            config.log_retention = retention;
        }
        let render = Box::new(ScriptedRender::new(Arc::clone(&script), setup.shared));
        let shown = build_shown(
            &executor,
            &GAME,
            config,
            Start::Fresh,
            cooked as Arc<dyn Cooked>,
            render,
            lane,
        )
        .expect("the shown world");
        Self { executor, script, shown: Some(shown), frames, present: Some(present), queued: 0 }
    }

    pub fn shown(&mut self) -> &mut Shown {
        self.shown.as_mut().expect("the rig holds its world")
    }

    pub fn stats(&self) -> ShownStats {
        self.shown.as_ref().expect("the rig holds its world").stats()
    }

    fn count(&mut self, rendered: Option<Rendered>) {
        if let Some(Rendered::Drew { queued: true, .. }) = rendered {
            self.queued += 1;
        }
    }

    fn keep(&mut self, error: ShownError) -> ShownError {
        match error.kept {
            ShownKept::Shown(shown) => {
                self.shown = Some(*shown);
                ShownError { error: error.error, kept: ShownKept::Nothing }
            }
            kept => ShownError { error: error.error, kept },
        }
    }

    /// One step and its frame, joined.
    pub fn try_step(
        &mut self,
        window: Option<Extent>,
    ) -> core::result::Result<Rendered, ShownError> {
        let shown = self.shown.take().expect("the rig holds its world");
        let tick = Tick(shown.server().tick().0 + 1);
        let frame = (self.frames)(tick).expect("a frame for every tick");
        let time = FrameTime { now: Instant::now(), fraction: 1.0 };
        let run = shown.step(&self.executor, frame, time, window);
        match bounded(&self.executor, run, "a step") {
            Ok((shown, _, rendered)) => {
                self.shown = Some(shown);
                self.count(Some(rendered));
                Ok(rendered)
            }
            Err(error) => Err(self.keep(error)),
        }
    }

    pub fn step(&mut self, window: Option<Extent>) -> Rendered {
        self.try_step(window).expect("the step")
    }

    /// The `now` of `when`, and the pacer resynced so that `ticks` ticks are due at it
    /// with `owed` of the next one owed, or, past the cap, so that the frame is behind.
    pub fn now(&mut self, when: When, ticks: u32, owed: Duration) -> Instant {
        let clock = Instant::now();
        let now = match when {
            When::Ahead => clock + Duration::from_secs(3600),
            When::Now => clock,
            When::Passed => clock.checked_sub(Duration::from_secs(1)).expect("a second of uptime"),
        };
        let owed = TICK_PERIOD * ticks + owed;
        let from = now.checked_sub(owed).expect("uptime enough for the owed ticks");
        self.shown().server_mut().resync(from);
        now
    }

    /// One paced frame at `when`, with `ticks` ticks due and half a period of the next
    /// owed.
    pub fn try_frame(
        &mut self,
        when: When,
        ticks: u32,
        window: Option<Extent>,
    ) -> core::result::Result<ShownOutput, ShownError> {
        self.try_frame_owing(when, ticks, TICK_PERIOD / 2, window)
    }

    /// One paced frame at `when`, with `ticks` ticks due and `owed` of the next.
    pub fn try_frame_owing(
        &mut self,
        when: When,
        ticks: u32,
        owed: Duration,
        window: Option<Extent>,
    ) -> core::result::Result<ShownOutput, ShownError> {
        let now = self.now(when, ticks, owed);
        self.frame_at(now, window)
    }

    /// One paced frame whose `now` is `ahead` of the clock, with `ticks` ticks due and half
    /// a period of the next owed: its limit falls a little after `ahead`, so a wait that has
    /// begun is ended by the executor's timer.
    pub fn try_frame_soon(
        &mut self,
        ahead: Duration,
        ticks: u32,
        window: Option<Extent>,
    ) -> core::result::Result<ShownOutput, ShownError> {
        let now = Instant::now() + ahead;
        let from = now.checked_sub(TICK_PERIOD * ticks + TICK_PERIOD / 2).expect("uptime");
        self.shown().server_mut().resync(from);
        self.frame_at(now, window)
    }

    fn frame_at(
        &mut self,
        now: Instant,
        window: Option<Extent>,
    ) -> core::result::Result<ShownOutput, ShownError> {
        let shown = self.shown.take().expect("the rig holds its world");
        let frames = &mut self.frames;
        let run = shown.frame(&self.executor, now, &mut **frames, window);
        match bounded(&self.executor, run, "a paced frame") {
            Ok((shown, output)) => {
                self.shown = Some(shown);
                self.count(output.joined);
                Ok(output)
            }
            Err(error) => Err(self.keep(error)),
        }
    }

    pub fn frame(&mut self, when: When, ticks: u32, window: Option<Extent>) -> ShownOutput {
        self.try_frame(when, ticks, window).expect("the frame")
    }

    /// Joins the frame in flight.
    pub fn flush(&mut self) -> Option<Rendered> {
        let shown = self.shown.take().expect("the rig holds its world");
        let (shown, rendered) =
            bounded(&self.executor, shown.flush(), "a flush").expect("the flush");
        self.shown = Some(shown);
        self.count(rendered);
        rendered
    }

    /// Waits until every present call queued by the frames joined so far has returned and
    /// been replied to. The render task of a paced frame still in flight may queue one
    /// after: flush first where that call matters.
    pub fn settle(&self) {
        let queued = self.queued;
        self.script.until("the present calls queued", |plan| plan.drained >= queued);
    }

    /// Ends the render side, joins the present thread and shuts the executor down.
    pub fn stop(mut self) -> Box<dyn RenderSide> {
        let shown = self.shown.take().expect("the rig holds its world");
        let stopped = bounded(&self.executor, shown.stop(), "the stop").expect("the stop");
        // The present thread ends at `close`: waited for within the bound, so that a stop
        // that never closes fails here and does not hang at the join.
        self.script.until("the present side's close", |plan| plan.closed);
        if let Some(present) = self.present.take() {
            present.join().expect("the present thread");
        }
        drop(stopped.server);
        let render = stopped.render;
        self.executor.clone().shutdown();
        render
    }

    /// The world, taken out: a test that drives a run itself.
    pub fn take(&mut self) -> Shown {
        self.shown.take().expect("the rig holds its world")
    }
}

/// What a drawn frame reported, or a panic naming what it was.
pub fn drew(rendered: Option<Rendered>) -> (TargetIndex, bool, bool, bool) {
    match rendered {
        Some(Rendered::Drew { target, queued, held, shed, .. }) => (target, queued, held, shed),
        other => panic!("a frame that drew, not {other:?}"),
    }
}
