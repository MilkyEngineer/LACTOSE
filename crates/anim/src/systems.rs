//! The five systems of this crate and the sets a game orders itself by (architecture
//! sections 5.2, 6.1 and 17; `lactose-sim`'s `Stage::Simulation` and `Stage::Post`). In
//! `Stage::Simulation`: the advance, which runs each character's state machine and phase
//! from its parameters and writes its root motion and events; and the evaluation, which
//! writes the root-local sim pose. In `Stage::Post`, after the physics write-back and in
//! the server world alone, which alone has bone rows: a limp ragdoll's bone rows report
//! where the step left them, the owner's pose is fitted to them, and an animated
//! character's bone rows are swept to its pose. The module is
//! private: [`register_systems`] places all five, and a consumer orders itself against
//! them through [`sets`] alone.

use lactose_asset_api::PinnedTable;
use lactose_core::{Result, StableKey};
use lactose_ecs::{Commands, Ctx, EventWriter, Events, Mut, StableKeyOf, system};
use lactose_math::Scalar;
use lactose_physics_api::Transform;
use lactose_sim::{Registration, Stage, TypeTable};

use crate::assets::{Clip, Condition, MAX_SIM_BONES, Skeleton, State, StateMachine, Transition};
use crate::components::{
    AnimParams, AnimRig, AnimState, BoneIndex, BoneLink, Bones, Outgoing, RootMotion, SimPose,
};
use crate::event::{AnimEvent, BoneMoved};
use crate::phase::Phase;

/// The system sets this crate declares: three in `Stage::Simulation`, three in
/// `Stage::Post`, each chain ordered as listed. A game system that touches a component of
/// this crate is in one of them or in a set declared `.after(..)` one of them, and the
/// schedule refuses the build otherwise (`README.md`, "The tick").
pub mod sets {
    use lactose_ecs::SystemSet;

    /// Where the game writes [`AnimParams`](crate::AnimParams), in `Stage::Simulation`
    /// before the advance: a game system placed `in_set = lactose_anim::sets::DRIVE` is
    /// ordered before every system of this crate. This crate places nothing in it.
    pub static DRIVE: SystemSet = SystemSet::new("anim::drive");

    /// The advance (`anim::advance`): the state machine, the phase, the root motion and the
    /// events. A reader of [`AnimState`](crate::AnimState), [`RootMotion`](crate::RootMotion)
    /// or `Events<AnimEvent>` in `Stage::Simulation` is in a set declared
    /// `.after(&[&ADVANCE])` or `.after(&[&EVALUATE])`.
    pub static ADVANCE: SystemSet = SystemSet::new("anim::advance").after(&[&DRIVE]);

    /// The evaluation (`anim::evaluate`): the root-local [`SimPose`](crate::SimPose). A
    /// reader of it in `Stage::Simulation` is in a set declared `.after(&[&EVALUATE])`.
    pub static EVALUATE: SystemSet = SystemSet::new("anim::evaluate").after(&[&ADVANCE]);

    /// A limp ragdoll's bone rows reporting their stepped transforms
    /// (`anim::report_bones`), in `Stage::Post` after the physics write-back. This set and
    /// the two after it hold systems in the server world alone and are empty in a
    /// prediction world, which has no bone rows.
    pub static REPORT: SystemSet = SystemSet::new("anim::report");

    /// The ragdoll fit (`anim::fit_ragdoll`): a limp character's `SimPose` rewritten from
    /// its bone rows.
    pub static FIT: SystemSet = SystemSet::new("anim::fit").after(&[&REPORT]);

    /// The bone sweep (`anim::drive_bones`): an animated character's bone rows written to
    /// its pose through commands, which apply at the stage's closing sync point and so
    /// conflict with nothing. A `Stage::Post` reader of [`SimPose`](crate::SimPose) is in
    /// a set declared `.after(&[&BONES])` (or `.after(&[&FIT])`, the fit being the writer
    /// it must follow); a `Stage::Post` reader of `Transform` needs no edge against this
    /// crate.
    pub static BONES: SystemSet = SystemSet::new("anim::bones").after(&[&FIT]);
}

/// Registers this crate's components with `types`: [`AnimRig`], [`AnimState`],
/// [`AnimParams`], [`RootMotion`], [`SimPose`], [`Bones`] and [`BoneLink`]. `app` calls it
/// beside every other crate's `register_types`; the `Transform`, `RigidBody`, `Collider`
/// and `Joint` the bone rows carry are `lactose_physics_api::register_types`'s.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<AnimRig>()?;
    types.register::<AnimState>()?;
    types.register::<AnimParams>()?;
    types.register::<RootMotion>()?;
    types.register::<SimPose>()?;
    types.register::<Bones>()?;
    types.register::<BoneLink>()?;
    Ok(())
}

/// Adds this crate's five systems and six sets: the advance and the evaluation to
/// `Stage::Simulation`, the report, the fit and the sweep to `Stage::Post`. What `app`
/// passes to `SimulationBuilder::systems`. The advance and the evaluation declare
/// `worlds = [server, prediction]` and the three `Stage::Post` systems `worlds = [server]`,
/// so a prediction registration adds the `Stage::Post` sets with nothing in them and a
/// `Client` registration adds the sets alone.
pub fn register_systems(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&sets::DRIVE, &sets::ADVANCE, &sets::EVALUATE])
        .add(&ADVANCE)
        .add(&EVALUATE);
    registration
        .stage(Stage::Post)
        .sets(&[&sets::REPORT, &sets::FIT, &sets::BONES])
        .add(&REPORT_BONES)
        .add(&FIT_RAGDOLL)
        .add(&DRIVE_BONES);
}

/// The advance: one character's state machine for one tick (`README.md`, "The state
/// machine", states the order). With the rig's machine from the region's assets: enters the
/// initial state if the state is unset or out of range; ends a blend whose outgoing state
/// is out of range or whose clip is missing; with the state's clip, tries the transitions
/// in order against the parameters and the state as they are, taking the first whose
/// conditions all hold; then advances the phase (and the outgoing phase of a blend) by the
/// state's rate, writes the root motion the clip moved by between the two phases, emits an
/// `AnimEvent` for every authored event the advance passed, and clears the triggers it
/// read. Writes `AnimParams` only when a trigger was set. With the machine missing from the
/// region's table, a hold: marks `AnimState` held and leaves the rest of it and `AnimParams`
/// as they are, writes `RootMotion::ZERO` and emits nothing; with the state's clip missing,
/// the same after the state has been set, except that a hold on the tick of an entry (the
/// initial state's, or a transition's whose target clip is missing) does not mark it held,
/// so the entry is reported on that tick. The character's own key is the events' target.
#[system(name = "anim::advance", worlds = [server, prediction], in_set = sets::ADVANCE)]
pub(crate) fn advance(
    ctx: Ctx,
    key: StableKeyOf,
    events: EventWriter<AnimEvent>,
    rig: &AnimRig,
    params: Mut<AnimParams>,
    state: Mut<AnimState>,
    motion: Mut<RootMotion>,
) {
    let mut row = *state;
    let outcome = advance_row(ctx.assets(), *key, rig, &params, &mut row);
    state.set_if_neq(row);
    motion.set_if_neq(outcome.motion);
    if let Some(passed) = outcome.passed {
        passed.emit(*key, &mut events);
    }
    if outcome.tried && params.triggers != 0 {
        params.set_if_neq(AnimParams { triggers: 0, ..*params });
    }
}

/// What one advance did beyond the row it left.
struct Outcome<'a> {
    /// Whether step 5 tried the transitions, so the triggers are consumed.
    tried: bool,
    /// Step 8's root motion; zero on a hold.
    motion: RootMotion,
    /// The current clip's advance, whose events step 9 emits; `None` on a hold.
    passed: Option<Passed<'a>>,
}

/// The phases a state's clip advanced between this tick.
struct Passed<'a> {
    clip: &'a Clip,
    state: u16,
    from: Phase,
    to: Phase,
    /// Whether the entry set the phase this tick, so an event at `from` is passed too.
    entry_set: bool,
}

impl Passed<'_> {
    /// Step 9: an `AnimEvent` for each of the clip's events the advance passed, in the order
    /// passed (`AnimState::phase_crossed`'s rule): those at the set entry phase, then
    /// `(from, to]`, or across a wrap `(from, END]` and then `[ZERO, to]`, each ascending as
    /// the clip lists them.
    fn emit(&self, target: StableKey, events: &mut EventWriter<'_, AnimEvent>) {
        let (from, to) = (self.from, self.to);
        let mut emit = |phase: Phase, event| {
            events.emit(AnimEvent { target, event, state: self.state, phase });
        };
        let all = &self.clip.events;
        if self.entry_set {
            all.iter().filter(|e| e.phase == from).for_each(|e| emit(e.phase, e.event));
        }
        if to >= from {
            all.iter()
                .filter(|e| from < e.phase && e.phase <= to)
                .for_each(|e| emit(e.phase, e.event));
        } else {
            all.iter().filter(|e| e.phase > from).for_each(|e| emit(e.phase, e.event));
            all.iter().filter(|e| e.phase <= to).for_each(|e| emit(e.phase, e.event));
        }
    }
}

/// Steps 1 to 8 of the advance (`README.md`, "The state machine") on `row`, a copy of the
/// character's `AnimState`; the caller writes the row, the root motion, the events of
/// step 9 and the trigger clear of step 10.
fn advance_row<'a>(
    assets: Option<&'a PinnedTable>,
    key: StableKey,
    rig: &AnimRig,
    params: &AnimParams,
    row: &mut AnimState,
) -> Outcome<'a> {
    let mut outcome = Outcome { tried: false, motion: RootMotion::ZERO, passed: None };
    // Step 1.
    let Some((assets, machine)) = assets.and_then(|a| Some((a, a.get(rig.machine).ok()?))) else {
        tracing::trace!(target: "anim", key = ?key, "held: the rig's state machine is missing");
        row.flags |= AnimState::HELD;
        return outcome;
    };
    if machine.state(machine.initial).is_none() {
        tracing::debug!(
            target: "anim",
            key = ?key,
            "held: the initial state is past the states, which the loader refuses"
        );
        row.flags |= AnimState::HELD;
        return outcome;
    }
    // Step 2.
    let entered = row.state().is_none_or(|index| machine.state(index).is_none());
    if entered {
        *row = AnimState::entering(machine.initial);
        row.ticks_in_state = 0;
    }
    // Step 3.
    if row.blend_ticks != 0 {
        let outgoing = machine.state(row.previous_state);
        if outgoing.and_then(|state| clip_of(assets, state)).is_none() {
            end_blend(row);
        }
    }
    // Step 4. In range: step 2 entered a state in range or found one.
    let mut current = &machine.states[usize::from(row.state)];
    let Some(mut clip) = clip_of(assets, current) else {
        tracing::trace!(target: "anim", key = ?key, state = row.state, "held: the clip is missing");
        if !entered {
            row.flags |= AnimState::HELD;
        }
        return outcome;
    };
    // Step 5.
    if !entered {
        outcome.tried = true;
        let before = *row;
        match first_taken(machine, &before, params, key) {
            Some(transition) => {
                take(row, transition, &before);
                current = &machine.states[usize::from(transition.to)];
                // The rule for a target whose clip is missing: the transition stands and
                // the tick holds as step 4's entry hold does, the row not marked held.
                let Some(target) = clip_of(assets, current) else {
                    tracing::trace!(
                        target: "anim",
                        key = ?key,
                        state = row.state,
                        "held: the clip of the state a transition entered is missing"
                    );
                    return outcome;
                };
                clip = target;
            }
            None => {
                row.flags &= !(AnimState::ENTERED | AnimState::ENTRY_SET);
                row.previous_phase = row.phase;
            }
        }
    }
    // Step 6.
    row.ticks_in_state = row.ticks_in_state.saturating_add(1);
    row.flags &= !AnimState::HELD;
    // Step 7.
    let from = row.previous_phase;
    row.phase = from.advanced(rate(clip, current, params, key), clip.looping);
    if clip.looping {
        row.flags &= !AnimState::FINISHED;
    } else if row.phase == Phase::END {
        row.flags |= AnimState::FINISHED;
    }
    let mut outgoing_motion = None;
    if row.blend_ticks != 0 {
        let outgoing = machine.state(row.previous_state);
        match outgoing.and_then(|state| Some((state, clip_of(assets, state)?))) {
            Some((state, out)) if row.ticks_in_state < u32::from(row.blend_ticks) => {
                let out_from = row.blend_phase;
                row.blend_phase = out_from.advanced(rate(out, state, params, key), out.looping);
                outgoing_motion = Some(out.root_motion(out_from, row.blend_phase));
            }
            _ => end_blend(row),
        }
    }
    // Step 8.
    let mut motion = clip.root_motion(from, row.phase);
    if let (Some(out), Some(blend)) = (outgoing_motion, row.blend()) {
        motion = RootMotion {
            translation: motion.translation.lerp(out.translation, blend.weight),
            yaw: motion.yaw.lerp(out.yaw, blend.weight),
        };
    }
    outcome.motion = motion;
    outcome.passed = Some(Passed {
        clip,
        state: row.state,
        from,
        to: row.phase,
        entry_set: row.entry_was_set(),
    });
    outcome
}

/// The clip `state` plays, from the region's table.
fn clip_of<'a>(assets: &'a PinnedTable, state: &State) -> Option<&'a Clip> {
    assets.get(state.clip).ok()
}

/// Ends the blend in progress: the outgoing state set equal to the state.
fn end_blend(row: &mut AnimState) {
    row.previous_state = row.state;
    row.blend_phase = Phase::ZERO;
    row.blend_ticks = 0;
}

/// The units the state advances this tick: `Phase::rate` of the clip's duration and the
/// state's speed times its parameter, one for a parameter past the slots (logged, since
/// the loader refuses it).
fn rate(clip: &Clip, state: &State, params: &AnimParams, key: StableKey) -> u32 {
    let value = match state.speed_param {
        None => Scalar::ONE,
        Some(param) => params.values.get(usize::from(param)).copied().unwrap_or_else(|| {
            tracing::debug!(
                target: "anim",
                key = ?key,
                param,
                "a speed parameter past the slots, which the loader refuses, read as one"
            );
            Scalar::ONE
        }),
    };
    Phase::rate(clip.duration_ticks, state.speed * value)
}

/// Step 5's search: the first transition in the machine's order that applies in the
/// current state and whose conditions all hold against `row` and `params` as they are
/// before this tick's advance. A transition with an index or a bit out of range, which the
/// loader refuses, is skipped and logged.
fn first_taken<'a>(
    machine: &'a StateMachine,
    row: &AnimState,
    params: &AnimParams,
    key: StableKey,
) -> Option<&'a Transition> {
    let current = row.state;
    machine.transitions.iter().enumerate().find_map(|(index, transition)| {
        if let Err(what) = transition.check(machine.states.len()) {
            tracing::debug!(
                target: "anim",
                key = ?key,
                transition = index,
                "skipped a transition the loader refuses: {what}"
            );
            return None;
        }
        let applies = match transition.from {
            Some(from) => from == current,
            None => transition.to != current,
        };
        let holds =
            applies && transition.conditions.iter().all(|condition| holds(*condition, row, params));
        holds.then_some(transition)
    })
}

/// Whether `condition` holds; every index and bit is in range (`Transition::check`).
fn holds(condition: Condition, row: &AnimState, params: &AnimParams) -> bool {
    let value = |param: u8| params.values[usize::from(param)];
    match condition {
        Condition::Finished => row.finished(),
        Condition::PhaseAtLeast(phase) => row.phase >= phase,
        Condition::TicksAtLeast(ticks) => row.ticks_in_state >= ticks,
        Condition::Above { param, value: threshold } => value(param) > threshold,
        Condition::Below { param, value: threshold } => value(param) < threshold,
        Condition::Flag { bit, set } => params.flag(bit) == set,
        Condition::Trigger(bit) => params.triggers & (1 << bit) != 0,
    }
}

/// Takes `transition` from the state `before` describes: the target entered, its phase
/// set or kept, the previous phase that entry phase, no tick run yet, not finished or held,
/// and the outgoing state and phase kept as the blend when it has a length.
fn take(row: &mut AnimState, transition: &Transition, before: &AnimState) {
    row.state = transition.to;
    row.phase = if transition.keep_phase { before.phase } else { transition.entry_phase };
    row.previous_phase = row.phase;
    row.ticks_in_state = 0;
    row.flags = AnimState::ENTERED;
    if !transition.keep_phase {
        row.flags |= AnimState::ENTRY_SET;
    }
    if transition.blend_ticks > 0 {
        row.previous_state = before.state;
        row.blend_phase = before.phase;
        row.blend_ticks = transition.blend_ticks;
    } else {
        end_blend(row);
    }
}

/// What the evaluation and the fit read from the region's table for a character: the
/// rig's skeleton, the state's clip and the blend's outgoing side, each clip cooked for that
/// skeleton with its bone count; `None` when any is missing, the state is unset or a clip
/// is another skeleton's.
struct Resolved<'a> {
    skeleton: &'a Skeleton,
    clip: &'a Clip,
    outgoing: Option<Outgoing<'a>>,
}

fn resolve<'a>(
    assets: Option<&'a PinnedTable>,
    rig: &AnimRig,
    state: &AnimState,
) -> Option<Resolved<'a>> {
    let assets = assets?;
    let skeleton = assets.get(rig.skeleton).ok()?;
    let machine = assets.get(rig.machine).ok()?;
    let clip_for = |index: u16| {
        let clip = clip_of(assets, machine.state(index)?)?;
        let fits = clip.skeleton == rig.skeleton && usize::from(clip.bones) == skeleton.bones.len();
        fits.then_some(clip)
    };
    let clip = clip_for(state.state()?)?;
    let outgoing = match state.blend() {
        None => None,
        Some(blend) => {
            Some(Outgoing { clip: clip_for(blend.from)?, phase: blend.phase, weight: blend.weight })
        }
    };
    Some(Resolved { skeleton, clip, outgoing })
}

/// The evaluation: [`SimPose::evaluate`] with the rig's skeleton, the state's clip at its
/// phase and the blend's outgoing clip at its phase, written whole into the row. With the
/// skeleton, the clip or (during a blend) the outgoing clip missing, a clip cooked for
/// another skeleton than the rig's, the state unset, or the character's bones limp (the fit
/// in `Stage::Post` writes the pose then), writes [`SimPose::EMPTY`]. The per-character
/// work the milestone 1 budget of 0.5 ms for 200 characters bounds (`README.md`, "Proof
/// for milestone 1").
#[system(name = "anim::evaluate", worlds = [server, prediction], in_set = sets::EVALUATE)]
pub(crate) fn evaluate(
    ctx: Ctx,
    rig: &AnimRig,
    state: &AnimState,
    bones: Option<&Bones>,
    pose: Mut<SimPose>,
) {
    let limp = bones.is_some_and(Bones::is_limp);
    let value = match resolve(ctx.assets(), rig, state) {
        Some(resolved) if !limp => {
            SimPose::evaluate(resolved.skeleton, resolved.clip, state.phase(), resolved.outgoing)
        }
        _ => SimPose::EMPTY,
    };
    pose.set_if_neq(value);
}

/// A bone row reporting its stepped `Transform` to its owner, when its link says the owner
/// is limp; an animated character's bone rows emit nothing. Server world only, as every
/// bone system is: bone rows exist there alone.
#[system(name = "anim::report_bones", worlds = [server], in_set = sets::REPORT)]
pub(crate) fn report_bones(moved: EventWriter<BoneMoved>, link: &BoneLink, transform: &Transform) {
    if let (true, Some(owner)) = (link.limp, link.owner.get()) {
        moved.emit(BoneMoved { owner, bone: link.bone, transform: *transform });
    }
}

/// A limp character's pose rebuilt from its bone rows, in skeleton order: a bone a
/// `BoneMoved` reported is taken into root-local space against the character's own
/// stepped `Transform`; a bone no row reported (one without a body) is its parent-local
/// sample from the state's clip at its phase (blended as `anim::evaluate` blends) composed
/// onto its parent's fitted pose, so hands and feet hang off the fallen limbs and sockets
/// with them. Written whole into `SimPose`. Does nothing for a character whose bones are
/// not limp; with the skeleton or the clip missing (or what else makes `anim::evaluate`
/// write it), writes `SimPose::EMPTY`.
#[system(name = "anim::fit_ragdoll", worlds = [server], in_set = sets::FIT)]
#[expect(clippy::too_many_arguments, reason = "a kernel's parameters are its row columns")]
pub(crate) fn fit_ragdoll(
    ctx: Ctx,
    moved: Events<BoneMoved>,
    key: StableKeyOf,
    rig: &AnimRig,
    state: &AnimState,
    bones: &Bones,
    root: &Transform,
    pose: Mut<SimPose>,
) {
    if !bones.is_limp() {
        return;
    }
    let value = match resolve(ctx.assets(), rig, state) {
        None => SimPose::EMPTY,
        Some(resolved) => {
            let mut reported = [None; MAX_SIM_BONES];
            for event in moved.for_target(*key) {
                if let Some(slot) = reported.get_mut(usize::from(event.bone)) {
                    *slot = Some(event.transform);
                }
            }
            SimPose::fit(resolved.skeleton, root, &reported, |bone| {
                let local = resolved.clip.sample(state.phase(), bone);
                match resolved.outgoing {
                    None => local,
                    Some(out) => local.blend(out.clip.sample(out.phase, bone), out.weight),
                }
            })
        }
    };
    pose.set_if_neq(value);
}

/// An animated character's bone rows swept to its pose: for every bone with a row,
/// `Commands::set` of the bone's `Transform` at `SimPose::world` of the character's stepped
/// root, which the next tick's physics sync sweeps the kinematic body to during its step.
/// Does nothing for a limp character, whose bodies move themselves, nor for a bone the
/// pose lacks (an empty pose, the rig's assets missing). The write of
/// `Transform` is deliberately not declared: the commands apply at the stage's closing sync
/// point, after every `Stage::Post` system has run, so no reader in the stage can see them
/// and a declaration would only order every `Transform` reader of the stage against this
/// set for nothing (and falsify physics-api's "every other `Stage::Post` system reads the
/// stepped rows after the write-back with no edge").
#[system(name = "anim::drive_bones", worlds = [server], in_set = sets::BONES)]
pub(crate) fn drive_bones(cmd: Commands, bones: &Bones, pose: &SimPose, root: &Transform) {
    if bones.is_limp() {
        return;
    }
    for bone in 0..bones.len().min(pose.len()) {
        // Below `MAX_SIM_BONES`, so the index is a `BoneIndex`.
        let bone = bone as BoneIndex;
        if let (Some(row), Some(transform)) = (bones.key(bone), pose.world(root, bone)) {
            cmd.set(row, transform);
        }
    }
}
