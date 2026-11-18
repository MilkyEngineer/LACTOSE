//! The presentation schedule (architecture sections 5.3 and 13): what runs once a render
//! frame on a world's presentation snapshot. It interpolates between the snapshot pair, places
//! the camera rig's pivot and boom, samples every remote entity of a client's replicated world
//! from the interpolation buffer in one batch call and writes the placements through the
//! renderer's ring, and hands the frame's presentation events to a sink, each once.
//!
//! Backend tier, outside the determinism boundary and no carrier of its lint file: the crate
//! declares nothing a world registers, no system, component, resource or applier, so its
//! floats and its state reach no simulation (`docs/plans/milestone-2.md`, decision 5.18). The
//! `README.md` is the contract.
//!
//! # Where to start
//!
//! - [`Presentation`] runs the schedule of one shown world with [`Presentation::frame`], on
//!   the pair the frame acquired, given a [`PresentView`]; [`Presented`] is the camera it placed,
//!   and [`Presentation::remote`] lends the remote rows it sampled.
//! - [`interpolate`] is the transform a frame shows a presented row at.
//! - [`CameraPose`] is the pivot and boom a frame's camera is latched with; [`Orbit`] is the
//!   rig's direction from the newest raw sample.
//! - [`EventSink`] is where the frame's events go; [`upload`] writes the remote rows from the
//!   frame's job.

#![forbid(unsafe_code)]

mod camera;
mod remote;
mod schedule;

pub use camera::{CameraPose, Orbit, interpolate};
pub use remote::upload;
pub use schedule::{EventSink, PresentView, Presentation, Presented, RemoteView};

// The schedule moves between the frame's jobs with what it holds.
const _: () = {
    const fn send<T: Send + 'static>() {}
    const fn plain<T: Copy + Send + Sync + 'static>() {}
    send::<Presentation>();
    plain::<CameraPose>();
    plain::<Orbit>();
    plain::<Presented>();
};
