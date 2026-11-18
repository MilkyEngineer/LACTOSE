//! The swapchain and the present role.

use std::time::Instant;

use lactose_core::Result;

use super::kit::{Body, Ctx, EXTENT, PATIENCE, buffer_to, clear, image_to};
use crate::{
    Acquired, BufferCopy, BufferState, BufferUsage, Device, Encoder, Extent, ImageHandle,
    ImageState, MemoryClass, Presented, QueuePath, QueueRole, Rule, Submit, Swapchain,
    SwapchainDesc, TimelinePoint,
};

type Applies = core::result::Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("second_acquire_before_a_present_is_refused", second_acquire_before_a_present_is_refused),
    ("present_of_an_image_not_acquired_is_refused", present_of_an_image_not_acquired_is_refused),
    ("present_behind_an_unmet_wait_is_refused", present_behind_an_unmet_wait_is_refused),
    (
        "present_of_an_image_not_in_present_is_refused",
        present_of_an_image_not_in_present_is_refused,
    ),
    (
        "swapchain_image_on_a_frame_encoder_is_refused",
        swapchain_image_on_a_frame_encoder_is_refused,
    ),
    (
        "swapchain_image_used_outside_its_acquire_is_refused",
        swapchain_image_used_outside_its_acquire_is_refused,
    ),
    ("second_swapchain_is_refused", second_swapchain_is_refused),
    (
        "swapchain_extent_with_a_zero_dimension_is_refused",
        swapchain_extent_with_a_zero_dimension_is_refused,
    ),
    ("resize_with_a_present_pending_is_refused", resize_with_a_present_pending_is_refused),
    ("image_of_before_a_resize_is_refused", image_of_before_a_resize_is_refused),
    (
        "frame_submits_complete_while_a_present_is_pending",
        frame_submits_complete_while_a_present_is_pending,
    ),
];

const P: QueueRole = QueueRole::Present;
const G: QueueRole = QueueRole::Graphics;
const DESC: SwapchainDesc = SwapchainDesc { extent: EXTENT };

fn swapchain(ctx: &Ctx<'_>, device: &dyn Device) -> Box<dyn Swapchain> {
    ctx.ok(device.create_swapchain(&DESC), "a swapchain")
}

/// The next image, acquired again while the bound passes with none.
fn acquire(ctx: &Ctx<'_>, swapchain: &mut dyn Swapchain) -> ImageHandle {
    let start = Instant::now();
    loop {
        match ctx.ok(swapchain.acquire(), "an acquire") {
            Acquired::Image(image) => return image,
            Acquired::None if start.elapsed() < PATIENCE => {}
            other => ctx.fail(format_args!("an acquire gave {other:?}")),
        }
    }
}

/// A present encoder that clears each of `images`, from `Undefined`, and leaves it in
/// `Present`.
fn frame(ctx: &Ctx<'_>, device: &dyn Device, images: &[ImageHandle]) -> Box<dyn Encoder> {
    let mut encoder = ctx.encoder(device, P);
    for &image in images {
        clear(&mut *encoder, image, None);
        encoder.transition_images(&[image_to(
            image,
            ImageState::ColourAttachment,
            ImageState::Present,
            P,
        )]);
    }
    encoder
}

fn present(
    swapchain: &mut dyn Swapchain,
    image: ImageHandle,
    encoders: &mut [Box<dyn Encoder>],
    waits: &[TimelinePoint],
    signals: &[TimelinePoint],
) -> Result<Presented> {
    swapchain.present(image, Submit { encoders, waits, signals })
}

/// Presents `image` with a frame of its own and waits for the present's submit.
fn show(ctx: &Ctx<'_>, device: &dyn Device, swapchain: &mut dyn Swapchain, image: ImageHandle) {
    let done = ctx.point(device, 1);
    let mut encoders = [frame(ctx, device, &[image])];
    let shown = ctx.ok(present(swapchain, image, &mut encoders, &[], &[done]), "a present");
    ctx.check(shown == Presented::Shown, "a present is shown");
    ctx.reached(device, done);
}

fn second_acquire_before_a_present_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let image = acquire(ctx, &mut *swapchain);
    ctx.refused(swapchain.acquire(), "a second acquire");
    show(ctx, &*device, &mut *swapchain, image);
    let image = acquire(ctx, &mut *swapchain);
    show(ctx, &*device, &mut *swapchain, image);
    Ok(())
}

fn present_of_an_image_not_acquired_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let target = ctx.target(&*device);
    let image = acquire(ctx, &mut *swapchain);
    ctx.refused(
        present(&mut *swapchain, target, &mut [], &[], &[]),
        "a present of an image not the swapchain's",
    );
    show(ctx, &*device, &mut *swapchain, image);
    ctx.refused(
        present(&mut *swapchain, image, &mut [], &[], &[]),
        "a present of an image presented already",
    );
    Ok(())
}

/// A present is refused past the bound while a wait it depends on is unmet, keeps the
/// image acquired, and is shown once the host signals.
fn present_behind_an_unmet_wait_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    // The present's own wait, or, on a queue it shares with graphics, an earlier frame
    // submit's.
    let image = acquire(ctx, &mut *swapchain);
    let unmet = ctx.point(&*device, 1);
    let done = ctx.point(&*device, 1);
    let mut encoders = [frame(ctx, &*device, &[image])];
    let waits: &[TimelinePoint] = if ctx.set.queues.present == QueuePath::Shared {
        ctx.submit(&*device, G, &mut [], &[unmet], &[]);
        &[]
    } else {
        &[unmet]
    };
    ctx.refused(
        present(&mut *swapchain, image, &mut encoders, waits, &[done]),
        "a present behind an unmet wait",
    );
    ctx.refused_by(
        swapchain.acquire(),
        Rule::Swapchain,
        "an acquire while the image is still acquired",
    );
    ctx.ok(device.signal_timeline(unmet), "the host signal of the wait");
    let shown =
        ctx.ok(present(&mut *swapchain, image, &mut encoders, waits, &[done]), "the present again");
    ctx.check(shown == Presented::Shown, "the present is shown");
    ctx.reached(&*device, done);
    // A frame submit waits, on the timeline it signals, for a point the host has not
    // signalled; the present waits for the frame's point.
    let image = acquire(ctx, &mut *swapchain);
    let timeline = ctx.timeline(&*device, 0);
    let (host, promised) =
        (TimelinePoint { timeline, value: 1 }, TimelinePoint { timeline, value: 2 });
    ctx.submit(&*device, G, &mut [], &[host], &[promised]);
    let done = ctx.point(&*device, 1);
    let mut encoders = [frame(ctx, &*device, &[image])];
    ctx.refused(
        present(&mut *swapchain, image, &mut encoders, &[promised], &[done]),
        "a present behind a frame's held promise",
    );
    ctx.ok(device.signal_timeline(host), "the host signal the frame waits for");
    let shown = ctx.ok(
        present(&mut *swapchain, image, &mut encoders, &[promised], &[done]),
        "the present again",
    );
    ctx.check(shown == Presented::Shown, "the present is shown");
    ctx.reached(&*device, done);
    Ok(())
}

fn present_of_an_image_not_in_present_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let image = acquire(ctx, &mut *swapchain);
    let mut encoder = ctx.encoder(&*device, P);
    clear(&mut *encoder, image, None);
    ctx.refused(
        present(&mut *swapchain, image, &mut [encoder], &[], &[]),
        "a present of an image left in `ColourAttachment`",
    );
    ctx.refused(
        present(&mut *swapchain, image, &mut [], &[], &[]),
        "a present of an image left `Undefined`",
    );
    show(ctx, &*device, &mut *swapchain, image);
    Ok(())
}

fn swapchain_image_on_a_frame_encoder_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let image = acquire(ctx, &mut *swapchain);
    let mut encoder = ctx.encoder(&*device, G);
    clear(&mut *encoder, image, None);
    let refused = device.submit(G, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a swapchain image on a graphics encoder");
    show(ctx, &*device, &mut *swapchain, image);
    Ok(())
}

fn swapchain_image_used_outside_its_acquire_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let first = acquire(ctx, &mut *swapchain);
    show(ctx, &*device, &mut *swapchain, first);
    let start = Instant::now();
    let second = loop {
        let image = acquire(ctx, &mut *swapchain);
        if image != first {
            break image;
        }
        if start.elapsed() > PATIENCE {
            ctx.fail("every acquire gave the same image");
        }
        show(ctx, &*device, &mut *swapchain, image);
    };
    let mut encoders = [frame(ctx, &*device, &[first, second])];
    ctx.refused(
        present(&mut *swapchain, second, &mut encoders, &[], &[]),
        "a frame that draws into the image of before",
    );
    show(ctx, &*device, &mut *swapchain, second);
    Ok(())
}

fn second_swapchain_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let first = swapchain(ctx, &*device);
    ctx.refused(device.create_swapchain(&DESC).map(drop), "a second swapchain");
    drop(first);
    let mut again = swapchain(ctx, &*device);
    let image = acquire(ctx, &mut *again);
    show(ctx, &*device, &mut *again, image);
    Ok(())
}

fn swapchain_extent_with_a_zero_dimension_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let narrow = SwapchainDesc { extent: Extent { width: 0, height: EXTENT.height } };
    ctx.refused(device.create_swapchain(&narrow).map(drop), "a swapchain of no width");
    let mut swapchain = swapchain(ctx, &*device);
    ctx.refused(
        swapchain.resize(Extent { width: EXTENT.width, height: 0 }),
        "a resize to no height",
    );
    let image = acquire(ctx, &mut *swapchain);
    show(ctx, &*device, &mut *swapchain, image);
    Ok(())
}

fn resize_with_a_present_pending_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let image = acquire(ctx, &mut *swapchain);
    let stalled = ctx.stall(&*device, true);
    show(ctx, &*device, &mut *swapchain, image);
    let larger = Extent { width: EXTENT.width * 2, height: EXTENT.height };
    if stalled {
        ctx.refused(swapchain.resize(larger), "a resize while the present's image is still read");
        ctx.stall(&*device, false);
    }
    ctx.ok(swapchain.resize(larger), "a resize once the present is done");
    let image = acquire(ctx, &mut *swapchain);
    show(ctx, &*device, &mut *swapchain, image);
    Ok(())
}

fn image_of_before_a_resize_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let before = acquire(ctx, &mut *swapchain);
    show(ctx, &*device, &mut *swapchain, before);
    ctx.ok(swapchain.resize(Extent { width: EXTENT.width, height: EXTENT.height * 2 }), "a resize");
    let image = acquire(ctx, &mut *swapchain);
    let mut encoders = [frame(ctx, &*device, &[image, before])];
    ctx.refused(
        present(&mut *swapchain, image, &mut encoders, &[], &[]),
        "a frame that draws into an image of before",
    );
    show(ctx, &*device, &mut *swapchain, image);
    Ok(())
}

/// While a present's image is still read, frame submits on the graphics queue complete,
/// and so does the present's own submit.
fn frame_submits_complete_while_a_present_is_pending(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.presenting()?;
    let mut swapchain = swapchain(ctx, &*device);
    let image = acquire(ctx, &mut *swapchain);
    let stalled = ctx.stall(&*device, true);
    show(ctx, &*device, &mut *swapchain, image);
    let size = 256;
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[3; 256]);
    let destination =
        ctx.buffer(&*device, size, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    for _ in 0..4 {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[
            buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
            buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, G),
        ]);
        encoder.copy_buffer(
            source,
            destination,
            &[BufferCopy { source_offset: 0, destination_offset: 0, size }],
        );
        let done = ctx.point(&*device, 1);
        ctx.submit(&*device, G, &mut [encoder], &[], &[done]);
        ctx.reached(&*device, done);
    }
    let next = acquire(ctx, &mut *swapchain);
    ctx.check(next != image || !stalled, "an acquire passes over an image still read");
    show(ctx, &*device, &mut *swapchain, next);
    if stalled {
        ctx.stall(&*device, false);
    }
    Ok(())
}
