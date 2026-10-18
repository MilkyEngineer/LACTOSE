//! A taker skipped for a resource an earlier exclusive system removed (review probe r7a):
//! the skip is logged at `error` at target `casein`, naming the channel it did not take,
//! and the run goes on. Alone in its binary, since it installs the process's tracing
//! subscriber.

#![forbid(unsafe_code)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use lactose_ecs::{
    ChannelTake, ChannelWriter, LayoutConfig, ResMut, Schedule, StableKeyOf, World, WorldKind,
    system,
};
use lactose_math::Vec2;
use lactose_reflect::resource;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

/// What the taker took.
#[resource(key = "skip::taken", persistent)]
pub struct Taken(#[field(id = 1)] pub u32);

/// What the taker names and the remover removes.
#[resource(key = "skip::gate", persistent)]
pub struct Gate(#[field(id = 1)] pub u32);

/// Every `error` event at target `casein`, its fields as `name=value`.
static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Errors;

struct Fields(Vec<String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }
}

impl Subscriber for Errors {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() == Level::ERROR && metadata.target() == "casein"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(Vec::new());
        event.record(&mut fields);
        LOGGED.lock().expect("the log").push(fields.0.join(" "));
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[system(name = "skip::writer", worlds = [server])]
fn writer(key: StableKeyOf, _bomb: &Bomb, physics: ChannelWriter<Physics>) {
    physics.apply_impulse(*key, Vec2::ZERO);
}

#[system(name = "skip::remover", worlds = [server], exclusive)]
fn remover(world: &mut World) {
    world.remove_resource::<Gate>();
}

/// Names `Gate`, which `remover` took out of the world before it runs.
#[system(name = "skip::gated_taker", worlds = [server], exclusive, after = [remover])]
fn gated_taker(
    _world: &mut World,
    _gate: ResMut<Gate>,
    commands: ChannelTake<Physics>,
    taken: ResMut<Taken>,
) {
    taken.0 += commands.len() as u32;
}

/// Puts `Gate` back, so the next tick prepares.
#[system(name = "skip::restorer", worlds = [server], exclusive, after = [gated_taker])]
fn restorer(world: &mut World) {
    world.insert_resource(Gate(0));
}

#[test]
fn a_skipped_taker_is_logged_with_the_channel_it_did_not_take() {
    tracing::subscriber::set_global_default(Errors).expect("the only subscriber");
    let executor = executor();
    let mut registry = base_registry();
    registry.register::<Taken>().unwrap();
    registry.register::<Gate>().unwrap();
    let mut world = world_with(WorldKind::Server, LayoutConfig::default(), Arc::new(registry));
    world.spawn(net(1), REGION, (Bomb { fuse: 180 },)).unwrap();
    world.insert_resource(Taken(0));
    world.insert_resource(Gate(0));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("simulation", &[]).add(&WRITER);
    builder.stage("physics", &[]).add(&REMOVER).add(&GATED_TAKER).add(&RESTORER);
    let mut schedule = builder.build().unwrap();

    // Two ticks: the second shows the entry the skipped taker left does not survive the
    // run that wrote it.
    for _ in 0..2 {
        world = tick(&mut schedule, world, &executor);
    }
    assert_eq!(world.resource::<Taken>().unwrap().0, 0, "the taker never ran");
    let logged = LOGGED.lock().expect("the log").clone();
    assert_eq!(logged.len(), 2, "{logged:?}");
    let channel = core::any::type_name::<Physics>();
    for line in &logged {
        assert!(line.contains("system=skip::gated_taker"), "{line}");
        assert!(line.contains(&format!("untaken_channels=[\"{channel}\"]")), "{line}");
    }
}
