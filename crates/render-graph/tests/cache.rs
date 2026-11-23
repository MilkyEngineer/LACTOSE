//! The plan cache on null: each input of the key, changed alone, forces a recompile; the
//! same declaration again, over other handles, does not; the cap evicts; and a use moved
//! from one of a pass's lists to another is a key input.

use lactose_render_graph::{
    BufferAccess, BufferImport, BufferUse, ColourUse, CopyPassDecl, DepthUse, FrameDecl, GraphKey,
    ImageId, ImageImport, RenderPassDecl, TransientImage,
};
use lactose_rhi_api::{
    BufferHandle, BufferState, Device, Extent, Format, ImageDesc, ImageHandle, ImageMemory,
    ImageState, ImageUsage, Load, QueueLayout, Sharing,
};

use crate::common::{
    C, G, LAYOUTS, SCRATCH, buffer, colour, device, draws, graph, image, image_import, on,
};

fn declare(
    views: u32,
    target: ImageHandle,
    initial: ImageState,
    label: &'static str,
) -> FrameDecl<'static> {
    let mut decl = FrameDecl::new(views);
    let target = decl.import_image(&image_import(
        "target",
        target,
        on(initial, G),
        on(ImageState::ColourAttachment, G),
    ));
    decl.render_pass(&draws(label, &[colour(target, Load::DontCare)]), |_| {});
    decl
}

/// Each input of the key, changed alone, forces a recompile, the queue layout among them by
/// one declaration on two devices; another handle under the same declaration does not; and
/// the cap evicts the least recently used plan.
#[test]
fn each_input_of_the_key_changed_alone_recompiles() {
    let device = device(QueueLayout::ONE);
    let targets =
        [image(&*device, "a", Sharing::Exclusive), image(&*device, "b", Sharing::Exclusive)];
    let mut graph = graph(&device, 1, 2);
    let key: GraphKey = {
        let base =
            graph.compile(declare(1, targets[0], ImageState::Undefined, "draw")).expect("a plan");
        assert!(base.fresh());
        base.plan().key()
    };
    let again =
        graph.compile(declare(1, targets[1], ImageState::Undefined, "draw")).expect("a plan");
    assert!(!again.fresh());
    assert_eq!(again.plan().key(), key);
    drop(again);

    for changed in [
        declare(2, targets[0], ImageState::Undefined, "draw"),
        declare(1, targets[0], ImageState::ColourAttachment, "draw"),
        declare(1, targets[0], ImageState::Undefined, "renamed"),
    ] {
        let frame = graph.compile(changed).expect("a plan");
        assert!(frame.fresh());
        assert_ne!(frame.plan().key(), key);
    }
    let stats = graph.stats();
    assert_eq!((stats.compiles, stats.hits), (4, 1));
    assert_eq!((stats.plans, stats.evictions), (2, 2));

    let several = crate::common::device(QueueLayout::SEVERAL);
    let target = image(&*several, "a", Sharing::Exclusive);
    let mut other = crate::common::graph(&several, 1, 1);
    let there = other.compile(declare(1, target, ImageState::Undefined, "draw")).expect("a plan");
    assert!(there.fresh());
    assert_ne!(there.plan().key(), key);
}

/// A copy pass that only reads `b` writes nothing and is culled; the same ids with `b` a
/// destination make a pass that writes an import and is kept. A hit would replay the culled
/// plan.
#[test]
fn the_key_tells_a_passs_lists_apart() {
    let device = device(QueueLayout::ONE);
    let images =
        [image(&*device, "a", Sharing::Exclusive), image(&*device, "b", Sharing::Exclusive)];
    let mut graph = graph(&device, 1, 2);
    let declare = |b_is_written: bool| {
        let mut decl = FrameDecl::new(1);
        let import = |label, image| ImageImport {
            sharing: Sharing::Exclusive,
            ..image_import(
                label,
                image,
                on(ImageState::CopySource, G),
                on(ImageState::CopySource, G),
            )
        };
        let a = decl.import_image(&import("a", images[0]));
        let b = decl.import_image(&import("b", images[1]));
        let (sources, destinations): (&[ImageId], &[ImageId]) =
            if b_is_written { (&[a], &[b]) } else { (&[a, b], &[]) };
        let pass = decl.copy_pass(
            &CopyPassDecl {
                label: "p",
                image_sources: sources,
                image_destinations: destinations,
                buffer_sources: &[],
                buffer_destinations: &[],
            },
            |_| {},
        );
        (decl, pass)
    };
    let (reads, pass) = declare(false);
    let culled = graph.compile(reads).expect("a plan");
    assert!(culled.plan().is_culled(pass));
    let (writes, pass) = declare(true);
    let kept = graph.compile(writes).expect("a plan");
    assert!(kept.fresh());
    assert!(!kept.plan().is_culled(pass));
    assert_ne!(kept.plan().key(), culled.plan().key());
}

/// One input of the key's base declaration changed alone, or none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Input {
    None,
    Views,
    ImportLabel,
    ImportMips,
    ImportLayers,
    ImportSharing,
    ImportInitialState,
    ImportInitialQueue,
    ImportLast,
    ImportHandoff,
    TransientLabel,
    TransientFormat,
    TransientExtent,
    TransientMips,
    TransientLayers,
    BufferLabel,
    BufferSharing,
    BufferInitial,
    BufferLast,
    PassLabel,
    ColourImage,
    ColourMip,
    ColourLayer,
    ColourLoad,
    LocalRead,
    DepthLoad,
    Sampled,
    BufferAccessOf,
}

const INPUTS: [Input; 28] = [
    Input::None,
    Input::Views,
    Input::ImportLabel,
    Input::ImportMips,
    Input::ImportLayers,
    Input::ImportSharing,
    Input::ImportInitialState,
    Input::ImportInitialQueue,
    Input::ImportLast,
    Input::ImportHandoff,
    Input::TransientLabel,
    Input::TransientFormat,
    Input::TransientExtent,
    Input::TransientMips,
    Input::TransientLayers,
    Input::BufferLabel,
    Input::BufferSharing,
    Input::BufferInitial,
    Input::BufferLast,
    Input::PassLabel,
    Input::ColourImage,
    Input::ColourMip,
    Input::ColourLayer,
    Input::ColourLoad,
    Input::LocalRead,
    Input::DepthLoad,
    Input::Sampled,
    Input::BufferAccessOf,
];

/// The key's base declaration, with `input` changed: an import, a sampled import, a colour
/// and a depth transient and a buffer, drawn by one render pass and read back by a copy.
fn declare_with(
    input: Input,
    target: ImageHandle,
    texture: ImageHandle,
    out: BufferHandle,
) -> FrameDecl<'static> {
    let changed = |which: Input| input == which;
    let mut decl = FrameDecl::new(if changed(Input::Views) { 2 } else { 1 });
    let initial = match input {
        Input::ImportInitialState => on(ImageState::SampledRead, G),
        Input::ImportInitialQueue => on(ImageState::ColourAttachment, C),
        _ => on(ImageState::ColourAttachment, G),
    };
    let target = decl.import_image(&ImageImport {
        label: if changed(Input::ImportLabel) { "target, renamed" } else { "target" },
        image: target,
        mips: if changed(Input::ImportMips) { 3 } else { 2 },
        layers: if changed(Input::ImportLayers) { 3 } else { 2 },
        sharing: if changed(Input::ImportSharing) {
            Sharing::Concurrent
        } else {
            Sharing::Exclusive
        },
        initial,
        last: on(
            if changed(Input::ImportLast) {
                ImageState::SampledRead
            } else {
                ImageState::ColourAttachment
            },
            G,
        ),
        handoff: changed(Input::ImportHandoff).then_some(on(ImageState::SampledRead, C)),
        ready: None,
    });
    let sampled = on(ImageState::SampledRead, G);
    let texture = decl.import_image(&image_import("texture", texture, sampled, sampled));
    let scratch = decl.transient_image(&TransientImage {
        label: if changed(Input::TransientLabel) { "scratch, renamed" } else { "scratch" },
        format: if changed(Input::TransientFormat) {
            Format::Rgba16Float
        } else {
            Format::Rgba8Unorm
        },
        extent: if changed(Input::TransientExtent) {
            Extent { width: 32, height: 32 }
        } else {
            SCRATCH.extent
        },
        mips: if changed(Input::TransientMips) { 2 } else { 1 },
        layers: if changed(Input::TransientLayers) { 2 } else { 1 },
    });
    let depth = decl.transient_image(&TransientImage {
        label: "depth",
        format: Format::Depth32Float,
        ..SCRATCH
    });
    let out = decl.import_buffer(&BufferImport {
        label: if changed(Input::BufferLabel) { "out, renamed" } else { "out" },
        buffer: out,
        sharing: if changed(Input::BufferSharing) {
            Sharing::Concurrent
        } else {
            Sharing::Exclusive
        },
        initial: on(
            if changed(Input::BufferInitial) {
                BufferState::ShaderWrite
            } else {
                BufferState::ShaderRead
            },
            G,
        ),
        last: on(
            if changed(Input::BufferLast) {
                BufferState::CopyDestination
            } else {
                BufferState::HostRead
            },
            G,
        ),
        ready: None,
    });
    let mut colour = [
        ColourUse {
            image: target,
            mip: if changed(Input::ColourMip) { 1 } else { 0 },
            layer: if changed(Input::ColourLayer) { 1 } else { 0 },
            load: if changed(Input::ColourLoad) { Load::Keep } else { Load::Clear([0.0; 4]) },
            local_read: false,
        },
        ColourUse {
            image: scratch,
            mip: 0,
            layer: 0,
            load: Load::Clear([0.0; 4]),
            local_read: changed(Input::LocalRead),
        },
    ];
    if changed(Input::ColourImage) {
        colour.swap(0, 1);
    }
    let sampled: &[ImageId] = if changed(Input::Sampled) { &[] } else { &[texture] };
    let buffers = [BufferUse {
        buffer: out,
        access: if changed(Input::BufferAccessOf) {
            BufferAccess::Write
        } else {
            BufferAccess::Read
        },
    }];
    decl.render_pass(
        &RenderPassDecl {
            label: if changed(Input::PassLabel) { "draw, renamed" } else { "draw" },
            colour: &colour,
            depth: Some(DepthUse {
                image: depth,
                mip: 0,
                layer: 0,
                load: if changed(Input::DepthLoad) { Load::DontCare } else { Load::Clear(1.0) },
            }),
            sampled,
            buffers: &buffers,
            slots: &[],
        },
        |_| {},
    );
    decl.copy_pass(
        &CopyPassDecl {
            label: "read",
            image_sources: &[scratch],
            image_destinations: &[],
            buffer_sources: &[],
            buffer_destinations: &[out],
        },
        |_| {},
    );
    decl
}

/// Every input the key hashes ("Compile and its cache"), changed alone from one base
/// declaration, compiles a plan of its own, and the same declaration again is a hit: an
/// input left out of the key would make the two share a plan, with its load, size, format or
/// closing transitions. The queue layout is the test above's.
#[test]
fn every_input_of_the_key_changed_alone_compiles_a_plan_of_its_own() {
    for queues in LAYOUTS {
        let device = device(queues);
        let target = device
            .create_image(&ImageDesc {
                label: "target",
                format: Format::Rgba8Unorm,
                extent: SCRATCH.extent,
                mips: 3,
                layers: 3,
                usage: ImageUsage::COLOUR.union(ImageUsage::SAMPLED),
                memory: ImageMemory::Own,
                sharing: Sharing::Exclusive,
            })
            .expect("an image");
        let texture = image(&*device, "texture", Sharing::Exclusive);
        let out = buffer(&*device, "out", Sharing::Exclusive);
        let mut graph = graph(&device, 1, 64);
        let mut shared = Vec::new();
        for input in INPUTS {
            let first = graph.compile(declare_with(input, target, texture, out)).expect("a plan");
            if !first.fresh() {
                shared.push(input);
            }
            drop(first);
            let again = graph.compile(declare_with(input, target, texture, out)).expect("a plan");
            assert!(!again.fresh(), "{queues:?}, {input:?}: compiled twice");
        }
        assert!(
            shared.is_empty(),
            "{queues:?}: changed alone, these hit another's plan: {shared:?}"
        );
        assert_eq!(graph.stats().compiles, INPUTS.len() as u64);
    }
}
