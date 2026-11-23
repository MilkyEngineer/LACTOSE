//! Tracing: one `pass` span for each kept pass, named by its label, and none for a culled
//! one, counted by a subscriber installed process-wide (`README.md`, "Proof"). The jobs'
//! spans open on the workers the jobs run on, so a subscriber set for the calling thread
//! alone would see the spans of a serial recording only.
//!
//! One test only: it installs a process-wide subscriber.

#![forbid(unsafe_code)]

mod common;

use core::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use common::{EXTENT, G, buffer, device, executor, in_job_of, lock, on};
use lactose_render_graph::{
    BufferImport, ColourUse, CopyPassDecl, FrameDecl, GraphDesc, RenderGraph, RenderPassDecl,
    TransientImage,
};
use lactose_rhi_api::{
    BufferImageCopy, BufferState, Format, Load, QueueLayout, Rect, Sharing, TimelinePoint,
};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// Records the label and the opening thread of every span under the `gouda` target.
#[derive(Clone, Default)]
struct Spans {
    opened: Arc<Mutex<Vec<(String, String, ThreadId)>>>,
    next: Arc<AtomicU64>,
}

struct Label(String);

impl Visit for Label {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "label" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "label" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for Spans {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "gouda"
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut label = Label(String::new());
        span.record(&mut label);
        let name = span.metadata().name().to_owned();
        lock(&self.opened).push((name, label.0, std::thread::current().id()));
        Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, _event: &Event<'_>) {}
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
}

const KEPT: [&str; 4] = ["draw a", "read a", "draw b", "read b"];

#[test]
fn a_frame_opens_one_span_for_each_kept_pass() {
    let spans = Spans::default();
    tracing::subscriber::set_global_default(spans.clone()).expect("the first subscriber");
    let device = device(QueueLayout::ONE);
    let readbacks = [
        buffer(&*device, "readback a", Sharing::Exclusive),
        buffer(&*device, "readback b", Sharing::Exclusive),
    ];
    let executor = executor(4, None);
    let caller = in_job_of(&executor, move |ctx| {
        // Four jobs, one a kept pass, so each may open its span on another worker.
        let mut graph =
            RenderGraph::new(device, &GraphDesc { jobs: 4, plans: 1 }).expect("a graph");
        for _ in 0..3 {
            let mut decl = FrameDecl::new(1);
            let scratch = TransientImage {
                label: "a",
                format: Format::Rgba8Unorm,
                extent: EXTENT,
                mips: 1,
                layers: 1,
            };
            let images = [
                decl.transient_image(&scratch),
                decl.transient_image(&TransientImage { label: "b", ..scratch }),
            ];
            let unused = decl.transient_image(&TransientImage { label: "unused", ..scratch });
            for (at, (&image, readback)) in images.iter().zip(readbacks).enumerate() {
                let out = decl.import_buffer(&BufferImport {
                    label: ["out a", "out b"][at],
                    buffer: readback,
                    sharing: Sharing::Exclusive,
                    initial: on(BufferState::Undefined, G),
                    last: on(BufferState::HostRead, G),
                    ready: None,
                });
                let colour = [ColourUse {
                    image,
                    mip: 0,
                    layer: 0,
                    load: Load::Clear([0.0; 4]),
                    local_read: false,
                }];
                decl.render_pass(
                    &RenderPassDecl {
                        label: KEPT[2 * at],
                        colour: &colour,
                        depth: None,
                        sampled: &[],
                        buffers: &[],
                        slots: &[],
                    },
                    |_| {},
                );
                decl.copy_pass(
                    &CopyPassDecl {
                        label: KEPT[2 * at + 1],
                        image_sources: &[image],
                        image_destinations: &[],
                        buffer_sources: &[],
                        buffer_destinations: &[out],
                    },
                    move |copy| {
                        let region = BufferImageCopy {
                            buffer_offset: 0,
                            mip: 0,
                            layer: 0,
                            rect: Rect { x: 0, y: 0, extent: EXTENT },
                        };
                        copy.copy_image_to_buffer(image, out, &[region]);
                    },
                );
            }
            let colour = [ColourUse {
                image: unused,
                mip: 0,
                layer: 0,
                load: Load::Clear([0.0; 4]),
                local_read: false,
            }];
            decl.render_pass(
                &RenderPassDecl {
                    label: "culled",
                    colour: &colour,
                    depth: None,
                    sampled: &[],
                    buffers: &[],
                    slots: &[],
                },
                |_| {},
            );
            let compiled = graph.compile(decl).expect("a plan");
            assert_eq!(compiled.plan().jobs(), 4);
            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            let point: TimelinePoint = graph.submit(recorded).expect("a submitted frame");
            assert!(graph.device().wait_timeline(point, common::PATIENCE).expect("a wait"));
        }
        std::thread::current().id()
    });
    executor.shutdown();

    let opened = lock(&spans.opened).clone();
    let passes: Vec<&(String, String, ThreadId)> =
        opened.iter().filter(|(name, _, _)| name == "pass").collect();
    assert_eq!(passes.len(), 3 * KEPT.len(), "{opened:?}");
    for label in KEPT {
        let count = passes.iter().filter(|(_, opened, _)| opened == label).count();
        assert_eq!(count, 3, "pass `{label}`: {opened:?}");
    }
    assert!(passes.iter().all(|(_, label, _)| label != "culled"), "{opened:?}");
    // Not a promise of where a job runs, only a witness that the count above is not the
    // calling worker's alone: four frame-critical workers took the jobs of three frames.
    let threads: HashSet<ThreadId> = passes.iter().map(|&&(_, _, thread)| thread).collect();
    eprintln!("pass spans opened on {} threads, the caller {caller:?}", threads.len());
}
