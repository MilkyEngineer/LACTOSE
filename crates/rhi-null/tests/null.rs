//! What null alone holds, beyond the suite's cases: the device loss and the stall through
//! [`Control`], the host read of a range the device wrote with no invalidate, the cache's
//! start, two races against a present (submits on two queues, and a destroy of the
//! timeline its wait names), and the frame beside a present thread whose submit is held.

use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use lactose_core::ErrorKind;
use lactose_rhi_api::{
    Acquired, BufferCopy, BufferDesc, BufferHandle, BufferReach, BufferState, BufferTransition,
    BufferUsage, CacheStart, Capabilities, ColourAttachment, ComputePass, Device, Encoder, Extent,
    Format, ImageDesc, ImageHandle, ImageMemory, ImageRange, ImageState, ImageTransition,
    ImageUsage, Load, MemoryClass, PassReach, Presented, QueueLayout, QueuePath, QueueRole,
    RenderPass, Rule, Sharing, StateOn, Store, Submit, Swapchain, SwapchainDesc, Target,
    TimelinePoint,
};
use lactose_rhi_null::NullDevice;

use crate::{Silent, request};

const PATIENCE: Duration = Duration::from_secs(20);
const SIZE: u64 = 64;

fn device(set: &Capabilities, presents: bool) -> NullDevice {
    NullDevice::new(request(set, presents, Arc::new(Silent)), *set).expect("a null device")
}

fn rule<T>(result: &lactose_core::Result<T>) -> Option<Rule> {
    result.as_ref().err().and_then(Rule::of)
}

fn buffer(device: &dyn Device, usage: BufferUsage, memory: MemoryClass) -> BufferHandle {
    let desc = BufferDesc { label: "null", size: SIZE, usage, memory, sharing: Sharing::Exclusive };
    device.create_buffer(&desc).expect("a buffer")
}

fn on_graphics(buffer: BufferHandle, from: BufferState, to: BufferState) -> BufferTransition {
    let queue = QueueRole::Graphics;
    BufferTransition {
        buffer,
        from: StateOn { state: from, queue },
        to: StateOn { state: to, queue },
    }
}

fn point(device: &dyn Device, value: u64) -> TimelinePoint {
    TimelinePoint { timeline: device.create_timeline(0).expect("a timeline"), value }
}

fn reached(device: &dyn Device, point: TimelinePoint) {
    assert!(device.wait_timeline(point, PATIENCE).expect("a wait"), "a point expected reached");
}

#[test]
fn a_loss_reaches_every_call_and_the_control_outlives_the_device() {
    let device = device(&Capabilities::LEAST, false);
    let control = device.control();
    let timeline = device.create_timeline(0).expect("a timeline");
    assert!(!device.is_lost());
    assert!(control.lose(), "a live device is lost");
    assert!(device.is_lost());
    let kind = |result: lactose_core::Result<()>| result.map_err(|error| error.kind());
    assert_eq!(
        kind(device.signal_timeline(TimelinePoint { timeline, value: 1 })),
        Err(ErrorKind::DeviceLost)
    );
    assert_eq!(
        device
            .wait_timeline(TimelinePoint { timeline, value: 1 }, PATIENCE)
            .map_err(|error| error.kind()),
        Err(ErrorKind::DeviceLost),
    );
    let submitted =
        device.submit(QueueRole::Graphics, Submit { encoders: &mut [], waits: &[], signals: &[] });
    assert_eq!(kind(submitted), Err(ErrorKind::DeviceLost));
    drop(device);
    assert!(!control.lose(), "a device that has gone is not lost again");
    assert!(!control.stall(true), "nor stalled");
}

/// On memory that is not coherent, a host read of a range a copy wrote is refused until
/// it is invalidated; on coherent memory the invalidate is still called, and the same
/// read without it is accepted.
#[test]
fn a_read_of_a_written_range_needs_an_invalidate_where_memory_is_not_coherent() {
    for coherent in [false, true] {
        let set = Capabilities { coherent_host_memory: coherent, ..Capabilities::LEAST };
        let device = device(&set, false);
        let source = buffer(&device, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
        let readback = buffer(&device, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
        device.write_buffer(source, 0, SIZE, &mut |lent| lent.fill(7)).expect("a host write");
        device.flush_buffer(source, 0, SIZE).expect("a flush");
        let mut encoder = device.create_encoder(QueueRole::Graphics, "null").expect("an encoder");
        encoder.transition_buffers(&[
            on_graphics(source, BufferState::Undefined, BufferState::CopySource),
            on_graphics(readback, BufferState::Undefined, BufferState::CopyDestination),
        ]);
        encoder.copy_buffer(
            source,
            readback,
            &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
        );
        encoder.transition_buffers(&[on_graphics(
            readback,
            BufferState::CopyDestination,
            BufferState::HostRead,
        )]);
        let done = point(&device, 1);
        device
            .submit(
                QueueRole::Graphics,
                Submit { encoders: &mut [encoder], waits: &[], signals: &[done] },
            )
            .expect("a submit");
        reached(&device, done);
        let mut out = Vec::new();
        let read = device.read_buffer(readback, 0, SIZE, &mut |lent| out.extend_from_slice(lent));
        if coherent {
            read.expect("a read of coherent memory with no invalidate");
            assert_eq!(out, [7; SIZE as usize]);
        } else {
            assert_eq!(rule(&read), Some(Rule::Unflushed), "{read:?}");
            device.invalidate_buffer(readback, 8, 8).expect("an invalidate of part");
            let part = device.read_buffer(readback, 8, 8, &mut |lent| assert_eq!(lent, [7; 8]));
            part.expect("a read of the invalidated part");
            let rest = device.read_buffer(readback, 0, 16, &mut |_| {});
            assert_eq!(rule(&rest), Some(Rule::Unflushed), "{rest:?}");
            // An invalidate made while the copy is pending is refused, and so is not taken
            // for the invalidate after it: the read once the copy has run still needs one.
            device.invalidate_buffer(readback, 0, SIZE).expect("an invalidate of the rest");
            let mut encoder =
                device.create_encoder(QueueRole::Graphics, "null").expect("an encoder");
            encoder.transition_buffers(&[on_graphics(
                readback,
                BufferState::HostRead,
                BufferState::CopyDestination,
            )]);
            encoder.copy_buffer(
                source,
                readback,
                &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
            );
            encoder.transition_buffers(&[on_graphics(
                readback,
                BufferState::CopyDestination,
                BufferState::HostRead,
            )]);
            let (gate, done) = (point(&device, 1), point(&device, 1));
            // Kept past the gate: the drop of a pending encoder waits for its submit.
            let mut encoders = [encoder];
            device
                .submit(
                    QueueRole::Graphics,
                    Submit { encoders: &mut encoders, waits: &[gate], signals: &[done] },
                )
                .expect("a held copy");
            let early = device.invalidate_buffer(readback, 0, SIZE);
            assert_eq!(
                rule(&early),
                Some(Rule::Pending),
                "an invalidate under the copy: {early:?}"
            );
            device.signal_timeline(gate).expect("the gate");
            reached(&device, done);
            let read = device.read_buffer(readback, 0, SIZE, &mut |_| {});
            assert_eq!(rule(&read), Some(Rule::Unflushed), "the read after the copy: {read:?}");
            device.invalidate_buffer(readback, 0, SIZE).expect("the invalidate after the copy");
            device.read_buffer(readback, 0, SIZE, &mut |_| {}).expect("the read after it");
        }
    }
}

#[test]
fn a_cache_starts_empty_loaded_or_rejected() {
    let set = Capabilities::LEAST;
    let first = device(&set, false);
    assert_eq!(first.pipeline_stats().cache, CacheStart::Empty);
    let bytes = first.pipeline_cache().expect("the cache's bytes");
    let start = |bytes: Vec<u8>| {
        let request = lactose_rhi_api::DeviceRequest {
            pipeline_cache: bytes,
            ..request(&set, false, Arc::new(Silent))
        };
        NullDevice::new(request, set).expect("a device whatever its cache").pipeline_stats().cache
    };
    assert_eq!(start(bytes.clone()), CacheStart::Loaded);
    let mut corrupt = bytes.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert_eq!(start(corrupt), CacheStart::Rejected);
    assert_eq!(start(bytes[..bytes.len() / 2].to_vec()), CacheStart::Rejected);
    assert_eq!(start(b"no cache".to_vec()), CacheStart::Rejected);
}

/// A destroyed buffer's index is taken by the next buffer made: an encoder that records the
/// old handle resolves nothing through it, and its submit is refused `destroyed`, not by the
/// new buffer's usage.
#[test]
fn a_handle_whose_index_another_buffer_took_resolves_nothing() {
    let device = device(&Capabilities::LEAST, false);
    let copied = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let old = buffer(&device, copied, MemoryClass::Device);
    device.destroy_buffer(old).expect("a destroy");
    let new = buffer(&device, BufferUsage::INDEX, MemoryClass::Device);
    assert_eq!((new.index(), new == old), (old.index(), false), "the index taken again");
    let other = buffer(&device, copied, MemoryClass::Device);
    let mut encoder = device.create_encoder(QueueRole::Graphics, "null").expect("an encoder");
    encoder.transition_buffers(&[
        on_graphics(old, BufferState::Undefined, BufferState::CopySource),
        on_graphics(other, BufferState::Undefined, BufferState::CopyDestination),
    ]);
    let region = BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE };
    encoder.copy_buffer(old, other, &[region]);
    let refused = device
        .submit(QueueRole::Graphics, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    assert_eq!(rule(&refused), Some(Rule::Destroyed), "{refused:?}");
}

/// Refusals null makes that no contract case provokes, each by the rule null gives it: a
/// copy from a buffer to itself (`usage`), a copy region of no bytes and an address past
/// its buffer's end (`bounds`), and a destroy of a swapchain's image (`swapchain`).
#[test]
fn the_refusals_null_alone_makes_name_their_rules() {
    let device = device(&Capabilities::LEAST, true);
    let submit = |encoder: Box<dyn Encoder>| {
        device.submit(
            QueueRole::Graphics,
            Submit { encoders: &mut [encoder], waits: &[], signals: &[] },
        )
    };
    let encoder = || device.create_encoder(QueueRole::Graphics, "null").expect("an encoder");
    let copied = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let both = buffer(&device, copied, MemoryClass::Device);
    let mut itself = encoder();
    itself.copy_buffer(
        both,
        both,
        &[BufferCopy { source_offset: 0, destination_offset: 32, size: 16 }],
    );
    let refused = submit(itself);
    assert_eq!(rule(&refused), Some(Rule::Usage), "a copy from a buffer to itself: {refused:?}");
    let other = buffer(&device, copied, MemoryClass::Device);
    let mut empty = encoder();
    empty.copy_buffer(
        both,
        other,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: 0 }],
    );
    let refused = submit(empty);
    assert_eq!(rule(&refused), Some(Rule::Bounds), "a copy region of no bytes: {refused:?}");
    let addressed = buffer(&device, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
    let reach = [BufferReach { buffer: addressed, state: BufferState::ShaderRead }];
    let mut past = encoder();
    past.begin_compute_pass(&ComputePass {
        label: "null",
        reach: PassReach { slots: &[], buffers: &reach },
    });
    past.push_address(0, addressed, SIZE);
    past.end_pass();
    let refused = submit(past);
    assert_eq!(rule(&refused), Some(Rule::Bounds), "an address past the buffer's end: {refused:?}");
    let mut swapchain = device
        .create_swapchain(&SwapchainDesc {
            extent: lactose_rhi_api::Extent { width: 8, height: 8 },
        })
        .expect("a swapchain");
    let image = acquire(&mut *swapchain);
    let refused = device.destroy_image(image);
    assert_eq!(
        rule(&refused),
        Some(Rule::Swapchain),
        "a destroy of a swapchain's image: {refused:?}"
    );
}

/// A present encoder that clears `image` and leaves it in `Present`.
fn frame(device: &dyn Device, image: ImageHandle) -> Box<dyn Encoder> {
    let queue = QueueRole::Present;
    let mut encoder = device.create_encoder(queue, "null").expect("an encoder");
    let on = |state| StateOn { state, queue };
    let range = ImageRange::FIRST;
    let to = |from, to| ImageTransition { image, range, from: on(from), to: on(to) };
    encoder.transition_images(&[to(ImageState::Undefined, ImageState::ColourAttachment)]);
    let colour = [ColourAttachment {
        target: Target { image, mip: 0, layer: 0 },
        load: Load::Clear([0.0; 4]),
        store: Store::Store,
        local_read: false,
    }];
    encoder.begin_render_pass(&RenderPass {
        label: "null",
        colour: &colour,
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.end_pass();
    encoder.transition_images(&[to(ImageState::ColourAttachment, ImageState::Present)]);
    encoder
}

fn acquire(swapchain: &mut dyn Swapchain) -> ImageHandle {
    for _ in 0..20 {
        match swapchain.acquire().expect("an acquire") {
            Acquired::Image(image) => return image,
            Acquired::None => {}
            Acquired::Stale => panic!("a null swapchain is never stale"),
        }
    }
    panic!("no image in twenty bounds")
}

/// The sets with a queue of a role's own and with roles sharing one.
const LAYOUTS: [QueueLayout; 3] = [
    QueueLayout::SEVERAL,
    QueueLayout { compute: QueuePath::Own, present: QueuePath::Shared },
    QueueLayout { compute: QueuePath::Shared, present: QueuePath::Own },
];

/// Rounds of each race.
const ROUNDS: u64 = 40;

/// A graphics thread and a compute thread submit while the present thread presents, each
/// round, a frame that waits for the graphics thread's point of that round, whether that
/// submit is made before the present or while it holds. Covers: the present's walk under
/// its queue's lock against submits to its queue and another, its wait with no lock held
/// (on a shared queue a wait under the lock would keep the graphics submit out until the
/// bound, and the present would be refused), and the order of the locks. Does not cover:
/// a particular interleaving inside the walk; the unit tests fix those. A failed round is
/// kept, not panicked on, until the other two threads have met every round's barrier, so
/// that the scope joins and the test fails with its message.
#[test]
fn submits_on_two_queues_race_a_present() {
    for queues in LAYOUTS {
        let set = Capabilities { queues, ..Capabilities::LEAST };
        let device = device(&set, true);
        let mut swapchain = device
            .create_swapchain(&SwapchainDesc {
                extent: lactose_rhi_api::Extent { width: 8, height: 8 },
            })
            .expect("a swapchain");
        let frames = device.create_timeline(0).expect("a timeline");
        let work = device.create_timeline(0).expect("a timeline");
        let presented = device.create_timeline(0).expect("a timeline");
        let start = Barrier::new(3);
        let device: &dyn Device = &device;
        let mut refusals = 0;
        let mut failure = None;
        thread::scope(|scope| {
            scope.spawn(|| {
                for round in 1..=ROUNDS {
                    start.wait();
                    let signal = [TimelinePoint { timeline: frames, value: round }];
                    let submit = Submit { encoders: &mut [], waits: &[], signals: &signal };
                    device.submit(QueueRole::Graphics, submit).expect("a frame submit");
                }
            });
            scope.spawn(|| {
                for round in 1..=ROUNDS {
                    start.wait();
                    let signal = [TimelinePoint { timeline: work, value: round }];
                    let submit = Submit { encoders: &mut [], waits: &[], signals: &signal };
                    device.submit(QueueRole::Compute, submit).expect("a compute submit");
                }
            });
            for round in 1..=ROUNDS {
                if failure.is_some() {
                    start.wait();
                    continue;
                }
                let image = acquire(&mut *swapchain);
                let mut encoders = [frame(device, image)];
                start.wait();
                let waits = [TimelinePoint { timeline: frames, value: round }];
                let signals = [TimelinePoint { timeline: presented, value: round }];
                let mut refused = 0;
                let outcome = loop {
                    let submit =
                        Submit { encoders: &mut encoders, waits: &waits, signals: &signals };
                    match swapchain.present(image, submit) {
                        Ok(Presented::Shown) => break Ok(()),
                        Ok(shown) => break Err(format!("the present returned {shown:?}")),
                        // A loaded machine may keep the graphics thread past the bound:
                        // the caller retries. Three bounds in a row are no load: the frame
                        // submit is kept out, as by a present that waits under the lock.
                        Err(error) if Rule::of(&error) == Some(Rule::Pending) => {
                            refusals += 1;
                            refused += 1;
                            if refused == 3 {
                                break Err(format!(
                                    "the present was refused past its bound {refused} times: \
                                     {error}"
                                ));
                            }
                        }
                        Err(error) => break Err(error.to_string()),
                    }
                };
                let outcome =
                    outcome.and_then(|()| match device.wait_timeline(signals[0], PATIENCE) {
                        Ok(true) => Ok(()),
                        Ok(false) => Err("the present's signal was not reached".to_owned()),
                        Err(error) => Err(error.to_string()),
                    });
                failure = outcome.err().map(|why| format!("round {round} under {queues:?}: {why}"));
            }
        });
        if let Some(failure) = failure {
            panic!("{failure}");
        }
        for timeline in [frames, work, presented] {
            reached(device, TimelinePoint { timeline, value: ROUNDS });
        }
        assert!(
            refusals < ROUNDS,
            "under {queues:?} {refusals} presents were refused past the bound"
        );
    }
}

/// A present waits for a point no one has signalled while another thread destroys the
/// timeline: the present is refused `destroyed`, before or after its wait, submits
/// nothing, and leaves the image acquired. Covers: the present's walk after a wait with no
/// lock held, which finds the timeline gone. Does not cover: the destroy landing between a
/// particular pair of the walk's holds; the outcome is the same in each.
#[test]
fn a_destroy_races_a_present_wait() {
    for queues in LAYOUTS {
        let set = Capabilities { queues, ..Capabilities::LEAST };
        let device = device(&set, true);
        let mut swapchain = device
            .create_swapchain(&SwapchainDesc {
                extent: lactose_rhi_api::Extent { width: 8, height: 8 },
            })
            .expect("a swapchain");
        let device: &dyn Device = &device;
        for round in 0..ROUNDS / 4 {
            let image = acquire(&mut *swapchain);
            let timeline = device.create_timeline(0).expect("a timeline");
            let waits = [TimelinePoint { timeline, value: 1 }];
            let mut encoders = [frame(device, image)];
            let start = Barrier::new(2);
            let refused = thread::scope(|scope| {
                scope.spawn(|| {
                    start.wait();
                    thread::sleep(Duration::from_micros(round * 500));
                    device
                        .destroy_timeline(timeline)
                        .expect("a destroy of a timeline no submit names");
                });
                start.wait();
                swapchain
                    .present(image, Submit { encoders: &mut encoders, waits: &waits, signals: &[] })
            });
            assert_eq!(
                rule(&refused),
                Some(Rule::Destroyed),
                "round {round} under {queues:?}: {refused:?}"
            );
            assert_eq!(
                rule(&swapchain.acquire()),
                Some(Rule::Swapchain),
                "the image is still acquired"
            );
            let done = point(device, 1);
            let submit = Submit { encoders: &mut encoders, waits: &[], signals: &[done] };
            assert_eq!(
                swapchain.present(image, submit).expect("the present again"),
                Presented::Shown
            );
            reached(device, done);
        }
    }
}

/// An offscreen target the frame draws and the present thread samples.
fn target(device: &dyn Device) -> ImageHandle {
    let desc = ImageDesc {
        label: "target",
        format: Format::Rgba8Unorm,
        extent: Extent { width: 8, height: 8 },
        mips: 1,
        layers: 1,
        usage: ImageUsage::COLOUR.union(ImageUsage::SAMPLED),
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    };
    device.create_image(&desc).expect("a target")
}

/// A frame submit on the graphics queue that takes `image` back from `Undefined`, clears it
/// and releases it to the present role, as the frame's last submit for a target does.
fn draw_target(
    device: &dyn Device,
    image: ImageHandle,
    waits: &[TimelinePoint],
    done: TimelinePoint,
) -> lactose_core::Result<()> {
    let submit = Submit { encoders: &mut [record_target(device, image)], waits, signals: &[done] };
    device.submit(QueueRole::Graphics, submit)
}

/// The encoder of [`draw_target`]'s submit.
fn record_target(device: &dyn Device, image: ImageHandle) -> Box<dyn Encoder> {
    let (graphics, present) = (QueueRole::Graphics, QueueRole::Present);
    let range = ImageRange::FIRST;
    let on = |state, queue| StateOn { state, queue };
    let mut encoder = device.create_encoder(graphics, "frame").expect("an encoder");
    encoder.transition_images(&[ImageTransition {
        image,
        range,
        from: on(ImageState::Undefined, graphics),
        to: on(ImageState::ColourAttachment, graphics),
    }]);
    encoder.begin_render_pass(&RenderPass {
        label: "frame",
        colour: &[ColourAttachment {
            target: Target { image, mip: 0, layer: 0 },
            load: Load::Clear([0.5; 4]),
            store: Store::Store,
            local_read: false,
        }],
        depth: None,
        reach: PassReach::NOTHING,
    });
    encoder.end_pass();
    encoder.transition_images(&[ImageTransition {
        image,
        range,
        from: on(ImageState::ColourAttachment, graphics),
        to: on(ImageState::SampledRead, present),
    }]);
    encoder
}

/// The display's read of a presented image held (`stall`), as a drawable that does not
/// come back holds it (decision 5.6): the present thread has taken a finished target across
/// to the present role and presented, and nothing has seen the present's point, so its
/// submit is pending. No lock is held across it. Meanwhile each frame submit on
/// the graphics queue, each writing a target the present did not take, completes. A frame
/// write of the taken target that does not wait for the present's point is refused
/// (`queue`) where the present role has a queue of its own, and taken where the roles share
/// one, whose order puts it behind the present ("The present thread" in `lactose-rhi-api`'s
/// README); once the present's point is seen, a write that waits for it
/// is taken on every layout. Then a loss with a frame in flight, and the drops after it,
/// end cleanly. One thread plays both parts: what the records refuse does not depend on
/// which thread submits. A present thread held inside its call, its queue's lock taken, is
/// `src/tests.rs`'s `frame_submits_complete_while_a_present_holds_its_queue`.
#[test]
fn a_present_the_display_holds_lets_the_frame_finish_other_targets_and_refuses_its_own() {
    let (graphics, present) = (QueueRole::Graphics, QueueRole::Present);
    for queues in LAYOUTS {
        let set = Capabilities { queues, ..Capabilities::LEAST };
        let device = device(&set, true);
        let control = device.control();
        let mut swapchain = device
            .create_swapchain(&SwapchainDesc { extent: Extent { width: 8, height: 8 } })
            .expect("a swapchain");
        let device: &dyn Device = &device;
        let targets = [target(device), target(device), target(device)];
        let finished = device.create_timeline(0).expect("a timeline");
        let at = |value| TimelinePoint { timeline: finished, value };
        draw_target(device, targets[0], &[], at(1)).expect("the first frame");
        reached(device, at(1));
        assert!(control.stall(true), "the display holds its image");
        let image = acquire(&mut *swapchain);
        let mut encoder = frame(device, image);
        // The acquire of the frame's release; where one queue serves both roles, the
        // release was the whole change.
        let own = queues.present == QueuePath::Own;
        if own {
            encoder.transition_images(&[ImageTransition {
                image: targets[0],
                range: ImageRange::FIRST,
                from: StateOn { state: ImageState::ColourAttachment, queue: graphics },
                to: StateOn { state: ImageState::SampledRead, queue: present },
            }]);
        }
        let read_done = point(device, 1);
        let submit = Submit { encoders: &mut [encoder], waits: &[at(1)], signals: &[read_done] };
        let shown = swapchain.present(image, submit).expect("the present");
        assert_eq!(shown, Presented::Shown);

        for value in 2..=9 {
            let image = targets[1 + value as usize % 2];
            draw_target(device, image, &[], at(value)).expect("a frame submit");
            reached(device, at(value));
        }
        let taken = draw_target(device, targets[0], &[], at(10));
        if own {
            assert_eq!(
                rule(&taken),
                Some(Rule::Queue),
                "under {queues:?} a write of the held target: {taken:?}"
            );
        } else {
            taken.expect("a write the shared queue orders behind the present's read");
            reached(device, at(10));
        }
        reached(device, read_done);
        assert!(control.stall(false), "the display lets the image go");
        draw_target(device, targets[0], &[read_done], at(11)).expect("a write after the read");
        reached(device, at(11));
        // The frame's encoder outlives the loss: its drop waits for a submit that has not
        // run, and the loss ends that wait.
        let mut flight = [record_target(device, targets[1])];
        let unmet = point(device, 1);
        let submit = Submit { encoders: &mut flight, waits: &[unmet], signals: &[at(12)] };
        device.submit(graphics, submit).expect("a frame left in flight");
        assert!(control.lose(), "the device is lost with a frame in flight");
        let acquired = swapchain.acquire().map_err(|error| error.kind());
        assert_eq!(acquired, Err(ErrorKind::DeviceLost), "an acquire after the loss");
        drop(swapchain);
        drop(flight);
    }
}
