// The scene of `cost.rs`, in a file of its own so that `crates/app/tests/cost.rs` runs the
// same scene with physics beside it (the composed tick). Included, not a module: its items
// and its imports are the including file's.

#[component(key = "cost::position", persistent, replicated)]
struct Position {
    #[field(id = 1)]
    x: Scalar,
    #[field(id = 2)]
    y: Scalar,
}

#[component(key = "cost::velocity", persistent)]
struct Velocity {
    #[field(id = 1)]
    x: Scalar,
    #[field(id = 2)]
    y: Scalar,
}

#[component(key = "cost::health", persistent, replicated)]
struct Health(#[field(id = 1)] i32);

#[component(key = "cost::timer", persistent)]
struct Timer(#[field(id = 1)] u32);

#[component(key = "cost::marker", persistent)]
struct Marker;

/// Contacts this tick: transient, zeroed at every input sync point.
#[component(key = "cost::contacts", transient)]
struct Contacts(#[field(id = 1)] u32);

/// A player's steering this tick: transient, written by the input applier.
#[component(key = "cost::steer", transient)]
struct Steer {
    #[field(id = 1)]
    dx: i32,
    #[field(id = 2)]
    dy: i32,
}

/// A region's wind, read by every mover's kernel and turned at the post sync point.
#[resource(key = "cost::wind", persistent)]
struct Wind(#[field(id = 1)] Scalar);

/// A region's gusts this tick: transient, zeroed at the input sync point.
#[resource(key = "cost::gusts", transient)]
struct Gusts(#[field(id = 1)] u32);

/// How many rows the scene was built with, from which a hit picks its target.
#[resource(key = "cost::population", derived)]
struct Population(#[field(id = 1)] u32);

/// What the reduction writes.
#[resource(key = "cost::energy", derived)]
struct Energy(#[field(id = 1)] Scalar);

/// One hit on one entity.
#[derive(Clone, Copy)]
struct Hit {
    target: StableKey,
    amount: i32,
}

impl Event for Hit {
    fn target(&self) -> StableKey {
        self.target
    }
}

const ROWS: u64 = 100_000;
const REGIONS: u64 = 16;
const PLAYERS: u32 = 64;
/// The first player's key, after every row's.
const FIRST_PLAYER: u64 = 1_000_000;

fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

fn region(n: u64) -> RegionId {
    RegionId(1 + n % REGIONS)
}

static MOVEMENT: SystemSet = SystemSet::new("cost::movement");
static INPUT: SystemSet = SystemSet::new("cost::input").after(&[&sets::INPUT]);

/// The declared writer of [`Steer`]: the applier writes it inside `sim`'s systems.
#[system(name = "cost::input_writer", worlds = [server], exclusive, in_set = INPUT, writes = [Steer])]
fn input_writer(_world: &mut World) {}

#[system(name = "cost::steer", worlds = [server], in_set = MOVEMENT)]
fn steer(input: &Steer, vel: Mut<Velocity>) {
    vel.x += Scalar::from_i32(input.dx);
    vel.y += Scalar::from_i32(input.dy);
}

#[system(name = "cost::integrate", worlds = [server], in_set = MOVEMENT, after = steer)]
fn integrate(dt: Res<TickDelta>, wind: RegionRes<Wind>, vel: &Velocity, pos: Mut<Position>) {
    let next = Position { x: pos.x + (vel.x + wind.0) * dt.0, y: pos.y + vel.y * dt.0 };
    pos.set_if_neq(next);
}

#[system(name = "cost::drag", worlds = [server], in_set = MOVEMENT, after = integrate)]
fn drag(vel: Mut<Velocity>, _plain: Without<Marker>) {
    let half = Scalar::from_i32(1) / Scalar::from_i32(2);
    vel.x *= half;
    vel.y *= half;
}

#[system(name = "cost::touch", worlds = [server], in_set = MOVEMENT, after = integrate)]
fn touch(pos: &Position, contacts: Mut<Contacts>) {
    contacts.0 = 1 + u32::from(pos.y < Scalar::ZERO);
}

/// One row in a hundred hits another, chosen by the RNG keyed by the striker.
#[system(name = "cost::strike", worlds = [server], in_set = MOVEMENT, after = touch)]
fn strike(
    ctx: Ctx,
    rows: Res<Population>,
    key: StableKeyOf,
    contacts: &Contacts,
    hits: EventWriter<Hit>,
) {
    let mut rng = ctx.rng(*key);
    if rng.next_below(100) == 0 {
        let target = self::key(1 + u64::from(rng.next_below(rows.0)));
        hits.emit(Hit { target, amount: contacts.0 as i32 });
    }
}

#[system(name = "cost::take_hits", worlds = [server], in_set = MOVEMENT, after = strike)]
fn take_hits(events: Events<Hit>, key: StableKeyOf, hp: Mut<Health>) {
    for hit in events.for_target(*key) {
        hp.0 -= hit.amount;
    }
}

/// One timed row in a thousand is replaced by a fresh one each tick.
#[system(name = "cost::timers", worlds = [server], in_set = MOVEMENT, after = integrate)]
fn timers(ctx: Ctx, key: StableKeyOf, timer: Mut<Timer>, pos: &Position, cmd: Commands) {
    timer.0 += 1;
    if ctx.rng(*key).next_below(1_000) == 0 {
        cmd.despawn(*key);
        let velocity = Velocity { x: Scalar::ONE, y: Scalar::ZERO };
        cmd.spawn((Position { x: pos.x, y: pos.y }, velocity, Timer(0)));
    }
}

#[system(name = "cost::energy", worlds = [server], in_set = MOVEMENT, after = drag, reduce = ExactSum<Scalar>)]
fn energy(vel: &Velocity) -> Scalar {
    vel.x * vel.x + vel.y * vel.y
}

#[finalize(energy)]
fn store_energy(sum: Scalar, out: ResMut<Energy>) {
    out.0 = sum;
}

/// Every simulated region's wind turns and its gusts count, at the post sync point.
#[system(name = "cost::weather", worlds = [server], exclusive)]
fn weather(world: &mut World) {
    let regions: Vec<RegionId> = world.regions().collect();
    for region in regions {
        world.update_region_resource::<Wind>(region, |wind| wind.0 = -wind.0).unwrap();
        world.update_region_resource::<Gusts>(region, |gusts| gusts.0 += 1).unwrap();
    }
}

fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&INPUT]).add(&INPUT_WRITER);
    registration
        .stage(Stage::Simulation)
        .sets(&[&MOVEMENT])
        .add(&STEER)
        .add(&INTEGRATE)
        .add(&DRAG)
        .add(&TOUCH)
        .add(&STRIKE)
        .add(&TAKE_HITS)
        .add(&TIMERS)
        .add(&ENERGY);
    registration.stage(Stage::Post).add(&WEATHER);
}

fn types() -> TypeTable {
    let mut types = TypeTable::new();
    types.register::<Position>().unwrap();
    types.register::<Velocity>().unwrap();
    types.register::<Health>().unwrap();
    types.register::<Timer>().unwrap();
    types.register::<Marker>().unwrap();
    types.register::<Contacts>().unwrap();
    types.register::<Steer>().unwrap();
    types.register::<Wind>().unwrap();
    types.register::<Gusts>().unwrap();
    types.register::<Energy>().unwrap();
    types.register::<Population>().unwrap();
    types
}

/// Writes each player's sample to the entity the player has claimed.
struct Steering;

impl InputApplier for Steering {
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()> {
        let value: Steer = lactose_reflect::decode(&sample.sample)?;
        let key = world.claimed(player).next().expect("every player has an entity");
        world.insert(key, value)
    }
}

/// The scene: `rows` rows of four archetypes spread over the regions, and the players.
fn scene(rows: u64) -> Simulation {
    scene_in(rows, LayoutConfig::default())
}

/// The scene under `layout`.
fn scene_in(rows: u64, layout: LayoutConfig) -> Simulation {
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types())
        .layout(layout)
        .setup(move |world| {
            world.insert_resource(Energy(Scalar::ZERO));
            world.insert_resource(Population(rows as u32));
            Ok(())
        })
        .systems(register_systems)
        .inputs(Box::new(Steering))
        .build()
        .unwrap();
    let world = simulation.world_mut();
    for n in 0..REGIONS {
        let id = RegionId(1 + n);
        world.create_region(RegionConfig { id, seed: n, mode: RegionMode::Simulated }).unwrap();
        world.insert_region_resource(id, Wind(Scalar::from_i32(n as i32 - 8))).unwrap();
        world.insert_region_resource(id, Gusts(0)).unwrap();
    }
    for n in 1..=rows {
        let at = Position { x: Scalar::from_i32(n as i32), y: Scalar::from_i32(-(n as i32 % 7)) };
        let moving = Velocity { x: Scalar::ONE, y: Scalar::from_i32(n as i32 % 3) };
        let result = match n % 10 {
            0..4 => world.spawn(key(n), region(n), (at, moving, Health(100), Contacts(0))),
            4..7 => world.spawn(key(n), region(n), (at, moving, Timer(n as u32 % 600))),
            7..9 => world.spawn(key(n), region(n), (at, Health(100))),
            _ => world.spawn(
                key(n),
                region(n),
                (at, moving, Health(200), Timer(0), Marker, Contacts(0)),
            ),
        };
        result.unwrap();
    }
    for player in 0..PLAYERS {
        let n = FIRST_PLAYER + u64::from(player);
        let row = (
            Position { x: Scalar::ZERO, y: Scalar::ZERO },
            Velocity { x: Scalar::ZERO, y: Scalar::ZERO },
            Health(1_000),
            Contacts(0),
            Steer { dx: 0, dy: 0 },
            ClaimedBy { player: PlayerId(player) },
        );
        world.spawn(key(n), region(n), row).unwrap();
    }
    simulation
}

/// Every player's input for `tick`.
fn frame(tick: u32) -> TickInput {
    let mut frame = TickInput::default();
    for player in 0..PLAYERS {
        let steer = Steer { dx: (tick % 3) as i32 - 1, dy: (player % 3) as i32 - 1 };
        let sample = PlayerSample {
            input_tick: Tick(tick),
            substituted: false,
            sample: lactose_reflect::bytemuck::bytes_of(&steer).to_vec(),
        };
        frame.inputs.insert(PlayerId(player), sample);
    }
    frame
}

fn step(executor: &Executor, simulation: Simulation) -> Simulation {
    let tick = simulation.tick().0 + 1;
    executor.block_on(simulation.step(executor, frame(tick))).unwrap()
}
