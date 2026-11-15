//! What a prediction slot holds in memory (D143), which `SlotBytes` no longer reports: a
//! ring's worth of world and physics snapshots of a prediction world with this backend,
//! counted by the tagged global allocator this test binary installs, under a tag the test
//! thread alone holds and across every tag, against the bytes the backend would write. The
//! scene is criterion 5's shape, not its scene: fifty claimed bodies thrown into contact
//! among five fixed colliders and 59 proxies. The figures are printed; what is asserted,
//! on the test thread's tag, is that asking a snapshot its size and hash leaves its live
//! bytes where the take put them, and that dropping the slots returns them exactly to
//! where they started.

#![forbid(unsafe_code)]
// A binary that installs `TaggedAlloc` runs under Miri only with Tree Borrows
// (`crates/core/SAFETY.md`).
#![cfg(not(miri))]

#[path = "../../physics-api/tests/common/mod.rs"]
mod common;

use common::*;
use lactose_core::{AllocTag, TaggedAlloc};
use lactose_ecs::{ClaimedBy, WorldKind};
use lactose_math::Scalar;
use lactose_physics_api::{Collider, PhysicsBackend, RigidBody, Shape, Transform, Velocity};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

/// Slots held, as `lactose-predict`'s default depth.
const SLOTS: usize = 60;
const CLAIMED: u64 = 50;
const PROXIES: u64 = 59;

/// Live bytes and allocations across every tag.
fn live() -> (usize, usize) {
    AllocTag::ALL.iter().map(|&tag| ALLOCATOR.stats(tag)).fold((0, 0), |(bytes, count), stats| {
        (bytes + stats.live_bytes, count + stats.live_allocations)
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test builds the solver pool that lactose-app builds in a shipped binary"
)]
fn rapier() -> Box<dyn PhysicsBackend> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

/// A floor walled on four sides, fifty claimed balls thrown at each other inside it, and
/// 59 unclaimed bodies in a row outside it, which the prediction world holds as kinematic
/// proxies; stepped until the balls lie in a heap.
fn thrown() -> Harness {
    let mut h = Options::new(WorldKind::Prediction).build_with(rapier(), CallLog::default());
    let floor = Collider::new(Shape::cuboid(v(20, 1, 20))).with_offset(v(0, -1, 0));
    h.spawn(key(1), (Transform::IDENTITY, floor));
    for (n, (x, z)) in (2..).zip([(6, 0), (-6, 0), (0, 6), (0, -6)]) {
        let (hx, hz) = if x == 0 { (6, 1) } else { (1, 6) };
        h.spawn(key(n), (at(x, 2, z), Collider::new(Shape::cuboid(v(hx, 2, hz)))));
    }
    let claim = ClaimedBy { player: PLAYER };
    for i in 0..CLAIMED {
        let (column, layer) = (i % 10, i / 10);
        let x = s(1.1) * Scalar::from_u64(column % 5) - s(2.2);
        let z = s(1.1) * Scalar::from_u64(column / 5) - s(0.55);
        let y = s(1.0) + s(1.1) * Scalar::from_u64(layer);
        let toward = if column % 2 == 0 { s(3.0) } else { s(-3.0) };
        let velocity = Velocity {
            linear: lactose_math::Vec3::new(toward, Scalar::ZERO, -toward),
            ..Velocity::ZERO
        };
        let at = Transform::at(lactose_math::Vec3::new(x, y, z));
        h.spawn(key(100 + i), (at, velocity, RigidBody::dynamic(), sphere(), claim));
    }
    for i in 0..PROXIES {
        let at =
            Transform::at(lactose_math::Vec3::new(s(-30.0) + Scalar::from_u64(i), s(0.5), s(30.0)));
        h.spawn(key(1_000 + i), (at, Velocity::ZERO, RigidBody::dynamic(), sphere()));
    }
    h.run(90)
}

#[test]
fn a_ring_of_slots_holds_its_copies_and_gives_them_back() {
    let h = thrown();
    let world = h.world();

    // Every take, ask and drop below runs on this thread under a tag no other thread of
    // this binary uses, so the tag's counters see this test's allocations alone. The
    // all-tag figures are printed beside them, not asserted: once, under SOUR's layout and
    // chaos modes, they held 48 bytes more after the asking, and the first review saw no
    // other thread allocate in 81 processes, so that cause is unknown.
    let _tag = AllocTag::Tools.scope();
    let tagged = || {
        let stats = ALLOCATOR.stats(AllocTag::Tools);
        (stats.live_bytes, stats.live_allocations)
    };
    let before = (tagged(), live());
    let worlds: Vec<_> = (0..SLOTS).map(|_| world.snapshot()).collect();
    let with_worlds = (tagged(), live());
    let physics: Vec<_> =
        (0..SLOTS).map(|_| lactose_physics_api::snapshot(world).unwrap()).collect();
    let with_physics = (tagged(), live());

    // Asking the size and the hash encodes each copy once and keeps two numbers.
    let written: usize = physics.iter().map(|taken| taken.size_in_bytes()).sum();
    assert!(physics.iter().all(|taken| taken.hash() == physics[0].hash()), "equal worlds");
    let asked = (tagged(), live());
    assert_eq!(asked.0, with_physics.0, "asking keeps none of the encoded bytes");

    let per_slot = |(bytes, count): (usize, usize), (from_bytes, from_count): (usize, usize)| {
        ((bytes - from_bytes) / SLOTS, (count - from_count) / SLOTS)
    };
    let (world_bytes, world_allocations) = per_slot(with_worlds.0, before.0);
    let (physics_bytes, physics_allocations) = per_slot(with_physics.0, with_worlds.0);
    let (all_world_bytes, all_world_allocations) = per_slot(with_worlds.1, before.1);
    let (all_physics_bytes, all_physics_allocations) = per_slot(with_physics.1, with_worlds.1);
    let kept_by_asking = asked.1.0.cast_signed() - with_physics.1.0.cast_signed();
    println!("memory: slots {SLOTS} claimed {CLAIMED} fixed 5 proxies {PROXIES}");
    println!(
        "memory: world snapshot {world_bytes} bytes in {world_allocations} allocations per \
         slot (every tag: {all_world_bytes} in {all_world_allocations}), size_in_bytes {}",
        worlds[0].size_in_bytes()
    );
    println!(
        "memory: physics snapshot {physics_bytes} bytes in {physics_allocations} allocations \
         per slot (every tag: {all_physics_bytes} in {all_physics_allocations}; the backend's \
         copy and physics-api's bookkeeping), size_in_bytes {}",
        written / SLOTS
    );
    println!("memory: every tag after asking the size and hash, {kept_by_asking} bytes more");
    assert!(physics_bytes > 0 && physics_allocations > 0);

    drop(physics);
    assert_eq!(tagged(), with_worlds.0, "the physics snapshots give back all they held");
    drop(worlds);
    assert_eq!(tagged(), before.0, "the world snapshots give back all they held");
}
