//! The simulation tests that record under SOUR (architecture section 8.3): each builds from
//! the runner's session through the glue the README shows (`tests/glue`), runs the program
//! for a few hundred ticks with the frames fed back from the lifecycle, and records
//! `(tick, hash)` after every step. Outside the runner the session is production and
//! nothing is recorded.

#![forbid(unsafe_code)]

use crate::common;
use crate::glue;

use common::*;
use glue::{Harness, protocol_error};
use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::{LayoutConfig, RegionMode};
use lactose_sim::RegionRequest;
use lactose_sim::harness::run_recorded;

/// What the program did, so a test shows it exercised what it claims.
#[derive(Default)]
struct Coverage {
    loads: usize,
    releases: usize,
    simulated: usize,
    frozen: usize,
}

/// Runs the program for `ticks` ticks under the session, recording every hash.
fn record_program(test: &str, ticks: u32) -> Result<Coverage> {
    let harness = Harness::from_env()?;
    let mut simulation = program(harness.layout);
    let mut recorder = harness.recorder(test, &simulation)?;
    let mut driver = Driver::default();
    let mut coverage = Coverage::default();
    let mut record =
        |tick: lactose_core::Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    for tick in 1..=ticks {
        let frame = program_frame(tick, &mut driver);
        simulation = run_recorded(&harness.executor, simulation, [frame], &mut record)?;
        driver.observe(&mut simulation);
        let modes = loaded(&simulation);
        coverage.simulated =
            coverage.simulated.max(modes.values().filter(|&&m| m == RegionMode::Simulated).count());
        coverage.frozen =
            coverage.frozen.max(modes.values().filter(|&&m| m == RegionMode::Frozen).count());
    }
    for request in &driver.requests {
        match request {
            RegionRequest::Load { .. } => coverage.loads += 1,
            RegionRequest::Release { .. } => coverage.releases += 1,
            RegionRequest::Cancel { .. } => {}
        }
    }
    harness.finish(test)?;
    Ok(coverage)
}

#[test]
fn anchored_world() {
    let coverage = record_program("lactose_sim::sour::anchored_world", 300).unwrap();
    assert!(coverage.loads > 10, "regions streamed in: {}", coverage.loads);
    assert!(coverage.releases > 0, "regions released");
    assert!(coverage.simulated > 1 && coverage.frozen > 1);
}

#[test]
fn run_recorded_records_after_every_step_and_hands_the_simulation_back() {
    let executor = executor();
    let simulation = program(LayoutConfig::default());
    let mut records = Vec::new();
    let frames = (1..=5).map(script);
    let simulation = run_recorded(&executor, simulation, frames, &mut |tick, hash| {
        records.push((tick.0, hash));
        Ok(())
    })
    .unwrap();
    assert_eq!(records.iter().map(|&(tick, _)| tick).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(records.last().unwrap().1, simulation.hash().unwrap());
    // An error from `record` ends the run with it.
    let error = run_recorded(&executor, simulation, (6..=9).map(script), &mut |tick, _| {
        if tick.0 == 7 { Err(Error::new(ErrorKind::Io, "disk full")) } else { Ok(()) }
    })
    .err()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::Io);
}
