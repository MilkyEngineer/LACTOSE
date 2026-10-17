//! What the predictor logs at warn: an entry the world refuses warns once, in the
//! predicted tick that popped its frame, and not again in the replays of that tick.
//! Alone in its binary, since it installs the process's tracing subscriber.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::Tick;
use lactose_ecs::RegionId;
use lactose_net_api::StagedFrame;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

/// The message of every warn event, in order.
static WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

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
        *metadata.level() == Level::WARN && metadata.target() == "predict"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        WARNED.lock().expect("the warnings").push(message.0);
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

fn warned() -> Vec<String> {
    WARNED.lock().expect("the warnings").clone()
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
