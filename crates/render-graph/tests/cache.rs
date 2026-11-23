//! The plan cache on null: each input of the key, changed alone, forces a recompile; the
//! same declaration again, over other handles, does not; the cap evicts; and a use moved
//! from one of a pass's lists to another is a key input.

use lactose_render_graph::{CopyPassDecl, FrameDecl, GraphKey, ImageId, ImageImport};
use lactose_rhi_api::{ImageHandle, ImageState, Load, QueueLayout, Sharing};

use crate::common::{G, colour, device, draws, graph, image, image_import, on};

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
