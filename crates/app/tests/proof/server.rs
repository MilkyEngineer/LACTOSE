//! "The frame", "Time scale, server", "Refusals", "Relevance" and the policy's `Stop`
//! reaching a run's `Result`: the server's timer root over a stand-in clock.

use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use lactose_app::{Cooked as _, FrameOutput, Server, Start, WorldConfig};
use lactose_asset_api::{AssetFailure, AssetId, PinnedTable};
use lactose_core::{ErrorKind, PlayerId, Tick};
use lactose_ecs::{Executor, RegionId, RegionMode};
use lactose_sim::{SessionEvent, TickInput, TimeScale};

use crate::common::{
    Clock, Content, anchored, executor, frame, load, mirror, passes, scales, server, session, step,
    table_of,
};

/// A server of the proof's game, its pacer resynced at the clock's base, so a frame at
/// `clock.at(n)` owes exactly `n` periods at the scale it paces by.
fn timed(executor: &Executor, content: &std::sync::Arc<Content>, clock: Clock) -> Server {
    let mut server = server(executor, WorldConfig::headless(), Start::Fresh, content);
    server.resync(clock.at(0));
    server
}

/// Frames at a fixed spacing, every tick's input the default.
struct Frames {
    clock: Clock,
    spacing: u32,
    at: u32,
}

impl Frames {
    fn next(&mut self, executor: &Executor, server: Server) -> (Server, FrameOutput) {
        self.with(executor, server, &mut |_| Some(TickInput::default()))
    }

    fn with(
        &mut self,
        executor: &Executor,
        server: Server,
        frames: &mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
    ) -> (Server, FrameOutput) {
        self.at += self.spacing;
        frame(executor, server, self.clock.at(self.at), frames)
    }
}

#[test]
fn a_frame_steps_consecutive_ticks_and_none_ends_it_before_its_tick() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let server = timed(&executor, &content, clock);
    let asked = Mutex::new(Vec::new());
    let stop_at = 5;
    let mut frames = |tick: Tick| {
        asked.lock().unwrap_or_else(PoisonError::into_inner).push(tick.0);
        (tick.0 != stop_at).then(TickInput::default)
    };
    let before = passes();
    let (server, out) = frame(&executor, server, clock.at(1), &mut frames);
    assert_eq!((out.ticks, out.behind), (1, false));
    let (server, out) = frame(&executor, server, clock.at(3), &mut frames);
    assert_eq!((out.ticks, out.behind), (2, false));
    // Three owed; the callback ends the frame before tick 5, which does not run.
    let (server, out) = frame(&executor, server, clock.at(6), &mut frames);
    assert_eq!(out.ticks, 1);
    assert_eq!(server.tick(), Tick(4));
    assert_eq!(passes() - before, 3, "the passes run once per frame, the cut one included");
    // What the cut frame forgot is not caught up.
    let (server, out) = frame(&executor, server, clock.at(7), &mut frames);
    assert_eq!(out.ticks, 0, "tick 5 is asked for again and refused again");
    assert_eq!(server.tick(), Tick(4));
    let asked = std::mem::take(&mut *asked.lock().unwrap_or_else(PoisonError::into_inner));
    assert_eq!(asked, [1, 2, 3, 4, 5, 5]);
    server.shutdown();
    executor.shutdown();
}

#[test]
fn a_frame_owing_more_than_the_cap_is_behind() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let server = timed(&executor, &content, clock);
    let mut frames = Frames { clock, spacing: 6, at: 0 };
    let (server, out) = frames.next(&executor, server);
    assert_eq!((out.ticks, out.behind), (lactose_app::CATCH_UP_CAP, true));
    assert_eq!(out.next_deadline, clock.at(6), "a frame behind is due again at once");
    assert_eq!(server.tick(), Tick(lactose_app::CATCH_UP_CAP));
    server.shutdown();
    executor.shutdown();
}

#[test]
fn a_poll_at_the_build_instant_owes_one_tick_and_a_resync_forgets_a_gap() {
    let executor = executor(2);
    let content = Content::new();
    let before = Instant::now();
    let server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let mut each = |_: Tick| Some(TickInput::default());
    // Before the build instant nothing is owed, and the first tick is due at it.
    let (server, out) = frame(&executor, server, before, &mut each);
    assert_eq!(out.ticks, 0);
    assert!(out.next_deadline >= before);
    let built = out.next_deadline;
    let (mut server, out) = frame(&executor, server, built, &mut each);
    assert_eq!(out.ticks, 1, "a poll at the build instant owes one tick");
    // A gap of a hundred periods, then a resync: the run starts with nothing owed.
    let clock = Clock::new();
    server.resync(clock.at(100));
    let (server, out) = frame(&executor, server, clock.at(100), &mut each);
    assert_eq!((out.ticks, out.behind), (0, false));
    let (server, out) = frame(&executor, server, clock.at(101), &mut each);
    assert_eq!((out.ticks, out.behind), (1, false));
    server.shutdown();
    executor.shutdown();
}

/// The time scale's server rows at one frame spacing (in periods) and one running scale.
fn pause_and_unpause(spacing: u32, scale: TimeScale) {
    let per = spacing * scale.thousandths / 1000;
    assert!(per >= 1);
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut server = timed(&executor, &content, clock);
    let mut frames = Frames { clock, spacing, at: 0 };
    if scale != TimeScale::REAL_TIME {
        server.set_time_scale(scale);
        assert_eq!(server.time_scale(), scale, "a nonzero scale holds from the call");
    }

    // Running.
    for _ in 0..3 {
        let out;
        (server, out) = frames.next(&executor, server);
        assert_eq!(out.ticks, per);
        assert_eq!(server.time_scale(), mirror(&server));
    }
    assert!(scales().iter().all(|&s| s == scale.thousandths));

    // A pause: one tick carries it, then no tick runs, and the passes still do.
    server.set_time_scale(TimeScale::PAUSED);
    assert_eq!(server.time_scale(), scale, "a zero holds only after the step that carries it");
    assert_eq!(mirror(&server), scale);
    let out;
    (server, out) = frames.next(&executor, server);
    assert_eq!(out.ticks, 1, "the pause ends its frame");
    assert_eq!(scales(), [0], "the one tick carried the pause");
    assert_eq!(mirror(&server), TimeScale::PAUSED);
    assert_eq!(server.time_scale(), TimeScale::PAUSED);
    let before = passes();
    for _ in 0..3 {
        let out;
        (server, out) = frames.next(&executor, server);
        assert_eq!(out.ticks, 0, "a paused server runs no tick");
    }
    assert_eq!(passes() - before, 3);
    let paused_at = server.tick();

    // An unpause: the scale holds at once, ticks are due at the next frame and the first
    // carries it.
    server.set_time_scale(scale);
    assert_eq!(server.time_scale(), scale);
    assert_eq!(mirror(&server), TimeScale::PAUSED);
    let out;
    (server, out) = frames.next(&executor, server);
    assert_eq!(out.ticks, per);
    assert_eq!(scales(), vec![scale.thousandths; per as usize]);
    assert_eq!(server.tick(), Tick(paused_at.0 + per));
    assert_eq!(server.time_scale(), mirror(&server));

    // The callback's own pause in the first of the frame's due ticks ends the frame there.
    let mut first = true;
    let mut own = |_: Tick| {
        let scale = std::mem::take(&mut first).then_some(TimeScale::PAUSED);
        Some(TickInput { time_scale: scale, ..TickInput::default() })
    };
    let out;
    (server, out) = frames.with(&executor, server, &mut own);
    assert_eq!(out.ticks, 1);
    assert_eq!(scales(), [0]);
    assert_eq!(server.time_scale(), TimeScale::PAUSED);
    assert_eq!(mirror(&server), TimeScale::PAUSED);
    let out;
    (server, out) = frames.next(&executor, server);
    assert_eq!(out.ticks, 0);

    // On a paused server, an unpause taken back in one gap runs no tick.
    server.set_time_scale(scale);
    server.set_time_scale(TimeScale::PAUSED);
    assert_eq!(server.time_scale(), TimeScale::PAUSED, "the cancelled unpause");
    let out;
    (server, out) = frames.next(&executor, server);
    assert_eq!(out.ticks, 0);
    assert_eq!(scales(), []);
    assert_eq!((server.time_scale(), mirror(&server)), (TimeScale::PAUSED, TimeScale::PAUSED));

    // On a running server, a pause taken back in one gap runs the frame's ticks.
    server.set_time_scale(scale);
    let out;
    (server, out) = frames.next(&executor, server);
    assert_eq!(out.ticks, per);
    let _ = scales();
    server.set_time_scale(TimeScale::PAUSED);
    server.set_time_scale(scale);
    let out;
    (server, out) = frames.next(&executor, server);
    assert_eq!(out.ticks, per);
    assert_eq!(scales(), vec![scale.thousandths; per as usize]);
    assert_eq!((server.time_scale(), mirror(&server)), (scale, scale));
    server.shutdown();
    executor.shutdown();
}

#[test]
fn time_scale_at_60_hz_frames() {
    pause_and_unpause(1, TimeScale::REAL_TIME);
}

#[test]
fn time_scale_at_30_hz_frames() {
    pause_and_unpause(2, TimeScale::REAL_TIME);
}

#[test]
fn time_scale_at_twice_real_time() {
    pause_and_unpause(1, TimeScale { thousandths: 2000 });
}

#[test]
fn time_scale_through_step() {
    let executor = executor(2);
    let content = Content::new();
    let mut server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    server.set_time_scale(TimeScale::PAUSED);
    assert_eq!(server.time_scale(), TimeScale::REAL_TIME);
    let (mut server, _) = step(&executor, server, TickInput::default());
    assert_eq!((server.time_scale(), mirror(&server)), (TimeScale::PAUSED, TimeScale::PAUSED));
    server.set_time_scale(TimeScale::REAL_TIME);
    assert_eq!((server.time_scale(), mirror(&server)), (TimeScale::REAL_TIME, TimeScale::PAUSED));
    let (server, _) = step(&executor, server, TickInput::default());
    assert_eq!(mirror(&server), TimeScale::REAL_TIME);
    assert_eq!(scales(), [0, 1000]);
    server.shutdown();
    executor.shutdown();
}

#[test]
fn a_repeated_scale_is_dropped() {
    let executor = executor(2);
    let content = Content::new();
    let mut server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    // Dropped: nothing replaces the frame's own scale, so the frame's pause is carried.
    server.set_time_scale(TimeScale::REAL_TIME);
    let paused = TickInput { time_scale: Some(TimeScale::PAUSED), ..TickInput::default() };
    let (server, _) = step(&executor, server, paused);
    assert_eq!(mirror(&server), TimeScale::PAUSED);
    assert_eq!(server.time_scale(), TimeScale::PAUSED);
    server.shutdown();
    executor.shutdown();
}

#[test]
fn fifty_pause_cycles_drift_by_at_most_one_tick_per_pause() {
    const CYCLES: u32 = 50;
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let mut server = timed(&executor, &content, clock);
    // 30 Hz frames, so a pause frame owes two ticks and runs one.
    let mut frames = Frames { clock, spacing: 2, at: 0 };
    let mut running_frames = 0;
    for _ in 0..CYCLES {
        server.set_time_scale(TimeScale::PAUSED);
        let out;
        (server, out) = frames.next(&executor, server);
        assert_eq!(out.ticks, 1);
        running_frames += 1;
        for _ in 0..2 {
            let out;
            (server, out) = frames.next(&executor, server);
            assert_eq!(out.ticks, 0);
        }
        server.set_time_scale(TimeScale::REAL_TIME);
        for _ in 0..3 {
            let out;
            (server, out) = frames.next(&executor, server);
            assert_eq!(out.ticks, 2);
            running_frames += 1;
        }
    }
    let scheduled = running_frames * frames.spacing;
    let ran = server.tick().0;
    assert!(ran <= scheduled && scheduled - ran <= CYCLES, "{ran} of {scheduled}");
    server.shutdown();
    executor.shutdown();
}

#[test]
fn refusals_reach_the_step_and_the_frame_output() {
    let executor = executor(2);
    let content = Content::new();
    let clock = Clock::new();
    let server = timed(&executor, &content, clock);
    let (server, out) = step(&executor, server, load(&content, [1]));
    assert!(out.failures.is_empty());
    // A bundle for a loaded region.
    let (server, out) = step(&executor, server, load(&content, [1]));
    assert_eq!(out.failures.loads.len(), 1);
    assert_eq!(out.failures.loads[0].region, RegionId(1));
    assert!(!out.failures.is_empty());
    // An asset failure the frame carries.
    let failure =
        AssetFailure { id: AssetId::from_bits(1), version: None, kind: ErrorKind::NotFound };
    let carried = TickInput { failures: vec![failure], ..TickInput::default() };
    let (server, out) = step(&executor, server, carried);
    assert_eq!(out.failures.assets.len(), 1);
    assert_eq!(out.failures.assets[0].failure, failure);
    // The same through a frame.
    let mut first = true;
    let mut again = |_: Tick| {
        let frame =
            if std::mem::take(&mut first) { load(&content, [1]) } else { TickInput::default() };
        Some(frame)
    };
    let mut server = server;
    server.resync(clock.at(10));
    let (server, out) = frame(&executor, server, clock.at(12), &mut again);
    assert_eq!(out.ticks, 2);
    assert_eq!(out.failures.loads.len(), 1);
    assert!(out.failures.sessions.is_empty() && out.failures.assets.is_empty());
    server.shutdown();
    executor.shutdown();
}

#[test]
fn a_join_the_sync_point_refuses_is_a_session_failure() {
    let executor = executor(2);
    let content = Content::new();
    let server = server(&executor, anchored([]), Start::Fresh, &content);
    let (server, out) =
        step(&executor, server, session(7, SessionEvent::Join { region: RegionId(999) }));
    assert_eq!(out.failures.sessions.len(), 1);
    assert_eq!(out.failures.sessions[0].player, PlayerId(7));
    assert_eq!(out.failures.sessions[0].error.kind(), ErrorKind::NotFound);
    let (server, out) = step(&executor, server, TickInput::default());
    assert!(out.failures.is_empty());
    server.shutdown();
    executor.shutdown();
}

#[test]
fn relevance_carries_the_worlds_table_once_one_entered() {
    let executor = executor(2);
    let content = Content::new();
    let server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let mut frozen = TickInput::default();
    let mut bundle = content.bundle(RegionId(1), RegionMode::Frozen);
    bundle.assets = None;
    frozen.loaded.insert(RegionId(1), bundle);
    let (server, out) = step(&executor, server, frozen);
    assert!(out.failures.is_empty(), "{out:?}");
    let relevance = server.relevance(&executor, RegionId(1)).expect("relevance");
    assert_eq!(relevance.region, RegionId(1));
    assert_eq!(relevance.assets, None);
    let mut table = TickInput::default();
    table.assets.insert(RegionId(1), pinned(&content));
    let (server, _) = step(&executor, server, table);
    let relevance = server.relevance(&executor, RegionId(1)).expect("relevance");
    assert_eq!(relevance.assets, Some(table_of(RegionId(1))));
    assert_eq!(
        server.relevance(&executor, RegionId(2)).err().map(|error| error.kind()),
        Some(ErrorKind::NotFound),
        "a region that is not loaded"
    );
    server.shutdown();
    executor.shutdown();
}

/// Region 1's table, pinned.
fn pinned(content: &Content) -> PinnedTable {
    content.pin(&table_of(RegionId(1))).expect("published")
}
