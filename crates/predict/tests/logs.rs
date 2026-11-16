//! What the predictor logs at warn: an entry the world refuses, a spawn's component that is
//! not a replicated one and a statement's key that is not a transient unreplicated component
//! each warn once, in the predicted tick that popped its frame, and not again in the replays
//! of that tick, which log a skipped component at debug. Each test installs the process's
//! tracing subscriber, so each needs a process of its own, which nextest gives it; the file is
//! a binary of its own so that no other test shares one.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::Tick;
use lactose_ecs::{LayoutConfig, RegionId};
use lactose_input::PlayerInput;
use lactose_net_api::StagedFrame;
use lactose_reflect::Reflect;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

/// The level and message of every warn and debug event, in order.
static LOGGED: Mutex<Vec<(Level, String)>> = Mutex::new(Vec::new());

struct Warnings;

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for Warnings {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        matches!(*metadata.level(), Level::WARN | Level::DEBUG) && metadata.target() == "predict"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        LOGGED.lock().expect("the log").push((*event.metadata().level(), message.0));
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

/// Every message logged at `level`, in order.
fn logged(level: Level) -> Vec<String> {
    let logged = LOGGED.lock().expect("the log");
    logged.iter().filter(|(at, _)| *at == level).map(|(_, message)| message.clone()).collect()
}

fn warned() -> Vec<String> {
    logged(Level::WARN)
}

/// Every message logged at debug for a skipped component.
fn skipped_in_replays() -> Vec<String> {
    logged(Level::DEBUG).into_iter().filter(|message| message.contains("skipped")).collect()
}

#[test]
fn a_refused_proxy_move_warns_once_however_often_its_tick_is_replayed() {
    tracing::subscriber::set_global_default(Warnings).expect("the only subscriber");
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    // The proxy moved for 17 into a region no step loads, popped at 21.
    let mut frame = StagedFrame::new(Tick(17));
    frame.moves.insert(proxy(), RegionId(5));
    rig.push(frame);
    rig.step(walking(0));
    let _ = rig.reconcile();
    assert_eq!(warned().len(), 1, "{:?}", warned());
    // Three rollbacks, from 18, 19 and 20, each replaying 21.
    for (tick, x) in [(18, 3000), (19, -3000), (20, 3000)] {
        let mut frame = StagedFrame::new(Tick(tick));
        exact(&mut frame, me(), &at(x, 0));
        rig.push(frame);
        rig.step(walking(0));
        let reconciled = rig.reconcile();
        assert_eq!(summaries(&reconciled).first().map(|s| s.0), Some(Tick(tick)));
    }
    let warned = warned();
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].contains("a move refused"), "{warned:?}");
}

/// A full frame's claimed spawn and a delta frame's proxy spawn carrying a derived and an
/// unreplicated transient component as values (`tests/hook.rs` holds that the row has
/// neither): each skipped component warns once in the predicted tick, and a rollback that
/// replays the delta frame's tick logs them at debug.
#[test]
fn a_spawn_writes_replicated_components_alone() {
    tracing::subscriber::set_global_default(Warnings).expect("the only subscriber");
    let extra = || [data(Trail { x: 999, updates: 77 }), data(Stance(5))];
    let mut rig = Rig::new(simulation(LayoutConfig::default(), None), 1);
    let mut frame = join_frame(JOIN);
    let cart_spawn = frame.spawns.get_mut(&net(2)).expect("the cart");
    cart_spawn.components.extend(extra());
    cart_spawn.components.sort_by_key(|data| data.component);
    rig.push(frame);
    rig.step(PlayerInput::IDLE);
    rig.reconcile();
    assert_eq!(warned().len(), 2, "{:?}", warned());
    // A delta frame for 14, popped at 19.
    rig.step_to(18, walking);
    let mut frame = StagedFrame::new(Tick(14));
    let mut components = vec![data(at(1, 1)), data(Health(2))];
    components.extend(extra());
    // `remote` sorts them.
    frame.spawns.insert(net(5), remote(REGION, 3, components));
    rig.push(frame);
    rig.step(walking(19));
    assert!(rig.reconcile().corrections.is_empty());
    let warned = warned();
    assert_eq!(warned.len(), 4, "{warned:?}");
    assert!(warned.iter().all(|message| message.contains("not a registered replicated")));
    assert_eq!(skipped_in_replays(), Vec::<String>::new());

    // A correction at 15 replays 19, whose hook popped the delta frame.
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, me(), &at(3000, 0));
    rig.push(frame);
    rig.step(walking(20));
    assert_eq!(summaries(&rig.reconcile())[0].0, Tick(15));
    assert_eq!(self::warned().len(), 4);
    assert_eq!(skipped_in_replays().len(), 2, "{:?}", skipped_in_replays());
}

/// A statement of the claimed cart naming a persistent and a replicated component beside a
/// transient unreplicated one: each of the two warns once, and the stated one is applied
/// (`tests/corrections.rs` holds that the other two are not).
#[test]
fn a_statement_key_that_is_not_a_transient_unreplicated_component_warns() {
    tracing::subscriber::set_global_default(Warnings).expect("the only subscriber");
    let mut rig = Rig::joined();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(16));
    exact(&mut frame, cart(), &at(50, 0));
    exact(&mut frame, cart(), &Health(3));
    frame.transient.insert(cart(), [Health::KEY, Aim::KEY, Stance::KEY].into());
    rig.push(frame);
    rig.step(walking(21));
    assert_eq!(rig.reconcile().corrections.len(), 1);
    assert!(rig.world().get::<Stance>(cart()).is_some());
    let warned = warned();
    assert_eq!(warned.len(), 2, "{warned:?}");
    assert!(warned.iter().all(|message| message.contains("a statement names a key")));
}
