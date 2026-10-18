//! What sampling costs (README, "The interpolation buffer": `sample_batch` "allocates
//! nothing"). Counted by the tagged global allocator this test binary installs, so the
//! count is of this binary alone: `sample` and `sample_batch` over live, released and stale
//! slots, at times that hold, interpolate and miss.

#![forbid(unsafe_code)]

use lactose_core::{AllocTag, TaggedAlloc, Tick};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::InterpolationSlot;
use lactose_net_host::InterpolationBuffer;
use lactose_net_host::interpolation::{PresentationTime, ReceivedState, RemoteSample};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

fn allocations_during(f: impl FnOnce()) -> u64 {
    let _scope = AllocTag::Tools.scope();
    let before = ALLOCATOR.stats(AllocTag::Tools).total_allocations;
    f();
    ALLOCATOR.stats(AllocTag::Tools).total_allocations - before
}

#[test]
fn sampling_allocates_nothing() {
    let counted = allocations_during(|| drop(std::hint::black_box(Vec::<u8>::with_capacity(8))));
    assert_eq!(counted, 1, "the counter sees this thread's allocations");
    let (mut writer, reader) = InterpolationBuffer::split(256);
    let mut slots: Vec<InterpolationSlot> = (0..200).map(|_| writer.allocate().unwrap()).collect();
    for (n, &slot) in slots.iter().enumerate() {
        for tick in 0..u32::try_from(n % 12).unwrap() {
            let x = Scalar::from_u32(tick);
            let state = ReceivedState {
                tick: Tick(tick * 3),
                teleported: Tick(if n % 5 == 0 { 7 } else { 0 }),
                position: Vec3::new(x, x, x),
                rotation: Quat::from_rotation_z(x),
            };
            writer.push(slot, state).unwrap();
        }
    }
    writer.release(slots[10]).unwrap();
    slots.push(InterpolationSlot { index: 9_999, generation: 1 });
    let mut batch = vec![RemoteSample::MISSING; slots.len()];
    let mut single = vec![RemoteSample::MISSING; slots.len()];
    let times: Vec<PresentationTime> = (0..40)
        .map(|tick| PresentationTime { tick: Tick(tick), fraction: (tick * 1_621) as u16 })
        .collect();
    for &time in &times {
        let allocations = allocations_during(|| {
            reader.sample_batch(&slots, time, &mut batch);
            for (sample, &slot) in single.iter_mut().zip(&slots) {
                *sample = reader.sample(slot, time);
            }
        });
        assert_eq!(allocations, 0, "{time:?}");
        assert_eq!(batch, single, "{time:?}");
    }
}
