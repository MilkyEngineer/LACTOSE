//! The prediction world's staged-state hook (architecture sections 9.2 and 9.4;
//! `crates/sim/README.md`, "The input sync point"): what the prediction world does at the
//! input sync point with the frames the server sent, and what it records for the
//! reconciliation that runs between ticks.

use std::collections::BTreeSet;
use std::sync::Arc;

use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, Error, ErrorKind, NetId, PredictionKey, Result, StableKey, Tick};
use lactose_ecs::{ClaimedBy, RegionId, World};
use lactose_net_api::{StagedFrame, StagedInbox, StagedSpawn, Tombstone, TombstoneCause};
use lactose_reflect::{Reflect, TypeRole};
use lactose_sim::{StagedState, StreamingAnchor};

use crate::fact::{Fact, spawned_components};
use crate::predictor::LocalPlayer;
use crate::presence::{self, Statement, unreplicated_transient};
use crate::staged_region::{place_residence, placed, placement_data};

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
/// position, moves, removals and tombstones, each proxy's
/// [`StagedRegion`](crate::StagedRegion) spawned with it and rewritten after its upsert,
/// move and release with where the entry left it; a full frame whole, as a rebuild. What a
/// delta
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
        keep_local(world, local, replaying);
        let mut facts = Vec::with_capacity(frames.len());
        let mut unechoed = 0;
        for frame in &frames {
            facts.push(apply_frame(world, local, frame, replaying, &mut unechoed));
        }
        keep_local(world, local, replaying);
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

/// Logs an entry of a frame the world refused; the rest of the frame applies. A replay
/// (`replaying`) logs at debug: the predicted tick that popped the frame logged it at warn,
/// and every rollback through that tick would repeat it.
fn refused(what: &str, key: StableKey, error: &Error, replaying: bool) {
    if replaying {
        tracing::debug!(target: "predict", ?key, %error, "{what} refused in a replay: skipped");
    } else {
        tracing::warn!(target: "predict", ?key, %error, "{what} refused: skipped");
    }
}

fn claimed_by(world: &World, local: LocalPlayer, key: StableKey) -> bool {
    world.get::<ClaimedBy>(key).is_some_and(|claim| claim.player == local.player)
}

/// Logs what the hook skipped of a frame's entry; at debug in a replay, as [`refused`].
fn skipped(what: &str, key: StableKey, component: ComponentKey, replaying: bool) {
    if replaying {
        tracing::debug!(target: "predict", ?key, component = component.0, "{what}: skipped");
    } else {
        tracing::warn!(target: "predict", ?key, component = component.0, "{what}: skipped");
    }
}

/// Whether the hook writes `data`, a spawn's component or a row of `key`: a registered
/// replicated component. Any other (unregistered, derived, an unreplicated transient or
/// persistent one) is a defect of the sender, logged and skipped, the rest of the entry
/// applied ("The hook"; D141).
fn replicated(world: &World, key: StableKey, data: &ComponentData, replaying: bool) -> bool {
    let info = world.registry().get(data.component);
    if info.is_some_and(|info| info.role() == TypeRole::Component && info.tracking().replicated) {
        return true;
    }
    let what = "a frame names a component that is not a registered replicated one";
    skipped(what, key, data.component, replaying);
    false
}

/// The registered replicated components of `components` ([`replicated`]).
fn replicated_only(
    world: &World,
    key: StableKey,
    components: &[ComponentData],
    replaying: bool,
) -> Vec<ComponentData> {
    let kept = components.iter().filter(|data| replicated(world, key, data, replaying));
    kept.cloned().collect()
}

/// What `frame` states of the unreplicated transient components of `key`, an entity the
/// server has claimed for the local player, a key that is not a registered transient
/// unreplicated component logged and dropped; `None` if the frame states none of it. Read
/// for a claimed entity alone: the hook gives a proxy no unreplicated transient component.
fn statement(world: &World, frame: &StagedFrame, key: StableKey, replaying: bool) -> Statement {
    let stated = frame.transient.get(&key)?;
    let registry = world.registry();
    let valid = |component: &ComponentKey| {
        let valid = registry.get(*component).is_some_and(unreplicated_transient);
        if !valid {
            let what = "a statement names a key that is not a transient unreplicated component";
            skipped(what, key, *component, replaying);
        }
        valid
    };
    Some(stated.iter().copied().filter(valid).collect())
}

/// Writes every replicated component of `components` to the live `key`, each only if its
/// bytes differ, and moves it alone to `region` if it resides elsewhere: an upsert. The
/// caller records where it left a proxy ([`placed_by_frame`]); a full frame upserts claimed
/// entities too, whose region is not the frames' to record.
fn upsert(
    world: &mut World,
    key: StableKey,
    region: RegionId,
    components: &[ComponentData],
    replaying: bool,
) {
    if world.region_of(key) != Some(region)
        && let Err(error) = world.move_alone_to_region(key, region)
    {
        refused("an upsert's move", key, &error, replaying);
    }
    for data in components {
        if !replicated(world, key, data, replaying) {
            continue;
        }
        if let Err(error) = world.insert_erased(key, data) {
            refused("an upsert's component", key, &error, replaying);
        }
    }
}

/// Records where a frame's entry left the live proxy `key` (`StagedRegion`): the entry's
/// region when the world could move the proxy there, else the region it stayed in. The
/// entry's region is not recorded when the world lacks it (the step's fold took the
/// region's `Relevant` and `Irrelevant` together): the server's owed `Left` tombstone
/// names the region the server has the entity in, so a later `Left` naming where the
/// proxy stayed means the server moved it back there while owing one, and the proxy is
/// that region's to settle, its unreplicated rows kept (review r1 of the proxy-region
/// change, S1).
fn placed_by_frame(world: &mut World, key: StableKey, replaying: bool) {
    if let Err(error) = place_residence(world, key) {
        refused("a proxy's placement", key, &error, replaying);
    }
}

/// Spawns the proxy `key` in `region` with the replicated components of `components` and
/// its placement among them, so the row is born in its final archetype rather than moved
/// to one after the spawn; a frame that names the placement itself, which no server does,
/// has it skipped as any unreplicated component is. A proxy gets no unreplicated transient
/// component. Returns whether the key is live.
fn spawn_proxy(
    world: &mut World,
    key: StableKey,
    region: RegionId,
    components: &[ComponentData],
    replaying: bool,
) -> bool {
    let mut spawn = replicated_only(world, key, components, replaying);
    spawn.push(placement_data(region));
    match world.spawn_erased(key, region, &spawn) {
        Ok(()) => true,
        Err(error) => {
            refused("a proxy's spawn", key, &error, replaying);
            false
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
        rebuild(world, local, frame, replaying);
    } else {
        apply_delta(world, local, frame, replaying, &mut facts);
    }
    if let Some(acked) = frame.acked_input {
        *unechoed += despawn_unechoed(world, local, acked, replaying);
    }
    facts
}

/// A full frame, whole: every spawn an upsert or a spawn with its claim set or removed, a
/// claimed one with the transient components the frame states of it; then moves, removals,
/// every row and tombstones. Every move and despawn is of the one entity named.
fn rebuild(world: &mut World, local: LocalPlayer, frame: &StagedFrame, replaying: bool) {
    for (&net, spawn) in &frame.spawns {
        let key = spawn.key(net);
        let live = world.contains(key);
        let claimed = spawn.slot.is_none();
        let transient = if claimed { statement(world, frame, key, replaying) } else { None };
        if live {
            upsert(world, key, spawn.region, &spawn.components, replaying);
        } else if !claimed {
            if !spawn_proxy(world, key, spawn.region, &spawn.components, replaying) {
                continue;
            }
        } else {
            let mut components = replicated_only(world, key, &spawn.components, replaying);
            let registry = world.registry();
            presence::born(registry, &mut components, &spawn.components, transient.as_ref());
            if let Err(error) = world.spawn_erased(key, spawn.region, &components) {
                refused("a spawn", key, &error, replaying);
                continue;
            }
        }
        bind(world, spawn.origin, net, replaying);
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
            refused("a claim", key, &error, replaying);
        }
        if live && claimed {
            let presence = presence::apply(world, key, &spawn.components, transient.as_ref());
            if let Err(error) = presence {
                refused("a statement of transient components", key, &error, replaying);
            }
        }
        if live && !claimed {
            placed_by_frame(world, key, replaying);
        }
    }
    for (&key, &region) in &frame.moves {
        if !world.contains(key) {
            continue;
        }
        if world.region_of(key) != Some(region)
            && let Err(error) = world.move_alone_to_region(key, region)
        {
            refused("a move", key, &error, replaying);
        }
        if !claimed_by(world, local, key) {
            placed_by_frame(world, key, replaying);
        }
    }
    for &(key, component) in &frame.removed {
        if world.contains(key)
            && let Err(error) = world.remove_erased(key, component)
        {
            refused("a removal", key, &error, replaying);
        }
    }
    for row in frame.rows() {
        if world.contains(row.key) {
            let data = ComponentData { component: row.component, bytes: row.bytes.to_vec() };
            if replicated(world, row.key, &data, replaying)
                && let Err(error) = world.insert_erased(row.key, &data)
            {
                refused("a row", row.key, &error, replaying);
            }
        }
    }
    for &key in frame.despawns.keys() {
        if world.contains(key)
            && let Err(error) = world.despawn_alone(key)
        {
            refused("a tombstone", key, &error, replaying);
        }
    }
}

/// A delta frame: what concerns a proxy is written, what concerns an entity the local
/// player has claimed is a fact. A claim or a release is a fact and is applied too, so that
/// the frame's later entries and the later frames of the tick find the entity as the
/// server has it; a re-simulated tick (`replaying`) leaves the claim as the corrections
/// wrote it at the frame's tick (or the tick it was popped at, for a fact kept there),
/// which a later fact may have changed since (a tombstone). A
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
        bind(world, spawn.origin, net, replaying);
        let key = spawn.key(net);
        match spawn.slot {
            None => {
                let components = spawn.components.clone();
                let transient = statement(world, frame, key, replaying);
                if !replaying {
                    claim_spawn(world, local, key, spawn, transient.as_ref());
                }
                facts.push(Fact::Claimed { key, region: spawn.region, components, transient });
            }
            Some(_) if !world.contains(key) => {
                spawn_proxy(world, key, spawn.region, &spawn.components, replaying);
            }
            Some(_) if claimed_by(world, local, key) => {
                let components = spawn.components.clone();
                facts.push(Fact::Released { key, region: spawn.region, components });
                if !replaying {
                    match world.remove::<ClaimedBy>(key) {
                        Ok(_) => {
                            upsert(world, key, spawn.region, &spawn.components, replaying);
                            placed_by_frame(world, key, replaying);
                        }
                        Err(error) => refused("a release", key, &error, replaying),
                    }
                }
            }
            Some(_) => {
                upsert(world, key, spawn.region, &spawn.components, replaying);
                placed_by_frame(world, key, replaying);
            }
        }
    }
    for (&key, &region) in &frame.moves {
        let claimed = claimed_by(world, local, key);
        if claimed || !world.contains(key) {
            facts.push(Fact::Moved { key, region, claimed });
            continue;
        }
        if world.region_of(key) != Some(region)
            && let Err(error) = world.move_alone_to_region(key, region)
        {
            refused("a move", key, &error, replaying);
        }
        placed_by_frame(world, key, replaying);
    }
    for &(key, component) in &frame.removed {
        let claimed = claimed_by(world, local, key);
        if claimed || !world.contains(key) {
            let region = world.region_of(key);
            facts.push(Fact::Removed { key, component, region, claimed });
        } else if let Err(error) = world.remove_erased(key, component) {
            refused("a removal", key, &error, replaying);
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
                        transient: statement(world, frame, row.key, replaying),
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
                if !replicated(world, row.key, &data, replaying) {
                    continue;
                }
                if let Err(error) = world.insert_erased(row.key, &data) {
                    refused("a row", row.key, &error, replaying);
                }
            }
            (true, _, _) if replaying => {}
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
    for (&key, tombstone) in &frame.despawns {
        if claimed_by(world, local, key) || !world.contains(key) {
            facts.push(Fact::Destroyed { key });
        } else if !settled_by_its_region(world, key, tombstone)
            && let Err(error) = world.despawn_alone(key)
        {
            refused("a tombstone", key, &error, replaying);
        }
    }
}

/// Whether a tombstone of the live `key` is its region's to settle, and skipped: a `Left`
/// tombstone of an entity residing in the region it names (so the world has that region
/// loaded, and its unload takes the entity), which is also where the frames last left it
/// (`StagedRegion`; where it resides, for an entity no frame has placed), so the server
/// has the entity there. The region's unload is still to come, or the server names the
/// region and the entity enters again whole; a later move or destruction comes as another
/// tombstone of the key (`crates/net-api/README.md`, "The staged frame"). A proxy a
/// prediction system moved into the region the tombstone names is not settled by it: the
/// server has the entity elsewhere, the tombstone is the last the server sends of it once
/// acknowledged, and keeping it would leave a ghost until the region unloads (D121's
/// addendum, review r2 of `lactose-net-host`, N1, closed here).
fn settled_by_its_region(world: &World, key: StableKey, tombstone: &Tombstone) -> bool {
    tombstone.cause == TombstoneCause::Left
        && world.region_of(key) == Some(tombstone.region)
        && placed(world, key) == Some(tombstone.region)
}

/// Applies a claimed spawn's claim as a full frame does: `ClaimedBy` inserted on the live
/// `key`, with the transient components the spawn and `transient` state where that claims
/// an entity that was not claimed; or the entity spawned claimed if it is not live, with
/// the components a correction writes and those transient ones, zeroed (`Teleported` and
/// a transient value are never written to a claimed entity). Its components are otherwise
/// left to the correction. A live key is not claimed into a region the world lacks. A
/// predicted tick only: a replay leaves the claim to the corrections.
fn claim_spawn(
    world: &mut World,
    local: LocalPlayer,
    key: StableKey,
    spawn: &StagedSpawn,
    transient: Option<&BTreeSet<ComponentKey>>,
) {
    if !world.contains(key) {
        let components = spawned_components(world, &spawn.components, transient);
        if let Err(error) = world.spawn_erased(key, spawn.region, &components) {
            refused("a claimed spawn", key, &error, false);
            return;
        }
    } else if world.region(spawn.region).is_none() {
        // A claim naming a region the world lacks: the comparison skips it for an entity
        // the record lacks, so a correction never writes it; neither does the hook.
        return;
    }
    if claimed_by(world, local, key) {
        return;
    }
    if let Err(error) = world.insert(key, ClaimedBy { player: local.player }) {
        refused("a claim", key, &error, false);
        return;
    }
    if let Err(error) = presence::apply(world, key, &spawn.components, transient) {
        refused("a statement of transient components", key, &error, false);
    }
}

/// Binds `origin` to `net` if it has no alias yet, whoever's prediction it was.
fn bind(world: &mut World, origin: Option<PredictionKey>, net: NetId, replaying: bool) {
    if let Some(origin) = origin
        && world.alias(origin).is_none()
        && let Err(error) = world.bind(origin, net)
    {
        refused("a bind", StableKey::Predicted(origin), &error, replaying);
    }
}

/// Despawns every live predicted spawn of the local player whose input tick is before
/// `acked` and that has no alias, alone: the server never echoed it. What it owns that is
/// itself such a spawn goes by this rule at its own input tick, and what the server has is
/// not the client's to destroy. Returns how many.
fn despawn_unechoed(world: &mut World, local: LocalPlayer, acked: Tick, replaying: bool) -> u64 {
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
        match world.despawn_alone(key) {
            Ok(()) => count += 1,
            Err(error) => refused("an unechoed spawn's despawn", key, &error, replaying),
        }
    }
    count
}

/// Gives the local player's own entity, if live, `ClaimedBy` and `StreamingAnchor`, and
/// takes `StreamingAnchor` from every other entity, so `lactose_input::player_entity`
/// resolves to it whatever path made the row live.
fn keep_local(world: &mut World, local: LocalPlayer, replaying: bool) {
    let own = world.origin(local.entity);
    if let Some(key) = own {
        if !claimed_by(world, local, key)
            && let Err(error) = world.insert(key, ClaimedBy { player: local.player })
        {
            refused("the local entity's claim", key, &error, replaying);
        }
        if world.get::<StreamingAnchor>(key).is_none()
            && let Err(error) = world.insert(key, StreamingAnchor { priority: 0 })
        {
            refused("the local entity's anchor", key, &error, replaying);
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
            refused("another entity's anchor", key, &error, replaying);
        }
    }
}

/// Whether `tick` is before `than` on the wrapping clock.
pub(crate) const fn before(tick: Tick, than: Tick) -> bool {
    tick.0 != than.0 && than.0.wrapping_sub(tick.0) < 1 << 31
}
