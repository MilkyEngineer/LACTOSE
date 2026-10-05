//! The asset interface: identity, typed references, versions, handles, the loader and
//! processor traits, and the store trait with its conformance suite.
//!
//! Foundation tier, outside the determinism boundary. See `README.md`.
//!
//! # Who holds what
//!
//! - A component holds an [`AssetRef`]: an [`AssetId`], typed at compile time, that keeps
//!   nothing alive.
//! - A region holds a [`VersionTable`] and, pinned from it, a [`PinnedTable`]. Code inside
//!   the determinism boundary reads assets only by borrowing from that pinned table, so it
//!   sees the version in its region's table, and only kinds declared [`SimulationAsset`].
//! - A world inside the boundary is handed a `dyn` [`PublishedAssets`], which answers only
//!   for an exact version.
//! - Code outside the boundary is handed the `dyn` [`AssetCatalog`], asks it for the newest
//!   version and holds an [`AssetHandle`], which keeps that one immutable version alive.

#![forbid(unsafe_code)]

mod budget;
#[cfg(feature = "conformance")]
pub mod conformance;
mod handle;
mod id;
mod pipeline;
mod reflect;
mod store;
mod version;

pub use budget::{LoadPriority, MemoryBudget};
pub use handle::{AssetHandle, PinnedTable, UntypedHandle};
pub use id::{Asset, AssetId, AssetKind, AssetRef, SimulationAsset};
pub use pipeline::{CookedAsset, CookedHeader, ErasedLoader, Loader, Processor};
pub use store::{AssetCatalog, AssetFailure, PublishedAssets, Readiness};
pub use version::{AssetSet, AssetVersion, VersionTable};
