//! Recording on null: a recorder's breach, the first in graph order of two, the encoders
//! under a lagging device and `record`'s own read of the timeline, a frame of another graph,
//! recording in parallel, and the same commands under chaos.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use lactose_core::ErrorKind;
use lactose_render_graph::{
    BufferAccess, BufferImport, BufferUse, ComputePassDecl, CopyPassDecl, FrameDecl,
};
use lactose_rhi_api::{BufferCopy, BufferState, ImageState, Load, QueueRole, Rule, Sharing};
use lactose_task::ChaosConfig;

use crate::common::{
    G, LAYOUTS, Recorded, buffer, colour, compute_pipeline, define_buffer, device, draws, executor,
    frame, fresh_buffer, graph, held, home, image, image_import, in_job, in_job_of, lines, lock,
    on, reached,
};

/// A copy call's regions reach the device as the recorder gave them: two that write apart
/// are accepted as one call, and two that write one byte are null's `bounds`, which
/// `submit` returns unchanged.
#[test]
fn a_copys_regions_reach_the_device_as_given() {
    for queues in LAYOUTS {
        let device = device(queues);
        let source = buffer(&*device, "source", Sharing::Exclusive);
        let out = buffer(&*device, "out", Sharing::Exclusive);
        define_buffer(&*device, source, on(BufferState::CopySource, G));
        device.take_submitted();
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 2);
            for overlap in [false, true] {
                let mut decl = FrameDecl::new(1);
                let source = decl.import_buffer(&BufferImport {
                    label: "source",
                    buffer: source,
                    sharing: Sharing::Exclusive,
                    initial: on(BufferState::CopySource, G),
                    last: on(BufferState::CopySource, G),
                    ready: None,
                });
                let out = decl.import_buffer(&fresh_buffer("out", out, G, BufferState::HostRead));
                let second = if overlap { 8 } else { 16 };
                let regions = [
                    BufferCopy { source_offset: 0, destination_offset: 0, size: 16 },
                    BufferCopy { source_offset: 16, destination_offset: second, size: 16 },
                ];
                decl.copy_pass(
                    &CopyPassDecl {
                        label: "two regions",
                        image_sources: &[],
                        image_destinations: &[],
                        buffer_sources: &[source],
                        buffer_destinations: &[out],
                    },
                    move |copy| copy.copy_buffer(source, out, &regions),
                );
                let result = frame(&mut graph, ctx, decl);
                if overlap {
                    let refused = result.expect_err("null refuses the overlap");
                    assert_eq!(Rule::of(&refused), Some(Rule::Bounds), "{refused}");
                    assert!(device.take_submitted().is_empty());
                } else {
                    reached(&**graph.device(), result.expect("null accepts the regions"));
                    let lines = lines(&device.take_submitted());
                    let copies: Vec<&String> =
                        lines.iter().filter(|line| line.starts_with("copy_buffer")).collect();
                    assert_eq!(copies.len(), 1, "{lines:#?}");
                    assert_eq!(copies[0].matches("BufferCopy {").count(), 2, "{}", copies[0]);
                }
            }
        });
    }
}

/// A copy pass's second call into one destination, and a recorder's use of an id its pass did
/// not declare for that use, each fail `record` with `InvalidData` naming the pass, submit
/// nothing, and leave the frame's encoders free: the next frame makes none.
#[test]
fn a_recorders_breach_fails_record_and_frees_the_encoders() {
    for queues in LAYOUTS {
        let device = device(queues);
        let handles = [
            buffer(&*device, "first", Sharing::Exclusive),
            buffer(&*device, "second", Sharing::Exclusive),
            buffer(&*device, "out", Sharing::Exclusive),
            buffer(&*device, "written", Sharing::Exclusive),
        ];
        let pipeline = compute_pipeline(&*device);
        for handle in &handles[..2] {
            define_buffer(&*device, *handle, on(BufferState::CopySource, G));
        }
        device.take_submitted();
        in_job(move |ctx| {
            let mut graph = graph(&device, 2, 2);
            let declare = |breach: Option<bool>| {
                let mut decl = FrameDecl::new(1);
                let source = |label, handle| BufferImport {
                    label,
                    buffer: handle,
                    sharing: Sharing::Exclusive,
                    initial: on(BufferState::CopySource, G),
                    last: on(BufferState::CopySource, G),
                    ready: None,
                };
                let first = decl.import_buffer(&source("first", handles[0]));
                let second = decl.import_buffer(&source("second", handles[1]));
                let out =
                    decl.import_buffer(&fresh_buffer("out", handles[2], G, BufferState::HostRead));
                let written = decl.import_buffer(&fresh_buffer(
                    "written",
                    handles[3],
                    G,
                    BufferState::ShaderWrite,
                ));
                let region = BufferCopy { source_offset: 0, destination_offset: 0, size: 16 };
                decl.copy_pass(
                    &CopyPassDecl {
                        label: "into out",
                        image_sources: &[],
                        image_destinations: &[],
                        buffer_sources: &[first, second],
                        buffer_destinations: &[out],
                    },
                    move |copy| {
                        copy.copy_buffer(first, out, &[region]);
                        if breach == Some(true) {
                            copy.copy_buffer(second, out, &[region]);
                        }
                    },
                );
                let uses = [
                    BufferUse { buffer: out, access: BufferAccess::Indirect },
                    BufferUse { buffer: written, access: BufferAccess::Write },
                ];
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "indirect",
                        sampled: &[],
                        buffers: &uses,
                        slots: &[],
                    },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        if breach == Some(false) {
                            // `out` is declared `Indirect`, not `Read` or `Write`.
                            pass.push_address(0, out, 0);
                        }
                        pass.dispatch_indirect(out, 0);
                    },
                );
                decl
            };
            let point = frame(&mut graph, ctx, declare(None)).expect("a frame");
            reached(&**graph.device(), point);
            let encoders = graph.stats().encoders;
            device.take_submitted();
            for (breach, pass, call) in
                [(true, "into out", "copy_buffer"), (false, "indirect", "push_address")]
            {
                let compiled = graph.compile(declare(Some(breach))).expect("a plan");
                let error = graph.record(compiled, ctx).err().expect("a breach");
                assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
                let message = error.to_string();
                assert!(message.contains(pass) && message.contains(call), "{message}");
                assert!(device.take_submitted().is_empty());
                assert_eq!(graph.stats().encoders, encoders, "the frame's encoders are free");
                let point =
                    frame(&mut graph, ctx, declare(None)).expect("a frame after the breach");
                reached(&**graph.device(), point);
                assert_eq!(graph.stats().encoders, encoders);
                device.take_submitted();
            }
        });
    }
}

/// An encoder is reset only after its frame's point (null refuses a reset while pending):
/// with the device made to lag, a second frame takes encoders of its own, and once the first
/// frame's point is reached a third reuses them.
#[test]
fn encoders_are_reused_only_after_their_frames_point() {
    for queues in LAYOUTS {
        let device = device(queues);
        let handles = (
            buffer(&*device, "held", Sharing::Exclusive),
            image(&*device, "target", Sharing::Exclusive),
        );
        let gate = held(&*device);
        in_job(move |ctx| {
            let queue = home(&*device);
            let mut graph = graph(&device, 2, 1);
            let declare = |first: bool| {
                let mut decl = FrameDecl::new(1);
                let held = decl.import_buffer(&BufferImport {
                    ready: Some(gate),
                    ..fresh_buffer("held", handles.0, queue, BufferState::ShaderWrite)
                });
                let target = decl.import_image(&image_import(
                    "target",
                    handles.1,
                    on(if first { ImageState::Undefined } else { ImageState::ColourAttachment }, G),
                    on(ImageState::ColourAttachment, G),
                ));
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "write",
                        sampled: &[],
                        buffers: &[BufferUse { buffer: held, access: BufferAccess::Write }],
                        slots: &[],
                    },
                    |_| {},
                );
                decl.render_pass(&draws("draw", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
                decl
            };
            let first = frame(&mut graph, ctx, declare(true)).expect("a frame");
            let after_one = graph.stats().encoders;
            let second = frame(&mut graph, ctx, declare(false)).expect("a frame");
            assert_eq!(graph.stats().encoders, 2 * after_one);
            assert_eq!(first.timeline, second.timeline);
            assert!(first.value < second.value);
            assert_eq!(graph.last_frame(), Some(second));
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), second);
            let third = frame(&mut graph, ctx, declare(false)).expect("a frame");
            assert_eq!(graph.stats().encoders, 2 * after_one);
            reached(&**graph.device(), third);
        });
    }
}

/// A frame's encoders are reused once its point is reached though a later frame is still
/// pending: the graph holds pending encoders in frame order and frees every frame reached.
#[test]
fn a_reached_frames_encoders_are_reused_while_a_later_frame_is_pending() {
    for queues in LAYOUTS {
        let device = device(queues);
        let target = image(&*device, "target", Sharing::Exclusive);
        let gates = [held(&*device), held(&*device)];
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 1);
            let declare = |ready| {
                let mut decl = FrameDecl::new(1);
                let target = decl.import_image(&lactose_render_graph::ImageImport {
                    ready,
                    ..image_import(
                        "target",
                        target,
                        on(ImageState::Undefined, G),
                        on(ImageState::ColourAttachment, G),
                    )
                });
                decl.render_pass(&draws("draw", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
                decl
            };
            let first = frame(&mut graph, ctx, declare(Some(gates[0]))).expect("a frame");
            let second = frame(&mut graph, ctx, declare(Some(gates[1]))).expect("a frame");
            assert_eq!(graph.stats().encoders, 2, "both frames pending");
            graph.device().signal_timeline(gates[0]).expect("the host's signal");
            reached(&**graph.device(), first);
            let third = frame(&mut graph, ctx, declare(None)).expect("a frame");
            assert_eq!(graph.stats().encoders, 2, "the third took the first's encoder");
            graph.device().signal_timeline(gates[1]).expect("the host's signal");
            reached(&**graph.device(), second);
            reached(&**graph.device(), third);
        });
    }
}
/// A frame compiled by one graph is refused by another's `record`, and a frame recorded by
/// one by another's `submit`.
#[test]
fn a_frame_of_another_graph_is_refused() {
    let device = device(lactose_rhi_api::QueueLayout::ONE);
    let target = image(&*device, "target", Sharing::Exclusive);
    in_job(move |ctx| {
        let mut ours = graph(&device, 1, 1);
        let mut theirs = graph(&device, 1, 1);
        let declare = || {
            let mut decl = FrameDecl::new(1);
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            decl.render_pass(&draws("draw", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            decl
        };
        let compiled = theirs.compile(declare()).expect("a plan");
        let error = ours.record(compiled, ctx).err().expect("a frame of another graph");
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        let compiled = theirs.compile(declare()).expect("a plan");
        let recorded = theirs.record(compiled, ctx).expect("a recorded frame");
        let error = ours.submit(recorded).expect_err("a frame of another graph");
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        assert_eq!(ours.last_frame(), None);
    });
}

// ---- Recording itself ----

/// Arrivals at a rendezvous, which each side waits on with a bound.
#[derive(Default)]
struct Rendezvous {
    arrived: Mutex<u32>,
    changed: Condvar,
}

impl Rendezvous {
    /// Arrives, and waits up to `bound` for `count` arrivals in all; whether they came.
    fn meet(&self, count: u32, bound: Duration) -> bool {
        let mut arrived = lock(&self.arrived);
        *arrived += 1;
        self.changed.notify_all();
        let (arrived, timeout) = self
            .changed
            .wait_timeout_while(arrived, bound, |arrived| *arrived < count)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(arrived);
        !timeout.timed_out()
    }
}

/// Four compute passes, each writing a buffer of its own and pushing its number, in two jobs:
/// the second pass of the first job and the first of the second each wait, with a bound, for
/// the other. The test's breach of "a recorder does not wait", made to detect serial
/// recording, which leaves the first waiter alone until its bound.
fn rendezvous_frame<'f>(
    pipeline: lactose_rhi_api::ComputePipelineHandle,
    buffers: [lactose_rhi_api::BufferHandle; 4],
    queue: QueueRole,
    meeting: &'f Arc<Rendezvous>,
    met: &'f Arc<Mutex<Vec<bool>>>,
) -> FrameDecl<'f> {
    let mut decl = FrameDecl::new(1);
    for (at, handle) in buffers.into_iter().enumerate() {
        let buffer =
            decl.import_buffer(&fresh_buffer("written", handle, queue, BufferState::ShaderWrite));
        let waits = at == 1 || at == 2;
        decl.compute_pass(
            &ComputePassDecl {
                label: ["one", "two", "three", "four"][at],
                sampled: &[],
                buffers: &[BufferUse { buffer, access: BufferAccess::Write }],
                slots: &[],
            },
            move |pass| {
                pass.bind_pipeline(pipeline);
                pass.push_bytes(0, &[u8::try_from(at).expect("four passes")]);
                if waits {
                    let came = meeting.meet(2, Duration::from_secs(10));
                    lock(met).push(came);
                }
                pass.push_address(8, buffer, 0);
                pass.dispatch([1, 1, 1]);
            },
        );
    }
    decl
}

/// Recording runs in parallel: on four frame-critical workers, a recorder of one job and a
/// recorder of another each wait for the other, so the test fails at its bound where the
/// jobs do not run at once; and each job's commands are still in graph order.
#[test]
fn recording_runs_its_jobs_in_parallel() {
    for queues in LAYOUTS {
        let device = device(queues);
        let pipeline = compute_pipeline(&*device);
        let buffers = ["a", "b", "c", "d"].map(|label| buffer(&*device, label, Sharing::Exclusive));
        let executor = executor(4, None);
        let logged = Arc::clone(&device);
        in_job_of(&executor, move |ctx| {
            let queue = home(&*device);
            let mut graph = graph(&device, 2, 1);
            let meeting = Arc::new(Rendezvous::default());
            let met = Arc::new(Mutex::new(Vec::new()));
            let compiled = graph
                .compile(rendezvous_frame(pipeline, buffers, queue, &meeting, &met))
                .expect("a plan");
            assert_eq!(compiled.plan().jobs(), 2);
            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            assert_eq!(*lock(&met), [true, true], "the two recorders did not run at once");
            let point = graph.submit(recorded).expect("a frame");
            reached(&**graph.device(), point);
        });
        executor.shutdown();
        let submitted = logged.take_submitted();
        let encoders: Vec<&Recorded> =
            submitted.iter().flat_map(|(_, encoders)| encoders).collect();
        assert_eq!(encoders.len(), 2, "{submitted:#?}");
        for (job, encoder) in encoders.iter().enumerate() {
            let pushed: Vec<&String> =
                encoder.commands.iter().filter(|line| line.starts_with("push_bytes")).collect();
            let expected: Vec<String> =
                (2 * job..2 * job + 2).map(|at| format!("push_bytes 0 [{at}]")).collect();
            assert_eq!(pushed, expected.iter().collect::<Vec<_>>(), "{encoder:#?}");
        }
    }
}

/// `submitted` with each handle written as the order it first appears in: null counts its
/// handles' generations across the process, so two fresh devices name one object apart.
fn numbered(submitted: Vec<(QueueRole, Vec<Recorded>)>) -> Vec<(QueueRole, Vec<Recorded>)> {
    let mut seen: Vec<String> = Vec::new();
    let mut number = |line: &str| {
        let mut out = String::new();
        let mut rest = line;
        while let Some(at) = rest.find("Handle(") {
            let (before, after) = rest.split_at(at);
            let end = after.find(')').expect("a closed handle") + 1;
            let handle = after[..end].to_owned();
            let index = seen.iter().position(|known| *known == handle).unwrap_or_else(|| {
                seen.push(handle);
                seen.len() - 1
            });
            out.push_str(before);
            out.push_str(&format!("#{index}"));
            rest = &after[end..];
        }
        out.push_str(rest);
        out
    };
    submitted
        .into_iter()
        .map(|(role, encoders)| {
            let encoders = encoders
                .into_iter()
                .map(|encoder| Recorded {
                    queue: encoder.queue,
                    commands: encoder.commands.iter().map(|line| number(line)).collect(),
                })
                .collect();
            (role, encoders)
        })
        .collect()
}

/// The commands two frames of nine passes in four jobs submit, on a fresh device.
fn chaos_commands(
    queues: lactose_rhi_api::QueueLayout,
    workers: u32,
    chaos: Option<ChaosConfig>,
) -> Vec<(QueueRole, Vec<Recorded>)> {
    let device = device(queues);
    let pipeline = compute_pipeline(&*device);
    let buffers: Vec<_> = (0..8).map(|_| buffer(&*device, "written", Sharing::Exclusive)).collect();
    let target = image(&*device, "target", Sharing::Exclusive);
    let executor = executor(workers, chaos);
    let logged = Arc::clone(&device);
    in_job_of(&executor, move |ctx| {
        let queue = home(&*device);
        let mut graph = graph(&device, 4, 1);
        for first in [true, false] {
            let mut decl = FrameDecl::new(1);
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(if first { ImageState::Undefined } else { ImageState::ColourAttachment }, G),
                on(ImageState::ColourAttachment, G),
            ));
            for (at, &handle) in buffers.iter().enumerate() {
                let buffer = decl.import_buffer(&fresh_buffer(
                    "written",
                    handle,
                    queue,
                    BufferState::ShaderWrite,
                ));
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "write",
                        sampled: &[],
                        buffers: &[BufferUse { buffer, access: BufferAccess::Write }],
                        slots: &[],
                    },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        for _ in 0..at {
                            std::thread::yield_now();
                        }
                        pass.push_bytes(0, &[u8::try_from(at).expect("eight passes")]);
                        pass.push_address(8, buffer, 0);
                        pass.dispatch([1, 1, 1]);
                    },
                );
            }
            decl.render_pass(&draws("draw", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            let point = frame(&mut graph, ctx, decl).expect("a frame");
            reached(&**graph.device(), point);
        }
    });
    executor.shutdown();
    numbered(logged.take_submitted())
}

/// The same frame records the same commands under chaos, shuffled steals and injected
/// yields over several seeds, at one and at four frame-critical workers.
#[test]
fn chaos_changes_no_command() {
    for queues in LAYOUTS {
        let reference = chaos_commands(queues, 1, None);
        assert!(!reference.is_empty());
        for workers in [1, 4] {
            for seed in 0..4 {
                let chaos = ChaosConfig {
                    seed,
                    shuffle_steals: true,
                    inject_yields: true,
                    ..ChaosConfig::default()
                };
                let commands = chaos_commands(queues, workers, Some(chaos));
                let (ours, theirs) =
                    (crate::common::lines(&commands), crate::common::lines(&reference));
                let first = ours.iter().zip(&theirs).position(|(a, b)| a != b);
                assert!(
                    commands == reference,
                    "{workers} workers, seed {seed}: at {first:?}: {:?} against {:?}",
                    first.map(|at| &ours[at]),
                    first.map(|at| &theirs[at])
                );
            }
        }
    }
}

/// `record` reads the frame timeline itself: a frame compiled while the frame before is
/// pending, and recorded once that frame's point is reached, takes that frame's encoders
/// rather than making its own, which `GraphStats::encoders` shows.
#[test]
fn record_frees_the_encoders_of_a_frame_reached_since_compile() {
    for queues in LAYOUTS {
        let device = device(queues);
        let target = image(&*device, "target", Sharing::Exclusive);
        let gate = held(&*device);
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 1);
            let declare = |ready| {
                let mut decl = FrameDecl::new(1);
                let target = decl.import_image(&lactose_render_graph::ImageImport {
                    ready,
                    ..image_import(
                        "target",
                        target,
                        on(ImageState::Undefined, G),
                        on(ImageState::ColourAttachment, G),
                    )
                });
                decl.render_pass(&draws("draw", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
                decl
            };
            let first = frame(&mut graph, ctx, declare(Some(gate))).expect("a frame");
            assert_eq!(graph.stats().encoders, 1, "the first frame's, pending");
            let compiled = graph.compile(declare(None)).expect("a plan");
            assert_eq!(graph.stats().encoders, 1, "still pending at compile");
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), first);
            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            assert_eq!(graph.stats().encoders, 0, "the second frame took the first's encoder");
            let second = graph.submit(recorded).expect("a frame");
            assert_eq!(graph.stats().encoders, 1);
            reached(&**graph.device(), second);
        });
    }
}

/// Two jobs breach, the later in graph order first: the later job's recorder breaches and
/// then lets the earlier job's go on, which waits for it with a bound and breaches too.
/// `record` names the earlier job's pass, the first breach in graph order, not the first to
/// happen. The bounded wait is the test's own breach of "Recording"'s rule that a recorder
/// does not wait, made to order the two; it fails rather than hangs where the jobs do not
/// run at once.
#[test]
fn the_first_breach_in_graph_order_is_reported_when_a_later_job_breaches_first() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;
    for queues in LAYOUTS {
        let device = device(queues);
        let pipeline = compute_pipeline(&*device);
        let handles =
            [buffer(&*device, "a", Sharing::Exclusive), buffer(&*device, "b", Sharing::Exclusive)];
        let executor = executor(4, None);
        let saw_later = in_job_of(&executor, move |ctx| {
            let queue = home(&*device);
            let mut graph = graph(&device, 2, 1);
            let later_breached = Arc::new(AtomicBool::new(false));
            let saw_later = Arc::new(Mutex::new(None));
            let mut decl = FrameDecl::new(1);
            let [a, b] = handles.map(|handle| {
                decl.import_buffer(&fresh_buffer(
                    "written",
                    handle,
                    queue,
                    BufferState::ShaderWrite,
                ))
            });
            let (flag, saw) = (Arc::clone(&later_breached), Arc::clone(&saw_later));
            decl.compute_pass(
                &ComputePassDecl {
                    label: "earlier",
                    sampled: &[],
                    buffers: &[BufferUse { buffer: a, access: BufferAccess::Write }],
                    slots: &[],
                },
                move |pass| {
                    pass.bind_pipeline(pipeline);
                    let start = Instant::now();
                    while !flag.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(10)
                    {
                        std::thread::yield_now();
                    }
                    *lock(&saw) = Some(flag.load(Ordering::Acquire));
                    // `b` is the other pass's.
                    pass.push_address(0, b, 0);
                },
            );
            let flag = Arc::clone(&later_breached);
            decl.compute_pass(
                &ComputePassDecl {
                    label: "later",
                    sampled: &[],
                    buffers: &[BufferUse { buffer: b, access: BufferAccess::Write }],
                    slots: &[],
                },
                move |pass| {
                    pass.bind_pipeline(pipeline);
                    pass.push_address(0, a, 0);
                    flag.store(true, Ordering::Release);
                },
            );
            let compiled = graph.compile(decl).expect("a plan");
            assert_eq!(compiled.plan().jobs(), 2);
            let error = graph.record(compiled, ctx).err().expect("a breach");
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
            assert!(error.to_string().contains("`earlier`"), "{error}");
            *lock(&saw_later)
        });
        executor.shutdown();
        assert_eq!(saw_later, Some(true), "{queues:?}: the later job breached first");
    }
}
