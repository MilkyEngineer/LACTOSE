//! The debug-draw buffer (architecture section 13): the coloured segments one frame draws
//! over the scene, in render space. It is the render side's: the presentation schedule
//! fills it every frame, from its own code, from `game-present`, and from the
//! [`DebugShape`] events a tick emitted, and a debug pass reads it when the frame is
//! recorded. No system names it, and nothing in it reaches a world.

use lactose_math::render::RenderPosition;

use crate::events::DebugShape;

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
        todo!()
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
