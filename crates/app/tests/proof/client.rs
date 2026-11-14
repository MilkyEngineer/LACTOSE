//! "Time scale, client" and "The client's frame": the prediction world's half of the timer
//! root over a stand-in clock. Every client starts with a `snap` estimate at the server's
//! tick zero, whose resync in the first frame puts the pacer at the clock's base.

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, PoisonError};

use lactose_app::{Client, ClientOutput, Cooked, Start, WorldConfig, build_client};
use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Executor, RegionId, World};
use lactose_input::{PlayerInput, Yaw};
use lactose_net_api::{OverlayPayload, RegionChange, RegionRelevance, StagedFrame, StagedInbox};
use lactose_net_host::ClockEstimate;
use lactose_net_host::message::JoinAccepted;
use lactose_sim::{StagedState, TimeScale};

use crate::common::{Clock, Content, GAME, capture, executor, logged, server, step, table_of};

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
