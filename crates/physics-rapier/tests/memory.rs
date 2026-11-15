//! What a prediction slot holds in memory (D143), which `SlotBytes` no longer reports: a
//! ring's worth of world and physics snapshots of a prediction world with this backend,
//! counted by the tagged global allocator this test binary installs, across every tag and
//! thread, against the bytes the backend would write. The scene is criterion 5's shape:
//! fifty claimed bodies thrown into contact among five fixed colliders and 59 proxies.
//! The figures are printed; what is asserted is that asking a snapshot its size and hash
//! keeps less than one encoding, and that dropping the slots gives back all but less than
//! one slot, the slack being what the harness's idle workers may allocate meanwhile.

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

    let before = live();
    let worlds: Vec<_> = (0..SLOTS).map(|_| world.snapshot()).collect();
    let with_worlds = live();
    let physics: Vec<_> =
        (0..SLOTS).map(|_| lactose_physics_api::snapshot(world).unwrap()).collect();
    let with_physics = live();

    // Asking the size and the hash encodes each copy once and keeps two numbers. The
    // harness's idle workers may allocate meanwhile, so what is asserted is that less than
    // one slot's bytes stayed, where keeping the encodings would keep sixty.
    let written: usize = physics.iter().map(|taken| taken.size_in_bytes()).sum();
    assert!(physics.iter().all(|taken| taken.hash() == physics[0].hash()), "equal worlds");
    let one_encoding = written / SLOTS;
    assert!(
        grown(with_physics, live()) < one_encoding,
        "the encoded bytes are not kept: {} bytes more after asking",
        grown(with_physics, live())
    );

    let per_slot = |(bytes, count): (usize, usize), (from_bytes, from_count): (usize, usize)| {
        ((bytes - from_bytes) / SLOTS, (count - from_count) / SLOTS)
    };
    let (world_bytes, world_allocations) = per_slot(with_worlds, before);
    let (physics_bytes, physics_allocations) = per_slot(with_physics, with_worlds);
    println!("memory: slots {SLOTS} claimed {CLAIMED} fixed 5 proxies {PROXIES}");
    println!(
        "memory: world snapshot {world_bytes} bytes in {world_allocations} allocations per \
         slot, size_in_bytes {}",
        worlds[0].size_in_bytes()
    );
    println!(
        "memory: physics snapshot {physics_bytes} bytes in {physics_allocations} allocations \
         per slot (the backend's copy and physics-api's bookkeeping), size_in_bytes \
         {one_encoding}"
    );
    assert!(physics_bytes > 0 && physics_allocations > 0);

    // Given back to within less than one slot: a slot kept by anything would stay whole.
    drop(physics);
    let kept = grown(with_worlds, live());
    assert!(kept < physics_bytes, "the physics snapshots give back all they held: {kept} kept");
    drop(worlds);
    let kept = grown(before, live());
    assert!(kept < world_bytes, "the world snapshots give back all they held: {kept} kept");
}

/// How many live bytes `now` holds beyond `then`, zero if fewer.
fn grown((then, _): (usize, usize), (now, _): (usize, usize)) -> usize {
    now.saturating_sub(then)
}
