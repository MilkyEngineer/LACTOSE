//! Entities, archetypes, chunks, change ticks, the structural log, queries, the schedule,
//! command buffers and the `#[system]` macro: the surface every simulation crate writes
//! against (architecture sections 6 and 6.1).
//!
//! Foundation tier, inside the determinism boundary. This crate depends on `lactose-task` for
//! execution only, and on `lactose-reflect`, `lactose-asset-api` and `lactose-bundle` for
//! types and metadata (architecture section 8.2). `README.md` is the contract: it states what
//! the signatures cannot, and what a caller may rely on.
//!
//! # Status
//!
//! The naive implementation: storage, change tracking, commands, events, snapshots and
//! bulk copy are complete, and a schedule run executes every system in order on one
//! worker. Milestone 1 rewrites the hot paths to fork over chunks.
//!
//! # Where to start
//!
//! - [`World`] holds entities, components, resources, regions and their change state;
//!   [`WorldBuilder`] makes one for a [`WorldKind`].
//! - A system is a plain function under `#[system]`, which declares its access through its
//!   parameter types ([`Res`], [`Mut`], `&T`, [`Ctx`], [`Commands`], [`Events`], ...) and
//!   emits a [`SystemDescriptor`] static.
//! - [`Schedule`] orders descriptors into stages separated by sync points and runs a tick,
//!   or a pass between ticks for extract and the replication gather.
//! - Change state is read per consumer: [`World::changed_since`] by column,
//!   [`World::structural_log`] for spawns, despawns and moves.
//! - [`World::snapshot`] and [`World::restore`] serve the prediction world; the bulk-copy
//!   methods move region rows to and from `lactose_bundle` documents.

mod access;
mod apply;
mod bulk;
mod change;
mod command;
mod event;
mod param;
mod query;
mod region;
mod rng;
mod run;
mod schedule;
mod snapshot;
mod storage;
mod system;
mod world;

pub use access::{Access, AccessKind, AccessTarget};
pub use bulk::{ExportedRow, RegionRows};
pub use change::{
    ChangeMark, ChangedRows, ColumnMut, ColumnRef, Columns, ColumnsMut, DirtyRows, ErasedColumn,
    ErasedColumns, LogCursor, LogEntries, LogEntry, StructuralChange, StructuralLog,
};
pub use command::{
    Bundle, Channel, ChannelEntries, ChannelEntry, ChannelWriter, CommandChannel, Commands,
};
pub use event::{
    Event, EventIter, EventWriter, Events, PresentEmission, PresentEvent, PresentEventBatch,
    PresentEventKey, PresentEvents, PresentKind,
};
/// Declares a reduction's finishing step: `#[finalize(system_fn)]` on a function whose first
/// parameter is the reducer's output and whose other parameters are those a once-per-tick
/// system may take. See the crate `README.md` for what the attribute emits.
pub use lactose_ecs_macros::finalize;
/// Declares a system: `#[system(name = "crate::name", worlds = [server, prediction], ...)]`
/// on a function whose parameters are [`SystemParam`] types. See the crate `README.md` for
/// the arguments and what the attribute emits.
pub use lactose_ecs_macros::system;
/// The executor a schedule runs on, re-exported so that a boundary crate which drives a
/// schedule (`lactose-sim`, `lactose-predict`) can name what [`Schedule::run_tick`] takes
/// without a dependency edge on `lactose-task`, which architecture section 8.2 grants to
/// this crate alone, for execution only.
pub use lactose_task::Executor;
pub use param::{
    Changed, Ctx, Job, Mut, RegionRes, Res, ResMut, StableKeyOf, SystemParam, With, Without,
};
pub use query::{Query, QueryItem, QueryItems};
pub use region::{
    ClaimedBy, InRegion, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionInfo, RegionMode,
};
pub use rng::Rng;
pub use schedule::{Schedule, ScheduleBuilder, ScheduleError};
pub use snapshot::WorldSnapshot;
pub use system::{SystemDescriptor, SystemSet};
pub use world::{
    ChunkOrder, Entities, ErasedResources, LayoutConfig, Referrer, Referrers, Regions, World,
    WorldBuilder, WorldKind, WorldSet,
};

/// The thread-safety the contract promises, pinned so the implementation cannot lose it
/// without a compile error: a scope child borrows the world and its column views, and the
/// timer root task owns the world and the schedule.
const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    const fn send<T: Send>() {}
    send_sync::<World>();
    send_sync::<ColumnRef<'_, InRegion>>();
    send_sync::<ErasedColumn<'_>>();
    send::<ColumnMut<'_, InRegion>>();
    send::<Schedule>();
    send::<WorldSnapshot>();
    send_sync::<SystemDescriptor>();
};

/// Items the `#[system]` and `#[finalize]` expansions name. Not part of the interface: the
/// implementation may reshape them together with the macros, in one change.
#[doc(hidden)]
pub mod __private {
    pub use crate::access::writes;
    pub use crate::system::{
        Fetch, Finalize, Invocation, Kernel, ReducerOps, fetch, finish, reduce,
    };
    pub use lactose_core::SystemKey;
}
