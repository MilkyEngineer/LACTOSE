//! Render space, architecture section 12.4.
//!
//! The GPU scene stores positions as an integer grid cell plus a 32-bit float offset, and
//! shaders subtract the camera's cell so precision holds anywhere on the map. This module is
//! used outside the determinism boundary. Its types are deliberately disjoint from the
//! simulation types: there is no `From`, `Into` or operator between the two spaces, only the
//! named conversion [`RenderPosition::from_sim`], so a simulation position cannot reach a
//! shader, or a 32-bit offset reach the simulation, by accident.

use bytemuck::{Pod, Zeroable};

use crate::Vec3;

/// A position in render space: a grid cell and an offset from the centre of that cell.
///
/// The position is `cell * CELL_SIZE + offset` per axis, in metres.
///
/// Layout: `repr(C)`, `cell` then `offset`, 24 bytes, no padding. `Pod`, so it can be written
/// to a GPU buffer directly.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct RenderPosition {
    /// The grid cell, per axis.
    pub cell: [i32; 3],
    /// The offset from the cell's centre in metres, per axis, within
    /// `[-CELL_SIZE / 2, CELL_SIZE / 2]`.
    pub offset: [f32; 3],
}

impl RenderPosition {
    /// The edge of a grid cell in metres. A power of two, so the split into cell and offset
    /// loses nothing before the offset is narrowed to 32 bits; an offset is then precise to
    /// about four micrometres.
    pub const CELL_SIZE: f32 = 64.0;

    /// Converts a simulation position.
    ///
    /// Each axis is split into the nearest cell and the remainder, and the remainder is
    /// rounded to 32 bits. The cell saturates beyond about `±1.4e11` metres, where the
    /// offset then carries the excess at reduced precision; a NaN coordinate gives cell
    /// zero and a NaN offset.
    #[must_use]
    pub fn from_sim(position: Vec3) -> Self {
        let mut out = Self::default();
        for (axis, coordinate) in position.to_array().into_iter().enumerate() {
            let metres = coordinate.to_f64();
            let size = f64::from(Self::CELL_SIZE);
            // Saturating, and zero for NaN.
            let cell = libm::floor(metres / size + 0.5) as i32;
            out.cell[axis] = cell;
            out.offset[axis] = (metres - f64::from(cell) * size) as f32;
        }
        out
    }

    /// The position relative to the centre of `origin_cell`, in metres: what a shader computes
    /// after subtracting the camera's cell. The cell difference wraps.
    #[must_use]
    pub fn relative_to(self, origin_cell: [i32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for axis in 0..3 {
            let cells = self.cell[axis].wrapping_sub(origin_cell[axis]);
            out[axis] = cells as f32 * Self::CELL_SIZE + self.offset[axis];
        }
        out
    }
}
