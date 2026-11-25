//! The client binary's library (`crates/app/README.md`, "What `lactose-app-client` wires"):
//! the two objects `lactose-app`'s shown frame is handed, [`ClientRender`] over the render
//! graph, the renderer and the presentation schedule, and [`ClientPresent`] over the
//! swapchain; the game it shows; and the run `main` calls, with decision 5.1's fixed-frame
//! mode and its CPU reference. A library, so that criterion 3(c)'s test links what ships.
//!
//! The package declares nothing a world registers (decision 5.18): the scene's component,
//! system and session applier are `game-sim`'s, and the registration values are the ones a
//! game crate may not name.

#![forbid(unsafe_code)]

mod game;
mod present;
pub mod reference;
mod render;
mod run;
mod shaders;

pub use game::{Frames, GAME, InMemory, REGION};
pub use present::{ClientPresent, HEAP, Waiting};
pub use render::{BOOM, ClientRender, INSTANCES, LastFrame, PROJECTION, TARGET_FORMAT};
pub use run::{
    FIXED_EXTENT, FIXED_FRACTION, FIXED_SAMPLE, LOG_TARGET, Mode, Options, Outcome, Wired, run,
    wire,
};
pub use shaders::{CookedShaders, PipelineCache};
