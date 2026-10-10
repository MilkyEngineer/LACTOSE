//! Compile-only consumer sketches against the facade: what a game kernel, `app`,
//! `lactose-predict`, `net`, a session applier, presentation and a milestone 5 source
//! write, type-checked and never run (every deciding body of the crate is `todo!()`). They
//! pin the surface the README promises: a change that breaks one is a signature change to
//! list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use lactose_core::{PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Mut, RegionId, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_input::latest::LatestSample;
use lactose_input::raw::RawSample;
use lactose_input::sources::Source;
use lactose_input::{
    AXIS_SCALE, Action, Actions, Axis2, Pitch, PlayerInput, PlayerInputApplier, REDUNDANT_SAMPLES,
    SAMPLE_SIZE, SampleHistory, UNITS_PER_TURN, Yaw, player_entity, sets,
};
use lactose_math::{Scalar, Vec3};
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
    let pitch: Scalar = input.pitch.radians();
    let stick = input.movement.to_vec2();
    let generic_bit_of_the_game = input.held.contains_bit(Action::NAMED + 2);
    vel.0 = planar * speed;
}

/// Another crate's sync-point set, ordered after this crate's writer declaration.
pub static SKETCH_SYNC: SystemSet = SystemSet::new("sketch::sync").after(&[&sets::APPLY]);

#[system(name = "sketch::sync_hook", worlds = [server, prediction], exclusive, in_set = SKETCH_SYNC)]
fn sync_hook(world: &mut World) {}

pub fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&THROW_BOMB).add(&MOVEMENT);
    registration.stage(Stage::Input).sets(&[&SKETCH_SYNC]).add(&SYNC_HOOK);
}

// ---- 2. How `app` builds every world kind with this crate wired in ----

fn app_build(kind: WorldKind) -> Result<Simulation> {
    let mut types = TypeTable::new();
    lactose_input::register_types(&mut types)?;
    types.register::<Velocity>()?;
    types.register::<Bomb>()?;
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

// ---- 3. The client and `lactose-predict`: quantise, round-trip, frame entry ----

fn client_tick(
    raw: &RawSample,
    previous: &PlayerInput,
    tick: Tick,
    history: &mut SampleHistory,
) -> Result<TickInput> {
    let input: PlayerInput = raw.quantise(previous.held);
    input.validate()?;
    let stored = history.insert(tick, input);
    let bytes: [u8; SAMPLE_SIZE] = input.to_sample_bytes();
    let back: PlayerInput = PlayerInput::from_sample_bytes(&bytes)?;
    assert_eq!(back, input);
    let mut frame = TickInput::default();
    frame.inputs.insert(PlayerId(1), input.to_player_sample(tick));
    Ok(frame)
}

/// A test's or `lactose-predict`'s sample, built field by field.
fn literal_sample() -> PlayerInput {
    PlayerInput {
        movement: Axis2 { x: 0, y: AXIS_SCALE },
        yaw: Yaw((UNITS_PER_TURN / 4) as u16),
        pitch: Pitch(-Pitch::LIMIT / 2),
        held: Actions::EMPTY.with(Action::Sprint),
        pressed: Actions::from_bits(1 << Action::Jump.bit()),
    }
}

// ---- 4. `net`: pack the window, merge a received window, substitute ----

fn pack(history: &SampleHistory) -> Vec<(Tick, [u8; SAMPLE_SIZE])> {
    history.iter().map(|(tick, input)| (tick, input.to_sample_bytes())).collect()
}

fn receive(history: &mut SampleHistory, window: &[(Tick, [u8; SAMPLE_SIZE])]) -> Result<()> {
    for (tick, bytes) in window {
        let input = PlayerInput::from_sample_bytes(bytes)?;
        history.insert(*tick, input);
    }
    Ok(())
}

/// The server's frame entry for one player at `tick`: the delivered sample, or the
/// previous one continued, journaled as substituted.
fn server_entry(
    history: &SampleHistory,
    last_applied: &mut PlayerInput,
    tick: Tick,
) -> PlayerSample {
    match history.get(tick) {
        Some(input) => {
            *last_applied = *input;
            input.to_player_sample(tick)
        }
        None => {
            let substitute = last_applied.substitute();
            *last_applied = substitute;
            let mut entry = substitute.to_player_sample(tick);
            entry.substituted = true;
            entry
        }
    }
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

// ---- 6. Presentation: the latest raw sample for the late-latched camera ----

static LATEST: LatestSample = LatestSample::new();

fn input_task_frame(raw: RawSample) {
    LATEST.publish(raw);
}

fn latch() -> (f32, f32) {
    let sample = LATEST.read();
    (sample.yaw, sample.pitch)
}

// ---- 7. A milestone 5 source ----

struct Gamepad;

impl Source for Gamepad {
    fn poll(&mut self, raw: &mut RawSample) {
        raw.movement_x = 0.0;
        raw.held = raw.held.with(Action::Jump);
    }
}

fn poll_all(sources: &mut [Box<dyn Source>], raw: &mut RawSample) {
    for source in sources {
        source.poll(raw);
    }
}

// ---- 8. What the wire and the journal rely on, pinned at compile time ----

const _: () = {
    assert!(SAMPLE_SIZE == 16);
    assert!(REDUNDANT_SAMPLES == 8);
    assert!(Pitch::LIMIT == 16384);
    assert!(AXIS_SCALE == 32767);
    assert!(UNITS_PER_TURN == 65536);
};

fn region_of_test() -> RegionId {
    RegionId(1)
}
