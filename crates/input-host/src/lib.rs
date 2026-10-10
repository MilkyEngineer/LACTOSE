//! The host side of input (architecture section 4, `input`; section 9.2): the frame's raw
//! sample in floats, the quantiser that turns it into the one `PlayerInput` both machines
//! simulate, the latest-sample slot the late-latched camera reads, and the named places
//! for device sources and action mapping (milestone 5).
//!
//! Foundation tier, outside the determinism boundary, and no boundary crate may depend on
//! it (`crate-tiers.toml`): floats and devices live here, and the only thing that crosses
//! into the simulation is the integer sample of `lactose-input`. `app` runs the input task
//! on it and presentation reads the latest sample. The `README.md` is the contract.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
//!
//! ```no_run
//! use lactose_core::{PlayerId, Tick};
//! use lactose_input_host::{LatestSample, Quantiser, RawSample};
//! use lactose_sim::TickInput;
//!
//! static LATEST: LatestSample = LatestSample::new();
//!
//! // The input task, once per tick: poll the sources into the raw sample, publish it for
//! // the latch, quantise it, and put the sample in the frame.
//! fn input_tick(raw: &mut RawSample, quantiser: &mut Quantiser, tick: Tick) -> TickInput {
//!     LATEST.publish(*raw);
//!     let input = quantiser.next(raw);
//!     let mut frame = TickInput::default();
//!     frame.inputs.insert(PlayerId(1), input.to_player_sample(tick));
//!     frame
//! }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every body is `todo!()` until the implementation lane replaces it, and \
              the fields and parameters it will use are declared now"
)]

pub mod latest;
pub mod mapping;
pub mod raw;
pub mod sources;

pub use latest::LatestSample;
pub use raw::{Quantiser, RawSample};
pub use sources::Source;

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the raw sample is a copyable value that travels between
/// threads, the latest-sample slot is shared between the input task and the render side
/// and can be a `static`, the quantiser is owned by one task, and a source is owned by one
/// thread.
const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    const fn send<T: Send + ?Sized>() {}
    const fn copy<T: Copy>() {}

    send_sync::<RawSample>();
    copy::<RawSample>();
    send_sync::<LatestSample>();
    send::<Quantiser>();
    send::<dyn Source>();
    assert!(size_of::<RawSample>() == 24, "six 32-bit words: what the slot publishes");
};
