//! Compile-only consumer sketches against the facade: what `app`'s input task, a milestone
//! 5 source and presentation's late latch write, type-checked and never run (every
//! deciding body of the crate is `todo!()`).

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use lactose_core::{PlayerId, Tick};
use lactose_input::{Action, Actions, PlayerInput, SampleHistory};
use lactose_input_host::{LatestSample, Quantiser, RawSample, Source};
use lactose_sim::TickInput;

// ---- 1. A milestone 5 source: folds its state in, assigns nothing directly ----

struct Gamepad;

impl Source for Gamepad {
    fn poll(&mut self, raw: &mut RawSample) {
        raw.hold(Actions::EMPTY.with(Action::Jump));
        raw.offer_movement(0.3, 0.9);
        raw.turn(0.01, -0.002);
    }
}

// ---- 2. `app`'s input task: one raw sample per frame, one quantised sample per tick ----

struct InputTask {
    sources: Vec<Box<dyn Source>>,
    raw: RawSample,
    quantiser: Quantiser,
    history: SampleHistory,
}

impl InputTask {
    fn frame(&mut self, latest: &LatestSample) {
        self.raw.begin_frame();
        for source in &mut self.sources {
            source.poll(&mut self.raw);
        }
        latest.publish(self.raw);
    }

    /// Called once per simulation tick, also for each catch-up tick of a frame, so the
    /// edges fire once.
    fn tick(&mut self, tick: Tick, local: PlayerId) -> TickInput {
        let input: PlayerInput = self.quantiser.next(&self.raw);
        debug_assert!(input.validate().is_ok());
        self.history.insert(tick, input);
        let mut frame = TickInput::default();
        frame.inputs.insert(local, input.to_player_sample(tick));
        frame
    }
}

// ---- 3. Presentation: the latest raw sample for the late-latched camera ----

static LATEST: LatestSample = LatestSample::new();

fn latch() -> (f32, f32) {
    let sample = LATEST.read();
    (sample.yaw, sample.pitch)
}

fn build_task() -> InputTask {
    InputTask {
        sources: vec![Box::new(Gamepad)],
        raw: RawSample::IDLE,
        quantiser: Quantiser::new(),
        history: SampleHistory::new(),
    }
}
