//! The heap, the sampler table and what a pass declares.

use std::collections::HashSet;

use lactose_core::ErrorKind;

use super::kit::{Body, Ctx, EXTENT, buffer_to, compute_pass, image_desc, image_to, whole_image};
use crate::{
    AddressMode, BufferReach, BufferState, BufferUsage, ComputePipelineHandle, Device, Encoder,
    Filter, Format, HeapSlot, ImageState, ImageUsage, MemoryClass, PassReach, QueueRole, Rule,
    SamplerDesc, Submit,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("heap_slot_outside_the_heap_is_refused", heap_slot_outside_the_heap_is_refused),
    ("declared_slot_empty_is_refused", declared_slot_empty_is_refused),
    ("slot_written_after_its_pass_began_is_refused", slot_written_after_its_pass_began_is_refused),
    ("declared_slot_not_in_sampled_read_is_refused", declared_slot_not_in_sampled_read_is_refused),
    ("heap_write_under_a_pending_reader_is_refused", heap_write_under_a_pending_reader_is_refused),
    ("heap_write_after_its_reader_completes_passes", heap_write_after_its_reader_completes_passes),
    ("heap_write_racing_a_submit_refuses_one", heap_write_racing_a_submit_refuses_one),
    ("destroy_of_an_image_a_slot_holds_is_refused", destroy_of_an_image_a_slot_holds_is_refused),
    (
        "sampler_destroyed_under_a_pending_submit_is_refused",
        sampler_destroyed_under_a_pending_submit_is_refused,
    ),
    ("undeclared_slot_argument_is_refused", undeclared_slot_argument_is_refused),
    ("undeclared_address_argument_is_refused", undeclared_address_argument_is_refused),
    ("declared_buffer_in_another_state_is_refused", declared_buffer_in_another_state_is_refused),
    ("sampler_table_is_bounded", sampler_table_is_bounded),
];

const G: QueueRole = QueueRole::Graphics;

/// The slot the cases declare: past zero, so a backend that reads slot zero alone fails.
const SLOT: HeapSlot = HeapSlot(5);

/// Rounds of the race: enough for both orders on most runs.
const ROUNDS: usize = 24;

const SAMPLER: SamplerDesc = SamplerDesc {
    magnify: Filter::Linear,
    minify: Filter::Linear,
    mip: Filter::Nearest,
    address_u: AddressMode::ClampToEdge,
    address_v: AddressMode::Repeat,
};

fn submit(device: &dyn Device, encoders: &mut [Box<dyn Encoder>]) -> lactose_core::Result<()> {
    device.submit(G, Submit { encoders, waits: &[], signals: &[] })
}

/// Records a compute pass that declares `slot`, and a dispatch.
fn reader(encoder: &mut dyn Encoder, pipeline: ComputePipelineHandle, slot: HeapSlot) {
    compute_pass(encoder, pipeline, PassReach { slots: &[slot], buffers: &[] });
    encoder.push_slot(0, slot);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
}

fn heap_slot_outside_the_heap_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = ctx.sampled(&*device);
    let past = HeapSlot(ctx.set.heap_slots);
    ctx.refused(device.write_heap_slot(past, image), "a write of the slot past the heap");
    ctx.refused(device.clear_heap_slot(past), "a clear of the slot past the heap");
    ctx.refused(device.write_heap_slot(HeapSlot(u32::MAX), image), "a write of the last number");
    ctx.ok(
        device.write_heap_slot(HeapSlot(ctx.set.heap_slots - 1), image),
        "a write of the last slot",
    );
    Ok(())
}

fn declared_slot_empty_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    reader(&mut *encoder, pipeline, SLOT);
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a dispatch in a pass that declares an empty slot",
    );
    let image = ctx.sampled(&*device);
    ctx.ok(device.write_heap_slot(SLOT, image), "a write");
    ctx.ok(device.clear_heap_slot(SLOT), "a clear");
    let mut encoder = ctx.encoder(&*device, G);
    reader(&mut *encoder, pipeline, SLOT);
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a dispatch in a pass that declares a cleared slot",
    );
    Ok(())
}

fn slot_written_after_its_pass_began_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let (first, second) = (ctx.sampled(&*device), ctx.sampled(&*device));
    ctx.ok(device.write_heap_slot(SLOT, first), "a write");
    let mut encoder = ctx.encoder(&*device, G);
    reader(&mut *encoder, pipeline, SLOT);
    ctx.ok(device.write_heap_slot(SLOT, second), "a write after the pass began");
    let mut encoders = [encoder];
    ctx.refused(submit(&*device, &mut encoders), "a pass whose slot was written after it began");
    let [mut encoder] = encoders;
    ctx.ok(encoder.reset(), "a reset");
    reader(&mut *encoder, pipeline, SLOT);
    ctx.ok(submit(&*device, &mut [encoder]), "the pass recorded again");
    Ok(())
}

fn declared_slot_not_in_sampled_read_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let usage = ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION);
    let image = ctx.image(&*device, &image_desc(Format::Rgba8Unorm, EXTENT, usage));
    let source = ctx.upload(
        &*device,
        BufferUsage::EMPTY,
        &vec![1; (EXTENT.width * EXTENT.height * 4) as usize],
    );
    ctx.ok(device.write_heap_slot(SLOT, image), "a write");
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    encoder.copy_buffer_to_image(source, image, &[whole_image()]);
    reader(&mut *encoder, pipeline, SLOT);
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a declared slot whose image is in `CopyDestination`",
    );
    Ok(())
}

/// A pass that declares [`SLOT`], holding `image`, submitted and held pending.
fn held_reader(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    image: crate::ImageHandle,
) -> (super::kit::Gate, [Box<dyn Encoder>; 1]) {
    let pipeline = ctx.compute(device);
    ctx.ok(device.write_heap_slot(SLOT, image), "a write");
    let mut encoder = ctx.encoder(device, G);
    reader(&mut *encoder, pipeline, SLOT);
    let mut encoders = [encoder];
    let gate = ctx.held(device, G, &mut encoders);
    (gate, encoders)
}

fn heap_write_under_a_pending_reader_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (first, second) = (ctx.sampled(&*device), ctx.sampled(&*device));
    let (gate, _encoders) = held_reader(ctx, &*device, first);
    ctx.refused(device.write_heap_slot(SLOT, second), "a write of a slot a pending pass declares");
    ctx.refused(device.clear_heap_slot(SLOT), "a clear of a slot a pending pass declares");
    ctx.ok(device.write_heap_slot(HeapSlot(SLOT.0 + 1), second), "a write of another slot");
    gate.open(ctx, &*device);
    Ok(())
}

fn heap_write_after_its_reader_completes_passes(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (first, second) = (ctx.sampled(&*device), ctx.sampled(&*device));
    let (gate, _encoders) = held_reader(ctx, &*device, first);
    gate.open(ctx, &*device);
    ctx.ok(device.write_heap_slot(SLOT, second), "a write once the reader is seen complete");
    ctx.ok(device.clear_heap_slot(SLOT), "a clear");
    Ok(())
}

/// A write of a slot races the submit of a pass that declared the slot before the race:
/// exactly one is refused.
fn heap_write_racing_a_submit_refuses_one(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let (first, second) = (ctx.sampled(&*device), ctx.sampled(&*device));
    let mut encoders = [ctx.encoder(&*device, G)];
    for _ in 0..ROUNDS {
        ctx.ok(encoders[0].reset(), "a reset");
        ctx.ok(device.write_heap_slot(SLOT, first), "the slot's write before the pass");
        reader(&mut *encoders[0], pipeline, SLOT);
        let gate = ctx.gate(&*device);
        let device: &dyn Device = &*device;
        let (write, submit) = std::thread::scope(|scope| {
            let write = scope.spawn(move || device.write_heap_slot(SLOT, second));
            let encoders = &mut encoders;
            let submit = scope.spawn(move || {
                device.submit(G, Submit { encoders, waits: &[gate.wait], signals: &[gate.done] })
            });
            (
                write.join().unwrap_or_else(|_| ctx.fail("the writer panicked")),
                submit.join().unwrap_or_else(|_| ctx.fail("the submitter panicked")),
            )
        });
        let rule = |result: &lactose_core::Result<()>| result.as_ref().err().and_then(Rule::of);
        match (rule(&write), rule(&submit)) {
            (Some(Rule::HeapWrite), None) if submit.is_ok() => gate.open(ctx, device),
            (None, Some(Rule::HeapWrite | Rule::HeapLayout)) if write.is_ok() => {}
            _ => ctx.fail(format_args!(
                "not exactly one refused: the write {write:?}, the submit {submit:?}"
            )),
        }
    }
    Ok(())
}

fn destroy_of_an_image_a_slot_holds_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = ctx.sampled(&*device);
    ctx.ok(device.write_heap_slot(SLOT, image), "a write");
    ctx.ok(device.write_heap_slot(HeapSlot(SLOT.0 + 1), image), "a second slot's write");
    ctx.refused(device.destroy_image(image), "a destroy of an image two slots hold");
    ctx.ok(device.clear_heap_slot(SLOT), "a clear");
    ctx.refused(device.destroy_image(image), "a destroy of an image a slot holds");
    let other = ctx.sampled(&*device);
    ctx.ok(device.write_heap_slot(HeapSlot(SLOT.0 + 1), other), "an overwrite");
    ctx.ok(device.destroy_image(image), "the destroy once no slot holds it");
    Ok(())
}

fn sampler_destroyed_under_a_pending_submit_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let sampler = ctx.ok(device.create_sampler(&SAMPLER), "a sampler");
    let gate = ctx.held(&*device, G, &mut []);
    ctx.refused(device.destroy_sampler(sampler), "a sampler's destroy under a pending submit");
    gate.open(ctx, &*device);
    ctx.ok(device.destroy_sampler(sampler), "the destroy once the submit is seen complete");
    Ok(())
}

fn undeclared_slot_argument_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    ctx.ok(device.write_heap_slot(SLOT, ctx.sampled(&*device)), "a write");
    let mut encoder = ctx.encoder(&*device, G);
    compute_pass(
        &mut *encoder,
        pipeline,
        PassReach { slots: &[HeapSlot(SLOT.0 + 1)], buffers: &[] },
    );
    encoder.push_slot(0, SLOT);
    encoder.end_pass();
    ctx.refused(submit(&*device, &mut [encoder]), "a push of a slot the pass did not declare");
    Ok(())
}

fn undeclared_address_argument_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let usage = BufferUsage::DEVICE_ADDRESS;
    let declared = ctx.buffer(&*device, 256, usage, MemoryClass::Device);
    let other = ctx.buffer(&*device, 256, usage, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        declared,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    let reach = [BufferReach { buffer: declared, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &reach });
    encoder.push_address(0, other, 0);
    encoder.end_pass();
    ctx.refused(submit(&*device, &mut [encoder]), "a push of an address the pass did not declare");
    Ok(())
}

fn declared_buffer_in_another_state_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let buffer = ctx.upload(&*device, BufferUsage::DEVICE_ADDRESS, &[1; 256]);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        buffer,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    let reach = [BufferReach { buffer, state: BufferState::ShaderRead }];
    compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &reach });
    encoder.push_address(0, buffer, 0);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a buffer declared `ShaderRead` in `ShaderWrite`",
    );
    Ok(())
}

fn sampler_table_is_bounded(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let mut indices = HashSet::new();
    for _ in 0..ctx.set.samplers {
        let index = ctx.ok(device.create_sampler(&SAMPLER), "a sampler within the table");
        ctx.check(index.0 < ctx.set.samplers, "an index inside the table");
        ctx.check(indices.insert(index), "distinct indices");
    }
    ctx.fails(
        device.create_sampler(&SAMPLER),
        ErrorKind::ResourceExhausted,
        "a sampler past the table",
    );
    let freed = *indices.iter().next().unwrap_or_else(|| ctx.fail("a table of no samplers"));
    ctx.ok(device.destroy_sampler(freed), "a destroy");
    let again = ctx.ok(device.create_sampler(&SAMPLER), "a sampler in the freed entry");
    ctx.check(again.0 < ctx.set.samplers, "an index inside the table");
    Ok(())
}
