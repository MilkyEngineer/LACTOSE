//! `physics-api`'s conformance run with this backend: ten ticks, a snapshot at three, a
//! restore and the re-run, under solver pools of one, two and four threads and two storage
//! layouts. Every run's report is equal: the world hashes of the first and the resumed run,
//! the snapshot's size, and the physics bytes' hashes at the end of both runs. And the
//! empty world's bytes: a backend emptied and restored to them steps as a new one does.
//! And the validity run: `physics-api`'s validity rules, held on the rows that run lists.

#![forbid(unsafe_code)]

use lactose_ecs::{Executor, LayoutConfig};
use lactose_physics_api::PhysicsConfig;
use lactose_physics_api::conformance::{
    SNAPSHOT_TICK, TICKS, empty_restore, snapshot_restore, validity,
};
use lactose_sim::harness::{ChunkCapacity, layout_mode};

fn backend(threads: usize) -> Box<dyn lactose_physics_api::PhysicsBackend> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

#[test]
fn the_conformance_run_is_equal_under_every_pool_size_and_layout() {
    let layouts = [LayoutConfig::default(), layout_mode(ChunkCapacity::Rows(7), 5).unwrap()];
    let executor = Executor::builder().frame_workers(2).background_workers(1).build().unwrap();
    let mut reports = Vec::new();
    for layout in &layouts {
        for threads in [1, 2, 4] {
            let report =
                snapshot_restore(&executor, backend(threads), PhysicsConfig::default(), *layout)
                    .unwrap_or_else(|error| panic!("{threads} threads, {layout:?}: {error}"));
            assert_eq!(report.first.len(), TICKS as usize);
            assert_eq!(report.resumed, report.first[SNAPSHOT_TICK as usize..]);
            // The resumed run reaches the first run's physics state byte for byte.
            assert_eq!(report.physics_hash.0, report.physics_hash.1, "{threads} threads");
            reports.push(report);
        }
    }
    println!("snapshot at tick {SNAPSHOT_TICK}: {} bytes", reports[0].snapshot_bytes);
    for report in &reports[1..] {
        assert_eq!(*report, reports[0]);
    }
}

#[test]
fn an_emptied_backend_steps_as_a_new_one_under_every_pool_size_and_layout() {
    let layouts = [LayoutConfig::default(), layout_mode(ChunkCapacity::Rows(7), 5).unwrap()];
    let executor = Executor::builder().frame_workers(2).background_workers(1).build().unwrap();
    let mut runs = Vec::new();
    for layout in &layouts {
        for threads in [1, 4] {
            let hashes =
                empty_restore(&executor, &|| backend(threads), PhysicsConfig::default(), *layout)
                    .unwrap_or_else(|error| panic!("{threads} threads, {layout:?}: {error}"));
            assert_eq!(hashes.len(), TICKS as usize);
            runs.push(hashes);
        }
    }
    for hashes in &runs[1..] {
        assert_eq!(*hashes, runs[0]);
    }
}

#[test]
fn the_backend_holds_to_the_validity_rules_under_every_pool_size() {
    for threads in [1, 4] {
        validity(backend(threads)).unwrap_or_else(|error| panic!("{threads} threads: {error}"));
    }
}
