//! The assembly (architecture sections 2, 4 and 5.1): what turns every crate's
//! registration into an executor, a solver pool and the worlds, then runs them. This crate
//! is the timer root, the owner of the error policy, the place the `dyn` resources are
//! wired, the client's loop around the prediction world, the server's store of saved
//! regions, and the save path from the sync point to the slot. The thin binaries
//! (`app-headless` at milestone 1, `app-client` at milestone 2 with a local world,
//! `app-server` at milestone 4) link it and add what one build needs: a platform, a
//! renderer, a transport. No crate that renders, presents, pumps a window or plays sound is
//! named here, which is what keeps the server's denylist satisfiable (D83): the shown frame
//! of [`Shown`] drives the render side through `dyn` objects whose traits name
//! `lactose-rhi-api` and `lactose-present-api` alone, and the binary builds them (D162).
//!
//! The README is the contract. `tests/proof/` and `tests/headless.rs` are the milestone 1
//! proof, the latter with the headless binary's `main` and the SOUR test of the assembly;
//! `tests/spike.rs` is a compile-only sketch of what the milestone 1 netcode spike writes
//! against this surface. The shown frame is a facade (`src/shown.rs`), and
//! `tests/shown.rs` is the compile-only sketch of what `lactose-app-client` and
//! `frame-impl`'s tests write against it.

#![forbid(unsafe_code)]

mod assemble;
mod client;
mod game;
mod headless;
mod policy;
mod runtime;
mod save;
mod server;
mod shown;
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
pub use shown::{
    Draw, Finished, FrameCounts, FrameTime, JOIN_MARGIN, KindCounts, PresentLane, PresentReport,
    PresentSide, RECORD_RESERVATION, Region, RenderSide, Rendered, SHED_DEPTH, Shown, ShownError,
    ShownFlush, ShownFrame, ShownKept, ShownOutput, ShownStats, ShownStep, ShownStop, Stopped,
    Submitted, TARGETS, TargetIndex, build_shown,
};
pub use world::{Start, WorldConfig};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: a registration is a value a game crate can put in a
/// `static` and every binary can copy; the configurations and a saved world cross into the
/// task that starts the worlds; the server and the client are owned by the timer root's
/// task, which runs on whichever frame-critical worker picks it up; every run future, every
/// output and the save future are `Send` for the same reason; the cooked-content stand-in
/// is shared with background tasks. For the shown frame: the shown world, its run futures
/// and what `stop` hands back are `Send` like the server's; the render side is `Send +
/// 'static`, which is what the task and the job it is moved into ask, and `Sync` is not
/// asked of it (`tests/shown.rs`, section 6, implements the trait for a type that is not);
/// the present side is the `Send + 'static` state of the present thread; a shown run's
/// failure does not convert into `lactose_core::Error`, since `?` would then drop what it
/// kept; every value that crosses to the job or to the present thread is plain data; the
/// frame has the two targets rhi-api's minimum names; the margin of a frame's wait is
/// inside a tick period; and no frame is shed at a count of one.
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

    const fn moved<T: Send + 'static>() {}
    send::<Shown>();
    send::<ShownStep<'_>>();
    send::<ShownFrame<'_>>();
    send::<ShownFlush>();
    send::<ShownStop>();
    send::<ShownOutput>();
    send::<ShownError>();
    send::<Stopped>();
    moved::<Box<dyn RenderSide>>();
    moved::<Box<dyn PresentSide>>();
    copy::<TargetIndex>();
    copy::<Region>();
    copy::<Draw>();
    copy::<Finished>();
    copy::<Submitted>();
    copy::<PresentReport>();
    copy::<KindCounts>();
    copy::<FrameCounts>();
    copy::<Rendered>();
    copy::<ShownStats>();
    send_sync::<FrameTime>();
    assert!(TARGETS >= 2);
    assert!(TargetIndex::new(TARGETS - 1).is_some() && TargetIndex::new(TARGETS).is_none());
    assert!(RECORD_RESERVATION >= 1);
    // A frame's limit falls inside its own period: after its start, so a frame on time
    // waits at all, and before the next deadline, so the wait delays no tick.
    assert!(!JOIN_MARGIN.is_zero() && JOIN_MARGIN.as_nanos() < TICK_PERIOD.as_nanos());
    // The picture presented a frame ago is on its way at any swapchain's floor.
    assert!(SHED_DEPTH >= 2);

    // `ShownError: !Into<lactose_core::Error>`. Both impls below apply to a type that
    // converts, and the marker then cannot be inferred, which fails the build.
    trait NoPlainError<Marker> {
        const HOLDS: () = ();
    }
    impl<T: ?Sized> NoPlainError<()> for T {}
    impl<T: Into<lactose_core::Error>> NoPlainError<u8> for T {}
    let () = <ShownError as NoPlainError<_>>::HOLDS;
};
