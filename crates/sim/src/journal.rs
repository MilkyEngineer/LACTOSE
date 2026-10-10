//! Journal replay (architecture section 15.5): a checkpoint plus the input journal, a debug
//! and replay tool and not the save system. Named here so `app` and the tools write against
//! one shape; this facade defines the types and the one conversion, and leaves the writer,
//! the reader and the replay driver to the milestone 1 tooling.
//!
//! A frame records what [`TickInput`] carried, in a form that travels: a table as its
//! `VersionTable` (a replay pins it again from a store), a bundle by the baseline's content
//! hash with the overlay, resources and transient keys it carried (the replay fetches the
//! baseline by its hash), samples, sessions and failures verbatim. The journal also records
//! what a save does not: the tables and modes in force at the checkpoint, and the session
//! state (the joined players' entities, the held joins and the requested regions), which
//! the replay driver restores before its first step, so every region and every player the
//! replay runs can be rebuilt.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_asset_api::{AssetFailure, VersionTable};
use lactose_bundle::{ComponentData, Overlay};
use lactose_core::{ComponentKey, PlayerId, StableKey, Tick};
use lactose_ecs::{RegionId, RegionMode};

use crate::input::{PlayerSample, SessionEvent, TickInput};
use crate::simulation::SavePoint;
use crate::time::TimeScale;

/// A checkpoint, what was in force at it, and every frame after it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Journal {
    /// The state the journal starts from, at its tick.
    pub checkpoint: SavePoint,
    /// The asset table of every region loaded at the checkpoint that had one.
    pub tables: BTreeMap<RegionId, VersionTable>,
    /// The mode of every region loaded at the checkpoint.
    pub modes: BTreeMap<RegionId, RegionMode>,
    /// Every joined player's claimed, anchored entity at the checkpoint: the replay driver
    /// re-claims each before its first step, as a `Join` would.
    pub players: BTreeMap<PlayerId, StableKey>,
    /// The joins held for a region not loaded at the checkpoint.
    pub held_joins: BTreeMap<PlayerId, RegionId>,
    /// The regions requested and not yet loaded at the checkpoint.
    pub requested: BTreeSet<RegionId>,
    /// One frame per tick after the checkpoint, ascending.
    pub frames: Vec<JournalFrame>,
}

/// What one tick's [`TickInput`] carried, as recorded; the fields mirror it, in its
/// application order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JournalFrame {
    /// The tick the input was applied for.
    pub tick: Tick,
    /// The time scale from this tick on, if the frame set it.
    pub time_scale: Option<TimeScale>,
    /// The asset failures, verbatim.
    pub failures: Vec<AssetFailure>,
    /// The regions unloaded through the frame.
    pub unloaded: BTreeSet<RegionId>,
    /// The bundles that loaded, by region.
    pub loaded: BTreeMap<RegionId, JournalLoad>,
    /// The tables that entered, by region: the publish events of architecture section 9.3
    /// as journaled.
    pub assets: BTreeMap<RegionId, VersionTable>,
    /// The mode changes through the frame.
    pub modes: BTreeMap<RegionId, RegionMode>,
    /// The sessions, verbatim.
    pub sessions: BTreeMap<PlayerId, SessionEvent>,
    /// The players' inputs, verbatim.
    pub inputs: BTreeMap<PlayerId, PlayerSample>,
}

/// A bundle that loaded: everything it carried but the baseline, which the replay fetches
/// by its content hash.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JournalLoad {
    /// The seed the region was created with.
    pub seed: u64,
    /// The mode the bundle carried.
    pub mode: RegionMode,
    /// The baseline's content hash (`Baseline::content_hash`).
    pub baseline_hash: u64,
    /// The overlay, or `None` for a cooked-state load.
    pub overlay: Option<Arc<Overlay>>,
    /// The persistent region resources the bundle carried.
    pub resources: Vec<ComponentData>,
    /// The transient region resources, by key.
    pub transient_resources: Vec<ComponentKey>,
    /// The table the bundle carried, as a `VersionTable`.
    pub assets: Option<VersionTable>,
}

impl JournalFrame {
    /// The record of `input`, applied for `tick`.
    #[must_use]
    pub fn record(tick: Tick, input: &TickInput) -> Self {
        let loaded = input
            .loaded
            .iter()
            .map(|(&region, bundle)| {
                let load = JournalLoad {
                    seed: bundle.seed,
                    mode: bundle.mode,
                    baseline_hash: bundle.baseline.content_hash(),
                    overlay: bundle.overlay.clone(),
                    resources: bundle.resources.clone(),
                    transient_resources: bundle.transient_resources.clone(),
                    assets: bundle.assets.as_ref().map(|table| table.table().clone()),
                };
                (region, load)
            })
            .collect();
        Self {
            tick,
            time_scale: input.time_scale,
            failures: input.failures.clone(),
            unloaded: input.unloaded.clone(),
            loaded,
            assets: input
                .assets
                .iter()
                .map(|(&region, table)| (region, table.table().clone()))
                .collect(),
            modes: input.modes.clone(),
            sessions: input.sessions.clone(),
            inputs: input.inputs.clone(),
        }
    }
}
