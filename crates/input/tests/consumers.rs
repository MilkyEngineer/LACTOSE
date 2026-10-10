//! Compile-only consumer sketches against the facade: what a game kernel, `app`,
//! `lactose-predict`, `net` and a session applier write, type-checked and never run (every
//! deciding body of the crate is `todo!()`). They pin the surface the README promises: a
//! change that breaks one is a signature change to list. The host side (sources, the
//! quantiser, the latest sample) is sketched in `lactose-input-host`.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use lactose_core::{PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Mut, RegionId, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_input::{
    AXIS_SCALE, Action, Actions, Axis2, Pitch, PlayerInput, PlayerInputApplier, REDUNDANT_SAMPLES,
    SAMPLE_SIZE, SampleHistory, UNITS_PER_TURN, Yaw, player_entity, sets,
};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_reflect::component;
use lactose_sim::{
    InputApplier, PlayerSample, Registration, SessionApplier, SessionEvent, Simulation,
    SimulationBuilder, Stage, StreamingAnchor, TickInput, TypeTable,
};

// ---- 1. Game kernels reading `&PlayerInput`: the section 6.1 example, and movement ----

#[component(key = "sketch::velocity", persistent, replicated)]
pub struct Velocity(#[field(id = 1)] pub Vec3);

#[component(key = "sketch::bomb", persistent)]
pub struct Bomb {
    #[field(id = 1)]
    pub fuse: u32,
}

#[component(key = "sketch::bow", persistent)]
pub struct Bow {
    #[field(id = 1)]
    pub drawn_ticks: u32,
}

/// `throw_bomb` as the architecture section 6.1 example reads with this crate's names:
/// the edge bit for `throw`, the derived direction for `aim`.
#[system(name = "sketch::throw_bomb", worlds = [server, prediction])]
fn throw_bomb(ctx: Ctx, cmd: Commands, key: StableKeyOf, input: &PlayerInput) {
    if input.pressed.contains(Action::Throw) {
        let bomb = cmd.spawn((Bomb { fuse: 180 },));
        let spread = ctx.rng(bomb).next_scalar();
        let impulse: Vec3 = input.aim() * spread;
        cmd.set(bomb, Velocity(impulse));
    }
}

/// Movement: the sample's yaw gives the direction (architecture section 5.3), a held
/// action the speed, and nothing here touches a float that did not come from an integer.
#[system(name = "sketch::movement", worlds = [server, prediction])]
fn movement(input: &PlayerInput, vel: Mut<Velocity>) {
    let speed =
        if input.held.contains(Action::Sprint) { Scalar::from_i32(8) } else { Scalar::from_i32(4) };
    let planar = input.world_movement();
    let yaw: Scalar = input.yaw.radians();
    let facing: Quat = input.yaw.rotation();
    let pitch: Scalar = input.pitch.radians();
    let stick = input.movement.to_vec2();
    let generic_bit_of_the_game = input.held.contains_bit(Action::NAMED + 2);
    vel.0 = planar * speed;
}

/// A hold-and-release action: draw while held, loose on the release edge.
#[system(name = "sketch::bow", worlds = [server, prediction])]
fn bow(input: &PlayerInput, bow: Mut<Bow>) {
    if input.held.contains(Action::Interact) {
        bow.drawn_ticks += 1;
    }
    if input.released.contains(Action::Interact) {
        bow.drawn_ticks = 0;
    }
}

/// Another crate's sync-point set, ordered after this crate's writer declaration.
pub static SKETCH_SYNC: SystemSet = SystemSet::new("sketch::sync").after(&[&sets::APPLY]);

#[system(name = "sketch::sync_hook", worlds = [server, prediction], exclusive, in_set = SKETCH_SYNC)]
fn sync_hook(world: &mut World) {}

pub fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&THROW_BOMB).add(&MOVEMENT).add(&BOW);
    registration.stage(Stage::Input).sets(&[&SKETCH_SYNC]).add(&SYNC_HOOK);
}

// ---- 2. How `app` builds every world kind with this crate wired in ----

fn app_build(kind: WorldKind) -> Result<Simulation> {
    let mut types = TypeTable::new();
    lactose_input::register_types(&mut types)?;
    types.register::<Velocity>()?;
    types.register::<Bomb>()?;
    types.register::<Bow>()?;
    SimulationBuilder::new(kind, types)
        .systems(lactose_input::register_systems)
        .systems(register_game)
        .systems(|registration| {
            registration.order(&sets::APPLY, &SKETCH_SYNC);
        })
        .inputs(Box::new(PlayerInputApplier::new()))
        .sessions(Box::new(Sessions))
        .build()
}

// ---- 3. `lactose-predict` and a test: a sample into a frame, and the round trip ----

fn predict_frame(input: &PlayerInput, tick: Tick, local: PlayerId) -> Result<TickInput> {
    input.validate()?;
    let bytes: [u8; SAMPLE_SIZE] = input.to_sample_bytes();
    let back: PlayerInput = PlayerInput::from_sample_bytes(&bytes)?;
    assert_eq!(back, *input);
    let mut frame = TickInput::default();
    frame.inputs.insert(local, input.to_player_sample(tick));
    Ok(frame)
}

/// A test's sample, built field by field.
fn literal_sample() -> PlayerInput {
    PlayerInput {
        movement: Axis2 { x: 0, y: AXIS_SCALE },
        yaw: Yaw((UNITS_PER_TURN / 4) as u16),
        pitch: Pitch(-Pitch::LIMIT / 2),
        held: Actions::EMPTY.with(Action::Sprint).with(Action::Jump),
        pressed: Actions::from_bits(1 << Action::Jump.bit()),
        released: Actions::EMPTY.with(Action::Throw),
    }
}

// ---- 4. `net`: pack the window, merge a received window, substitute and repair ----

fn pack(history: &SampleHistory) -> Vec<(Tick, [u8; SAMPLE_SIZE])> {
    history.iter().map(|(tick, input)| (tick, input.to_sample_bytes())).collect()
}

/// The server's receive path: every entry validated, a failure treated as lost, every
/// accepted entry bounded by the last tick applied.
fn receive(history: &mut SampleHistory, consumed: Tick, window: &[(Tick, [u8; SAMPLE_SIZE])]) {
    for (tick, bytes) in window {
        if let Ok(input) = PlayerInput::from_sample_bytes(bytes) {
            history.insert_bounded(consumed, *tick, input);
        }
    }
}

/// The server's per-player state between ticks.
struct PlayerQueue {
    history: SampleHistory,
    last_applied: PlayerInput,
    consumed: Tick,
    substituting: bool,
}

/// The server's frame entry for one player at `tick`: the delivered sample, with the edges
/// a substitution swallowed restored on the first one after it, or the previous one
/// continued, journaled as substituted.
fn server_entry(queue: &mut PlayerQueue, tick: Tick) -> PlayerSample {
    let entry = match queue.history.get(tick) {
        Some(input) => {
            let input = if queue.substituting {
                input.after_substitute(&queue.last_applied)
            } else {
                *input
            };
            queue.substituting = false;
            queue.last_applied = input;
            input.to_player_sample(tick)
        }
        None => {
            let substitute = queue.last_applied.substitute();
            queue.substituting = true;
            queue.last_applied = substitute;
            let mut entry = substitute.to_player_sample(tick);
            entry.substituted = true;
            entry
        }
    };
    queue.consumed = tick;
    entry
}

fn newest_and_count(history: &SampleHistory) -> (Option<Tick>, usize) {
    let newest = history.newest().map(|(tick, _)| tick);
    let count = history.into_iter().count();
    debug_assert!(count <= SampleHistory::CAPACITY && count == history.len());
    (newest, count)
}

// ---- 5. A session applier that uses the same entity rule as the input applier ----

struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                // Spawned with an idle sample so the kernels run from the first tick.
                world.spawn(
                    key,
                    region,
                    (
                        Velocity(Vec3::ZERO),
                        Bow { drawn_ticks: 0 },
                        PlayerInput::IDLE,
                        ClaimedBy { player },
                        StreamingAnchor { priority: 0 },
                    ),
                )
            }
            SessionEvent::Leave => match player_entity(world, player) {
                Some(key) => world.despawn(key),
                None => Ok(()),
            },
        }
    }
}

/// `app` or a test may wrap the applier, or replace it after build.
struct Logging(PlayerInputApplier);

impl InputApplier for Logging {
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()> {
        self.0.apply(world, player, sample)
    }
}

fn replace_applier(simulation: &mut Simulation) {
    simulation
        .world_mut()
        .insert_service::<dyn InputApplier>(Box::new(Logging(PlayerInputApplier::default())));
}

/// The lookup takes a shared world: `net` and `predict` call it between ticks too.
fn lookup(world: &World, player: PlayerId) -> Option<StableKey> {
    player_entity(world, player)
}

// ---- 6. What the wire and the journal rely on, pinned at compile time ----

const _: () = {
    assert!(SAMPLE_SIZE == 20);
    assert!(REDUNDANT_SAMPLES == 8);
    assert!(Pitch::LIMIT == 16384);
    assert!(AXIS_SCALE == 32767);
    assert!(UNITS_PER_TURN == 65536);
};

fn region_of_test() -> RegionId {
    RegionId(1)
}
