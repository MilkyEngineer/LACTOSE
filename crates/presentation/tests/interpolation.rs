//! Interpolation between the snapshot pair (verification row 99): a body re-spawned under its
//! key without `Teleported` is interpolated between the two snapshots, and one re-spawned with
//! it is shown where it now is; the fraction is the weight over a pair however many ticks
//! apart, at its ends and for a pair the snapshot flags; and the camera's pivot is where the
//! renderer draws the mesh it follows, at every fraction, over pairs one, two and five ticks
//! apart.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::common;

use common::{Ignored, Scene, at, claim, key, mesh, scalar, shown_x, view};
use lactose_core::{StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Teleported, Transform};
use lactose_present_api::{CameraRig, ExtractPass, InstanceRow, InstanceSink};
use lactose_presentation::{PresentView, interpolate};

/// A prediction world with the body `key(1)` at x 0, carrying the camera's rig, its first
/// snapshot acquired.
fn body() -> Scene {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 2);
    let rig = CameraRig::new(Vec3::ZERO, scalar(3.0));
    scene.world_mut().spawn(key(1), common::REGION, (at(0.0), rig, claim())).unwrap();
    scene.publish();
    scene.frame(&view(0.0), &mut Ignored).unwrap();
    scene
}

/// The body despawned and spawned again under its key at x 10, between ticks, with a
/// `Teleported` of `teleported` if `Some`; then a tick and its publication.
fn respawn(scene: &mut Scene, teleported: Option<Tick>) {
    scene.world_mut().despawn(key(1)).unwrap();
    let rig = CameraRig::new(Vec3::ZERO, scalar(3.0));
    let row = (at(10.0), rig, claim());
    match teleported {
        None => scene.world_mut().spawn(key(1), common::REGION, row).unwrap(),
        Some(tick) => {
            let row = (at(10.0), rig, claim(), Teleported(tick));
            scene.world_mut().spawn(key(1), common::REGION, row).unwrap();
        }
    }
    scene.tick_publish();
}

/// Row 99, without `Teleported`: the pair holds the body in both snapshots, so the frame
/// blends from where it was to where it is, and the camera's pivot with it.
#[test]
fn a_respawn_without_teleported_interpolates() {
    let mut scene = body();
    respawn(&mut scene, None);
    let focus = PresentView { focus: Some(key(1)), ..view(0.5) };
    let shown = scene.frame(&focus, &mut Ignored).unwrap();
    let pair = shown.frame.transform(key(1)).unwrap();
    assert_eq!((pair.previous, pair.current, pair.no_interpolate), (at(0.0), at(10.0), false));
    assert_eq!(shown_x(&shown.frame, key(1), 0.5), 5.0);
    assert_eq!(shown_x(&shown.frame, key(1), 0.0), 0.0);
    assert_eq!(shown_x(&shown.frame, key(1), 1.0), 10.0);
    let pivot = shown.camera.unwrap().pivot;
    assert_eq!(pivot, RenderPosition::from_sim(at(5.0).translation));
}

/// Row 99, with `Teleported` at the newer snapshot's tick: the pair is flagged, and every
/// fraction shows the body where it is; the camera's pivot jumps with it.
#[test]
fn a_respawn_with_teleported_does_not_interpolate() {
    let mut scene = body();
    let newer = Tick(scene.tick().0 + 1);
    respawn(&mut scene, Some(newer));
    let focus = PresentView { focus: Some(key(1)), ..view(0.5) };
    let shown = scene.frame(&focus, &mut Ignored).unwrap();
    assert_eq!(shown.frame.newer().tick(), newer);
    let pair = shown.frame.transform(key(1)).unwrap();
    assert_eq!((pair.previous, pair.current, pair.no_interpolate), (at(0.0), at(10.0), true));
    for fraction in [0.0, 0.5, 1.0] {
        assert_eq!(shown_x(&shown.frame, key(1), fraction), 10.0, "{fraction}");
    }
    let pivot = shown.camera.unwrap().pivot;
    assert_eq!(pivot, RenderPosition::from_sim(at(10.0).translation));
}

/// A `Teleported` at or before the older snapshot's tick is in the past: the re-spawn blends.
#[test]
fn a_teleport_before_the_pair_interpolates() {
    let mut scene = body();
    let older = scene.tick();
    respawn(&mut scene, Some(older));
    let shown = scene.frame(&view(0.5), &mut Ignored).unwrap();
    assert!(!shown.frame.transform(key(1)).unwrap().no_interpolate);
    assert_eq!(shown_x(&shown.frame, key(1), 0.5), 5.0);
}

/// The weight over a pair two ticks apart, as a frame gets when two ticks ran between its
/// publications: the fraction, over the whole pair, as the renderer draws. The ends are exact,
/// a fraction out of `[0, 1]` is clamped and NaN shows the newer transform; the rotation is
/// `slerp`ed.
#[test]
fn the_weight_spans_the_pair() {
    let mut scene = body();
    let quarter = Quat::from_rotation_y(Scalar::FRAC_PI_2);
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().translation.x = scalar(4.0);
    scene.step();
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().translation.x = scalar(8.0);
    scene.world_mut().get_mut::<Transform>(key(1)).unwrap().rotation = quarter;
    scene.tick_publish();
    let shown = scene.frame(&view(0.0), &mut Ignored).unwrap();
    let frame = shown.frame;
    assert_eq!(frame.newer().tick().0 - frame.older().tick().0, 2, "two ticks in one pair");
    assert_eq!(shown_x(&frame, key(1), 0.0), 0.0);
    assert_eq!(shown_x(&frame, key(1), 0.25), 2.0);
    assert_eq!(shown_x(&frame, key(1), 0.5), 4.0);
    assert_eq!(shown_x(&frame, key(1), 1.0), 8.0);
    assert_eq!(shown_x(&frame, key(1), -3.0), 0.0);
    assert_eq!(shown_x(&frame, key(1), 7.0), 8.0);
    assert_eq!(shown_x(&frame, key(1), f32::NAN), 8.0);
    assert_eq!(interpolate(&frame, key(1), 0.0).unwrap().rotation, Quat::IDENTITY);
    let half = interpolate(&frame, key(1), 0.5).unwrap().rotation.to_array();
    let eighth = Quat::from_rotation_y(Scalar::FRAC_PI_2 * Scalar::HALF).to_array();
    for (a, b) in half.into_iter().zip(eighth) {
        assert!((a - b).abs() < scalar(1e-12), "{half:?}");
    }
    assert_eq!(interpolate(&frame, key(1), 1.0).unwrap().rotation, quarter);
    assert!(interpolate(&frame, key(9), 0.5).is_none());
}

/// The first snapshot is published alone, so its pair is one tick: shown as it is.
#[test]
fn a_pair_of_one_tick_is_not_interpolated() {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 1);
    let row = (at(2.0), CameraRig::new(Vec3::ZERO, Scalar::ONE), claim());
    scene.world_mut().spawn(key(1), common::REGION, row).unwrap();
    scene.publish();
    let shown = scene.frame(&view(0.5), &mut Ignored).unwrap();
    assert_eq!(shown.frame.older().tick(), shown.frame.newer().tick());
    assert_eq!(shown_x(&shown.frame, key(1), 0.5), 2.0);
}

/// What the renderer holds of one instance: the placements it draws between, kept by the
/// rules of `crates/present-api/README.md`, "What the sink keeps".
#[derive(Default, Clone, Copy)]
struct Held {
    previous: Option<RenderPosition>,
    current: Option<RenderPosition>,
    base: Option<RenderPosition>,
}

/// A model of the renderer's sink: at the first pass of a tick each instance's current
/// placement becomes its base (none after a pass of an earlier tick); a row written makes the
/// base its previous placement and itself the current one, or both itself where it has no
/// base or is flagged.
#[derive(Default)]
struct Placements {
    held: Mutex<BTreeMap<StableKey, Held>>,
    tick: Mutex<Option<Tick>>,
}

struct Sink(Arc<Placements>);

impl InstanceSink for Sink {
    fn begin(&mut self, pass: ExtractPass) {
        let mut tick = self.0.tick.lock().expect("unpoisoned");
        if *tick != Some(pass.tick) {
            let earlier = tick.is_some_and(|t| pass.tick.0.wrapping_sub(t.0) > i32::MAX as u32);
            for held in self.0.held.lock().expect("unpoisoned").values_mut() {
                held.base = if earlier { None } else { held.current };
            }
        }
        *tick = Some(pass.tick);
    }

    fn removed(&mut self, keys: &[StableKey]) {
        let mut held = self.0.held.lock().expect("unpoisoned");
        for key in keys {
            held.remove(key);
        }
    }

    fn write(&self, rows: &[InstanceRow]) {
        let mut held = self.0.held.lock().expect("unpoisoned");
        for row in rows {
            let entry = held.entry(row.key).or_default();
            let placed = Some(row.transform.position);
            entry.previous = if row.no_interpolate { placed } else { entry.base.or(placed) };
            entry.current = placed;
        }
    }

    fn end(&mut self) {}
}

impl Placements {
    /// The x the renderer draws `key` at, from cell zero, at `fraction`, by
    /// `FrameView::fraction`'s rule: the previous placement at zero, the current one at one.
    fn drawn_x(&self, key: StableKey, fraction: f32) -> f32 {
        let held = self.held.lock().expect("unpoisoned")[&key];
        let previous = held.previous.expect("placed").relative_to([0, 0, 0])[0];
        let current = held.current.expect("placed").relative_to([0, 0, 0])[0];
        previous + (current - previous) * fraction
    }
}

/// The local player is a mesh the renderer draws and a row whose `CameraRig` the camera
/// follows. Moved 4 m a tick over `span` ticks between two publications, the camera's pivot
/// is where the mesh is drawn at every fraction of the frame, on one acquired pair.
fn the_pivot_follows_the_mesh(span: u32) {
    let mut scene = Scene::new(WorldKind::Prediction, Tick(10), 2);
    let placements = Arc::new(Placements::default());
    scene.install_sink(Box::new(Sink(Arc::clone(&placements))));
    let rig = CameraRig::new(Vec3::ZERO, scalar(3.0));
    scene.world_mut().spawn(key(1), common::REGION, (at(0.0), rig, mesh(1), claim())).unwrap();
    scene.publish();
    scene.frame(&view(0.0), &mut Ignored).unwrap();
    for n in 1..=span {
        let x = scalar(4.0 * f64::from(n));
        scene.world_mut().get_mut::<Transform>(key(1)).unwrap().translation.x = x;
        scene.step();
    }
    scene.publish();

    let frame = scene.reader.acquire().unwrap();
    assert_eq!(frame.newer().tick().0 - frame.older().tick().0, span);
    for step in 0..=8u8 {
        let fraction = f32::from(step) / 8.0;
        let focus = PresentView { focus: Some(key(1)), ..view(fraction) };
        let pose = scene.presentation.frame(&frame, &focus, &mut Ignored).camera.unwrap();
        let pivot = pose.pivot.relative_to([0, 0, 0])[0];
        let drawn = placements.drawn_x(key(1), fraction);
        let moved = 4.0 * span as f32 * fraction;
        assert!((pivot - drawn).abs() < 1e-4, "span {span} at {fraction}: {pivot}, {drawn}");
        assert!((drawn - moved).abs() < 1e-4, "span {span} at {fraction}: {drawn}");
    }
}

#[test]
fn the_pivot_follows_the_mesh_over_one_tick() {
    the_pivot_follows_the_mesh(1);
}

#[test]
fn the_pivot_follows_the_mesh_over_two_ticks() {
    the_pivot_follows_the_mesh(2);
}

#[test]
fn the_pivot_follows_the_mesh_over_five_ticks() {
    the_pivot_follows_the_mesh(5);
}
