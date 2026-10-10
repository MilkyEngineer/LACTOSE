//! The executor handle and its builder.

use core::pin::pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::Arc;
use std::sync::mpsc;
use std::task::Wake;
use std::thread::Thread;
use std::time::Instant;

use crate::deque::Worker;
use lactose_core::{AllocTag, Error, ErrorKind, Result, ThreadClass};

use crate::WorkerClass;
use crate::chaos::{Chaos, ChaosConfig, ExecutionLog};
use crate::graph::{self, Graph, GraphRun};
use crate::platform::{Platform, StdPlatform, ThreadHandle, ThreadSpec};
use crate::platform::{default_background_workers, default_frame_workers};
use crate::pool::{Shared, WorkerSeed, on_worker_of, on_worker_thread, worker_thread};
use crate::task::{self, Task};
use crate::time::Sleep;

/// A handle to the worker pools. Cheap to clone; every clone refers to the same pools.
///
/// The pools run until [`Executor::shutdown`], or until the last handle is dropped, which
/// shuts them down the same way. A handle held by a task keeps the pools alive like any
/// other.
#[derive(Clone)]
pub struct Executor {
    owner: Arc<Owner>,
}

/// What the user's handles share. The workers hold the pool state itself, not this, so the
/// count of this `Arc` is the count of user handles.
struct Owner {
    shared: Arc<Shared>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        // The last handle may go on one of this executor's own workers, when a task that
        // held it finishes or is dropped. A worker cannot wait for itself, so there the
        // threads are only told. Anywhere else, another executor's worker included, the
        // threads are joined.
        self.shared.shut_down(!on_worker_of(&self.shared));
    }
}

impl Executor {
    /// Starts configuring an executor.
    #[must_use]
    pub fn builder() -> ExecutorBuilder {
        ExecutorBuilder {
            frame_workers: None,
            background_workers: None,
            platform: None,
            chaos: None,
            replay: None,
        }
    }

    /// Starts `future` on a worker of `class` and returns a handle to its result.
    ///
    /// The task inherits the caller's `lactose_core::AllocTag`, and the executor re-enters
    /// that tag on whichever worker polls it. A task is cancelled by dropping its [`Task`].
    /// A panic in the future aborts the process.
    pub fn spawn<F>(&self, class: WorkerClass, future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        task::spawn(&self.owner.shared, class, future)
    }

    /// Runs every node of `graph` on the frame-critical workers.
    ///
    /// The returned future resolves, handing the graph back, when every node has finalized.
    /// A node is released when all nodes it depends on have finalized; execution order
    /// between independent nodes is unspecified.
    ///
    /// The run owns the graph, so one graph can never have two runs in flight. Dropping the
    /// future does not cancel the run: nodes own their data, the run finishes on its own,
    /// and the graph is then dropped.
    pub fn run_graph(&self, graph: Graph) -> GraphRun {
        graph::start(&self.owner.shared, graph)
    }

    /// The number of frame-critical workers.
    #[must_use]
    pub fn frame_worker_count(&self) -> u32 {
        self.owner.shared.frame().workers() as u32
    }

    /// The number of background workers. For the benchmark's report.
    pub(crate) fn background_worker_count(&self) -> u32 {
        self.owner.shared.pool(WorkerClass::Background).workers() as u32
    }

    /// One more than the largest value [`crate::JobContext::slot`] can return.
    #[must_use]
    pub fn slot_count(&self) -> u32 {
        // A slot is the index of the frame-critical worker running the job.
        self.frame_worker_count()
    }

    /// The platform this executor creates threads through. Crates that own a dedicated
    /// thread (network, present, audio) create it here, never with `std::thread` directly,
    /// so a console platform controls every engine thread.
    #[must_use]
    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.owner.shared.platform
    }

    /// A future that resolves at `deadline`. The tick loop paces itself with this.
    pub fn sleep_until(&self, deadline: Instant) -> Sleep {
        Sleep::new(Arc::downgrade(&self.owner.shared), deadline)
    }

    /// Drives `future` to completion on the calling thread while the workers run whatever it
    /// spawns. For `main`, tests and tools.
    ///
    /// This is the crate's only blocking wait. Calling it on a worker thread is a debug
    /// assertion, because a worker that blocks stalls the frame.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        struct Signal {
            thread: Thread,
            woken: AtomicBool,
        }

        impl Wake for Signal {
            fn wake(self: Arc<Self>) {
                self.wake_by_ref();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                self.woken.store(true, Ordering::Release);
                self.thread.unpark();
            }
        }

        debug_assert!(!on_worker_thread(), "block_on called on a worker thread stalls the frame");
        let signal =
            Arc::new(Signal { thread: std::thread::current(), woken: AtomicBool::new(false) });
        let waker = Waker::from(signal.clone());
        let mut context = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
            while !signal.woken.swap(false, Ordering::Acquire) {
                std::thread::park();
            }
        }
    }

    /// Everything recorded since the executor was built, if [`ChaosConfig::record`] was
    /// set. The log keeps growing; a later call returns a longer log that starts with this
    /// one.
    ///
    /// A log replays from the executor's first job, so this returns the whole history
    /// rather than draining it.
    #[must_use]
    pub fn execution_log(&self) -> Option<ExecutionLog> {
        self.owner.shared.chaos.as_ref().and_then(Chaos::log)
    }

    /// The chaos configuration scheduling runs under, or `None` when scheduling is free:
    /// what [`ExecutorBuilder::chaos`] was given, or the default configuration under a
    /// [`ExecutorBuilder::replay`] built without one. What the determinism harness reports
    /// it ran with.
    #[must_use]
    pub fn chaos_config(&self) -> Option<ChaosConfig> {
        self.owner.shared.chaos.as_ref().map(|chaos| chaos.config)
    }

    /// Stops accepting work, lets running jobs finish, cancels unfinished tasks and joins
    /// every worker. Returns once all threads have exited.
    ///
    /// Queued graph work is dropped. A [`GraphRun`], [`Task`] or [`Sleep`] that was cut off
    /// resolves to an error of kind `Cancelled`. Other clones of the handle stay valid and
    /// inert: what they start is cancelled at once.
    ///
    /// Called on one of this executor's own workers, from a task or a job, it cannot wait
    /// for the thread it is on: it does everything else and returns without joining, and
    /// the workers exit on their own once their running jobs return.
    pub fn shutdown(self) {
        let shared = &self.owner.shared;
        shared.shut_down(!on_worker_of(shared));
    }
}

/// Configures and starts an [`Executor`].
pub struct ExecutorBuilder {
    frame_workers: Option<u32>,
    background_workers: Option<u32>,
    platform: Option<Arc<dyn Platform>>,
    chaos: Option<ChaosConfig>,
    replay: Option<ExecutionLog>,
}

impl ExecutorBuilder {
    /// Overrides the number of frame-critical workers. The default is the platform's
    /// performance-core count.
    #[must_use]
    pub fn frame_workers(mut self, count: u32) -> Self {
        self.frame_workers = Some(count);
        self
    }

    /// Overrides the number of background workers. The default is the platform's
    /// efficiency-core count, or two on homogeneous CPUs.
    #[must_use]
    pub fn background_workers(mut self, count: u32) -> Self {
        self.background_workers = Some(count);
        self
    }

    /// Replaces the thread and topology provider. The default is [`crate::StdPlatform`].
    #[must_use]
    pub fn platform(mut self, platform: Arc<dyn Platform>) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Enables chaos scheduling for the determinism harness. A runtime option, never a Cargo
    /// feature, and a no-op when not set.
    #[must_use]
    pub fn chaos(mut self, config: ChaosConfig) -> Self {
        self.chaos = Some(config);
        self
    }

    /// Replays a recorded execution log instead of scheduling freely, so a chaos failure
    /// reproduces exactly: each graph job and scope child runs on the worker, and starts in
    /// the order, the log names.
    ///
    /// The executor must have the frame-critical worker count the log was recorded with,
    /// and the program must start the same graph runs in the same order; a log from a
    /// different program stalls. Once the log is exhausted, scheduling is free again but
    /// stays serialised. Replay is a debugging mode and is slow.
    #[must_use]
    pub fn replay(mut self, log: ExecutionLog) -> Self {
        self.replay = Some(log);
        self
    }

    /// Starts the worker threads.
    ///
    /// Fails with `Unsupported` if either worker count is zero, with `InvalidData` or
    /// `VersionMismatch` if a replay log is malformed, and with the platform's error if a
    /// thread cannot be created.
    pub fn build(self) -> Result<Executor> {
        let _tag = AllocTag::Task.scope();
        let platform = self.platform.unwrap_or_else(|| Arc::new(StdPlatform));
        let (frame, background) = match (self.frame_workers, self.background_workers) {
            (Some(frame), Some(background)) => (frame, background),
            (frame, background) => {
                let topology = platform.topology();
                let threads = platform.hardware_threads();
                let frame = frame.unwrap_or_else(|| default_frame_workers(topology, threads));
                let background = background
                    .unwrap_or_else(|| default_background_workers(topology, threads, frame));
                (frame, background)
            }
        };
        if frame == 0 || background == 0 {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "an executor needs at least one worker of each class",
            ));
        }
        let chaos = match (self.chaos, &self.replay) {
            (None, None) => None,
            (config, replay) => {
                Some(Chaos::new(config.unwrap_or_default(), replay.as_ref(), frame)?)
            }
        };

        // Each worker reports its thread handle, then waits for the shared state, which
        // needs every handle. If a thread cannot be created, dropping `shared_senders`
        // sends the ones already running home.
        let mut handles = Vec::new();
        let mut shared_senders = Vec::new();
        let mut pools = Vec::new();
        let classes = [
            (WorkerClass::FrameCritical, ThreadClass::FrameCritical, "whisk-frame", frame),
            (WorkerClass::Background, ThreadClass::Background, "whisk-background", background),
        ];
        let spawned = (|| {
            for (class, thread_class, name, count) in classes {
                let mut stealers = Vec::new();
                let mut threads = Vec::new();
                for index in 0..count {
                    let deque = Worker::new_lifo();
                    stealers.push(deque.stealer());
                    let (thread_sender, thread_receiver) = mpsc::channel();
                    let (shared_sender, shared_receiver) = mpsc::channel();
                    let seed = WorkerSeed {
                        class,
                        index: index as usize,
                        deque,
                        thread: thread_sender,
                        shared: shared_receiver,
                    };
                    let spec = ThreadSpec {
                        name: format!("{name}-{index}"),
                        class: thread_class,
                        index_in_class: index,
                    };
                    handles
                        .push(platform.spawn_thread(spec, Box::new(move || worker_thread(seed)))?);
                    shared_senders.push(shared_sender);
                    threads.push(thread_receiver.recv().map_err(|_| {
                        Error::new(ErrorKind::Platform, "a worker thread exited before starting")
                    })?);
                }
                pools.push((stealers, threads));
            }
            Ok::<(), Error>(())
        })();
        if let Err(error) = spawned {
            drop(shared_senders);
            handles.into_iter().for_each(ThreadHandle::join);
            return Err(error);
        }

        let background_pool = pools.pop().expect("two pools were built");
        let frame_pool = pools.pop().expect("two pools were built");
        let shared = Arc::new(Shared::new(frame_pool, background_pool, platform, chaos, handles));
        for sender in shared_senders {
            // A worker that is gone already cannot be told; shutdown joins it all the same.
            let _ = sender.send(shared.clone());
        }
        Ok(Executor { owner: Arc::new(Owner { shared }) })
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use core::ops::Range;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};
    use std::sync::Arc;

    use crate::{Executor, GraphBuilder, GraphNode, JobContext};

    struct Nothing;

    impl GraphNode for Nothing {
        fn name(&self) -> &'static str {
            "executor::tests::nothing"
        }

        fn units(&self) -> u32 {
            1
        }

        fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {}
    }

    /// A graph that is run again the instant its run resolves. The finishing worker used
    /// to unregister the run after the future could resolve, by an ordinal it read late:
    /// it then removed the new run and left the old one registered for ever.
    #[test]
    fn a_run_is_unregistered_before_its_graph_can_run_again() {
        let executor = Executor::builder().frame_workers(2).background_workers(1).build().unwrap();
        let mut builder = GraphBuilder::new();
        builder.add_node(Arc::new(Nothing));
        let mut graph = builder.build().unwrap();
        let mut context = Context::from_waker(Waker::noop());
        for _ in 0..if cfg!(miri) { 50 } else { 50_000 } {
            let mut run = pin!(executor.run_graph(graph));
            // Polled in a tight loop, so the new run starts as early as it possibly can.
            graph = loop {
                if let Poll::Ready(result) = run.as_mut().poll(&mut context) {
                    break result.unwrap();
                }
                core::hint::spin_loop();
            };
            // The run that just resolved is gone from the registry, and no other is live.
            assert_eq!(executor.owner.shared.live_run_count(), 0);
        }
        executor.shutdown();
    }

    /// With nothing queued, the worker that fires a timer runs the task it wakes and wakes
    /// no sleeper for it. Every frame-critical worker, the keeper among them, is asleep
    /// shortly before the deadline; when the task resumes, every worker but the one running
    /// it must still be asleep and unclaimed. A task queued with a wake-up would have had a
    /// sleeper claimed for it, which cannot be back asleep by the task's first instruction.
    /// An attempt in which some worker was still awake shortly before the deadline proves
    /// nothing and is repeated with a longer lead.
    #[test]
    #[cfg_attr(miri, ignore = "needs every worker parked ahead of a real deadline")]
    fn a_timer_task_runs_on_the_worker_that_fires_it_and_wakes_nobody() {
        use core::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        use crate::WorkerClass;

        const WORKERS: u32 = 3;
        let executor =
            Executor::builder().frame_workers(WORKERS).background_workers(1).build().unwrap();
        let shared = executor.owner.shared.clone();
        for lead in [20, 100, 500, 2000].map(Duration::from_millis) {
            let deadline = Instant::now() + lead;
            let registered = Arc::new(AtomicBool::new(false));
            let task = {
                let (sleeper, shared, registered) =
                    (executor.clone(), shared.clone(), registered.clone());
                executor.spawn(WorkerClass::FrameCritical, async move {
                    let mut sleep = pin!(sleeper.sleep_until(deadline));
                    core::future::poll_fn(|cx| {
                        let poll = sleep.as_mut().poll(cx);
                        if poll.is_pending() {
                            registered.store(true, Ordering::SeqCst);
                        }
                        poll
                    })
                    .await
                    .unwrap();
                    shared.frame().sleepers()
                })
            };
            while Instant::now() + lead / 4 < deadline {
                std::thread::sleep(Duration::from_micros(200));
            }
            // Between this look and the resumption nothing claims a sleeper but a wake-up
            // for the task itself: nothing else is queued, and the keeper's own timeout
            // clears only its own bit.
            let all_asleep = registered.load(Ordering::SeqCst)
                && shared.frame().sleepers() == WORKERS
                && Instant::now() < deadline;
            let asleep_at_resumption = executor.block_on(task).unwrap();
            if all_asleep {
                assert_eq!(asleep_at_resumption, WORKERS - 1, "a sleeper was woken for the task");
                executor.shutdown();
                return;
            }
        }
        panic!("some worker was still awake shortly before every deadline");
    }
}
