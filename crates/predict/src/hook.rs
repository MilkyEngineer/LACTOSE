//! The prediction world's staged-state hook (architecture sections 9.2 and 9.4;
//! `crates/sim/README.md`, "The input sync point"): what the prediction world does at the
//! input sync point with the frames the server sent, and what it records for the
//! reconciliation that runs between ticks.

use std::collections::BTreeSet;
use std::sync::Arc;

use lactose_bundle::ComponentData;
use lactose_core::{Error, ErrorKind, NetId, PredictionKey, Result, StableKey, Tick};
use lactose_ecs::{ClaimedBy, RegionId, World};
use lactose_net_api::{StagedFrame, StagedInbox, StagedSpawn};
use lactose_reflect::Reflect;
use lactose_sim::{StagedState, StreamingAnchor};

use crate::fact::Fact;
use crate::predictor::LocalPlayer;

/// The `StagedState` hook of the prediction world, installed by
/// [`Predictor::new`](crate::Predictor::new) as the world's `dyn StagedState` service and
/// reinstalled by [`Predictor::resync`](crate::Predictor::resync);
/// `sim::apply_staged_state` calls it once per tick after the inputs have been applied.
///
/// It holds nothing: the player and entity it keeps the claim and the anchor on are the
/// record's, the one copy, which `set_local` replaces. In a predicted tick it pops every
/// frame due at the tick from the world's `StagedInbox` (`pop_through(tick)`); in a
/// re-simulated tick it applies the frames the ring's slot holds for the tick instead and
/// pops nothing. It writes the tick it ran at into the record, which is how a step knows
/// it ran. For each frame, oldest first, it applies what the README states under "The
/// hook": binds, the spawns of proxies as upserts, the proxies' rows as this tick's
/// position, moves, removals and tombstones; a full frame whole, as a rebuild. What a delta
/// frame carries of an entity the server has or had claimed is a fact for the
/// reconciliation, which it records with the tick it popped the frame at; of those it
/// writes only a claim or a release, and only in a predicted tick. Before and after
/// the frames it keeps `ClaimedBy` for the local player and `StreamingAnchor` on the
/// player's own entity while it is live, and the anchor on no other entity; and it
/// despawns a predicted spawn of the local player that the server never echoed once a
/// frame's acknowledgement is past its input tick. An entry that fails is logged and
/// skipped, as the replicated world's hook does; the tick runs.
pub struct PredictionHook {
    _private: (),
}

impl PredictionHook {
    /// The hook. Only the predictor makes one, since the record it fills is the
    /// predictor's.
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }
}

impl StagedState for PredictionHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        let entry = record(world)?;
        entry.ran_at = Some(tick);
        let local = entry.local;
        let replay = entry.replay.take();
        let replaying = replay.is_some();
        let frames: Vec<Arc<StagedFrame>> = match replay {
            Some(frames) => frames,
            None => {
                let inbox = world.service_mut::<StagedInbox>().ok_or_else(|| {
                    Error::new(ErrorKind::NotFound, "the prediction world holds no `StagedInbox`")
                })?;
                core::iter::from_fn(|| inbox.pop_through(tick)).collect()
            }
        };
        keep_local(world, local);
        let mut facts = Vec::with_capacity(frames.len());
        let mut unechoed = 0;
        for frame in &frames {
            facts.push(apply_frame(world, local, frame, replaying, &mut unechoed));
        }
        keep_local(world, local);
        let entry = record(world)?;
        if !replaying {
            entry.unechoed += unechoed;
            entry.popped.extend(frames.iter().zip(facts).map(|(frame, facts)| Popped {
                frame: Arc::clone(frame),
                applied_at: tick,
                facts,
            }));
        }
        entry.stepped = frames;
        Ok(())
    }
}

/// What the hook leaves for [`Predictor::reconcile`](crate::Predictor::reconcile) and what
/// the predictor leaves for the hook: a service resource of the prediction world under
/// this type, inserted by `Predictor::new` and replaced by `Predictor::resync`. Not state:
/// not snapshotted, not hashed.
pub(crate) struct PredictionRecord {
    /// The player and entity the hook keeps the claim and the anchor on: the only copy the
    /// world holds, replaced by `Predictor::set_local`.
    pub(crate) local: LocalPlayer,
    /// The tick the hook last ran at, or `None` before its first run: a step whose tick
    /// is not here resolves to an error, since the hook was replaced.
    pub(crate) ran_at: Option<Tick>,
    /// Every frame the hook popped since the last reconcile, in the order it popped them,
    /// each with the tick it was popped at and its facts.
    pub(crate) popped: Vec<Popped>,
    /// The frames the hook must apply in the next step instead of popping, during a
    /// re-simulation; `None` in a predicted tick.
    pub(crate) replay: Option<Vec<Arc<StagedFrame>>>,
    /// The frames the hook applied in its last run, popped or replayed: the slot's.
    pub(crate) stepped: Vec<Arc<StagedFrame>>,
    /// Predicted spawns the server never echoed, despawned in predicted ticks and not yet
    /// counted by the predictor.
    pub(crate) unechoed: u64,
}

impl PredictionRecord {
    pub(crate) fn new(local: LocalPlayer) -> Self {
        Self {
            local,
            ran_at: None,
            popped: Vec::new(),
            replay: None,
            stepped: Vec::new(),
            unechoed: 0,
        }
    }
}

/// One frame as the hook popped it.
pub(crate) struct Popped {
    pub(crate) frame: Arc<StagedFrame>,
    /// The prediction tick the frame was popped at: at or after `frame.tick`, since the
    /// prediction world runs ahead of the server.
    pub(crate) applied_at: Tick,
    /// The facts of a delta frame, in the order the hook met them; none for a full frame,
    /// which is a rebuild.
    pub(crate) facts: Vec<Fact>,
}

/// The record of `world`, which only the predictor inserts.
pub(crate) fn record(world: &mut World) -> Result<&mut PredictionRecord> {
    world
        .service_mut::<PredictionRecord>()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "the prediction world holds no record"))
}

/// Logs an entry of a frame the world refused; the rest of the frame applies.
fn refused(what: &str, key: StableKey, error: &Error) {
    tracing::warn!(target: "predict", ?key, %error, "{what} refused: skipped");
}

fn claimed_by(world: &World, local: LocalPlayer, key: StableKey) -> bool {
    world.get::<ClaimedBy>(key).is_some_and(|claim| claim.player == local.player)
}

/// Writes every component of `components` to the live `key`, each only if its bytes
/// differ, and moves it to `region` if it resides elsewhere: an upsert.
fn upsert(world: &mut World, key: StableKey, region: RegionId, components: &[ComponentData]) {
    if world.region_of(key) != Some(region)
        && let Err(error) = world.move_to_region(key, region)
    {
        refused("an upsert's move", key, &error);
    }
    for data in components {
        if let Err(error) = world.insert_erased(key, data) {
            refused("an upsert's component", key, &error);
        }
    }
}

/// Applies one frame as the README states under "The hook", and returns its facts;
/// `replaying` in a re-simulated tick.
fn apply_frame(
    world: &mut World,
    local: LocalPlayer,
    frame: &StagedFrame,
    replaying: bool,
    unechoed: &mut u64,
) -> Vec<Fact> {
    let mut facts = Vec::new();
    if frame.full {
        rebuild(world, local, frame);
    } else {
        apply_delta(world, local, frame, replaying, &mut facts);
    }
    if let Some(acked) = frame.acked_input {
        *unechoed += despawn_unechoed(world, local, acked);
    }
    facts
}

/// A full frame, whole: every spawn an upsert or a spawn with its claim set or removed,
/// then moves, removals, every row and tombstones.
fn rebuild(world: &mut World, local: LocalPlayer, frame: &StagedFrame) {
    for (&net, spawn) in &frame.spawns {
        let key = spawn.key(net);
        if world.contains(key) {
            upsert(world, key, spawn.region, &spawn.components);
        } else if let Err(error) = world.spawn_erased(key, spawn.region, &spawn.components) {
            refused("a spawn", key, &error);
            continue;
        }
        bind(world, spawn.origin, net);
        let result = match spawn.slot {
            None if !claimed_by(world, local, key) => {
                world.insert(key, ClaimedBy { player: local.player })
            }
            Some(_) if world.get::<ClaimedBy>(key).is_some() => {
                world.remove::<ClaimedBy>(key).map(drop)
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            refused("a claim", key, &error);
        }
    }
    for (&key, &region) in &frame.moves {
        if world.contains(key)
            && world.region_of(key) != Some(region)
            && let Err(error) = world.move_to_region(key, region)
        {
            refused("a move", key, &error);
        }
    }
    for &(key, component) in &frame.removed {
        if world.contains(key)
            && let Err(error) = world.remove_erased(key, component)
        {
            refused("a removal", key, &error);
        }
    }
    for row in frame.rows() {
        if world.contains(row.key) {
            let data = ComponentData { component: row.component, bytes: row.bytes.to_vec() };
            if let Err(error) = world.insert_erased(row.key, &data) {
                refused("a row", row.key, &error);
            }
        }
    }
    for &key in frame.despawns.keys() {
        if world.contains(key)
            && let Err(error) = world.despawn(key)
        {
            refused("a tombstone", key, &error);
        }
    }
}

/// A delta frame: what concerns a proxy is written, what concerns an entity the local
/// player has claimed is a fact. A claim or a release is a fact and is applied too, so that
/// the frame's later entries and the later frames of the tick find the entity as the
/// server has it; a re-simulated tick (`replaying`) leaves the claim as the corrections
/// wrote it at the frame's tick, which a later fact may have changed since (a tombstone). A
/// move, removal, row or tombstone of an entity that is not live is a fact too, which the
/// reconciliation keeps only for an entity it knows the server claimed.
fn apply_delta(
    world: &mut World,
    local: LocalPlayer,
    frame: &StagedFrame,
    replaying: bool,
    facts: &mut Vec<Fact>,
) {
    for (&net, spawn) in &frame.spawns {
        bind(world, spawn.origin, net);
        let key = spawn.key(net);
        match spawn.slot {
            None => {
                let components = spawn.components.clone();
                facts.push(Fact::Claimed { key, region: spawn.region, components });
                if !replaying {
                    claim_spawn(world, local, key, spawn);
                }
            }
            Some(_) if !world.contains(key) => {
                if let Err(error) = world.spawn_erased(key, spawn.region, &spawn.components) {
                    refused("a proxy's spawn", key, &error);
                }
            }
            Some(_) if claimed_by(world, local, key) => {
                facts.push(Fact::Released { key, components: spawn.components.clone() });
                if !replaying {
                    match world.remove::<ClaimedBy>(key) {
                        Ok(_) => upsert(world, key, spawn.region, &spawn.components),
                        Err(error) => refused("a release", key, &error),
                    }
                }
            }
            Some(_) => upsert(world, key, spawn.region, &spawn.components),
        }
    }
    for (&key, &region) in &frame.moves {
        let claimed = claimed_by(world, local, key);
        if claimed || !world.contains(key) {
            facts.push(Fact::Moved { key, region, claimed });
        } else if world.region_of(key) != Some(region)
            && let Err(error) = world.move_to_region(key, region)
        {
            refused("a move", key, &error);
        }
    }
    for &(key, component) in &frame.removed {
        let claimed = claimed_by(world, local, key);
        if claimed || !world.contains(key) {
            let region = world.region_of(key);
            facts.push(Fact::Removed { key, component, region, claimed });
        } else if let Err(error) = world.remove_erased(key, component) {
            refused("a removal", key, &error);
        }
    }
    for row in frame.rows() {
        let live = world.contains(row.key);
        let claimed = claimed_by(world, local, row.key);
        match (live, claimed, row.exact) {
            // The frame's exact rows of one entity are one fact: rows come ascending by
            // (key, component), so they are consecutive.
            (false, _, true) | (true, true, true) => {
                let data = ComponentData { component: row.component, bytes: row.bytes.to_vec() };
                match facts.last_mut() {
                    Some(Fact::Rows { key, components, .. }) if *key == row.key => {
                        components.push(data);
                    }
                    _ => facts.push(Fact::Rows {
                        key: row.key,
                        components: vec![data],
                        region: world.region_of(row.key),
                        claimed,
                    }),
                }
            }
            // Its spawn was lost; the server resends it.
            (false, _, false) => {}
            // The proxy's state for this tick, a kinematic body's sweep target among it.
            (true, false, false) => {
                let data = ComponentData { component: row.component, bytes: row.bytes.to_vec() };
                if let Err(error) = world.insert_erased(row.key, &data) {
                    refused("a row", row.key, &error);
                }
            }
            (true, _, _) => tracing::warn!(
                target: "predict",
                key = ?row.key,
                component = row.component.0,
                exact = row.exact,
                "an exact row of an unclaimed entity or a dequantised row of a claimed one: \
                 a defect of the sender, skipped"
            ),
        }
    }
    for &key in frame.despawns.keys() {
        if claimed_by(world, local, key) || !world.contains(key) {
            facts.push(Fact::Destroyed { key });
        } else if let Err(error) = world.despawn(key) {
            refused("a tombstone", key, &error);
        }
    }
}

/// Applies a claimed spawn's claim as a full frame does: `ClaimedBy` inserted on the live
/// `key`, or the entity spawned with every component and claimed if it is not live. Its
/// components are left to the correction at the frame's tick.
fn claim_spawn(world: &mut World, local: LocalPlayer, key: StableKey, spawn: &StagedSpawn) {
    if !world.contains(key)
        && let Err(error) = world.spawn_erased(key, spawn.region, &spawn.components)
    {
        refused("a claimed spawn", key, &error);
        return;
    }
    if !claimed_by(world, local, key)
        && let Err(error) = world.insert(key, ClaimedBy { player: local.player })
    {
        refused("a claim", key, &error);
    }
}

/// Binds `origin` to `net` if it has no alias yet, whoever's prediction it was.
fn bind(world: &mut World, origin: Option<PredictionKey>, net: NetId) {
    if let Some(origin) = origin
        && world.alias(origin).is_none()
        && let Err(error) = world.bind(origin, net)
    {
        refused("a bind", StableKey::Predicted(origin), &error);
    }
}

/// Despawns every live predicted spawn of the local player whose input tick is before
/// `acked` and that has no alias: the server never echoed it. Returns how many.
fn despawn_unechoed(world: &mut World, local: LocalPlayer, acked: Tick) -> u64 {
    // A predicted spawn inherits its spawner's claim, so the claim index holds every one.
    let unechoed: Vec<StableKey> = world
        .claimed(local.player)
        .filter(|&key| match key {
            StableKey::Predicted(predicted) => {
                predicted.player == local.player
                    && before(predicted.input_tick, acked)
                    && world.alias(predicted).is_none()
            }
            _ => false,
        })
        .collect();
    let mut count = 0;
    for key in unechoed {
        // An owner despawned earlier in the loop took what it owns with it.
        if world.contains(key) {
            match world.despawn(key) {
                Ok(()) => count += 1,
                Err(error) => refused("an unechoed spawn's despawn", key, &error),
            }
        }
    }
    count
}

/// Gives the local player's own entity, if live, `ClaimedBy` and `StreamingAnchor`, and
/// takes `StreamingAnchor` from every other entity, so `lactose_input::player_entity`
/// resolves to it whatever path made the row live.
fn keep_local(world: &mut World, local: LocalPlayer) {
    let own = world.origin(local.entity);
    if let Some(key) = own {
        if !claimed_by(world, local, key)
            && let Err(error) = world.insert(key, ClaimedBy { player: local.player })
        {
            refused("the local entity's claim", key, &error);
        }
        if world.get::<StreamingAnchor>(key).is_none()
            && let Err(error) = world.insert(key, StreamingAnchor { priority: 0 })
        {
            refused("the local entity's anchor", key, &error);
        }
    }
    // The anchor column holds a few rows in the layout's chunk order, so the keys are
    // sorted before anything is written.
    let others: BTreeSet<StableKey> = world
        .columns_erased(StreamingAnchor::KEY)
        .flat_map(|column| column.keys().to_vec())
        .filter(|&key| Some(key) != own)
        .collect();
    for key in others {
        if let Err(error) = world.remove::<StreamingAnchor>(key) {
            refused("another entity's anchor", key, &error);
        }
    }
}

/// Whether `tick` is before `than` on the wrapping clock.
pub(crate) const fn before(tick: Tick, than: Tick) -> bool {
    tick.0 != than.0 && than.0.wrapping_sub(tick.0) < 1 << 31
}
