//! Root motion (`README.md`, "Root motion"; "Proof for milestone 1", the root motion row):
//! a loop's motion over a cycle is the track's total at every rate, a quarter-circle
//! turning clip applied tick by tick ends where it was authored, a one-shot's reaches its
//! end, a blend interpolates, and `apply` and `velocity` against hand-composed transforms.

mod common;

use common::{
    Fixture, Library, MACHINE, SKELETON, chain, clip, clip_ref, key, s, state, transition, v,
    with_root, yaw,
};
use lactose_anim::{AnimRig, Clip, Condition, Phase, RootMotion, RootSample, StateMachine};
use lactose_asset_api::AssetRef;
use lactose_ecs::WorldKind;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Transform, Velocity};

const UNITS: u64 = 1 << 32;

/// A straight walk: four frames a quarter of a unit apart along `-Z`, a loop.
fn walk() -> Clip {
    with_root(clip(2, 4, 40, true, 0.0), 0.25, 0.0)
}

/// `root` moved through `motions` tick by tick, as a kinematic character applies them.
fn applied(root: Transform, motions: impl IntoIterator<Item = RootMotion>) -> Transform {
    motions.into_iter().fold(root, |root, motion| motion.apply(&root))
}

#[test]
fn a_loops_motion_over_whole_cycles_is_its_total_at_every_rate() {
    let walk = walk();
    let total = walk.root.last().unwrap().translation;
    assert_eq!(total, v(0.0, 0.0, -1.0));
    for rate in [12_345_678u32, 1 << 20, 107_374_183, 300_000_007, 1 << 31, u32::MAX - 12] {
        // From an arbitrary start, steps of `rate` until a whole number of cycles is near,
        // then one last step back onto the start: the advance sums to whole cycles.
        let start = Phase(123_456_789);
        let mut phase = start;
        let mut moved: u64 = 0;
        let mut motions = Vec::new();
        let cycles = 3;
        while moved + u64::from(rate) < cycles * UNITS {
            let next = phase.advanced(rate, true);
            motions.push(walk.root_motion(phase, next));
            phase = next;
            moved += u64::from(rate);
        }
        motions.push(walk.root_motion(phase, start));
        let end = applied(Transform::IDENTITY, motions);
        assert_eq!(end.translation, total * Scalar::from_u64(cycles), "rate {rate}");
        assert_eq!(end.rotation, Quat::IDENTITY);
    }
}

#[test]
fn a_loops_motion_in_the_world_tracks_its_phase() {
    let mut fixture = walk_fixture(walk());
    let mut root = Transform::IDENTITY;
    for tick in 1..=200u32 {
        fixture.step();
        root = fixture.motion(key(1)).apply(&root);
        // The root is where the unwrapped phase says: whole cycles plus the track.
        let unwrapped = u64::from(Phase::rate(40, s(1.0))) * u64::from(tick);
        let cycles = unwrapped / UNITS;
        let within = Phase((unwrapped % UNITS) as u32);
        let track = walk().root_motion(Phase::ZERO, within).translation;
        let expected = v(0.0, 0.0, -1.0) * Scalar::from_u64(cycles) + track;
        assert_eq!(root.translation, expected, "tick {tick}");
    }
}

fn walk_fixture(clip: Clip) -> Fixture {
    let machine = StateMachine {
        states: vec![state("walk", WALK_CLIP, 1.0)],
        transitions: Vec::new(),
        initial: 0,
    };
    let library = Library::new()
        .with(SKELETON, chain(2, |_| false))
        .with(MACHINE, machine)
        .with(WALK_CLIP, clip);
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library.pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture
}

const WALK_CLIP: AssetRef<Clip> = clip_ref("root/walk");
const OTHER_CLIP: AssetRef<Clip> = clip_ref("root/other");

/// A one-shot of `ticks` ticks whose root walks a quarter circle of radius `radius` to the
/// left, turning with it: `frames` entries, frame `i` at angle `(π / 2) i / (frames - 1)`.
fn quarter_circle(frames: u32, ticks: u32, radius: f64) -> Clip {
    let mut clip = clip(2, frames, ticks, false, 0.0);
    let quarter = Scalar::from_f64(core::f64::consts::FRAC_PI_2);
    let r = s(radius);
    clip.root = (0..frames)
        .map(|frame| {
            let angle = quarter * Scalar::from_u32(frame) / Scalar::from_u32(frames - 1);
            let (sin, cos) = angle.sin_cos();
            // Heading at yaw `a` is `(-sin a, 0, -cos a)`; the circle's centre is at `-X`.
            RootSample {
                translation: Vec3::new(r * (cos - Scalar::ONE), Scalar::ZERO, -r * sin),
                yaw: angle,
            }
        })
        .collect();
    clip
}

/// The distance in units in the last place between two scalars of one sign; the absolute
/// difference in units of `Scalar::EPSILON` when either is zero.
fn ulps(a: Scalar, b: Scalar) -> u64 {
    if a == b {
        return 0;
    }
    if a == Scalar::ZERO || b == Scalar::ZERO || (a < Scalar::ZERO) != (b < Scalar::ZERO) {
        return ((a - b).abs() / Scalar::EPSILON).ceil().to_i64().unsigned_abs();
    }
    a.to_canonical_bits().abs_diff(b.to_canonical_bits())
}

/// The clip's own motion is exact to the track (each tick's displacement is the track's
/// difference turned back by the yaw already turned), but a root moved by `apply` tick after
/// tick composes a rotation and adds a turned displacement every tick, each rounded once,
/// so the end drifts from the authored pose by rounding that grows with the ticks: measured
/// at eight-byte scalars, 11 units in the last place after 60 ticks and 258 after 600. The
/// bound held is one unit in the last place per tick applied.
#[test]
fn a_quarter_circle_applied_tick_by_tick_ends_where_it_was_authored() {
    for (frames, ticks, radius) in [(16, 60, 1.0), (31, 90, 3.0), (2, 30, 2.0), (61, 600, 1.5)] {
        let clip = quarter_circle(frames, ticks, radius);
        let authored = *clip.root.last().unwrap();
        let mut fixture = walk_fixture(clip);
        let mut root = Transform::IDENTITY;
        for _ in 0..ticks {
            fixture.step();
            root = fixture.motion(key(1)).apply(&root);
        }
        assert_eq!(fixture.state(key(1)).phase(), Phase::END);
        fixture.step();
        assert_eq!(fixture.motion(key(1)), RootMotion::ZERO, "a finished one-shot moves no more");
        let target = Transform::new(authored.translation, Quat::from_rotation_y(authored.yaw));
        let distance = [
            ulps(root.translation.x, target.translation.x),
            ulps(root.translation.y, target.translation.y),
            ulps(root.translation.z, target.translation.z),
            ulps(root.rotation.x, target.rotation.x),
            ulps(root.rotation.y, target.rotation.y),
            ulps(root.rotation.z, target.rotation.z),
            ulps(root.rotation.w, target.rotation.w),
        ];
        eprintln!("frames {frames}, ticks {ticks}, radius {radius}: ulps {distance:?}");
        let worst = distance.into_iter().max().unwrap();
        assert!(
            worst <= u64::from(ticks),
            "{ticks} ticks end {worst} units in the last place away"
        );
    }
}

#[test]
fn a_one_shots_motion_reaches_its_end() {
    let clip = with_root(clip(2, 5, 17, false, 0.0), 0.375, 0.0);
    let end = clip.root.last().unwrap().translation;
    let mut fixture = walk_fixture(clip);
    let mut root = Transform::IDENTITY;
    for _ in 0..25 {
        fixture.step();
        root = fixture.motion(key(1)).apply(&root);
    }
    assert!(fixture.state(key(1)).finished());
    assert_eq!(root.translation, end);
}

#[test]
fn a_blend_interpolates_the_two_clips_motion() {
    let turn = with_root(clip(2, 4, 24, true, 0.2), 0.5, 0.125);
    let mut walk_state = state("walk", WALK_CLIP, 1.0);
    walk_state.speed_param = None;
    let mut cross = transition(Some(0), 1, &[Condition::Trigger(0)]);
    cross.blend_ticks = 8;
    cross.entry_phase = Phase::HALF;
    let machine = StateMachine {
        states: vec![walk_state, state("turn", OTHER_CLIP, 1.0)],
        transitions: vec![cross],
        initial: 0,
    };
    let library = Library::new()
        .with(SKELETON, chain(2, |_| false))
        .with(MACHINE, machine)
        .with(WALK_CLIP, walk())
        .with(OTHER_CLIP, turn.clone());
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library.pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.run(5);
    fixture.set_params(key(1), |params| params.trigger(0));
    let mut blended = 0;
    let mut outgoing_from = fixture.state(key(1)).phase();
    for _ in 0..10 {
        fixture.step();
        let row = fixture.state(key(1));
        let current = turn.root_motion(row.previous_phase(), row.phase());
        let motion = fixture.motion(key(1));
        match row.blend() {
            Some(blend) => {
                let out = walk().root_motion(outgoing_from, blend.phase);
                let expected = RootMotion {
                    translation: current.translation.lerp(out.translation, blend.weight),
                    yaw: current.yaw.lerp(out.yaw, blend.weight),
                };
                assert_eq!(motion, expected);
                assert_ne!(motion, current);
                outgoing_from = blend.phase;
                blended += 1;
            }
            None => assert_eq!(motion, current),
        }
    }
    assert_eq!(blended, 7, "the blend's ticks before its length");
}

#[test]
fn apply_and_velocity_match_hand_composed_transforms() {
    let root = Transform::new(v(3.0, 1.0, -2.0), yaw(0.7));
    let motion = RootMotion { translation: v(0.25, 0.5, -1.0), yaw: s(0.3) };
    let moved = motion.apply(&root);
    let (sin, cos) = s(0.7).sin_cos();
    // `R(0.7)` of `(x, y, z)` is `(x cos + z sin, y, -x sin + z cos)`.
    let turned = v(0.0, 0.0, 0.0)
        + Vec3::new(s(0.25) * cos + s(-1.0) * sin, s(0.5), -s(0.25) * sin + s(-1.0) * cos);
    let close =
        |a: Scalar, b: Scalar| (a - b).abs() <= Scalar::EPSILON * s(8.0) * b.abs().max(Scalar::ONE);
    assert!(close(moved.translation.x, s(3.0) + turned.x));
    assert!(close(moved.translation.y, s(1.5)));
    assert!(close(moved.translation.z, s(-2.0) + turned.z));
    assert_eq!(moved.rotation, yaw(0.3) * yaw(0.7));
    let composed = yaw(1.0);
    assert!(close(moved.rotation.y, composed.y) && close(moved.rotation.w, composed.w));

    let current = Velocity { linear: v(9.0, -4.0, 9.0), angular: v(0.5, 9.0, -0.5) };
    let dt = s(1.0 / 64.0);
    let velocity = motion.velocity(&root, &current, dt);
    assert!(close(velocity.linear.x, turned.x * s(64.0)));
    assert_eq!(velocity.linear.y, s(-4.0), "the vertical is the row's");
    assert!(close(velocity.linear.z, turned.z * s(64.0)));
    assert_eq!(velocity.angular, v(0.5, 0.3 * 64.0, -0.5));
    let still = motion.velocity(&root, &current, Scalar::ZERO);
    assert_eq!(still.linear, v(0.0, -4.0, 0.0));
    assert_eq!(still.angular, v(0.5, 0.0, -0.5));
}

#[test]
fn no_root_track_and_no_advance_move_nothing() {
    let quiet = clip(2, 4, 40, true, 0.0);
    assert_eq!(quiet.root_motion(Phase::ZERO, Phase::HALF), RootMotion::ZERO);
    assert_eq!(walk().root_motion(Phase::HALF, Phase::HALF), RootMotion::ZERO);
    // Across the wrap, by the formula: from fifteen sixteenths to one eighth.
    let turn = with_root(clip(2, 4, 24, true, 0.2), 0.5, 0.125);
    let (from, to) = (Phase(u32::MAX - (1 << 28) + 1), Phase(1 << 29));
    let wrapped = turn.root_motion(from, to);
    assert_eq!(wrapped.yaw, s(0.5) - s(0.468_75) + s(0.0625));
    let expected = yaw(-0.468_75) * v(0.0, 0.0, -0.125) + yaw(0.5 - 0.468_75) * v(0.0, 0.0, -0.25);
    assert_eq!(wrapped.translation, expected);
}
