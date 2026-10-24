//! The assembly (architecture sections 2, 4 and 5.1): what turns every crate's
//! registration into an executor, a solver pool and the worlds, then runs them. This crate
//! is the timer root, the owner of the error policy, the place the `dyn` resources are
//! wired, the client's loop around the prediction world, the server's store of saved
//! regions, and the save path from the sync point to the slot. The thin binaries
//! (`app-headless` at milestone 1, `app-client` and `app-server` at milestone 4) link it
//! and add what one build needs: a platform, a renderer, a transport. Nothing that
//! renders, presents, pumps a window or plays sound is named here, which is what keeps the
//! server's denylist satisfiable (D83).
//!
//! Facade: every public signature is final and every deciding body is `todo!()`. The
//! README is the contract; `tests/headless.rs` and `tests/spike.rs` are compile-only
//! sketches of what the headless binary, a SOUR test and the milestone 1 netcode spike
//! write against this surface.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every deciding body is `todo!()`; the implementation removes this attribute with the last one"
)]

mod client;
mod game;
mod headless;
mod policy;
mod runtime;
mod save;
mod server;
mod world;

pub use client::{Client, ClientError, ClientFrame, ClientOutput, build_client};
pub use game::{CrateRegistration, Game};
pub use headless::{Headless, HeadlessConfig, RunReport};
pub use policy::{ErrorPolicy, LOG_TARGET, Response, Source};
pub use runtime::{CATCH_UP_CAP, LOG_RETENTION, TICK_PERIOD, Workers, start_executor};
pub use save::{Cooked, SaveEncode, SavedRegion, SavedWorld, load};
pub use server::{
    FrameOutput, LifecycleFailures, Server, ServerError, ServerFrame, ServerSave, ServerStep,
    StepOutput, build_server,
};
pub use world::{Start, WorldConfig};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: a registration is a value a game crate can put in a
/// `static` and every binary can copy; the configurations and a saved world cross into the
/// task that starts the worlds; the server and the client are owned by the timer root's
/// task, which runs on whichever frame-critical worker picks it up; every run future, every
/// output and the save future are `Send` for the same reason; the cooked-content stand-in
/// is shared with background tasks.
const _: () = {
    const fn send<T: Send>() {}
    const fn send_sync<T: Send + Sync + ?Sized>() {}
    const fn copy<T: Copy + Send + Sync + 'static>() {}
    copy::<CrateRegistration>();
    copy::<Game>();
    send::<Workers>();
    send_sync::<WorldConfig>();
    send_sync::<Start>();
    send_sync::<SavedRegion>();
    send_sync::<SavedWorld>();
    send::<Server>();
    send::<Client>();
    send::<Headless>();
    send::<ServerStep<'_>>();
    send::<ServerFrame<'_>>();
    send::<ServerSave<'_>>();
    send::<ClientFrame<'_>>();
    send::<SaveEncode>();
    send::<StepOutput>();
    send::<LifecycleFailures>();
    send::<FrameOutput>();
    send::<ClientOutput>();
    send_sync::<dyn Cooked>();
    copy::<ErrorPolicy>();
};
