//! Layout independence (architecture section 8.3): one program, frames fed back from the
//! lifecycle's requests, under the production layout and SOUR's layout mode with every
//! chunk capacity and several seeds, and under one and several workers: identical hashes
//! every tick and identical save points.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_ecs::{Executor, LayoutConfig};
use lactose_sim::harness::{ChunkCapacity, layout_mode};

const TICKS: u32 = 200;

fn save(executor: &Executor, simulation: lactose_sim::Simulation) -> lactose_sim::SavePoint {
    match executor.block_on(simulation.save_point(executor)) {
        Ok((_, save)) => save,
        Err(failure) => panic!("{:?}", failure.error),
    }
}

#[test]
fn every_layout_and_worker_count_gives_one_hash_every_tick_and_one_save() {
    let executor = executor();
    let (reference, simulation) = run_program(&executor, LayoutConfig::default(), TICKS);
    let reference_save = save(&executor, simulation);
    assert!(reference_save.regions.len() > 1, "the program loads regions");
    assert!(
        reference.windows(2).all(|pair| pair[0] != pair[1]),
        "the program changes the world every tick"
    );
    for capacity in [
        ChunkCapacity::Rows(1),
        ChunkCapacity::Rows(7),
        ChunkCapacity::Rows(64),
        ChunkCapacity::Bytes(16384),
        ChunkCapacity::Production,
    ] {
        for seed in [1, 0x5eed_1a70, 0xFFFF_FFFF_0000_0001] {
            let layout = layout_mode(capacity, seed).unwrap();
            let (hashes, simulation) = run_program(&executor, layout, TICKS);
            let first = hashes.iter().zip(&reference).position(|(a, b)| a != b);
            assert_eq!(first, None, "{capacity:?} seed {seed}: first differing tick");
            assert_eq!(save(&executor, simulation), reference_save, "{capacity:?} seed {seed}");
        }
    }
    for workers in [1, 4] {
        let executor = Executor::builder().frame_workers(workers).build().unwrap();
        let (hashes, _) = run_program(&executor, LayoutConfig::default(), TICKS);
        assert_eq!(hashes, reference, "{workers} workers");
    }
}

#[test]
fn two_simulations_stepped_with_equal_frames_hash_equal_every_tick() {
    let executor = executor();
    let (first, _) = run_program(&executor, LayoutConfig::default(), 120);
    let (second, _) = run_program(&executor, LayoutConfig::default(), 120);
    assert_eq!(first, second);
}
