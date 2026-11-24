//! The sink and the frame agree (`README.md`, "Proof"; present-api's proof row "A rollback in
//! a frame with no step", D157): one claimed row in a prediction world, moved one unit a
//! tick, four frames a tick, through every sequence of present-api's `tests/rollback.rs`.
//! Each frame runs the passes into the renderer's sink, publishes and acquires, as `app`
//! does; then `prepare`, and the frame's batch scattered on the host by
//! `InstanceRecord::scatter` over what the region holds. The record's previous and current
//! placements are `PresentFrame::transform`'s pair, and equal exactly where the frame's
//! `no_interpolate` is set.

use std::collections::BTreeMap;

use lactose_core::Tick;
use lactose_ecs::{World, WorldKind, WorldSnapshot};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::Teleported;
use lactose_present_api::{Light, RenderTransform};
use lactose_renderer::{InstanceRecord, Renderer, RendererDesc};

use crate::common::{
    Ender, PLAYER, REGION, RegionLog, Rig, at, key, null, ok, open_waiting, place, scatter,
    simulation,
};

/// What a frame of a sequence does before its passes.
#[derive(Clone, Copy, Debug)]
enum Event {
    /// This many ticks, the last one a teleport to the position if `Some`.
    Step(u32, Option<f64>),
    /// The current tick corrected to the position, with `Teleported` at the tick.
    Rollback(f64, u32),
    /// The world back at the earlier tick, at the position.
    Resync(u32, f64),
    /// This many ticks in which the row does not move.
    Hold(u32),
}

use Event::{Hold, Resync, Rollback, Step};

struct Sequence {
    name: &'static str,
    events: &'static [(u32, Event)],
    frames: u32,
}

/// present-api's sequences, D157's row; a frame a multiple of four with no event of its own
/// steps once.
const SEQUENCES: &[Sequence] = &[
    Sequence { name: "S2", events: &[(5, Rollback(1.1, 0))], frames: 13 },
    Sequence { name: "S2B", events: &[(5, Rollback(1.1, 1))], frames: 13 },
    Sequence { name: "T", events: &[(4, Step(1, Some(10.0))), (5, Rollback(1.0, 0))], frames: 13 },
    Sequence {
        name: "T2",
        events: &[(20, Step(1, Some(50.0))), (21, Rollback(5.0, 2))],
        frames: 29,
    },
    Sequence { name: "R2", events: &[(5, Rollback(1.1, 1)), (6, Rollback(1.2, 0))], frames: 13 },
    Sequence { name: "R2b", events: &[(5, Rollback(1.1, 0)), (6, Rollback(1.2, 0))], frames: 13 },
    Sequence { name: "CU", events: &[(4, Step(3, None)), (5, Rollback(3.1, 0))], frames: 13 },
    Sequence {
        name: "CUT",
        events: &[(4, Step(3, Some(20.0))), (5, Rollback(20.1, 3))],
        frames: 13,
    },
    Sequence { name: "F", events: &[(1, Rollback(0.1, 0)), (2, Rollback(0.2, 0))], frames: 9 },
    Sequence { name: "RS", events: &[(9, Resync(1, 1.05))], frames: 17 },
    Sequence { name: "RSR", events: &[(9, Resync(1, 1.05)), (10, Rollback(1.15, 0))], frames: 17 },
    Sequence { name: "LZ", events: &[(9, Resync(1, 2.0))], frames: 17 },
    Sequence { name: "RS2", events: &[(17, Resync(3, 3.05)), (21, Resync(1, 1.05))], frames: 33 },
    Sequence {
        name: "RSS",
        events: &[(12, Hold(1)), (16, Hold(1)), (17, Resync(3, 2.5))],
        frames: 25,
    },
    Sequence {
        name: "RSS3",
        events: &[(12, Hold(1)), (16, Hold(1)), (20, Hold(1)), (21, Resync(4, 2.5))],
        frames: 29,
    },
    Sequence {
        name: "RSSR",
        events: &[(12, Hold(1)), (16, Hold(1)), (17, Resync(3, 2.5)), (18, Rollback(2.6, 0))],
        frames: 25,
    },
];

/// Sets a row's `Teleported`, inserting it if the row has none.
fn teleport(world: &mut World, n: u64, tick: Tick) {
    match world.get_mut::<Teleported>(key(n)) {
        Some(mut teleported) => teleported.0 = tick,
        None => world.insert(key(n), Teleported(tick)).expect("a live row"),
    }
}

/// Runs `sequence` and returns every frame at which the record and the frame disagree, and
/// how many frames drew a movement.
fn run(sequence: &Sequence) -> (Vec<String>, u32) {
    const ROW: u64 = 1;
    let desc = RendererDesc { instances: 4, rows: 64, regions: 2, shaders: None };
    let mut renderer = ok(Renderer::new(null(), &desc));
    let mut ender = Ender::new(&**renderer.device());
    let log = RegionLog::default();
    let mut rig = Rig::new(simulation(WorldKind::Prediction), Box::new(log.sink(renderer.sink())));
    // A light, so that the snapshot presents the row.
    let light = Light::point(Vec3::ONE, Scalar::ONE, Scalar::ONE);
    let claim = lactose_ecs::ClaimedBy { player: PLAYER };
    let mesh = lactose_present_api::MeshInstance::new(lactose_asset_api::AssetRef::new(
        lactose_asset_api::AssetId::from_bits(ROW),
    ));
    let spawned = (at(0.0), mesh, Teleported(Tick(0)), light, claim);
    rig.world_mut().spawn(key(ROW), REGION, spawned).expect("a spawn");

    let mut records = Vec::new();
    let mut position: BTreeMap<u32, f64> = BTreeMap::from([(0, 0.0)]);
    let mut history: BTreeMap<u32, WorldSnapshot> = BTreeMap::from([(0, rig.world().snapshot())]);
    let mut failures = Vec::new();
    let mut moving = 0;
    for frame in 0..sequence.frames {
        let scripted =
            sequence.events.iter().find(|&&(at, _)| at == frame).map(|&(_, event)| event);
        let event = scripted.or((frame > 0 && frame.is_multiple_of(4)).then_some(Step(1, None)));
        let mut tick = rig.tick().0;
        match event {
            Some(Step(ticks, teleport_to)) => {
                for n in 1..=ticks {
                    tick += 1;
                    let mut x = position[&(tick - 1)] + 1.0;
                    if let Some(to) = teleport_to.filter(|_| n == ticks) {
                        x = to;
                        teleport(rig.world_mut(), ROW, Tick(tick));
                    }
                    position.insert(tick, x);
                    place(rig.world_mut(), ROW, x);
                    rig.step();
                    history.insert(tick, rig.world().snapshot());
                }
            }
            Some(Rollback(x, teleported)) => {
                rig.world_mut().restore(&history[&tick]).expect("a restore");
                position.insert(tick, x);
                place(rig.world_mut(), ROW, x);
                teleport(rig.world_mut(), ROW, Tick(teleported));
                history.insert(tick, rig.world().snapshot());
            }
            Some(Hold(ticks)) => {
                for _ in 1..=ticks {
                    tick += 1;
                    let x = position[&(tick - 1)];
                    position.insert(tick, x);
                    rig.step();
                    history.insert(tick, rig.world().snapshot());
                }
            }
            Some(Resync(to, x)) => {
                rig.world_mut().restore(&history[&to]).expect("a restore");
                history.retain(|&at, _| at <= to);
                position.retain(|&at, _| at <= to);
                position.insert(to, x);
                place(rig.world_mut(), ROW, x);
                history.insert(to, rig.world().snapshot());
            }
            None => {}
        }
        rig.frame();
        let stats = ok(renderer.prepare());
        assert!(!stats.any_lost(), "{}: frame {frame} lost a kind", sequence.name);
        scatter(&mut records, renderer.batch(), &log.take());

        let tick = rig.tick().0;
        let acquired = rig.reader.acquire().expect("published");
        let pair = acquired.transform(key(ROW)).expect("a presented row with a transform");
        let current = RenderTransform::from_sim(&pair.current);
        let shown = if pair.no_interpolate {
            (current, current)
        } else {
            (RenderTransform::from_sim(&pair.previous), current)
        };
        moving += u32::from(shown.0 != shown.1);
        let slot = renderer.slots().get(WorldKind::Prediction, key(ROW)).expect("held");
        let record = records[slot.index() as usize];
        if record.flags != InstanceRecord::LIVE || (record.previous, record.current) != shown {
            failures.push(format!(
                "frame {frame} (tick {tick}): the record holds ({:?}, {:?}), the frame ({:?}, {:?})",
                record.previous.position,
                record.current.position,
                shown.0.position,
                shown.1.position
            ));
        }
        ender.end(&mut renderer);
        open_waiting(&mut renderer);
    }
    (failures, moving)
}

#[test]
fn the_records_and_the_frame_agree_in_every_sequence() {
    for sequence in SEQUENCES {
        let (failures, moving) = run(sequence);
        assert!(failures.is_empty(), "{}:\n{}", sequence.name, failures.join("\n"));
        // Every sequence draws movements, so the agreement is not of still placements alone.
        assert!(moving >= 4, "{}: {moving} frames moved", sequence.name);
    }
}
