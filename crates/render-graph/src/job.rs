//! One recording job: the steps of its plan, recorded into its one encoder (`README.md`,
//! "Recording").
//!
//! A job reads the declaration, the plan and the frame's handles, which every job shares,
//! and writes its encoder, its passes' recorders and its breach, which are its own: lent
//! as `&mut` from slices split by job.

use lactose_rhi_api::{
    BufferHandle, BufferReach, BufferTransition, ColourAttachment, ComputePass, DepthAttachment,
    Encoder, ImageHandle, ImageTransition, PassReach, RenderPass, Target,
};

use crate::decl::{Declared, DeclaredKind, Recorder};
use crate::graph::LOG_TARGET;
use crate::plan::{Begin, JobPlan, PassStep, Step, Transitions};
use crate::recorder::{ComputeRecorder, CopyRecorder, PassScope, RenderRecorder};

/// What every job of a frame reads.
pub(crate) struct Shared<'a> {
    pub(crate) declared: &'a Declared,
    /// The frame's images by id: an import's handle, a made transient's, or `None`.
    pub(crate) images: &'a [Option<ImageHandle>],
    /// The frame's buffers by id.
    pub(crate) buffers: &'a [BufferHandle],
}

/// What one job owns.
pub(crate) struct Owned<'a, 'f> {
    pub(crate) encoder: &'a mut dyn Encoder,
    /// The recorders of the declared passes `job.passes`, by pass from its start.
    pub(crate) recorders: &'a mut [Option<Recorder<'f>>],
    /// Where the job's first breach is written.
    pub(crate) breach: &'a mut Option<String>,
}

/// Records `job` into its encoder, stopping at its first breach.
pub(crate) fn run(shared: &Shared<'_>, job: &JobPlan, owned: Owned<'_, '_>) {
    let Owned { encoder, recorders, breach } = owned;
    for step in &job.steps {
        match step {
            Step::Transitions(transitions) => transition(shared, encoder, transitions),
            Step::Pass(step) => {
                let recorder = recorders[step.pass - job.passes.start].take();
                let recorder = recorder.expect("a kept pass's recorder is called once");
                pass(shared, step, recorder, encoder, breach);
                if breach.is_some() {
                    return;
                }
            }
        }
    }
}

fn transition(shared: &Shared<'_>, encoder: &mut dyn Encoder, transitions: &Transitions) {
    if !transitions.images.is_empty() {
        let images: Vec<ImageTransition> = transitions
            .images
            .iter()
            .map(|planned| ImageTransition {
                image: image(shared, planned.image),
                range: planned.range,
                from: planned.from,
                to: planned.to,
            })
            .collect();
        encoder.transition_images(&images);
    }
    if !transitions.buffers.is_empty() {
        let buffers: Vec<BufferTransition> = transitions
            .buffers
            .iter()
            .map(|planned| BufferTransition {
                buffer: shared.buffers[planned.buffer as usize],
                from: planned.from,
                to: planned.to,
            })
            .collect();
        encoder.transition_buffers(&buffers);
    }
}

/// The handle of an image a kept pass uses, which is made.
fn image(shared: &Shared<'_>, image: u32) -> ImageHandle {
    shared.images[image as usize].expect("an image a kept pass uses is made")
}

/// One kept pass, inside its span: its alias calls and transitions, its beginning, its
/// recorder and its end.
fn pass(
    shared: &Shared<'_>,
    step: &PassStep,
    recorder: Recorder<'_>,
    encoder: &mut dyn Encoder,
    breach: &mut Option<String>,
) {
    let declared = &shared.declared.passes[step.pass];
    let label = declared.label.as_str();
    let _span = tracing::trace_span!(target: LOG_TARGET, "pass", label).entered();
    for &transient in &step.alias {
        encoder.alias(image(shared, transient));
    }
    transition(shared, encoder, &step.before);
    let scope = PassScope {
        label,
        kind: &declared.kind,
        images: shared.images,
        buffers: shared.buffers,
        breach,
    };
    let reach = |reach: &[(u32, lactose_rhi_api::BufferState)]| -> Vec<BufferReach> {
        reach
            .iter()
            .map(|&(buffer, state)| BufferReach { buffer: shared.buffers[buffer as usize], state })
            .collect()
    };
    match (&declared.kind, &step.begin, recorder) {
        (
            DeclaredKind::Render { colour, depth, slots, .. },
            Begin::Render { colour: stores, depth: depth_store, reach: buffers },
            Recorder::Render(record),
        ) => {
            let colour: Vec<ColourAttachment> = colour
                .iter()
                .zip(stores)
                .map(|(use_, &store)| ColourAttachment {
                    target: Target {
                        image: image(shared, use_.image.index()),
                        mip: use_.mip,
                        layer: use_.layer,
                    },
                    load: use_.load,
                    store,
                    local_read: use_.local_read,
                })
                .collect();
            let depth = depth.zip(*depth_store).map(|(use_, store)| DepthAttachment {
                target: Target {
                    image: image(shared, use_.image.index()),
                    mip: use_.mip,
                    layer: use_.layer,
                },
                load: use_.load,
                store,
            });
            let buffers = reach(buffers);
            encoder.begin_render_pass(&RenderPass {
                label,
                colour: &colour,
                depth,
                reach: PassReach { slots, buffers: &buffers },
            });
            record(&mut RenderRecorder::new(encoder, scope));
            encoder.end_pass();
        }
        (
            DeclaredKind::Compute { slots, .. },
            Begin::Compute { reach: buffers },
            Recorder::Compute(record),
        ) => {
            let buffers = reach(buffers);
            encoder.begin_compute_pass(&ComputePass {
                label,
                reach: PassReach { slots, buffers: &buffers },
            });
            record(&mut ComputeRecorder::new(encoder, scope));
            encoder.end_pass();
        }
        (DeclaredKind::Copy { .. }, Begin::Copy, Recorder::Copy(record)) => {
            record(&mut CopyRecorder::new(encoder, scope));
        }
        _ => unreachable!("a pass's declaration, beginning and recorder are of its kind"),
    }
}
