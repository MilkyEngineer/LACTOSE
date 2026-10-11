//! The reconciliation (`README.md`, "Reconciliation"): which frames are compared, the
//! substituted samples replaced and repaired, the oldest difference found, and the
//! rollback from it.

use std::collections::BTreeMap;

use lactose_core::{Result, Tick};
use lactose_ecs::{Executor, World};
use lactose_input::PlayerInput;
use lactose_net_api::StagedInbox;
use lactose_sim::{RunError, Simulation};

use super::{PredictError, Predictor, State, next, prev};
use crate::fact::{self, Checked, Fact, Judge, Mismatches, Resolved};
use crate::hook::{Popped, before, record};
use crate::reconcile::{Correction, CorrectionCause, Divergence, Reconciled, ResyncReason};

/// What a call found before anything was restored: the facts to compare at each tick and
/// every tick a rollback could start from, with its cause.
#[derive(Default)]
struct Plan {
    facts: BTreeMap<Tick, Vec<Resolved>>,
    points: BTreeMap<Tick, CorrectionCause>,
}

impl Predictor {
    #[expect(
        clippy::result_large_err,
        reason = "the run's public output: a failure hands the predictor back by value"
    )]
    pub(super) async fn run_reconcile(
        self,
        executor: &Executor,
    ) -> core::result::Result<(Self, Reconciled), PredictError> {
        let Self { mut simulation, mut state } = self;
        let present = simulation.tick();
        let popped = match record(simulation.world_mut()) {
            Ok(record) => core::mem::take(&mut record.popped),
            Err(error) => {
                return Err(PredictError { error, predictor: Some(state.join(simulation)) });
            }
        };
        let mut out = Reconciled::default();
        let mut plan = state.plan(popped, present, &mut out);
        let mut absorbed = BTreeMap::new();
        let judge = Judge { registry: simulation.world().registry(), tolerance: &state.tolerance };
        for (&tick, facts) in &plan.facts {
            if let Some(slot) = state.ring.slot(tick) {
                let checked = fact::check(facts, slot, state.ring.slot(prev(tick)), &judge);
                if !checked.mismatches.is_empty() {
                    plan.points.insert(tick, CorrectionCause::Mismatch);
                }
                absorbed.insert(tick, checked.absorbed);
            }
        }
        let oldest = plan.points.keys().copied().reduce(|a, b| if before(b, a) { b } else { a });
        // What a tick a rollback re-simulates absorbed is counted there, against its new slot.
        for (&tick, within) in &absorbed {
            if oldest.is_none_or(|from| !before(from, tick)) {
                state.stats.absorbed.add(within);
            }
        }
        if let Some(from) = oldest {
            simulation =
                match state.rollback(simulation, executor, &plan, from, present, &mut out).await {
                    Ok(simulation) => simulation,
                    Err(failure) => {
                        state.count(&out);
                        return Err(state.failed(failure));
                    }
                };
        }
        let world = simulation.world_mut();
        state.horizon(world);
        if out.resync.is_none()
            && let Some(server) =
                world.service::<StagedInbox>().and_then(|inbox| inbox.oldest_after(present))
        {
            out.resync = Some(ResyncReason::Behind { server, present });
        }
        state.count(&out);
        Ok((state.join(simulation), out))
    }
}

impl State {
    /// Adds what a reconciliation did to the counters: all of it, or, for one whose
    /// rollback failed part way, what it did before the failure.
    fn count(&mut self, out: &Reconciled) {
        let stats = &mut self.stats;
        stats.frames_reconciled += u64::from(out.frames);
        for correction in &out.corrections {
            stats.corrections += 1;
            stats.mismatched_entities += u64::from(correction.divergence.entities);
            stats.mismatched_fields += u64::from(correction.divergence.fields);
            stats.deepest_correction = stats.deepest_correction.max(correction.depth);
        }
    }

    /// The facts of one frame the reconciliation acts on, each resolved and learned in the
    /// frame's order.
    fn resolve(&mut self, facts: Vec<Fact>) -> Vec<Resolved> {
        let known = &mut self.known;
        facts
            .into_iter()
            .filter_map(|fact| {
                let resolved = fact.resolve(known)?;
                resolved.fact.learn(resolved.region, known);
                Some(resolved)
            })
            .collect()
    }

    /// Classifies the frames the hook popped, oldest first, by the README's four rules;
    /// replaces and repairs the ring's samples in ascending input tick; and returns the
    /// facts to compare (a band frame's, its exact rows dropped, at the restart's slot) with
    /// the substitution points. Replaces the regions of claimed entities at a full frame
    /// and learns them from every fact it keeps.
    fn plan(&mut self, popped: Vec<Popped>, present: Tick, out: &mut Reconciled) -> Plan {
        let mut plan = Plan::default();
        let mut replaced: Vec<(Tick, CorrectionCause)> = Vec::new();
        let mut band: Vec<Resolved> = Vec::new();
        let newest_full = popped.iter().rposition(|popped| popped.frame.full);
        for (index, popped) in popped.into_iter().enumerate() {
            let tick = popped.frame.tick;
            let confirm = |out: &mut Reconciled| {
                out.frames += 1;
                out.confirmed = Some(tick);
            };
            // Rule 1, and rule 2's first half: the newest full frame restarts the ring at
            // the tick it was applied at and supersedes what was popped before it.
            match newest_full {
                Some(full) if index < full => {
                    confirm(out);
                    continue;
                }
                Some(full) if index == full => {
                    self.ring.rebuild_at(popped.applied_at);
                    let restart = popped.applied_at;
                    self.awaiting.retain(|&tick| before(restart, tick));
                    // A full frame states every entity the server has claimed for the
                    // player, so it replaces the map: an entry whose tombstone was lost
                    // (a resync's gap) goes here.
                    self.known = popped
                        .frame
                        .spawns
                        .iter()
                        .filter(|(_, spawn)| spawn.slot.is_none())
                        .map(|(&net, spawn)| (spawn.key(net), spawn.region))
                        .collect();
                    for key in popped.frame.despawns.keys() {
                        self.known.remove(key);
                    }
                    confirm(out);
                    continue;
                }
                _ => {}
            }
            // Rule 2's second half: a frame from the band between the full frame's tick and
            // the restart has its facts but exact rows compared at the restart's slot,
            // since its own slot is gone; the first later frame that carries an entity
            // restates its rows, never its tombstone, move, removal, claim or release.
            let restart = self.ring.rebuilt_at();
            if restart.is_some_and(|restart| before(tick, restart)) {
                let facts = self.resolve(popped.facts);
                band.extend(facts.into_iter().filter(|r| !matches!(r.fact, Fact::Rows { .. })));
                confirm(out);
                continue;
            }
            let Some(oldest) = self.ring.oldest() else { break };
            if before(tick, oldest) {
                out.resync = Some(ResyncReason::TooOld { frame: tick, oldest });
                break;
            }
            let Some(frame_replaced) = self.substitutions(&popped, present, oldest) else {
                out.resync = Some(ResyncReason::TooOld { frame: tick, oldest });
                break;
            };
            self.stats.substitutions += frame_replaced
                .iter()
                .filter(|(_, cause)| *cause == CorrectionCause::Substituted)
                .count() as u64;
            replaced.extend(frame_replaced);
            let facts = self.resolve(popped.facts);
            if !facts.is_empty() {
                plan.facts.entry(tick).or_default().extend(facts);
            }
            confirm(out);
        }
        // Before the facts of a frame for the restart's own tick.
        if let Some(restart) = self.ring.rebuilt_at()
            && !band.is_empty()
        {
            band.extend(plan.facts.remove(&restart).unwrap_or_default());
            plan.facts.insert(restart, band);
        }
        let base = self.ring.oldest().unwrap_or(present);
        replaced.sort_by_key(|&(tick, _)| tick.0.wrapping_sub(base.0));
        for (tick, cause) in replaced {
            if self.replace_sample(tick, cause) {
                plan.points.entry(prev(tick)).or_insert(cause);
            }
        }
        plan
    }

    /// The substitutions a frame reports and the repairs it makes due, by input tick,
    /// with the awaited repairs updated; `None` if one of them reaches before `oldest`,
    /// which is `TooOld`, and then nothing changed.
    fn substitutions(
        &mut self,
        popped: &Popped,
        present: Tick,
        oldest: Tick,
    ) -> Option<Vec<(Tick, CorrectionCause)>> {
        let restart = self.ring.rebuilt_at();
        let skipped = |tick: Tick| restart.is_some_and(|restart| !before(restart, tick));
        let mut awaiting = self.awaiting.clone();
        let mut replaced = Vec::new();
        for &tick in &popped.frame.substituted {
            if skipped(tick) {
                continue;
            }
            if before(prev(tick), oldest) {
                return None;
            }
            if before(present, tick) {
                tracing::warn!(
                    target: "predict",
                    tick = tick.0,
                    "the server substituted an input tick the prediction world has not run: \
                     skipped"
                );
                continue;
            }
            awaiting.remove(&tick);
            awaiting.insert(next(tick));
            replaced.push((tick, CorrectionCause::Substituted));
        }
        let due: Vec<Tick> =
            awaiting.iter().copied().filter(|&tick| !before(popped.frame.tick, tick)).collect();
        for tick in due {
            awaiting.remove(&tick);
            if skipped(tick) || before(present, tick) {
                continue;
            }
            if before(prev(tick), oldest) {
                return None;
            }
            replaced.push((tick, CorrectionCause::Repaired));
        }
        self.awaiting = awaiting;
        Some(replaced)
    }

    /// Replaces the ring's sample of the local player for input tick `tick`: with the
    /// previous tick's (as replaced) through `PlayerInput::substitute`, or with its own
    /// through `PlayerInput::after_substitute` against the previous tick's. Whether it did;
    /// a sample that does not decode is logged and left.
    fn replace_sample(&mut self, tick: Tick, cause: CorrectionCause) -> bool {
        let player = self.local.player;
        let decode = |bytes: &[u8]| {
            PlayerInput::from_sample_bytes(bytes)
                .inspect_err(|error| {
                    tracing::warn!(
                        target: "predict",
                        tick = tick.0,
                        %error,
                        "a recorded sample does not decode: not replaced"
                    );
                })
                .ok()
        };
        // A slot without the player's sample (the first, or one after a resync) applied
        // none, which the server's substitute treats as the idle sample.
        let applied =
            match self.ring.slot(prev(tick)).and_then(|slot| slot.input.inputs.get(&player)) {
                Some(sample) => match decode(&sample.sample) {
                    Some(applied) => applied,
                    None => return false,
                },
                None => PlayerInput::IDLE,
            };
        let Some(sample) =
            self.ring.slot_mut(tick).and_then(|slot| slot.input.inputs.get_mut(&player))
        else {
            return false;
        };
        let replacement = match cause {
            CorrectionCause::Repaired => match decode(&sample.sample) {
                Some(own) => own.after_substitute(&applied),
                None => return false,
            },
            _ => applied.substitute(),
        };
        sample.sample = replacement.to_sample_bytes().to_vec();
        sample.substituted = cause == CorrectionCause::Substituted;
        true
    }

    /// Restores the slot of `from`, corrects it and re-simulates every tick after it to
    /// `present`, comparing and correcting again at every tick with facts. A failure hands
    /// the simulation back where the world survived, the ring trimmed to its tick.
    #[expect(
        clippy::result_large_err,
        reason = "sim's `RunError`, which hands the simulation back by value"
    )]
    async fn rollback(
        &mut self,
        mut simulation: Simulation,
        executor: &Executor,
        plan: &Plan,
        from: Tick,
        present: Tick,
        out: &mut Reconciled,
    ) -> core::result::Result<Simulation, RunError> {
        let kept = |error, simulation| RunError { error, simulation: Some(simulation) };
        let slot = self.ring.slot(from).expect("a correction point is held by the ring");
        let world = simulation.world_mut();
        if let Err(error) = world.restore(&slot.world) {
            return Err(kept(error, simulation));
        }
        let had_physics = slot.physics.is_some();
        if let Some(physics) = &slot.physics
            && let Err(error) = lactose_physics_api::restore(world, physics)
        {
            self.ring.truncate_after(from);
            return Err(kept(error, simulation));
        }
        let cause = plan.points.get(&from).copied().unwrap_or(CorrectionCause::Mismatch);
        let facts = plan.facts.get(&from).map_or(&[][..], Vec::as_slice);
        // What it absorbed was counted by the call's first comparison of the same slot.
        let mismatches = self.check(world, from, facts).mismatches;
        out.corrections.push(self.correct(
            world,
            from,
            facts,
            mismatches,
            cause,
            had_physics,
            present,
        ));
        out.resimulated = present.0.wrapping_sub(from.0);
        let mut tick = from;
        while tick != present {
            tick = next(tick);
            let slot = self.ring.slot(tick).expect("the ring holds every tick to the present");
            let stored = slot.input.clone();
            let mut input = stored.clone();
            // The first application stamped them.
            input.failures.clear();
            if let Err(error) = replay(simulation.world_mut(), slot.frames.clone()) {
                self.ring.truncate_after(prev(tick));
                return Err(kept(error, simulation));
            }
            simulation = match simulation.step(executor, input).await {
                Ok(simulation) => simulation,
                Err(mut failure) => {
                    if let Some(simulation) = &mut failure.simulation {
                        let _ = record(simulation.world_mut()).map(|record| record.replay = None);
                        self.ring.truncate_after(simulation.tick());
                    }
                    return Err(failure);
                }
            };
            self.stats.ticks_resimulated += 1;
            let world = simulation.world_mut();
            if let Err(error) = self.finish_step(world, stored, true) {
                self.ring.truncate_after(tick);
                return Err(kept(error, simulation));
            }
            let facts = plan.facts.get(&tick).map_or(&[][..], Vec::as_slice);
            let Checked { mismatches, absorbed } = self.check(world, tick, facts);
            self.stats.absorbed.add(&absorbed);
            if !mismatches.is_empty() {
                let cause = CorrectionCause::Mismatch;
                out.corrections
                    .push(self.correct(world, tick, facts, mismatches, cause, true, present));
            }
        }
        Ok(simulation)
    }

    /// The facts of `tick` compared with its slot.
    fn check(&self, world: &World, tick: Tick, facts: &[Resolved]) -> Checked {
        let judge = Judge { registry: world.registry(), tolerance: &self.tolerance };
        match self.ring.slot(tick) {
            Some(slot) if !facts.is_empty() => {
                fact::check(facts, slot, self.ring.slot(prev(tick)), &judge)
            }
            _ => Checked::default(),
        }
    }

    /// Writes the facts of `tick` into `world`, which is at that tick, tells the physics
    /// which rows were written, and takes the slot again (its physics part only if it had
    /// one, `physics`); the correction it made.
    #[expect(clippy::too_many_arguments, reason = "one correction's inputs, private")]
    fn correct(
        &mut self,
        world: &mut World,
        tick: Tick,
        facts: &[Resolved],
        mismatches: Mismatches,
        cause: CorrectionCause,
        physics: bool,
        present: Tick,
    ) -> Correction {
        let written: Vec<_> =
            fact::write(world, self.local, &self.recorded, facts).into_iter().collect();
        if self.physics
            && !written.is_empty()
            && let Err(error) = lactose_physics_api::corrected(world, &written)
        {
            tracing::warn!(target: "predict", %error, "the physics was not told of a correction");
        }
        if let Some(slot) = self.ring.slot(tick) {
            let (input, frames) = (slot.input.clone(), slot.frames.clone());
            let slot = self.take(world, input, frames, physics);
            self.ring.replace(slot);
        }
        let divergence = Divergence {
            entities: u32::try_from(mismatches.len()).unwrap_or(u32::MAX),
            fields: mismatches.values().fold(0u32, |sum, &fields| sum.saturating_add(fields)),
        };
        let cause = if mismatches.is_empty() { cause } else { CorrectionCause::Mismatch };
        Correction {
            tick,
            depth: present.0.wrapping_sub(tick.0),
            cause,
            entities: written,
            divergence,
        }
    }
}

/// Hands the hook the frames to apply in the next step instead of popping.
fn replay(world: &mut World, frames: Vec<super::FrameRef>) -> Result<()> {
    record(world)?.replay = Some(frames);
    Ok(())
}
