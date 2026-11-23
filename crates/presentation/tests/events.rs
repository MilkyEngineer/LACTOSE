//! The frame's presentation events, delivered once (`docs/plans/milestone-2.md`, decision
//! 5.5): a delivered event is not delivered again after a rollback that re-simulates its tick
//! and emits it again; one whose re-simulated tick no longer emits it is cancelled, once; a
//! frame with nothing published since delivers nothing; and under rollbacks and every cadence
//! of frames against publications, no key is delivered in two frames and what was delivered
//! of each key nets to what was published.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::common;

use common::{Delivered, Scene, claim, key, mesh, view};
use lactose_core::{StableKey, Tick};
use lactose_ecs::{PresentEventKey, WorldKind};
use lactose_present_api::MeshInstance;

/// A prediction world with three rows with a mesh, each of which the chirp emits a request for
/// every tick.
fn chirping() -> Scene {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 2);
    for n in 1..=3 {
        scene.world_mut().spawn(key(n), common::REGION, (mesh(n), claim())).unwrap();
    }
    scene
}

/// One frame of the schedule into `delivered`, numbered `frame`.
fn frame(scene: &mut Scene, delivered: &mut Delivered, frame: u32) {
    delivered.frame = frame;
    scene.frame(&view(0.5), delivered).expect("a publication");
}

fn origins(keys: &[(u32, PresentEventKey)]) -> Vec<(StableKey, Tick)> {
    let mut origins: Vec<_> = keys.iter().map(|(_, key)| (key.origin, key.tick)).collect();
    origins.sort_unstable();
    origins
}

#[test]
fn a_delivered_event_is_not_delivered_again_after_a_rollback() {
    let mut scene = chirping();
    let mut delivered = Delivered::default();
    scene.step();
    let saved = scene.world().snapshot();
    let resimulated = Tick(scene.tick().0 + 1);
    scene.tick_publish();
    frame(&mut scene, &mut delivered, 1);
    assert_eq!(delivered.emitted.len(), 6, "two ticks of three rows");

    // Nothing published since: nothing delivered.
    frame(&mut scene, &mut delivered, 2);
    assert_eq!((delivered.emitted.len(), delivered.cancelled.len()), (6, 0));

    // The second tick again, every row emitting again: delivered before, not again.
    scene.world_mut().restore(&saved).unwrap();
    scene.step();
    let (published, _) = scene.publish();
    assert!(published.is_empty(), "the world's keys: re-emissions are not taken again");
    frame(&mut scene, &mut delivered, 3);
    assert_eq!((delivered.emitted.len(), delivered.cancelled.len()), (6, 0));

    // A new tick: three new events.
    scene.tick_publish();
    frame(&mut scene, &mut delivered, 4);
    let new: Vec<_> = delivered.emitted[6..].to_vec();
    assert_eq!(new.len(), 3);
    assert!(new.iter().all(|&(frame, _)| frame == 4));

    // The last two ticks again without row 1's mesh: its two delivered events are cancelled,
    // once each, and nothing is emitted.
    scene.world_mut().restore(&saved).unwrap();
    scene.world_mut().remove::<MeshInstance>(key(1)).unwrap();
    scene.step();
    scene.tick_publish();
    frame(&mut scene, &mut delivered, 5);
    assert_eq!(delivered.emitted.len(), 9);
    let last = Tick(resimulated.0 + 1);
    assert_eq!(origins(&delivered.cancelled), [(key(1), resimulated), (key(1), last)]);
    assert!(delivered.cancelled.iter().all(|&(frame, _)| frame == 5));
    frame(&mut scene, &mut delivered, 6);
    assert_eq!((delivered.emitted.len(), delivered.cancelled.len()), (9, 2));
}

/// A count per key: emissions minus cancellations.
#[derive(Default)]
struct Net(Vec<(PresentEventKey, i32)>);

impl Net {
    fn add(&mut self, key: PresentEventKey, by: i32) {
        match self.0.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, count)) => *count += by,
            None => self.0.push((key, by)),
        }
    }

    fn nonzero(&self) -> Vec<(PresentEventKey, i32)> {
        self.0.iter().copied().filter(|&(_, count)| count != 0).collect()
    }
}

/// Under four cadences (a frame after every publication, after every other, two frames per
/// publication, and frames only at the end), with a two-tick rollback every eighth frame, the
/// second of each pair dropping row 1's mesh: no key is emitted or cancelled in two frames, and
/// the net of each key delivered is the net published.
#[test]
fn each_event_is_delivered_once_under_rollbacks() {
    let cadences: [&[u8]; 4] = [&[1, 1, 1, 1], &[1, 0, 1, 0], &[2, 2, 2, 2], &[0, 0, 0, 0]];
    for (index, cadence) in cadences.iter().enumerate() {
        let mut scene = chirping();
        let mut delivered = Delivered::default();
        let mut published = Net::default();
        let mut saved = BTreeMap::new();
        let mut rollbacks = 0;
        let mut frames = 0;
        for step in 0..48u32 {
            if step % 8 == 7 {
                let tick = scene.tick().0;
                scene.world_mut().restore(&saved[&(tick - 2)]).unwrap();
                if rollbacks % 2 == 1 && scene.world().get::<MeshInstance>(key(1)).is_some() {
                    scene.world_mut().remove::<MeshInstance>(key(1)).unwrap();
                }
                rollbacks += 1;
                scene.step();
            }
            scene.step();
            saved.insert(scene.tick().0, scene.world().snapshot());
            let (emitted, cancelled) = scene.publish();
            emitted.into_iter().for_each(|key| published.add(key, 1));
            cancelled.into_iter().for_each(|key| published.add(key, -1));
            for _ in 0..cadence[step as usize % cadence.len()] {
                frames += 1;
                frame(&mut scene, &mut delivered, frames);
            }
        }
        frames += 1;
        frame(&mut scene, &mut delivered, frames);

        let mut net = Net::default();
        for &(_, key) in &delivered.emitted {
            net.add(key, 1);
        }
        for &(_, key) in &delivered.cancelled {
            net.add(key, -1);
        }
        // The key is not `Ord`, and the lists are a few hundred long.
        for list in [&delivered.emitted, &delivered.cancelled] {
            let mut unique: Vec<PresentEventKey> = Vec::new();
            for &(_, key) in list.iter() {
                assert!(!unique.contains(&key), "cadence {index}: {key:?} in two frames");
                unique.push(key);
            }
        }
        let (delivered_net, published_net) = (net.nonzero(), published.nonzero());
        assert_eq!(delivered_net.len(), published_net.len(), "cadence {index}");
        assert!(published_net.iter().all(|entry| delivered_net.contains(entry)), "cadence {index}");
        assert!(delivered.emitted.len() > 80, "cadence {index}: {}", delivered.emitted.len());
        assert!(!delivered.cancelled.is_empty() || index == 3, "cadence {index}: cancellations");
    }
}
