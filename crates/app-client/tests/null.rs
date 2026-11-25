//! The wiring that ships, on `lactose-rhi-null`: the client's render side and present side
//! stepped on each queue layout with the extent going E1, hidden, E2 and E1 again, through
//! the presents null stalls, and refused nothing; every target made destroyed, a target
//! whose make failed among them; the frame's wait for the present's last read of a target it
//! takes back, with every read kept pending; the fixed-frame mode's readback and the
//! placements it is held to; a clean shutdown that logs nothing; and a lost device, which
//! ends the run with its error and leaves a shutdown that completes.
//!
//! Null reaches drawn, hidden, held and dropped frames through this wiring, and the test's
//! device tears a frame by refusing a target's make. A frame that waits for a region and one
//! torn by a refused submit are not reached: null runs each submit as it is made, and its
//! control offers a loss and a stall of presents alone, neither of which leaves a frame's
//! point unreached or refuses a call. `lactose-app`'s `tests/frame/states.rs` drives both
//! over a scripted render side.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use lactose_app::{
    PresentSide, RenderSide, Rendered, Shown, ShownError, ShownKept, Stopped, TICK_PERIOD, Workers,
    WorldConfig, start_executor,
};
use lactose_app_client::{
    ClientRender, FIXED_SAMPLE, Frames, LastFrame, PresentBarrier, Wired, wire,
};
use lactose_core::{ErrorKind, Result};
use lactose_input_host::LatestSample;
use lactose_platform::PresentThread;
use lactose_renderer::InstanceRecord;
use lactose_rhi_api::{Capabilities, Device, Extent, QueueLayout, SwapchainDesc};
use lactose_rhi_null::Control;
use lactose_task::{ExecutionLog, Executor};

use crate::support::{
    Said, Watch, Watched, WatchedDevice, bounded, lock, null_device, null_shaders, warnings,
};

const E1: Extent = Extent { width: 64, height: 48 };
const E2: Extent = Extent { width: 80, height: 40 };

/// A shown world on null, stepped one joined frame at a time, as the fixed-frame mode
/// steps it.
pub struct Run {
    pub executor: Executor,
    pub control: Control,
    pub said: Arc<Said>,
    /// What the swapchain and the device were asked.
    pub watch: Arc<Watch>,
    shown: Option<Shown>,
    present: Option<PresentThread<Box<dyn PresentSide>>>,
    frames: Frames,
    input: Arc<LatestSample>,
    /// A barrier behind the present calls ([`Wired::barrier`]).
    barrier: PresentBarrier,
    start: Instant,
    steps: u32,
}

/// How a [`Run`] is made beside its layout.
#[derive(Clone, Copy, Default)]
pub struct Setup {
    /// Every second acquire answered with no image.
    pub skip: bool,
    /// Every present's read kept pending until a wait sees it ([`Watch::lag`]).
    pub lag: bool,
    /// The fixed-frame mode's readback, at this extent.
    pub readback: Option<Extent>,
}

impl Run {
    pub fn new(capabilities: Capabilities, workers: Workers, world: WorldConfig) -> Self {
        Self::with(capabilities, workers, world, Setup::default())
    }

    /// A run whose device is a [`WatchedDevice`] and whose swapchain is [`Watched`], as
    /// `setup` says.
    pub fn with(
        capabilities: Capabilities,
        workers: Workers,
        world: WorldConfig,
        setup: Setup,
    ) -> Self {
        let executor = start_executor(workers).expect("an executor");
        let (inner, control, said) = null_device(capabilities);
        let platform = Arc::clone(executor.platform());
        let watch = Arc::new(Watch::default());
        watch.lag.store(setup.lag, Ordering::Release);
        let device: Arc<dyn Device> =
            Arc::new(WatchedDevice { inner: Arc::clone(&inner), watch: Arc::clone(&watch) });
        // As the binary makes it, beside the device, at the window's first size.
        let inner = inner.create_swapchain(&SwapchainDesc { extent: E1 }).expect("a swapchain");
        let swapchain = Box::new(Watched { inner, watch: Arc::clone(&watch), skip: setup.skip });
        let shaders = null_shaders();
        let wired =
            wire(&executor, &*platform, &device, swapchain, &shaders, world, setup.readback);
        let Wired { shown, present, frames, input, barrier } = wired.expect("wired");
        Self {
            executor,
            control,
            said,
            watch,
            shown: Some(shown),
            present: Some(present),
            frames,
            input,
            barrier,
            start: Instant::now(),
            steps: 0,
        }
    }

    pub fn shown(&self) -> &Shown {
        self.shown.as_ref().expect("a shown world")
    }

    /// One step at `extent`, joined, at a time one tick after the last: one tick a step.
    pub fn try_step(&mut self, extent: Option<Extent>) -> Result<Rendered, ShownError> {
        self.input.publish(FIXED_SAMPLE);
        let shown = self.shown.take().expect("a shown world");
        let input = self.frames.next(shown.server().tick());
        let time =
            lactose_app::FrameTime { now: self.start + TICK_PERIOD * self.steps, fraction: 0.5 };
        self.steps += 1;
        let stepped =
            bounded(&self.executor, shown.step(&self.executor, input, time, extent), "a step");
        let (shown, output, rendered) = stepped?;
        assert!(output.failures.is_empty(), "step {}: {:?}", self.steps, output.failures);
        self.shown = Some(shown);
        Ok(rendered)
    }

    pub fn step(&mut self, extent: Option<Extent>) -> Rendered {
        let steps = self.steps;
        self.try_step(extent).unwrap_or_else(|failure| panic!("step {steps}: {:?}", failure.error))
    }

    /// One step, then [`settle`](Self::settle): the next step finds no present call in
    /// flight. Asserted names rest on it, never on a pace.
    pub fn settled(&mut self, extent: Option<Extent>) -> &'static str {
        let rendered = self.step(extent);
        self.settle();
        name(&rendered)
    }

    /// Waits until the present call of every frame queued so far has begun and made its
    /// last swapchain call ([`Watch::settle`]), then for a call queued on the present lane
    /// behind them, which runs once each of them has been replied to.
    pub fn settle(&self) {
        let queued = usize::try_from(self.shown().stats().queued).expect("a count");
        self.watch.settle(queued);
        let barrier = self.barrier.passed();
        bounded(&self.executor, barrier, "the present lane's barrier").expect("the barrier");
    }

    /// [`stop`](Self::stop), reading the fixed-frame mode's last frame from the render side
    /// after `Shown::stop` and before its drop, as the binary does.
    pub fn stop_reading(mut self) -> Result<Option<LastFrame>> {
        let shown = self.shown.take().expect("a shown world");
        let stopped = bounded(&self.executor, shown.stop(), "the stop").expect("stopped");
        if let Some(present) = self.present.take() {
            drop(bounded(&self.executor, present.stop(), "the present thread's stop"));
        }
        let render: &dyn std::any::Any = &*stopped.render;
        let last = render.downcast_ref::<ClientRender>().expect("the client's render side");
        let read = last.last_frame();
        self.finish(Some(stopped));
        read
    }

    /// The shutdown's order: `Shown::stop`, the present thread, the render side's drop on
    /// this thread, which is no worker, the world, then the executor. Returns the execution
    /// log chaos recorded, read before the executor's shutdown.
    pub fn stop(mut self) -> Option<ExecutionLog> {
        let shown = self.shown.take().expect("a shown world");
        let stopped = bounded(&self.executor, shown.stop(), "the stop").expect("stopped");
        self.finish(Some(stopped))
    }

    /// The shutdown after a step that failed: what the failure kept is stopped as `stop`
    /// stops a shown world.
    pub fn stop_after(mut self, failure: ShownError) -> Option<ExecutionLog> {
        let stopped = match failure.kept {
            ShownKept::Shown(kept) => {
                Some(bounded(&self.executor, kept.stop(), "the stop").expect("stopped"))
            }
            ShownKept::Render(render) => {
                self.finish_render(render);
                None
            }
            ShownKept::Nothing => None,
        };
        self.finish(stopped)
    }

    fn finish_render(&mut self, render: Box<dyn RenderSide>) {
        if let Some(present) = self.present.take() {
            drop(bounded(&self.executor, present.stop(), "the present thread's stop"));
        }
        drop(render);
    }

    fn finish(mut self, stopped: Option<Stopped>) -> Option<ExecutionLog> {
        if let Some(present) = self.present.take() {
            drop(bounded(&self.executor, present.stop(), "the present thread's stop"));
        }
        if let Some(Stopped { server, render }) = stopped {
            drop(render);
            server.shutdown();
        }
        let log = self.executor.execution_log();
        self.executor.shutdown();
        log
    }
}

/// What a step did, by name.
pub fn name(rendered: &Rendered) -> &'static str {
    match rendered {
        Rendered::Drew { queued: true, .. } => "drawn",
        Rendered::Drew { held: true, .. } => "held",
        Rendered::Drew { shed: true, .. } => "shed",
        Rendered::Drew { .. } => "dropped",
        Rendered::Waited(_) => "waited",
        Rendered::Hidden => "hidden",
        Rendered::Torn => "torn",
        Rendered::Busy => "busy",
    }
}

fn layouts() -> [(&'static str, Capabilities); 2] {
    [
        ("one queue", Capabilities { queues: QueueLayout::ONE, ..Capabilities::LEAST }),
        ("several", Capabilities { queues: QueueLayout::SEVERAL, ..Capabilities::LEAST }),
    ]
}

fn workers() -> Workers {
    Workers { frame_workers: Some(2), ..Workers::default() }
}

/// On each layout: E1, hidden, E2 and E1 again, each extent's frames drawn and queued, the
/// targets made anew at each change and the swapchain resized; then presents stalled over
/// E1, E2 and E1 again, which on one queue holds frames behind the present call in flight
/// and with a present queue of its own drops them, so that an extent's frames may reach no
/// present and the present side meets new targets at an extent it already has; and never
/// sheds one, since null reports no display time; then the stall let go and frames queued
/// again. Null refuses nothing and says nothing throughout.
///
/// Before the stall and after it each step waits for the present calls to end
/// ([`Run::settled`]), so each frame is queued. A stalled present's acquire finds no image
/// and returns at null's bound, a second, so a call is in flight for a second at a time
/// while eight steps run: what is asserted there is that some frame was held or dropped
/// among them, never which.
#[test]
fn each_layout_steps_e1_e2_e1_through_stalled_presents_and_refuses_nothing() {
    for (layout, capabilities) in layouts() {
        let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
        let mut seen = Vec::new();
        let extents = [Some(E1), Some(E1), Some(E1), None, Some(E2), Some(E2), Some(E1), Some(E1)];
        for extent in extents {
            seen.push(run.settled(extent));
        }
        assert_eq!(
            seen,
            ["drawn", "drawn", "drawn", "hidden", "drawn", "drawn", "drawn", "drawn"],
            "{layout}"
        );
        assert!(run.control.stall(true), "{layout}: the device is gone");
        let stalled_extents = [E1, E1, E2, E2, E2, E2, E1, E1];
        let stalled: Vec<_> =
            stalled_extents.into_iter().map(|extent| name(&run.step(Some(extent)))).collect();
        let (kept_back, never) =
            if layout == "one queue" { ("held", "dropped") } else { ("dropped", "held") };
        assert!(stalled.contains(&kept_back), "{layout}: {stalled:?}");
        assert!(!stalled.contains(&never), "{layout}: {stalled:?}");
        assert!(
            stalled.iter().all(|name| ["drawn", "held", "dropped"].contains(name)),
            "{layout}: {stalled:?}"
        );
        assert!(run.control.stall(false), "{layout}: the device is gone");
        run.settle();
        let after: Vec<_> = (0..2).map(|_| run.settled(Some(E1))).collect();
        assert_eq!(after, ["drawn", "drawn"], "{layout}");
        assert_eq!(
            run.said.count.load(Ordering::Relaxed),
            0,
            "{layout}: {:?}",
            lock(&run.said.texts)
        );
        // A stalled present holds the resize or the present after it past null's bound,
        // which the side meets as pending and takes back; nothing else is refused.
        let refusals = lock(&run.watch.refusals).clone();
        let other: Vec<_> = refusals.iter().filter(|text| !text.contains(": Pending: ")).collect();
        assert!(other.is_empty(), "{layout}: {other:?}");
        assert!(run.stop().is_none(), "no chaos, no log");
    }
}

/// On each layout, E1, E2 and E1 again, each extent's frames queued: every target the render
/// side made is destroyed by the time the render side is dropped, the old one of each change
/// of extent among them, since a target that was retired and never destroyed would be
/// destroyed by nothing (`retired-never-destroyed`).
#[test]
fn each_layout_destroys_every_target_it_made() {
    for (layout, capabilities) in layouts() {
        let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
        for extent in [E1, E1, E1, E2, E2, E2, E1, E1, E1] {
            assert_eq!(run.settled(Some(extent)), "drawn", "{layout}");
        }
        let watch = Arc::clone(&run.watch);
        assert!(run.stop().is_none(), "no chaos, no log");
        let made = lock(&watch.made).clone();
        let destroyed = lock(&watch.destroyed).clone();
        assert!(made.len() >= 6, "{layout}: two targets at each of three extents: {made:?}");
        let left: Vec<_> = made.iter().filter(|image| !destroyed.contains(image)).collect();
        assert!(left.is_empty(), "{layout}: never destroyed: {left:?}");
        assert_eq!(*lock(&watch.refusals), Vec::<String>::new(), "{layout}");
    }
}

/// On each layout, frames at E1, then at E2 with the make of the frame's new target
/// refused, then E2 and E1 again: the frame whose make failed is torn, and the target it
/// retired is in no slot, so that no later frame retires it again or draws it; every later
/// frame is drawn and queued, and every target made is destroyed by the end, with nothing
/// refused (`r3-m3-slot-kept`).
#[test]
fn a_target_whose_make_fails_at_a_change_of_extent_leaves_its_old_image_in_no_slot() {
    for (layout, capabilities) in layouts() {
        let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(run.settled(Some(E1)));
        }
        run.watch.fail_make.store(true, Ordering::Release);
        seen.push(run.settled(Some(E2)));
        for extent in [E2, E2, E2, E1, E1, E1] {
            seen.push(run.settled(Some(extent)));
        }
        let watch = Arc::clone(&run.watch);
        assert!(run.stop().is_none(), "no chaos, no log");
        assert_eq!(seen[..4], ["drawn", "drawn", "drawn", "torn"], "{layout}: {seen:?}");
        assert!(seen[4..].iter().all(|step| *step == "drawn"), "{layout}: {seen:?}");
        let made = lock(&watch.made).clone();
        let destroyed = lock(&watch.destroyed).clone();
        let left: Vec<_> = made.iter().filter(|image| !destroyed.contains(image)).collect();
        assert!(left.is_empty(), "{layout}: never destroyed: {left:?}");
        assert_eq!(*lock(&watch.refusals), Vec::<String>::new(), "{layout}");
    }
}

/// On each layout, frames at E1 with every present's read kept pending until a wait sees it
/// ([`Watch::lag`]): from the third frame on, each frame takes back a target whose last read
/// by a present is still pending, and its import waits for that read on the device, so null
/// refuses nothing. A frame that took the target back with no wait would write it while the
/// read is pending, which null refuses (`render-no-ready`).
#[test]
fn a_frame_that_takes_back_a_target_waits_for_its_last_read() {
    for (layout, capabilities) in layouts() {
        let setup = Setup { lag: true, ..Setup::default() };
        let mut run = Run::with(capabilities, workers(), WorldConfig::shown(), setup);
        let seen: Vec<_> = (0..8).map(|_| run.settled(Some(E1))).collect();
        assert_eq!(seen, ["drawn"; 8], "{layout}");
        assert_eq!(*lock(&run.watch.refusals), Vec::<String>::new(), "{layout}");
        let said = run.said.count.load(Ordering::Relaxed);
        assert_eq!(said, 0, "{layout}: {:?}", lock(&run.said.texts));
        assert!(run.stop().is_none(), "no chaos, no log");
    }
}

/// On each layout, with the fixed-frame mode's readback at E1: after frames at E1 the last
/// frame is read with the placements extract handed the renderer known, and the scene's
/// seven instances live in them (decision 5.1: "the scene is seven instances"), each in a
/// slot of its own below the slot table's extent; a join suppressed, or a scene of six, is
/// told apart here. After a last frame at E2, which copies nothing, no last frame is read
/// (`readback-any-extent`). Null runs no shader, so the records it read back are not held
/// to the placements here: the device run does that.
#[test]
fn the_readback_reads_the_last_frame_at_its_extent_with_the_scenes_placements() {
    for (layout, capabilities) in layouts() {
        let setup = Setup { readback: Some(E1), ..Setup::default() };
        let mut run = Run::with(capabilities, workers(), WorldConfig::shown(), setup);
        for _ in 0..4 {
            assert_eq!(run.settled(Some(E1)), "drawn", "{layout}");
        }
        let last = run.stop_reading().expect("read").expect("a last frame");
        assert_eq!(last.extent, E1, "{layout}");
        assert_eq!(last.pixels.len(), 64 * 48, "{layout}");
        assert_eq!(last.expected.len(), last.records.len(), "{layout}");
        let live = |record: &&InstanceRecord| record.flags & InstanceRecord::LIVE != 0;
        assert_eq!(last.expected.iter().filter(live).count(), 7, "{layout}");

        let mut run = Run::with(capabilities, workers(), WorldConfig::shown(), setup);
        for extent in [E1, E1, E2] {
            assert_eq!(run.settled(Some(extent)), "drawn", "{layout}");
        }
        assert!(run.stop_reading().expect("read").is_none(), "{layout}: a frame at E2");
    }
}

/// On each layout, with every second present call making no submit, its acquire answered
/// with no image: the extent E1, hidden, E2 and E1 again, then more frames at E1. A target
/// whose call made no submit was released toward the present role, on two queues, with no
/// acquire to meet it, and the frame that draws it next takes it back from `Undefined`
/// (D160's item 7 (c), verification row 230): null refuses nothing and says nothing. Each
/// step is settled ([`Run::settled`]), so every frame but the hidden one is drawn and queued;
/// at least three of their calls made no submit, and the swapchain was resized to E1, E2 and
/// E1 alone.
#[test]
fn each_layout_takes_back_the_targets_of_present_calls_that_made_no_submit() {
    for (layout, capabilities) in layouts() {
        let setup = Setup { skip: true, ..Setup::default() };
        let mut run = Run::with(capabilities, workers(), WorldConfig::shown(), setup);
        let watch = Arc::clone(&run.watch);
        let extents = [Some(E1), Some(E1), None, Some(E2), Some(E2), Some(E1), Some(E1)];
        let mut seen: Vec<_> = extents.into_iter().map(|extent| run.settled(extent)).collect();
        seen.extend((0..8).map(|_| run.settled(Some(E1))));
        let drawn = seen.iter().filter(|name| **name == "drawn").count();
        assert!(seen[2] == "hidden" && drawn == seen.len() - 1, "{layout}: {seen:?}");
        let said = run.said.count.load(Ordering::Relaxed);
        assert_eq!(said, 0, "{layout}: {:?}", lock(&run.said.texts));
        assert!(run.stop().is_none(), "no chaos, no log");
        // A call whose acquire was skipped made no submit.
        let skipped = watch.skipped.load(Ordering::Relaxed);
        assert!(skipped >= 3, "{layout}: {skipped} calls made no submit");
        assert_eq!(*lock(&watch.resizes), [E1, E2, E1], "{layout}");
    }
}

/// On each layout, three frames and a clean stop, the render side dropped on this thread
/// after the present thread's stop: no thread logs a warning on the way, the present thread
/// and the workers among them. The renderer's drop once waited on the graph's timeline after
/// the graph's drop had destroyed it, and warned under `cheddar` at every clean shutdown; a
/// present side's `close` that destroyed its timeline twice would warn on the present thread
/// (`r3-close-warns-on-its-thread`).
#[test]
fn a_clean_shutdown_logs_no_warning() {
    for (layout, capabilities) in layouts() {
        let ((), logged) = warnings(|| {
            let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
            for _ in 0..3 {
                assert_eq!(run.settled(Some(E1)), "drawn", "{layout}");
            }
            assert!(run.stop().is_none(), "no chaos, no log");
        });
        assert!(logged.is_empty(), "{layout}: {logged}");
    }
}

/// A device lost between two frames ends the next step with `DeviceLost`, and what the
/// failure kept shuts down in the order a stop takes, on each layout.
#[test]
fn a_lost_device_ends_the_run_with_its_error_and_the_shutdown_completes() {
    for (layout, capabilities) in layouts() {
        let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
        for _ in 0..3 {
            assert_eq!(run.settled(Some(E1)), "drawn", "{layout}");
        }
        assert!(run.control.lose(), "{layout}: the device is gone");
        let failure = run.try_step(Some(E1)).expect_err("a step on a lost device");
        assert_eq!(failure.error.kind(), ErrorKind::DeviceLost, "{layout}: {:?}", failure.error);
        assert!(run.stop_after(failure).is_none(), "no chaos, no log");
    }
}
