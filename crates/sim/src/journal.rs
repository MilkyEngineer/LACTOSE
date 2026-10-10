//! Journal replay (architecture section 15.5): a checkpoint plus the input journal, a debug
//! and replay tool and not the save system. Named here so `app` and the tools write against
//! one shape; this facade defines the types and the one conversion, and leaves the writer,
//! the reader and the replay driver to the milestone 1 tooling.
//!
//! A frame records what [`TickInput`] carried, in a form that travels: a table as its
//! `VersionTable` (a replay pins it again from a store), a bundle by its identity (the
//! replay fetches the baseline by its content hash and the overlay from the checkpoint's
//! records), samples and failures verbatim.

use lactose_asset_api::{AssetFailure, VersionTable};
use lactose_core::Tick;
use lactose_ecs::{RegionId, RegionMode};

use crate::input::{PlayerInput, RegionModeChange, TickInput};
use crate::simulation::SavePoint;

/// A checkpoint and every frame after it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Journal {
    /// The state the journal starts from, at its tick.
    pub checkpoint: SavePoint,
    /// One frame per tick after the checkpoint, ascending.
    pub frames: Vec<JournalFrame>,
}

/// What one tick's [`TickInput`] carried, as recorded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JournalFrame {
    /// The tick the input was applied for.
    pub tick: Tick,
    /// The players' inputs, verbatim.
    pub inputs: Vec<PlayerInput>,
    /// The tables that entered, by region.
    pub assets: Vec<JournalAssets>,
    /// The bundles that loaded, by identity.
    pub loaded: Vec<JournalLoad>,
    /// The regions unloaded through the frame.
    pub unloaded: Vec<RegionId>,
    /// The mode changes through the frame.
    pub modes: Vec<RegionModeChange>,
    /// The asset failures, verbatim.
    pub failures: Vec<AssetFailure>,
}

/// A table that entered: the publish event of architecture section 9.3 as journaled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JournalAssets {
    /// The region.
    pub region: RegionId,
    /// Every asset with its version.
    pub table: VersionTable,
}

/// A bundle that loaded, by identity rather than content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct JournalLoad {
    /// The region.
    pub region: RegionId,
    /// The seed the region was created with.
    pub seed: u64,
    /// The mode the bundle carried.
    pub mode: RegionMode,
    /// The baseline's content hash (`Baseline::content_hash`).
    pub baseline_hash: u64,
    /// The overlay's digest (`Overlay::digest`), or `None` for a cooked-state load.
    pub overlay_digest: Option<u64>,
}

impl JournalFrame {
    /// The record of `input`, applied for `tick`.
    #[must_use]
    pub fn record(tick: Tick, input: &TickInput) -> Self {
        todo!()
    }
}
