//! What a sync point does to the world: applying command buffers in their order, the input
//! sync point's clearing and zeroing, and SOUR's row permutation (architecture sections 5.2,
//! 6 and 8.3).

use lactose_core::{PlayerId, PredictionKey, StableKey, SystemKey, xxh64};
use lactose_reflect::{Persistence, Reflect};

use crate::change::Stamp;
use crate::command::{Command, CommandBuffer, ErasedComponent, is_worlds_own};
use crate::region::{ClaimedBy, RegionId};
use crate::rng::{LayoutRng, key_bytes};
use crate::schedule::Source;
use crate::storage::{StableKeyMap, mark};
use crate::world::{Aliases, World, WorldKind, has_entity_ref};

/// The command buffer of one invocation, waiting for its sync point.
pub(crate) struct Pending {
    /// The system's position in the run's execution order.
    pub(crate) order: u32,
    pub(crate) source: Source,
    /// The source entity's claim when the kernel ran, which predicts its spawns.
    pub(crate) claim: Option<PlayerId>,
    pub(crate) system: SystemKey,
    /// Whether the system's worlds include both `server` and `prediction`.
    pub(crate) predicts: bool,
    pub(crate) buffer: CommandBuffer,
}

/// The ordinal of a predicted spawn: the low 32 bits of xxh64, seed zero, over the source's
/// origin key in its canonical bytes, the system key and the spawn index, both
/// little-endian (architecture section 7).
pub(crate) fn predicted_ordinal(source: StableKey, system: SystemKey, index: u32) -> u32 {
    let mut bytes = Vec::with_capacity(32);
    key_bytes(source, &mut bytes);
    bytes.extend_from_slice(&system.0.to_le_bytes());
    bytes.extend_from_slice(&index.to_le_bytes());
    xxh64(&bytes, 0) as u32
}

/// The key a command names: a placeholder through the buffer's spawns, anything else
/// through the alias table; `None` if the entity is not live.
fn resolve(
    entities: &StableKeyMap,
    aliases: &Aliases,
    placeholders: &[Option<StableKey>],
    key: StableKey,
) -> Option<StableKey> {
    let key = match key {
        StableKey::Placeholder(index) => placeholders.get(index as usize).copied().flatten()?,
        StableKey::Net(net) if !entities.contains_key(&key) => {
            aliases.by_net.get(&net).map_or(key, |&predicted| StableKey::Predicted(predicted))
        }
        key => key,
    };
    entities.contains_key(&key).then_some(key)
}

fn dropped(what: &str, key: StableKey) {
    tracing::debug!(target: "casein", ?key, "{what} dropped: the entity does not exist");
}

impl World {
    /// Applies `pending` in execution order, then by source, each buffer in sequence.
    pub(crate) fn apply(&mut self, mut pending: Vec<Pending>, stamp: Stamp) {
        pending.sort_by_key(|pending| (pending.order, pending.source));
        for buffer in pending {
            self.apply_buffer(buffer, stamp);
        }
    }

    fn apply_buffer(&mut self, pending: Pending, stamp: Stamp) {
        let source = match pending.source {
            Source::Entity(key) => Some(key),
            Source::Tick | Source::Region(_) => None,
        };
        let info = SpawnInfo {
            source,
            claim: pending.claim,
            system: pending.system,
            predicts: pending.predicts,
        };
        let mut placeholders: Vec<Option<StableKey>> = vec![None; pending.buffer.spawns as usize];
        for command in pending.buffer.commands {
            match command {
                Command::Spawn { index, region, components } => {
                    self.apply_spawn(&info, index, region, components, &mut placeholders, stamp);
                }
                Command::Despawn(key) => {
                    match resolve(&self.storage.entities, &self.aliases, &placeholders, key) {
                        Some(key) => self.despawn_tree(key),
                        None => dropped("a despawn", key),
                    }
                }
                Command::Set(key, mut component) => {
                    match resolve(&self.storage.entities, &self.aliases, &placeholders, key) {
                        Some(key) => {
                            self.resolve_references(&mut component, Some(&placeholders), None);
                            if let Err(error) = self.write_component(key, component, stamp) {
                                tracing::debug!(target: "casein", ?key, %error, "a set dropped");
                            }
                        }
                        None => dropped("a set", key),
                    }
                }
                Command::Remove(key, component) => {
                    match resolve(&self.storage.entities, &self.aliases, &placeholders, key) {
                        Some(key) => {
                            self.remove_component(key, component);
                        }
                        None => dropped("a remove", key),
                    }
                }
                Command::Move(key, region) => {
                    if !self.regions.contains_key(&region) {
                        tracing::debug!(target: "casein", ?key, ?region, "a move dropped: the region is not loaded");
                        continue;
                    }
                    match resolve(&self.storage.entities, &self.aliases, &placeholders, key) {
                        Some(key) => self.move_tree(key, region, stamp),
                        None => dropped("a move", key),
                    }
                }
                Command::Channel(command) => {
                    let World { storage, aliases, channels, .. } = self;
                    let mut remap = |key: &mut StableKey| {
                        *key = match *key {
                            StableKey::Placeholder(index) => {
                                placeholders.get(index as usize).copied().flatten().unwrap_or(*key)
                            }
                            other => {
                                resolve(&storage.entities, aliases, &[], other).unwrap_or(other)
                            }
                        };
                    };
                    (command.append)(channels, command.entry, source, &mut remap);
                }
            }
        }
    }

    /// Applies one spawn and records the key it received in `placeholders`.
    fn apply_spawn(
        &mut self,
        info: &SpawnInfo,
        index: u32,
        region: Option<RegionId>,
        mut components: Vec<ErasedComponent>,
        placeholders: &mut [Option<StableKey>],
        stamp: Stamp,
    ) {
        let Some(region) = region else {
            tracing::debug!(target: "casein", "a spawn dropped: it has no region; use `spawn_in`");
            return;
        };
        if !self.regions.contains_key(&region) {
            tracing::debug!(target: "casein", ?region, "a spawn dropped: the region is not loaded");
            return;
        }
        if components.iter().any(|component| is_worlds_own(component.key())) {
            tracing::debug!(target: "casein", "a spawn dropped: `InRegion` and `PredictedOrigin` are the world's");
            return;
        }
        let predicted = match (info.predicts, info.source, info.claim) {
            (true, Some(source), Some(player)) => {
                self.input_ticks.get(&player).map(|&tick| PredictionKey {
                    player,
                    input_tick: tick,
                    ordinal: predicted_ordinal(source, info.system, index),
                })
            }
            _ => None,
        };
        let predicted = predicted.filter(|&key| {
            let live = self.storage.entities.contains_key(&StableKey::Predicted(key));
            if live {
                tracing::debug!(target: "casein", ?key, "a predicted key collides; the spawn is not predicted");
            }
            !live
        });
        let key = match predicted {
            Some(predicted) => {
                // The spawn inherits the source's claim, whatever claim its bundle named.
                components.retain(|component| component.key() != ClaimedBy::KEY);
                components.push(ErasedComponent::of(ClaimedBy { player: predicted.player }));
                if self.kind == WorldKind::Server && self.alias(predicted).is_none() {
                    match self.allocate_net_id() {
                        Ok(net) => {
                            self.bind(predicted, net).expect("a fresh NetId is unbound");
                        }
                        Err(error) => {
                            tracing::debug!(target: "casein", %error, "a spawn dropped");
                            return;
                        }
                    }
                }
                StableKey::Predicted(predicted)
            }
            None if self.kind == WorldKind::Server => match self.allocate_net_id() {
                Ok(net) => StableKey::Net(net),
                Err(error) => {
                    tracing::debug!(target: "casein", %error, "a spawn dropped");
                    return;
                }
            },
            None => {
                tracing::debug!(target: "casein", "a spawn dropped: only the server assigns NetIds");
                return;
            }
        };
        placeholders[index as usize] = Some(key);
        for component in &mut components {
            self.resolve_references(component, Some(placeholders), Some(key));
        }
        if let Err(error) = self.spawn_entity(key, region, components, stamp) {
            placeholders[index as usize] = None;
            tracing::debug!(target: "casein", ?key, %error, "a spawn dropped");
        }
    }

    /// The input sync point: events, channels and input ticks cleared, transient columns
    /// zeroed (architecture section 6).
    pub(crate) fn input_sync(&mut self, stamp: Stamp) {
        self.events.clear();
        self.channels.clear();
        self.input_ticks.clear();
        // The references a zeroed value held leave the reverse index with it.
        let mut zeroed_references = Vec::new();
        for arch in &mut self.storage.archetypes {
            let transient: Vec<usize> = arch
                .columns
                .iter()
                .enumerate()
                .filter(|(_, column)| column.info.persistence() == Persistence::Transient)
                .map(|(index, _)| index)
                .collect();
            if transient.is_empty() {
                continue;
            }
            for chunk in &mut arch.chunks {
                for &index in &transient {
                    let column = arch.columns[index];
                    for row in 0..chunk.len() {
                        let start = column.offset + column.size * row;
                        let bytes = &mut chunk.slab.as_bytes_mut()[start..start + column.size];
                        if bytes.iter().any(|&byte| byte != 0) {
                            if has_entity_ref(column.info) {
                                zeroed_references.push((
                                    chunk.keys[row],
                                    column.info,
                                    bytes.to_vec(),
                                ));
                            }
                            bytes.fill(0);
                            mark(chunk, index, row, stamp);
                        }
                    }
                }
            }
        }
        for (key, info, bytes) in zeroed_references {
            self.index_references(key, info, &bytes, false);
        }
        // Transient region resources are zeroed with the columns.
        for state in self.regions.values_mut() {
            for resource in state.resources.values_mut() {
                if resource.info.persistence() == Persistence::Transient {
                    resource.bytes.as_bytes_mut().fill(0);
                }
            }
        }
    }

    /// Permutes every chunk's rows if SOUR's layout mode asks for it; counts the sync point.
    pub(crate) fn permute_rows(&mut self) {
        self.sync_points += 1;
        if self.layout.permute_rows {
            let mut rng = LayoutRng::new(self.layout.seed, self.sync_points, self.tick.0.into());
            self.storage.permute(&mut |len| rng.permutation(len));
        }
    }

    /// Records the emissions of the run's tick, with origins as origin keys.
    pub(crate) fn end_present(
        &mut self,
        mut emissions: Vec<crate::event::PendingEmission>,
        pass: bool,
    ) {
        for emission in &mut emissions {
            emission.record.origin = self.canonical(emission.record.origin);
        }
        self.present.end_run(self.tick, emissions, pass);
    }

    /// The `Net` alias table, for events normalised to origin keys.
    pub(crate) fn event_target(&self, target: StableKey) -> (StableKey, Option<StableKey>) {
        let target = self.canonical(target);
        (target, self.net_alias(target))
    }
}

/// What a spawn's key depends on, beyond the command.
struct SpawnInfo {
    source: Option<StableKey>,
    claim: Option<PlayerId>,
    system: SystemKey,
    predicts: bool,
}
