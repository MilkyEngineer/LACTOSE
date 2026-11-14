//! "Time scale, client" and "The client's frame": the prediction world's half of the timer
//! root over a stand-in clock. Every client starts with a `snap` estimate at the server's
//! tick zero, whose resync in the first frame puts the pacer at the clock's base.

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, PoisonError};

use lactose_app::{Client, ClientOutput, Cooked, Start, TICK_PERIOD, WorldConfig, build_client};
use lactose_bundle::ComponentData;
use lactose_core::{ErrorKind, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Executor, RegionId, RegionMode, World};
use lactose_input::{PlayerInput, Yaw};
use lactose_math::{Scalar, Vec3};
use lactose_net_api::{
    OverlayPayload, RegionChange, RegionRelevance, StagedFrame, StagedInbox, StagedSpawn,
};
use lactose_net_host::ClockEstimate;
use lactose_net_host::message::JoinAccepted;
use lactose_physics_api::{Collider, RigidBody, Shape, Transform, Velocity};
use lactose_reflect::{Reflect, TypeRole};
use lactose_sim::{SessionEvent, StagedState, TickInput, TimeScale};

use crate::common::{
    Clock, Content, Counter, GAME, anchored, capture, executor, load, logged, server, step,
    table_of,
};

const LEAD: u32 = 2;

fn estimate(server: u32, snap: bool) -> ClockEstimate {
    paced(server, LEAD, 1000, snap)
}

fn paced(server: u32, lead: u32, rate: u32, snap: bool) -> ClockEstimate {
    ClockEstimate {
        server_tick: Tick(server),
        lead_ticks: lead as f32,
        rate_thousandths: rate,
        snap,
    }
}

/// A client of the proof's game joined at tick zero with `lead`, its pacer as built:
/// owing its first tick at once.
fn built(executor: &Executor, content: &Arc<Content>, lead: u32) -> Client {
    let accepted = JoinAccepted { player: PlayerId(1), tick: Tick(0) };
    let entity = StableKey::Net(NetId::runtime(NonZeroU64::MIN).expect("a runtime id"));
    let cooked = Arc::clone(content) as Arc<dyn Cooked>;
    build_client(executor, &GAME, WorldConfig::headless(), accepted, entity, lead, cooked)
        .expect("the client builds")
}

/// A client of the proof's game joined at tick zero, its pacer put at `clock.at(0)` by a
/// first frame there, which runs no tick.
fn client(executor: &Executor, content: &Arc<Content>, clock: Clock) -> Client {
    let mut client = built(executor, content, LEAD);
    assert_eq!(client.tick(), Tick(LEAD));
    client.clock(estimate(0, true));
    let (client, out) = frame(executor, client, clock, 0, &mut sampled);
    assert_eq!((out.ticks, out.resync), (0, Some(Tick(LEAD))));
    client
}

/// The sample of `tick`: its yaw is the tick.
fn sampled(tick: Tick) -> PlayerInput {
    PlayerInput { yaw: Yaw(tick.0 as u16), ..PlayerInput::default() }
}

fn frame(
    executor: &Executor,
    client: Client,
    clock: Clock,
    periods: u32,
    sample: &mut (dyn FnMut(Tick) -> PlayerInput + Send),
) -> (Client, ClientOutput) {
    executor.block_on(client.frame(executor, clock.at(periods), sample)).expect("the frame")
}

/// Pushes `frame`, which the inbox must accept.
fn push(client: &mut Client, frame: StagedFrame) {
    assert!(client.push(Arc::new(frame)), "the inbox accepts it");
}

fn world(client: &Client) -> &World {
    client.predictor().simulation().world()
}

fn queued(client: &Client) -> usize {
    world(client).service::<StagedInbox>().map_or(0, StagedInbox::len)
}

fn regions(client: &Client) -> Vec<u64> {
    world(client).regions().map(|region| region.0).collect()
}

/// What a server says of regions `ids` as they enter a client's set.
fn relevance(executor: &Executor, content: &Arc<Content>, ids: &[u64]) -> Vec<RegionRelevance> {
    let server = server(executor, WorldConfig::headless(), Start::Fresh, content);
    let (server, out) = step(executor, server, crate::common::load(content, ids.iter().copied()));
    assert!(out.failures.is_empty(), "{out:?}");
    let messages = ids
        .iter()
        .map(|&id| server.relevance(executor, RegionId(id)).expect("relevance"))
        .collect();
    server.shutdown();
    messages
}

/// A message whose baseline no cooked content holds: its fold fails.
fn unresolvable(region: u64) -> RegionChange {
    let content = Content::new();
    RegionChange::Relevant(RegionRelevance {
        region: RegionId(region),
        seed: region,
        baseline_hash: content.baseline.content_hash() ^ 1,
        overlay: OverlayPayload::Cooked,
        resources: Vec::new(),
        transient_resources: Vec::new(),
        assets: None,
    })
}

/// A `dyn StagedState` inserted over the prediction hook: the next step's check fails and
/// hands the predictor back.
struct Foreign;

impl StagedState for Foreign {
    fn apply(&mut self, _: &mut World, _: Tick) -> Result<()> {
        Ok(())
    }
}

fn break_the_hook(client: &mut Client) {
    let world = client.predictor_mut().simulation_mut().world_mut();
    world.insert_service::<dyn StagedState>(Box::new(Foreign));
}

#[test]
fn a_replicated_pause_stops_the_ticks_at_the_fold_and_an_unpause_restarts_them() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let client = client(&executor, &content, clock);
    let (mut client, out) = frame(&executor, client, clock, 1, &mut sampled);
    assert_eq!(out.ticks, 1);
    let present = client.tick();

    let mut paused = StagedFrame::new(present);
    paused.time_scale = Some(TimeScale::PAUSED);
    push(&mut client, paused);
    let (mut client, out) = frame(&executor, client, clock, 2, &mut sampled);
    assert_eq!(out.ticks, 0, "the pause reaches the pacer at the fold");
    assert_eq!(client.tick(), present);
    assert_eq!(queued(&client), 1, "nothing is popped");
    for periods in 3..10 {
        let out;
        (client, out) = frame(&executor, client, clock, periods, &mut sampled);
        assert_eq!(out.ticks, 0);
    }

    let mut unpaused = StagedFrame::new(Tick(present.0 + 1));
    unpaused.time_scale = Some(TimeScale::REAL_TIME);
    push(&mut client, unpaused);
    let (client, out) = frame(&executor, client, clock, 10, &mut sampled);
    assert_eq!(out.ticks, 1, "the unpause makes ticks due at its fold");
    assert_eq!(world(&client).resource::<TimeScale>().copied(), Some(TimeScale::REAL_TIME));
    assert_eq!(queued(&client), 0, "the hook popped both frames");
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_full_frame_loads_exactly_its_regions() {
    let executor = executor(2);
    let content = Content::new();
    let messages = relevance(&executor, &content, &[1, 2]);
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);

    let mut full = StagedFrame::resync(Tick(client.tick().0 + 1));
    full.regions.insert(RegionId(1), RegionChange::Relevant(messages[0].clone()));
    push(&mut client, full);
    let (mut client, out) = frame(&executor, client, clock, 1, &mut sampled);
    assert_eq!(out.ticks, 1);
    assert_eq!(regions(&client), [1]);

    let mut full = StagedFrame::resync(Tick(client.tick().0 + 1));
    full.regions.insert(RegionId(2), RegionChange::Relevant(messages[1].clone()));
    push(&mut client, full);
    let (client, _) = frame(&executor, client, clock, 2, &mut sampled);
    assert_eq!(regions(&client), [2], "a full frame unloads what it does not name");
    assert_eq!(
        world(&client).region_assets(RegionId(2)).map(|pinned| pinned.table().clone()),
        Some(table_of(RegionId(2))),
        "loaded with the message's table"
    );
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_kept_failure_keeps_the_client_and_the_next_frame_resyncs() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    break_the_hook(&mut client);
    let failure = executor
        .block_on(client.frame(&executor, clock.at(1), &mut sampled))
        .err()
        .expect("the hook did not run");
    assert_eq!(logged(&log, tracing::Level::WARN, "an error the run survives"), 1);
    let client = failure.client.expect("the predictor came back");
    let (client, out) = frame(&executor, client, clock, 2, &mut sampled);
    // The estimate (server 0) advanced by the one tick the failed frame's pacer granted.
    assert_eq!(out.resync, Some(Tick(1 + LEAD)));
    let (_, out) = frame(&executor, client, clock, 3, &mut sampled);
    assert_eq!(out.resync, None, "one resync");
    executor.shutdown();
}

#[test]
fn a_snap_resyncs_at_the_next_frame() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    client.clock(ClockEstimate { lead_ticks: 3.0, ..estimate(40, true) });
    assert_eq!(client.lead(), 3);
    let (mut client, out) = frame(&executor, client, clock, 1, &mut sampled);
    assert_eq!(out.resync, Some(Tick(43)));
    assert_eq!(out.ticks, 0, "the pacer resynced at the frame");
    assert_eq!(client.tick(), Tick(43));
    client.clock(estimate(40, false));
    let (client, out) = frame(&executor, client, clock, 2, &mut sampled);
    assert_eq!((out.resync, out.ticks), (None, 1));
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_failing_fold_resyncs_once_per_frame_tick() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    for periods in 1..=2 {
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    let present = client.tick();

    // A frame whose region cannot be built, carrying a pause, stays queued through 120
    // frames: it resyncs the world at its own tick once.
    let mut failing = StagedFrame::new(Tick(present.0 - 1));
    failing.time_scale = Some(TimeScale::PAUSED);
    failing.regions.insert(RegionId(5), unresolvable(5));
    push(&mut client, failing);
    let mut resyncs = 0;
    for periods in 3..123 {
        let out;
        (client, out) = frame(&executor, client, clock, periods, &mut sampled);
        assert_eq!(out.ticks, 0);
        assert_eq!(out.next_deadline, clock.at(periods + 1), "the pacer is untouched");
        if let Some(tick) = out.resync {
            assert_eq!(tick, present, "at the world's own tick");
            resyncs += 1;
        }
    }
    assert_eq!(resyncs, 1, "once, not per frame");
    assert_eq!(queued(&client), 1);
    assert_eq!(logged(&log, tracing::Level::WARN, "Cooked"), 1);

    // A newer failing frame, carrying the unpause, resyncs again, and the pacer's
    // schedule is the one it had.
    let mut newer = StagedFrame::new(present);
    newer.time_scale = Some(TimeScale::REAL_TIME);
    newer.regions.insert(RegionId(6), unresolvable(6));
    push(&mut client, newer);
    let (client, out) = frame(&executor, client, clock, 123, &mut sampled);
    assert_eq!(out.resync, Some(present));
    assert_eq!((out.ticks, out.next_deadline), (1, clock.at(124)));
    assert_eq!(queued(&client), 0);
    assert!(regions(&client).is_empty(), "a region that cannot be built is left out");
    assert_eq!(logged(&log, tracing::Level::WARN, "Cooked"), 2);
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_backward_resync_predicts_again_with_the_historys_samples() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    let asked = Mutex::new(Vec::new());
    let mut recording = |tick: Tick| {
        asked.lock().unwrap_or_else(PoisonError::into_inner).push(tick.0);
        sampled(tick)
    };
    for periods in 1..=5 {
        client = frame(&executor, client, clock, periods, &mut recording).0;
    }
    assert_eq!(client.tick(), Tick(7));
    // Back to 4: ticks 5 to 7 were predicted and sent.
    client.clock(estimate(2, true));
    let (mut client, out) = frame(&executor, client, clock, 6, &mut recording);
    assert_eq!((out.resync, out.ticks), (Some(Tick(4)), 0));
    for periods in 7..=10 {
        client = frame(&executor, client, clock, periods, &mut recording).0;
    }
    assert_eq!(client.tick(), Tick(8));
    let asked = std::mem::take(&mut *asked.lock().unwrap_or_else(PoisonError::into_inner));
    assert_eq!(asked, [3, 4, 5, 6, 7, 8], "the sampler is not called for 5 to 7 again");
    assert_eq!(client.history().get(Tick(6)), Some(&sampled(Tick(6))));
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_server_ahead_resyncs_to_at_least_its_tick_plus_the_lead() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    let present = client.tick();
    // The estimate is stale: the server is fifty ticks further on.
    push(&mut client, StagedFrame::new(Tick(present.0 + 50)));
    let (client, out) = frame(&executor, client, clock, 1, &mut sampled);
    let resync = out.resync.expect("behind");
    assert!(resync.0 >= present.0 + 50 + LEAD, "{resync:?}");
    assert_eq!(client.tick(), resync);
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_frame_older_than_the_ring_resyncs_to_at_least_its_tick_plus_the_lead() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    client.predictor_mut().set_depth(2).expect("a depth of two");
    for periods in 1..=10 {
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    let old = Tick(client.tick().0 - 5);
    push(&mut client, StagedFrame::new(old));
    let (client, out) = frame(&executor, client, clock, 11, &mut sampled);
    let resync = out.resync.expect("too old");
    assert!(resync.0 >= old.0 + LEAD, "{resync:?}");
    client.shutdown();
    executor.shutdown();
}

#[test]
fn the_clock_target_advances_by_the_ticks_granted_not_the_time_paused() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    // The estimate is taken at the first frame: server tick zero, nothing granted.
    let mut client = client(&executor, &content, clock);
    for periods in 1..=3 {
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    let present = client.tick();
    let mut paused = StagedFrame::new(present);
    paused.time_scale = Some(TimeScale::PAUSED);
    push(&mut client, paused);
    for periods in 4..124 {
        let out;
        (client, out) = frame(&executor, client, clock, periods, &mut sampled);
        assert_eq!(out.ticks, 0);
    }
    let mut unpaused = StagedFrame::new(Tick(present.0 + 1));
    unpaused.time_scale = Some(TimeScale::REAL_TIME);
    push(&mut client, unpaused);
    for periods in 124..126 {
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    // Six ticks granted: three, then the two after the unpause, then the failing frame's.
    break_the_hook(&mut client);
    let failure = executor.block_on(client.frame(&executor, clock.at(126), &mut sampled));
    let client = failure.err().and_then(|failure| failure.client).expect("kept");
    let (client, out) = frame(&executor, client, clock, 127, &mut sampled);
    assert_eq!(out.resync, Some(Tick(6 + LEAD)), "not the pause's length further");
    client.shutdown();
    executor.shutdown();
}

#[test]
fn the_history_holds_the_last_eight_samples() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    for periods in 1..=20 {
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    let held: Vec<u32> = client.history().iter().map(|(tick, _)| tick.0).collect();
    let newest = client.tick().0;
    assert_eq!(held, ((newest - 7)..=newest).collect::<Vec<_>>());
    client.shutdown();
    executor.shutdown();
}

#[test]
fn the_fold_carries_a_replicated_scale_into_the_step() {
    // A due frame's time scale is the step's input, so the mirror follows it.
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    let mut slow = StagedFrame::new(Tick(client.tick().0 + 1));
    slow.time_scale = Some(TimeScale { thousandths: 500 });
    push(&mut client, slow);
    let (client, out) = frame(&executor, client, clock, 2, &mut sampled);
    assert_eq!(out.ticks, 1, "half of two periods");
    assert_eq!(
        world(&client).resource::<TimeScale>().copied(),
        Some(TimeScale { thousandths: 500 })
    );
    client.shutdown();
    executor.shutdown();
}

/// A new client, whose pacer owes its first tick, told rate zero without a snap: its first
/// frame runs no tick.
#[test]
fn a_new_client_paced_at_rate_zero_runs_no_tick() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = built(&executor, &content, LEAD);
    client.clock(paced(0, LEAD, 0, false));
    let at = client.tick();
    let (client, out) = frame(&executor, client, clock, 3, &mut sampled);
    assert_eq!(out.ticks, 0, "a client paced at rate zero ran a tick");
    assert_eq!(client.tick(), at);
    client.shutdown();
    executor.shutdown();
}

/// A new client whose first due frame carries a pause: no tick.
#[test]
fn a_replicated_pause_on_a_new_client_runs_no_tick() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = built(&executor, &content, LEAD);
    let mut paused = StagedFrame::new(client.tick());
    paused.time_scale = Some(TimeScale::PAUSED);
    push(&mut client, paused);
    let at = client.tick();
    let (client, out) = frame(&executor, client, clock, 3, &mut sampled);
    assert_eq!(out.ticks, 0, "a paused client ran a tick");
    assert_eq!(client.tick(), at);
    client.shutdown();
    executor.shutdown();
}

/// `TooOld` whose frame is later than the estimate advanced by the granted ticks: the
/// target is the frame plus the lead, not the estimate's (step 8).
#[test]
fn a_frame_older_than_the_ring_targets_its_tick_plus_the_lead_when_that_is_later() {
    const LONG: u32 = 10;
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = built(&executor, &content, LONG);
    client.clock(paced(0, LONG, 1000, true));
    let (mut client, out) = frame(&executor, client, clock, 0, &mut sampled);
    assert_eq!((out.ticks, out.resync), (0, Some(Tick(LONG))));
    client.predictor_mut().set_depth(2).expect("a depth of two");
    for periods in 1..=10 {
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    let old = Tick(client.tick().0 - 5);
    push(&mut client, StagedFrame::new(old));
    let (client, out) = frame(&executor, client, clock, 11, &mut sampled);
    assert_eq!(out.resync, Some(Tick(old.0 + LONG)), "the frame's tick plus the lead");
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_prediction_world_under_anchored_regions_is_refused() {
    let executor = executor(2);
    let content = Content::new();
    let accepted = JoinAccepted { player: PlayerId(1), tick: Tick(0) };
    let entity = StableKey::Net(NetId::runtime(NonZeroU64::MIN).expect("a runtime id"));
    let cooked = Arc::clone(&content) as Arc<dyn Cooked>;
    let error = build_client(&executor, &GAME, anchored([100]), accepted, entity, LEAD, cooked)
        .err()
        .expect("an anchored prediction world is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    // The builder of `lactose-sim` refuses it too, but only after the solver pool started.
    assert!(error.to_string().contains("`Directed`"), "refused before the build: {error}");
    executor.shutdown();
}

// ---- The fold's mode: a server and a client in one process ----

const PLAYER: PlayerId = PlayerId(1);
const REGION: RegionId = RegionId(1);

/// Every replicated component `key` has in `world`, in ascending key order: what a packer
/// sends of a claimed entity.
fn replicated(world: &World, key: StableKey) -> Vec<ComponentData> {
    let mut components: Vec<ComponentData> = world
        .registry()
        .iter()
        .filter(|info| info.role() == TypeRole::Component && info.tracking().replicated)
        .filter_map(|info| {
            let bytes = world.get_erased(key, info.key())?;
            Some(ComponentData { component: info.key(), bytes: bytes.to_vec() })
        })
        .collect();
    components.sort_by_key(|data| data.component);
    components
}

/// What the joined player's entity carries besides the session applier's counter.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Body {
    None,
    /// A dynamic sphere two metres above a fixed slab whose top is at half a metre.
    Landing,
}

/// What [`joined`] measured once its frames ran.
struct Joined {
    mode: Option<RegionMode>,
    /// The tick the full frame was folded at: the join's rows, `lead` ticks stale.
    fold: u32,
    /// Every correction: the tick of the frame that made it, and the tick it restored.
    corrected: Vec<(u32, u32)>,
    resyncs: u32,
    /// The server's tick, its counter and its body's height.
    server: (u32, u32, Option<Scalar>),
    /// The same of the client.
    client: (u32, u32, Option<Scalar>),
}

/// A server that loads region 1 and joins the player, and a client that folds the join's
/// full frame (the region's relevance message and the player's entity, claimed) and then
/// runs `frames` frames, each after one server step and the claimed entity's exact rows at
/// that step, as a packer sends them. The server is given the samples the client predicted
/// with, so a prediction that runs the kernels and the physics as the server does is
/// corrected once, by the first frame of the fold's tick (the full frame carries the rows
/// of the join, which the fold places `lead` ticks later), and never after it. `frozen`
/// sets the server's region `Frozen` by a step after the join, before the full frame.
fn joined(body: Body, frozen: bool, frames: u32) -> Joined {
    let executor = executor(2);
    let content = Content::new();
    let server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let mut first = load(&content, [REGION.0]);
    first.sessions.insert(PLAYER, SessionEvent::Join { region: REGION });
    let (mut server, out) = step(&executor, server, first);
    assert!(out.failures.is_empty(), "{out:?}");
    let entity = lactose_input::player_entity(server.simulation().world(), PLAYER).expect("joined");
    if body == Body::Landing {
        let world = server.simulation_mut().world_mut();
        let up = Transform::at(Vec3::new(Scalar::ZERO, Scalar::from_i32(2), Scalar::ZERO));
        world.insert(entity, up).expect("transform");
        world.insert(entity, RigidBody::dynamic()).expect("body");
        world.insert(entity, Collider::new(Shape::sphere(Scalar::HALF))).expect("collider");
        world.insert(entity, Velocity::ZERO).expect("velocity");
        let floor = StableKey::Net(world.allocate_net_id().expect("an id"));
        let slab =
            Shape::cuboid(Vec3::new(Scalar::from_i32(10), Scalar::HALF, Scalar::from_i32(10)));
        let fixed = (Transform::at(Vec3::ZERO), Collider::new(slab));
        world.spawn(floor, REGION, fixed).expect("the floor");
    }
    if frozen {
        let mut freeze = TickInput::default();
        freeze.modes.insert(REGION, RegionMode::Frozen);
        let (stepped, out) = step(&executor, server, freeze);
        assert!(out.failures.is_empty(), "{out:?}");
        server = stepped;
        assert_eq!(server.simulation().world().region_mode(REGION), Some(RegionMode::Frozen));
    }
    let join = server.tick();

    let mut full = StagedFrame::resync(join);
    full.time_scale = Some(server.time_scale());
    full.acked_input = Some(join);
    let relevance = server.relevance(&executor, REGION).expect("relevance");
    full.regions.insert(REGION, RegionChange::Relevant(relevance));
    let StableKey::Net(net) = entity else { panic!("a server entity has a NetId") };
    let components = replicated(server.simulation().world(), entity);
    full.spawns.insert(net, StagedSpawn { origin: None, region: REGION, slot: None, components });

    let accepted = JoinAccepted { player: PLAYER, tick: join };
    let cooked = Arc::clone(&content) as Arc<dyn Cooked>;
    let mut client =
        build_client(&executor, &GAME, WorldConfig::headless(), accepted, entity, LEAD, cooked)
            .expect("the client builds");
    client.clock(estimate(join.0, true));
    push(&mut client, full);
    let clock = Clock::new();
    let (client, out) = frame(&executor, client, clock, 0, &mut sampled);
    assert_eq!(out.ticks, 0);
    let (mut client, out) = frame(&executor, client, clock, 1, &mut sampled);
    assert_eq!(out.ticks, 1);
    let (mode, fold) = (world(&client).region_mode(REGION), client.tick().0);

    let (mut corrected, mut resyncs) = (Vec::new(), 0);
    for i in 1..=frames {
        let next = Tick(server.tick().0 + 1);
        let mut input = TickInput::default();
        if next.0 > join.0 + LEAD {
            input.inputs.insert(PLAYER, sampled(next).to_player_sample(next));
        }
        let (stepped, out) = step(&executor, server, input);
        assert!(out.failures.is_empty(), "{out:?}");
        server = stepped;

        let mut delta = StagedFrame::new(server.tick());
        delta.acked_input = Some(server.tick());
        for data in replicated(server.simulation().world(), entity) {
            assert!(delta.push_row(entity, data.component, delta.tick, true, &data.bytes));
        }
        push(&mut client, delta);
        let out;
        (client, out) = frame(&executor, client, clock, 1 + i, &mut sampled);
        assert_eq!(out.ticks, 1);
        let at = server.tick().0;
        corrected.extend(out.reconciled.corrections.iter().map(|done| (at, done.tick.0)));
        resyncs += u32::from(out.resync.is_some());
    }
    let of = |world: &World, tick: Tick| {
        let counter = world.get::<Counter>(entity).map_or(u32::MAX, |counter| counter.0);
        (tick.0, counter, world.get::<Transform>(entity).map(|at| at.translation.y))
    };
    let joined = Joined {
        mode,
        fold,
        corrected,
        resyncs,
        server: of(server.simulation().world(), server.tick()),
        client: of(world(&client), client.tick()),
    };
    client.shutdown();
    server.shutdown();
    executor.shutdown();
    joined
}

#[test]
fn a_folded_region_runs_the_prediction_kernels_and_the_frames_after_correct_nothing() {
    let run = joined(Body::None, false, 60);
    assert_eq!(run.mode, Some(RegionMode::Simulated), "the fold loads the region simulated");
    let ((server_tick, server_count, _), (client_tick, client_count, _)) = (run.server, run.client);
    assert!(client_tick > server_tick);
    assert_eq!(
        client_count,
        server_count + (client_tick - server_tick),
        "the kernel advanced the claimed counter through every predicted tick"
    );
    assert_eq!(run.corrected, [(run.fold, run.fold)], "the fold's lag alone is corrected");
    assert_eq!(run.resyncs, 0);
}

#[test]
fn a_predicted_body_lands_as_the_servers_does_and_the_frames_after_correct_nothing() {
    let run = joined(Body::Landing, false, 240);
    assert_eq!(run.mode, Some(RegionMode::Simulated));
    let ((server_tick, server_count, server_y), (client_tick, client_count, client_y)) =
        (run.server, run.client);
    assert_eq!(client_count, server_count + (client_tick - server_tick));
    let landed = server_y.expect("a transform");
    assert!(landed < Scalar::from_i32(2), "the server's body fell");
    assert!(client_y == Some(landed), "the prediction's body rests where the server's does");
    assert_eq!(run.corrected, [(run.fold, run.fold)], "the fold's lag alone is corrected");
    assert_eq!(run.resyncs, 0);
}

/// Pins today's behaviour, a cost stated in the README: `RegionChange` carries no mode,
/// so a region the server holds `Frozen` with its table published is folded `Simulated`,
/// the client's kernel advances the counter the server's does not, and nearly every
/// frame's rows correct it. A mode variant is a `lactose-net-api` change; when it lands,
/// this test asserts the region folds `Frozen` and the frames correct nothing.
#[test]
fn a_region_the_server_holds_frozen_is_simulated_and_corrected_at_every_frame_today() {
    let run = joined(Body::None, true, 60);
    assert_eq!(run.mode, Some(RegionMode::Simulated));
    let ((_, server_count, _), (_, client_count, _)) = (run.server, run.client);
    assert!(client_count > server_count, "the client's kernel ran where the server's did not");
    assert!(run.corrected.len() >= 55, "a correction at nearly every frame: {:?}", run.corrected);
    assert!(run.corrected.iter().all(|&(at, restored)| at == restored));
    assert_eq!(run.resyncs, 0);
}

/// Architecture section 14: a region enters `Simulated` only once its table is published.
/// A relevance message built before its table carries none: the fold loads the region
/// `Frozen`, and the frame that brings the table as `RegionChange::Assets` makes it
/// `Simulated`.
#[test]
fn a_region_folded_without_its_table_is_frozen_until_the_table_enters() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut message = relevance(&executor, &content, &[1]).remove(0);
    let published = message.assets.take().expect("the server's region has its table");
    let mut client = client(&executor, &content, clock);
    let mut full = StagedFrame::resync(Tick(client.tick().0 + 1));
    full.regions.insert(RegionId(1), RegionChange::Relevant(message));
    push(&mut client, full);
    let (mut client, _) = frame(&executor, client, clock, 1, &mut sampled);
    assert_eq!(world(&client).region_mode(RegionId(1)), Some(RegionMode::Frozen));
    assert!(world(&client).region_assets(RegionId(1)).is_none());

    let mut later = StagedFrame::new(Tick(client.tick().0 + 1));
    later.regions.insert(RegionId(1), RegionChange::Assets(published));
    push(&mut client, later);
    let (client, _) = frame(&executor, client, clock, 2, &mut sampled);
    assert_eq!(world(&client).region_mode(RegionId(1)), Some(RegionMode::Simulated));
    assert!(world(&client).region_assets(RegionId(1)).is_some());
    client.shutdown();
    executor.shutdown();
}

/// Region 1's mode and whether its table entered, after one fold of `changes`: one frame
/// per change at ascending ticks up to the world's next, the last one `full` if `full`.
fn folded(changes: Vec<RegionChange>, full: bool) -> (Option<RegionMode>, bool) {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    let next = client.tick().0 + 1;
    let first = next + 1 - u32::try_from(changes.len()).expect("a few");
    for (at, change) in (first..=next).zip(changes) {
        let mut staged = if full && at == next {
            StagedFrame::resync(Tick(at))
        } else {
            StagedFrame::new(Tick(at))
        };
        staged.regions.insert(RegionId(1), change);
        push(&mut client, staged);
    }
    let (client, out) = frame(&executor, client, clock, 1, &mut sampled);
    assert_eq!(out.ticks, 1);
    let world = world(&client);
    let folded = (world.region_mode(RegionId(1)), world.region_assets(RegionId(1)).is_some());
    client.shutdown();
    executor.shutdown();
    folded
}

/// A table the fold drops takes the `Simulated` its `Assets` change set with it: a full
/// frame after the table, or an `Irrelevant` between the table and a `Relevant` with none,
/// leaves the region `Frozen` with no table.
#[test]
fn a_mode_the_fold_drops_with_its_table_does_not_outlive_it() {
    let executor = executor(2);
    let mut message = relevance(&executor, &Content::new(), &[1]).remove(0);
    executor.shutdown();
    let published = message.assets.take().expect("the server's region has its table");
    let relevant = || RegionChange::Relevant(message.clone());
    let assets = || RegionChange::Assets(published.clone());

    let kept = folded(vec![assets(), relevant()], false);
    assert_eq!(kept, (Some(RegionMode::Simulated), true), "the table and its mode enter");
    let full = folded(vec![assets(), relevant()], true);
    assert_eq!(full, (Some(RegionMode::Frozen), false), "a full frame drops both");
    let dropped = folded(vec![assets(), RegionChange::Irrelevant, relevant()], false);
    assert_eq!(dropped, (Some(RegionMode::Frozen), false), "so does `Irrelevant`");
}

// ---- The resync hint: the newest frame accepted ----

/// A client that receives one empty frame per period from the server's tick one on, then
/// misses `stall` periods while their frames queue, then takes them all at once and runs on
/// for twenty periods: one resync, to at or beyond the newest frame plus the lead, and the
/// lead kept after it.
fn stalled(stall: u32) {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut client = client(&executor, &content, clock);
    for periods in 1..=10 {
        push(&mut client, StagedFrame::new(Tick(periods)));
        client = frame(&executor, client, clock, periods, &mut sampled).0;
    }
    assert_eq!(client.tick(), Tick(10 + LEAD));
    let resyncs_before = client.predictor().stats().resyncs;

    let newest = 10 + stall;
    for tick in 11..=newest {
        push(&mut client, StagedFrame::new(Tick(tick)));
    }
    let mut resyncs = Vec::new();
    for periods in newest..newest + 20 {
        if periods > newest {
            push(&mut client, StagedFrame::new(Tick(periods)));
        }
        let out;
        (client, out) = frame(&executor, client, clock, periods, &mut sampled);
        if let Some(tick) = out.resync {
            resyncs.push((periods, tick));
            assert!(tick.0 >= newest + LEAD, "{tick:?} lands before a frame already queued");
        }
        assert!(client.tick().0 >= periods + LEAD, "the lead is kept at {periods}");
    }
    assert_eq!(resyncs.len(), 1, "{resyncs:?}");
    assert_eq!(client.predictor().stats().resyncs - resyncs_before, 1);
    client.shutdown();
    executor.shutdown();
}

#[test]
fn a_stall_of_twelve_frames_resyncs_once_beyond_the_newest() {
    stalled(12);
}

#[test]
fn a_stall_of_twenty_five_frames_resyncs_once_beyond_the_newest() {
    stalled(25);
}

// ---- A bundle the prediction world refuses ----

fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: lactose_reflect::bytemuck::bytes_of(&value).to_vec() }
}

/// A row of the local player's counter the prediction did not make, at `frame`'s tick.
fn mismatch(client: &Client, frame: &mut StagedFrame, value: u32) {
    let entity = client.predictor().local().entity;
    let bytes = lactose_reflect::bytemuck::bytes_of(&Counter(value)).to_vec();
    assert!(frame.push_row(entity, Counter::KEY, frame.tick, true, &bytes));
}

/// A client holding region 1 from a full frame, with the local player's claimed counter
/// in it, eight frames on; and the region's relevance message.
fn holding(executor: &Executor, content: &Arc<Content>, clock: Clock) -> (Client, RegionRelevance) {
    let (client, mut messages) = holding_all(executor, content, clock, &[1]);
    (client, messages.remove(0))
}

/// [`holding`] of regions `ids`, the counter in the first; and their relevance messages.
fn holding_all(
    executor: &Executor,
    content: &Arc<Content>,
    clock: Clock,
    ids: &[u64],
) -> (Client, Vec<RegionRelevance>) {
    let messages = relevance(executor, content, ids);
    let mut client = client(executor, content, clock);
    let StableKey::Net(net) = client.predictor().local().entity else { panic!("a NetId") };
    let mut full = StagedFrame::resync(Tick(client.tick().0 + 1));
    for message in &messages {
        full.regions.insert(message.region, RegionChange::Relevant(message.clone()));
    }
    let components = vec![data(Counter(0))];
    let region = RegionId(ids[0]);
    full.spawns.insert(net, StagedSpawn { origin: None, region, slot: None, components });
    push(&mut client, full);
    for periods in 1..=8 {
        client = frame(executor, client, clock, periods, &mut sampled).0;
    }
    assert_eq!(regions(&client), ids);
    (client, messages)
}

const REFUSED: &str = "a region bundle was refused";

/// A frame naming the loaded region `Relevant` again, older than the world, with a row the
/// prediction did not make: the bundle is refused at the step, and the reconciliation's
/// rollback re-applies the refused tick. One report and one content resync at the world's
/// own tick, which leaves the pacer as it was: the frame runs half a period late, and the
/// next, on the period, still owes its tick. Five later frames roll back five times and
/// report nothing more.
#[test]
fn a_refused_bundle_is_reported_once_and_resyncs_the_content() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let (mut client, message) = holding(&executor, &content, clock);
    let present = client.tick();

    let mut again = StagedFrame::new(Tick(present.0 - 1));
    again.regions.insert(RegionId(1), RegionChange::Relevant(message));
    mismatch(&client, &mut again, 900);
    push(&mut client, again);
    let late = clock.at(9) + TICK_PERIOD / 2;
    let (mut client, out) =
        executor.block_on(client.frame(&executor, late, &mut sampled)).expect("the frame");
    assert_eq!(out.ticks, 1);
    assert_eq!(out.next_deadline, clock.at(10));
    assert!(out.reconciled.resimulated > 1, "the rollback re-applied the refused tick");
    assert_eq!(out.resync, Some(client.tick()), "a content resync at the world's own tick");
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 1, "reported once");
    let restart = client.tick();

    for n in 0..5 {
        let mut later = StagedFrame::new(Tick(restart.0 + n));
        mismatch(&client, &mut later, 910 + 10 * n);
        push(&mut client, later);
        let out;
        (client, out) = frame(&executor, client, clock, 10 + n, &mut sampled);
        assert_eq!(out.ticks, 1, "the content resync left the pacer's debt");
        assert_eq!(out.next_deadline, clock.at(11 + n));
        assert!(out.reconciled.resimulated > 0, "a rollback");
        assert_eq!(out.resync, None);
    }
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 1, "and only once");
    assert_eq!(regions(&client), [1]);
    client.shutdown();
    executor.shutdown();
}

/// A refused bundle in a frame step 2 already resynced, its fold having failed for another
/// region: no second resync, and a later frame's rollback from the restart re-applies the
/// refused tick without a second report.
#[test]
fn a_refusal_a_later_rollback_re_applies_is_not_reported_again() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let (mut client, message) = holding(&executor, &content, clock);
    let present = client.tick();

    let mut failing = StagedFrame::new(Tick(present.0 - 1));
    failing.regions.insert(RegionId(1), RegionChange::Relevant(message));
    failing.regions.insert(RegionId(5), unresolvable(5));
    push(&mut client, failing);
    let (mut client, out) = frame(&executor, client, clock, 9, &mut sampled);
    assert_eq!(out.ticks, 1);
    assert_eq!(out.resync, Some(present), "the fold's content resync, and no other");
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 1);

    let mut at_restart = StagedFrame::new(present);
    mismatch(&client, &mut at_restart, 900);
    push(&mut client, at_restart);
    let (client, out) = frame(&executor, client, clock, 10, &mut sampled);
    assert!(out.reconciled.resimulated > 1, "the rollback re-applied the refused tick");
    assert_eq!(out.resync, None);
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 1, "reported once");
    client.shutdown();
    executor.shutdown();
}

/// Two loaded regions named `Relevant` again by one frame older than the world: both
/// bundles are refused at the same tick, and each is reported.
#[test]
fn two_regions_refused_at_one_tick_are_each_reported() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let (mut client, messages) = holding_all(&executor, &content, clock, &[1, 2]);
    let present = client.tick();

    let mut again = StagedFrame::new(Tick(present.0 - 1));
    for message in messages {
        again.regions.insert(message.region, RegionChange::Relevant(message));
    }
    push(&mut client, again);
    let (client, out) = frame(&executor, client, clock, 9, &mut sampled);
    assert_eq!(out.resync, Some(client.tick()), "one content resync");
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 2, "one report per region");
    assert_eq!(regions(&client), [1, 2]);
    client.shutdown();
    executor.shutdown();
}

/// Entity X, a runtime id that is not the local player's.
fn x_key() -> StableKey {
    StableKey::Net(NetId::runtime(NonZeroU64::new(50).expect("non-zero")).expect("runtime"))
}

/// Region 2's relevance message, its overlay holding X.
fn relevance_with_x(executor: &Executor, content: &Arc<Content>) -> RegionRelevance {
    let server = server(executor, WorldConfig::headless(), Start::Fresh, content);
    let (mut server, out) = step(executor, server, load(content, [2]));
    assert!(out.failures.is_empty(), "{out:?}");
    server.simulation_mut().world_mut().spawn(x_key(), RegionId(2), (Counter(7),)).expect("X");
    let message = server.relevance(executor, RegionId(2)).expect("relevance");
    server.shutdown();
    message
}

/// A refusal its first application never made. Frame A (older than the world) makes region
/// 2 relevant: its bundle, with X in the overlay, loads at the world's next tick T. Frame B
/// (before T) claims X for the player in region 1: the reconciliation corrects its tick by
/// spawning X there, and the re-simulation re-applies region 2's bundle at T against a
/// world where X is live, which the sync point refuses. Reported once, with a content
/// resync, though its tick is one an earlier frame ran.
#[test]
fn a_refusal_only_the_rollback_makes_is_reported_and_resyncs_the_content() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let (mut client, _) = holding(&executor, &content, clock);
    let two = relevance_with_x(&executor, &content);

    let mut a = StagedFrame::new(Tick(4));
    a.regions.insert(RegionId(2), RegionChange::Relevant(two));
    push(&mut client, a);
    let (mut client, out) = frame(&executor, client, clock, 9, &mut sampled);
    assert_eq!(regions(&client), [1, 2], "region 2 loaded at its first application");
    assert!(world(&client).contains(x_key()));
    assert_eq!((out.resync, logged(&log, tracing::Level::WARN, REFUSED)), (None, 0));

    let StableKey::Net(x) = x_key() else { panic!("a NetId") };
    let mut b = StagedFrame::new(Tick(5));
    let components = vec![data(Counter(7))];
    b.spawns.insert(x, StagedSpawn { origin: None, region: RegionId(1), slot: None, components });
    push(&mut client, b);
    let (client, out) = frame(&executor, client, clock, 10, &mut sampled);
    assert!(out.reconciled.resimulated > 1, "the rollback re-applied region 2's bundle");
    assert_eq!(regions(&client), [1], "the re-application was refused");
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 1, "and reported");
    assert_eq!(out.resync, Some(client.tick()), "with a content resync");
    client.shutdown();
    executor.shutdown();
}

/// The kept refusals go with the ring: a refusal in a frame whose fold failed is kept, a
/// clock resync takes the world back before its tick, and the same region refused at the
/// same tick again is a new refusal, reported and resynced.
#[test]
fn a_refusal_after_a_backward_resync_is_reported_again() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let (mut client, message) = holding(&executor, &content, clock);
    let present = client.tick();

    let mut failing = StagedFrame::new(Tick(present.0 - 1));
    failing.regions.insert(RegionId(1), RegionChange::Relevant(message.clone()));
    failing.regions.insert(RegionId(5), unresolvable(5));
    push(&mut client, failing);
    let (mut client, out) = frame(&executor, client, clock, 9, &mut sampled);
    assert_eq!((out.ticks, out.resync), (1, Some(present)));
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 1);

    client.clock(estimate(present.0 - LEAD, true));
    let (mut client, out) = frame(&executor, client, clock, 10, &mut sampled);
    assert_eq!((out.ticks, out.resync), (0, Some(present)), "back before the refused tick");

    let mut again = StagedFrame::new(present);
    again.regions.insert(RegionId(1), RegionChange::Relevant(message));
    push(&mut client, again);
    let (client, out) = frame(&executor, client, clock, 11, &mut sampled);
    assert_eq!(out.ticks, 1);
    assert_eq!(logged(&log, tracing::Level::WARN, REFUSED), 2, "reported again");
    assert_eq!(out.resync, Some(client.tick()));
    client.shutdown();
    executor.shutdown();
}
