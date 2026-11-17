//! The debug-draw buffer (architecture section 13): the coloured segments one frame draws
//! over the scene, in render space. It is the render side's: the presentation schedule
//! fills it every frame, from its own code, from `game-present`, and from the
//! [`DebugShape`] events a tick emitted, and a debug pass reads it when the frame is
//! recorded. No system names it, and nothing in it reaches a world.

use lactose_math::render::RenderPosition;
use lactose_math::{Scalar, Vec3};

use crate::events::{DebugShape, DebugShapeKind};

/// One segment of the buffer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DebugLine {
    /// Where the segment starts.
    pub from: RenderPosition,
    /// Where the segment ends.
    pub to: RenderPosition,
    /// The colour as `0xRRGGBBAA`: sRGB red, green and blue, then straight alpha.
    pub color: u32,
}

/// The segments one frame draws, in the order they were added. A plain value its owner
/// passes by `&mut`: one per frame in flight, cleared when the frame starts.
#[derive(Clone, Debug, Default)]
pub struct DebugDraw {
    lines: Vec<DebugLine>,
}

impl DebugDraw {
    /// How many segments each of a sphere's three circles is drawn with.
    pub const CIRCLE_SEGMENTS: usize = 24;

    /// An empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Adds the segment from `from` to `to`.
    pub fn line(&mut self, from: RenderPosition, to: RenderPosition, color: u32) {
        self.lines.push(DebugLine { from, to, color });
    }

    /// Adds the segments of a shape a tick emitted, each endpoint converted with
    /// `RenderPosition::from_sim`: one for a line; twelve for a box, its edges; and for a
    /// sphere three circles about its centre, one in each plane of two axes, each of
    /// [`CIRCLE_SEGMENTS`](Self::CIRCLE_SEGMENTS) segments. The segments of one shape are
    /// added in an order that depends on the shape alone.
    pub fn shape(&mut self, shape: &DebugShape) {
        let mut segment = |from: Vec3, to: Vec3| {
            self.line(RenderPosition::from_sim(from), RenderPosition::from_sim(to), shape.color);
        };
        let (a, b) = (shape.a, shape.b);
        match shape.kind {
            DebugShapeKind::Line => segment(a, b),
            DebugShapeKind::Box => {
                // Corner `i` takes `b`'s x, y and z where bits 0, 1 and 2 of `i` are set. Each
                // edge joins a corner to the one that differs in one bit: four along x, four
                // along y, four along z.
                let corner = |i: usize| {
                    Vec3::new(
                        if i & 1 == 0 { a.x } else { b.x },
                        if i & 2 == 0 { a.y } else { b.y },
                        if i & 4 == 0 { a.z } else { b.z },
                    )
                };
                for axis in [1, 2, 4] {
                    for i in (0..8).filter(|i| i & axis == 0) {
                        segment(corner(i), corner(i | axis));
                    }
                }
            }
            DebugShapeKind::Sphere => {
                let radius = b.x;
                // One circle's points at the radius, as (u, v) in its plane; the last segment
                // closes on the first point, so every circle is closed exactly.
                let steps = u32::try_from(Self::CIRCLE_SEGMENTS).expect("a small count");
                let points: Vec<(Scalar, Scalar)> = (0..steps)
                    .map(|i| {
                        let angle = Scalar::TAU * Scalar::from_u32(i) / Scalar::from_u32(steps);
                        let (sin, cos) = angle.sin_cos();
                        (cos * radius, sin * radius)
                    })
                    .collect();
                let planes: [fn(Scalar, Scalar) -> Vec3; 3] = [
                    |u, v| Vec3::new(u, v, Scalar::ZERO),
                    |u, v| Vec3::new(Scalar::ZERO, u, v),
                    |u, v| Vec3::new(u, Scalar::ZERO, v),
                ];
                for plane in planes {
                    for (i, &(u, v)) in points.iter().enumerate() {
                        let (next_u, next_v) = points[(i + 1) % points.len()];
                        segment(a + plane(u, v), a + plane(next_u, next_v));
                    }
                }
            }
        }
    }

    /// The segments, in the order they were added.
    #[must_use]
    pub fn lines(&self) -> &[DebugLine] {
        &self.lines
    }

    /// Whether the buffer holds no segment.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Empties the buffer and keeps its allocation.
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}
