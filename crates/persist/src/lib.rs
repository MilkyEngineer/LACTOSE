//! Saves (architecture section 4, `persist`; sections 15.1 to 15.5): the persistence
//! policy as read from the registry, the world overlay diffed from a region's export
//! against its cooked baseline into `lactose_bundle`'s records, the save container that
//! holds one such record set per region, and the migrations that bring a document written
//! under an older schema to the registry's. SKIM is the save system's name on the things
//! that run; CARTON the container's.
//!
//! Backend tier, outside the determinism boundary: it reads what the boundary exported
//! and builds what the boundary loads, computes no float, reads no clock, and orders
//! everything by key, so its outputs are functions of its inputs on every machine.
//! `README.md` is the contract: it states what the signatures cannot, and what a caller
//! may rely on.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
//!
//! # Where to start
//!
//! - [`SchemaTable`] is the policy as a value: built once from a `Registry`, it says which
//!   types a save carries, with the schema and [`LayoutDigest`] each document carries for
//!   them, and hashes to the [`SchemaHash`] a container records.
//! - [`diff`] is the one diff of a region's export against its baseline; [`RegionRecords`]
//!   is a region's part of a save, the overlay with the seed, the resources and the
//!   digests beside it, and the relevance message through
//!   [`RegionRecords::to_relevance`].
//! - [`Container`] holds the records of every region under a [`ContainerHeader`]; each
//!   region is an [`EncodedSection`] made on a worker and assembled, and read back by the
//!   index and one section at a time.
//! - [`Migrations`] holds each crate's [`Migration`] steps; a [`Loader`] migrates a
//!   region's records to the registry's schemas and builds the `RegionBundle` the sync
//!   point loads.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use lactose_bundle::Baseline;
//! use lactose_ecs::RegionMode;
//! use lactose_persist::{Container, ContainerHeader, Loader, Migrations, RegionRecords, SchemaTable};
//! use lactose_reflect::Registry;
//! use lactose_sim::SavePoint;
//!
//! # fn baseline_of(_: lactose_ecs::RegionId) -> Arc<Baseline> { unimplemented!() }
//! # fn main() -> lactose_core::Result<()> {
//! # let registry = Registry::new();
//! # let save: SavePoint = unimplemented!();
//! // The save: one diff and one section per region on background workers, then assembled.
//! let schemas = SchemaTable::from_registry(&registry)?;
//! let header = ContainerHeader::of(&save, &schemas);
//! let mut sections = Vec::new();
//! for export in &save.regions {
//!     let records = RegionRecords::from_export(export, &baseline_of(export.region), &schemas)?;
//!     sections.push(records.encode_section()?);
//! }
//! let bytes = Container::assemble(&header, sections)?; // `app` writes these: temp file, sync, rename
//!
//! // The load: the header into the builder, one bundle per region into `TickInput::loaded`.
//! let container = Container::decode(&bytes)?;
//! let migrations = Migrations::new();
//! let loader = Loader::new(&schemas, &migrations);
//! for records in container.regions() {
//!     let baseline = baseline_of(records.region);
//!     let bundle = loader.bundle(records, baseline, RegionMode::Simulated, None)?;
//!     # let _ = bundle;
//! }
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every body is `todo!()` until the implementation lane replaces it, and \
              the fields and parameters it will use are declared now"
)]

mod container;
mod diff;
mod load;
mod migrate;
mod schema;

pub use container::{
    CONTAINER_FORMAT_VERSION, Container, ContainerHeader, ContainerIndex, EncodedSection,
    EngineVersion, MAX_CONTAINER_BYTES, MAX_SECTION_BYTES, PREAMBLE_BYTES, RegionIndexEntry,
    Sections,
};
pub use diff::{RegionRecords, diff};
pub use load::Loader;
pub use migrate::{FieldIds, FieldValues, MigrateFn, Migration, Migrations};
pub use schema::{LayoutDigest, SchemaHash, SchemaTable, Schemas};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the table, the migrations and a container are built once
/// and shared by the tasks that diff, encode and load; a region's records and its encoded
/// section travel from the worker that made them to the one that assembles; a migration
/// step is a plain function; and the caps nest so a section that fits its cap fits a
/// container that fits its own and an overlay at `lactose_bundle`'s cap fits a section.
const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    const fn copy<T: Copy>() {}

    send_sync::<SchemaTable>();
    send_sync::<SchemaHash>();
    send_sync::<LayoutDigest>();
    send_sync::<RegionRecords>();
    send_sync::<EncodedSection>();
    send_sync::<Container>();
    send_sync::<ContainerHeader>();
    send_sync::<ContainerIndex>();
    send_sync::<Migrations>();
    send_sync::<Migration>();
    send_sync::<FieldValues>();
    send_sync::<Loader<'_>>();

    copy::<MigrateFn>();
    copy::<Migration>();
    copy::<ContainerHeader>();
    copy::<RegionIndexEntry>();
    copy::<LayoutDigest>();
    copy::<Loader<'_>>();

    assert!(CONTAINER_FORMAT_VERSION == 1);
    assert!(PREAMBLE_BYTES == 16);
    assert!(MAX_SECTION_BYTES >= lactose_bundle::MAX_DOCUMENT_BYTES);
    assert!(MAX_CONTAINER_BYTES >= MAX_SECTION_BYTES);
};
