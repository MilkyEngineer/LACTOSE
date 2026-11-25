//! The wiring that ships, on `lactose-rhi-null`: the client's render side and present side
//! stepped on each queue layout with the extent going E1, hidden, E2 and E1 again, through
//! the presents null stalls, and refused nothing; and a lost device, which ends the run with
//! its error and leaves a shutdown that completes.
//!
//! Null reaches drawn, hidden, held and dropped frames through this wiring. A frame that
//! waits for a region and one torn by a refusal are not reached: null runs each submit as it
//! is made, and its control offers a loss and a stall of presents alone, neither of which
//! leaves a frame's point unreached or refuses a call. `lactose-app`'s `tests/frame/states.rs`
//! drives both over a scripted render side.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use lactose_app::{
    PresentSide, RenderSide, Rendered, Shown, ShownError, ShownKept, Stopped, TICK_PERIOD, Workers,
    WorldConfig, start_executor,
};
use lactose_app_client::{FIXED_SAMPLE, Frames, Wired, wire};
use lactose_core::ErrorKind;
use lactose_input_host::LatestSample;
use lactose_platform::PresentThread;
use lactose_rhi_api::{Capabilities, Extent, QueueLayout, SwapchainDesc};
use lactose_rhi_null::Control;
use lactose_task::{ExecutionLog, Executor};

use crate::support::{Said, Watch, Watched, bounded, lock, null_device, null_shaders};

const E1: Extent = Extent { width: 64, height: 48 };
const E2: Extent = Extent { width: 80, height: 40 };

/// A shown world on null, stepped one joined frame at a time, as the fixed-frame mode
/// steps it.
pub struct Run {
    pub executor: Executor,
    pub control: Control,
    pub said: Arc<Said>,
    shown: Option<Shown>,
    present: Option<PresentThread<Box<dyn PresentSide>>>,
    frames: Frames,
    input: Arc<LatestSample>,
    start: Instant,
    steps: u32,
}

impl Run {
    pub fn new(capabilities: Capabilities, workers: Workers, world: WorldConfig) -> Self {
        Self::watched(capabilities, workers, world, false).0
    }

    /// A run whose swapchain is [`Watched`], answering every second acquire with no image
    /// where `skip` is set.
    pub fn watched(
        capabilities: Capabilities,
        workers: Workers,
        world: WorldConfig,
        skip: bool,
    ) -> (Self, Arc<Watch>) {
        let executor = start_executor(workers).expect("an executor");
        let (device, control, said) = null_device(capabilities);
        let platform = Arc::clone(executor.platform());
        // As the binary makes it, beside the device, at the window's first size.
        let inner = device.create_swapchain(&SwapchainDesc { extent: E1 }).expect("a swapchain");
        let watch = Arc::new(Watch::default());
        let swapchain = Box::new(Watched { inner, watch: Arc::clone(&watch), skip });
        let shaders = null_shaders();
        let Wired { shown, present, frames, input } =
            wire(&executor, &*platform, &device, swapchain, &shaders, world, None).expect("wired");
        let run = Self {
            executor,
            control,
            said,
            shown: Some(shown),
            present: Some(present),
            frames,
            input,
            start: Instant::now(),
            steps: 0,
        };
        (run, watch)
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
/// A stalled present's acquire finds no image and returns at null's bound, a second, so a
/// call is in flight for a second at a time while eight steps run: what is asserted is that
/// some frame was held or dropped among them, never which.
#[test]
fn each_layout_steps_e1_e2_e1_through_stalled_presents_and_refuses_nothing() {
    for (layout, capabilities) in layouts() {
        let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
        let mut seen = Vec::new();
        let extents = [Some(E1), Some(E1), Some(E1), None, Some(E2), Some(E2), Some(E1), Some(E1)];
        for extent in extents {
            seen.push(name(&run.step(extent)));
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
        let after: Vec<_> = (0..4).map(|_| name(&run.step(Some(E1)))).collect();
        assert_eq!(after.last(), Some(&"drawn"), "{layout}: {after:?}");
        assert_eq!(
            run.said.count.load(Ordering::Relaxed),
            0,
            "{layout}: {:?}",
            lock(&run.said.texts)
        );
        assert!(run.stop().is_none(), "no chaos, no log");
    }
}

/// On each layout, with every second present call making no submit, its acquire answered
/// with no image: the extent E1, hidden, E2 and E1 again, then more frames at E1. A target
/// whose call made no submit was released toward the present role, on two queues, with no
/// acquire to meet it, and the frame that draws it next takes it back from `Undefined`
/// (D160's item 7 (c), verification row 230): null refuses nothing and says nothing. Every
/// frame but the hidden one draws; whether it is queued, held or dropped is the present
/// thread's pace and not asserted.
#[test]
fn each_layout_takes_back_the_targets_of_present_calls_that_made_no_submit() {
    for (layout, capabilities) in layouts() {
        let (mut run, watch) = Run::watched(capabilities, workers(), WorldConfig::shown(), true);
        let extents = [Some(E1), Some(E1), None, Some(E2), Some(E2), Some(E1), Some(E1)];
        let mut seen: Vec<_> = extents.into_iter().map(|extent| name(&run.step(extent))).collect();
        seen.extend((0..8).map(|_| name(&run.step(Some(E1)))));
        assert_eq!(seen[2], "hidden", "{layout}: {seen:?}");
        let drew = seen.iter().filter(|name| ["drawn", "held", "dropped"].contains(name)).count();
        assert_eq!(drew, seen.len() - 1, "{layout}: {seen:?}");
        let said = run.said.count.load(Ordering::Relaxed);
        assert_eq!(said, 0, "{layout}: {:?}", lock(&run.said.texts));
        assert!(run.stop().is_none(), "no chaos, no log");
        // The stop awaited every call; a call whose acquire was skipped made no submit.
        let skipped = watch.skipped.load(Ordering::Relaxed);
        assert!(skipped >= 3, "{layout}: {skipped} calls made no submit");
        assert_eq!(*lock(&watch.resizes), [E1, E2, E1], "{layout}");
    }
}

/// A device lost between two frames ends the next step with `DeviceLost`, and what the
/// failure kept shuts down in the order a stop takes, on each layout.
#[test]
fn a_lost_device_ends_the_run_with_its_error_and_the_shutdown_completes() {
    for (layout, capabilities) in layouts() {
        let mut run = Run::new(capabilities, workers(), WorldConfig::shown());
        for _ in 0..3 {
            assert_eq!(name(&run.step(Some(E1))), "drawn", "{layout}");
        }
        assert!(run.control.lose(), "{layout}: the device is gone");
        let failure = run.try_step(Some(E1)).expect_err("a step on a lost device");
        assert_eq!(failure.error.kind(), ErrorKind::DeviceLost, "{layout}: {:?}", failure.error);
        assert!(run.stop_after(failure).is_none(), "no chaos, no log");
    }
}
