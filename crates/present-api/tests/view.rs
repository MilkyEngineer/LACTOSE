//! The render side's two plain values (`README.md`, "View-models" and "Debug draw"): the
//! view-model table by type, by key and field ID, in key order, and its key-collision panic;
//! the segments `DebugDraw::shape` adds for each shape. That a non-`derived` view-model does
//! not build is `compile_fail.rs`'s.

#![forbid(unsafe_code)]

use lactose_core::FieldId;
use lactose_math::render::RenderPosition;
use lactose_math::{Scalar, Vec3};
use lactose_present_api::{DebugDraw, DebugShape, ViewModels};
use lactose_reflect::{Reflect, bytemuck, resource};

/// A health bar: a screen position may be a raw `f32` in a view-model.
#[resource(key = "test::health_bar", derived)]
#[derive(PartialEq, Debug)]
struct HealthBar {
    #[field(id = 1)]
    fill: u32,
    #[field(id = 7)]
    opacity: f32,
}

#[resource(key = "test::score", derived)]
#[derive(PartialEq, Eq, Debug)]
struct Score(#[field(id = 1)] u64);

/// A second type declared with `Score`'s key.
mod elsewhere {
    #[lactose_reflect::resource(key = "test::score", derived)]
    pub struct Score(#[field(id = 1)] pub u64);
}

#[test]
fn view_models_by_type_by_key_and_in_key_order() {
    let mut table = ViewModels::new();
    assert!(table.is_empty());
    assert_eq!(table.set(Score(3)), None);
    assert_eq!(table.set(HealthBar { fill: 40, opacity: 0.5 }), None);
    assert_eq!(table.set(Score(4)), Some(Score(3)), "the value it replaces");
    assert_eq!(table.len(), 2);
    assert_eq!(table.get::<Score>(), Some(Score(4)));
    assert_eq!(table.get::<HealthBar>(), Some(HealthBar { fill: 40, opacity: 0.5 }));

    // By key and field ID, as a cooked layout binds.
    let bar = table.get_erased(HealthBar::KEY).expect("stored");
    assert_eq!(bar.info().name(), "test::health_bar");
    assert_eq!(bar.bytes().len(), core::mem::size_of::<HealthBar>());
    let opacity = bar.info().field_bytes(bar.bytes(), FieldId(7)).unwrap();
    assert_eq!(opacity, bytemuck::bytes_of(&0.5f32));
    let fill = bar.info().field_bytes(bar.bytes(), FieldId(1)).unwrap();
    assert_eq!(fill, bytemuck::bytes_of(&40u32));
    assert!(bar.info().field_bytes(bar.bytes(), FieldId(2)).is_err());
    assert!(table.get_erased(lactose_core::ComponentKey::from_name("test::absent")).is_none());

    // Every stored value, in ascending key order, whatever the order of `set`.
    let keys: Vec<_> = table.iter().map(|model| model.info().key()).collect();
    let mut sorted = vec![Score::KEY, HealthBar::KEY];
    sorted.sort_unstable();
    assert_eq!(keys, sorted);
    assert_eq!((&table).into_iter().len(), 2);

    assert_eq!(table.remove::<Score>(), Some(Score(4)));
    assert_eq!(table.remove::<Score>(), None);
    assert_eq!(table.get::<Score>(), None);
    assert_eq!(table.len(), 1);
}

/// Two types with one key: `set` panics, `get` and `remove` of the type not stored see
/// nothing and change nothing.
#[test]
#[should_panic(expected = "have one key")]
fn a_key_collision_panics() {
    assert_eq!(Score::KEY, elsewhere::Score::KEY);
    let mut table = ViewModels::new();
    table.set(Score(1));
    assert!(table.get::<elsewhere::Score>().is_none());
    assert!(table.remove::<elsewhere::Score>().is_none());
    assert_eq!(table.get::<Score>(), Some(Score(1)));
    table.set(elsewhere::Score(2));
}

fn point(position: RenderPosition) -> [f32; 3] {
    position.relative_to([0; 3])
}

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(Scalar::from_f64(x), Scalar::from_f64(y), Scalar::from_f64(z))
}

#[test]
fn the_segments_of_each_shape() {
    let mut draw = DebugDraw::new();
    assert!(draw.is_empty());
    draw.shape(&DebugShape::line(v(1.0, 2.0, 3.0), v(4.0, 5.0, 6.0), 0xff00_00ff));
    assert_eq!(draw.lines().len(), 1);
    let line = draw.lines()[0];
    assert_eq!((point(line.from), point(line.to)), ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));
    assert_eq!(line.color, 0xff00_00ff);
    draw.clear();
    assert!(draw.is_empty());

    // A box: twelve edges, each joining two corners that differ on one axis, each corner on
    // three of them.
    draw.shape(&DebugShape::aabb(v(0.0, 0.0, 0.0), v(1.0, 2.0, 3.0), 7));
    let lines = draw.lines();
    assert_eq!(lines.len(), 12);
    let mut corners: Vec<([f32; 3], u32)> = Vec::new();
    for line in lines {
        let (from, to) = (point(line.from), point(line.to));
        assert_eq!((0..3).filter(|&axis| from[axis] != to[axis]).count(), 1, "{from:?} {to:?}");
        for end in [from, to] {
            assert!(end.iter().zip([1.0, 2.0, 3.0]).all(|(&c, max)| c == 0.0 || c == max));
            match corners.iter_mut().find(|(corner, _)| *corner == end) {
                Some((_, count)) => *count += 1,
                None => corners.push((end, 1)),
            }
        }
    }
    assert_eq!(corners.len(), 8);
    assert!(corners.iter().all(|&(_, count)| count == 3));
    draw.clear();

    // A sphere: three closed circles of `CIRCLE_SEGMENTS` segments about the centre, each
    // in a plane of two axes, every endpoint at the radius.
    let centre = [10.0f32, -4.0, 70.0];
    draw.shape(&DebugShape::sphere(v(10.0, -4.0, 70.0), Scalar::from_f64(2.0), 1));
    let lines = draw.lines();
    assert_eq!(lines.len(), 3 * DebugDraw::CIRCLE_SEGMENTS);
    for (plane, circle) in lines.chunks(DebugDraw::CIRCLE_SEGMENTS).enumerate() {
        let flat = [2, 0, 1][plane];
        for (i, line) in circle.iter().enumerate() {
            let (from, to) = (point(line.from), point(line.to));
            for end in [from, to] {
                let offset: Vec<f32> = (0..3).map(|axis| end[axis] - centre[axis]).collect();
                assert!(offset[flat].abs() < 1e-4, "plane {plane}: {offset:?}");
                let radius = offset.iter().map(|o| o * o).sum::<f32>().sqrt();
                assert!((radius - 2.0).abs() < 1e-4, "{radius}");
            }
            assert_eq!(to, point(circle[(i + 1) % circle.len()].from), "closed");
        }
    }
    // The order depends on the shape alone.
    let again = {
        let mut draw = DebugDraw::new();
        draw.shape(&DebugShape::sphere(v(10.0, -4.0, 70.0), Scalar::from_f64(2.0), 1));
        draw
    };
    assert_eq!(again.lines(), draw.lines());
}
