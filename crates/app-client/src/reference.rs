//! Decision 5.1's CPU reference: the last fixed frame rasterised from the placements extract
//! handed the renderer, the latched camera and the fraction, the instance records the
//! device's scatter wrote held to those placements exactly, and the target's pixels held to
//! the reference at a quarter of a pixel's margin. `lactose-renderer`'s device test, "The
//! CPU reference", is the same method on its own scenes.

use std::fmt::Write as _;
use std::path::Path;

use lactose_core::{Error, ErrorKind, Result};
use lactose_renderer::shader::{PALETTE, TRIANGLE};
use lactose_renderer::{CameraData, InstanceRecord};
use lactose_rhi_api::Extent;

use crate::render::LastFrame;

/// How far a pixel's centre must lie inside or outside every edge the reference computes
/// for the device's pixel to be held to it: a quarter of a pixel, decision 5.1's margin.
///
/// Vulkan's least `subPixelPrecisionBits` is 4, so a device snaps each vertex to a sixteenth
/// of a pixel, a thirty-second at most on each axis, which moves an edge by at most
/// `sqrt(2) / 32` of a pixel, under 0.05. The vertex stage computes what the reference
/// computes, `InstanceRecord::drawn`, `CellTransform::apply` and `CameraData::clip`, in
/// 32-bit floats, and differs only in the order and the fusion of its operations. Where
/// every coordinate of a vertex relative to the camera's cell is under [`REACH`] and every
/// product the clip coordinates sum is under [`PRODUCT`], the two differ by a few times 1e-5
/// at most, which the division by a `w` of [`NEAREST`] or more and the few hundred pixels of
/// half the extent leave at a few thousandths of a pixel. A quarter of a pixel holds both
/// about five times over, and is still far less than the pixel that decision 5.1's mutation
/// moves a vertex by. [`reference`] refuses a scene where a vertex breaks one of the three.
pub const MARGIN: f64 = 0.25;

/// The bound of a vertex's coordinates relative to the camera's cell, in metres: the scene
/// lies within six metres of the origin.
pub const REACH: f32 = 10.0;

/// The bound of each product the clip coordinates sum.
pub const PRODUCT: f32 = 100.0;

/// The least clip `w` of a vertex: the camera stands about fourteen metres from the scene's
/// centre, and the nearest vertex is about ten metres before it.
pub const NEAREST: f64 = 8.0;

/// The clear colour's bytes: `shader::CLEAR`.
pub const CLEAR: [u8; 4] = [0, 0, 0, 255];

/// Each slot's colour: `PALETTE[slot % 7]`.
#[must_use]
pub fn colour(slot: usize) -> [u8; 4] {
    PALETTE[slot % PALETTE.len()]
}

/// What the reference holds a pixel to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Expected {
    /// Inside one triangle by the margin and outside every other by it: its colour.
    Drawn([u8; 4]),
    /// Outside every triangle by the margin: the clear colour.
    Clear,
    /// Within the margin of an edge: the colour of a triangle whose edge it is near, or of
    /// the one it is inside by the margin if any, and the clear colour only where it is
    /// inside none.
    Either {
        /// The triangle the pixel is inside by the margin.
        inside: Option<[u8; 4]>,
        /// The triangles whose edges the pixel is within the margin of.
        near: Vec<[u8; 4]>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Inside,
    Outside,
    Margin,
}

/// The three counts of a comparison that found no wrong pixel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counts {
    /// Pixels surely inside one triangle, each its colour.
    pub drawn: usize,
    /// Pixels surely outside every triangle, each the clear colour.
    pub clear: usize,
    /// Pixels within the margin of an edge.
    pub margin: usize,
}

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// The reference image of `records` drawn at `fraction` with `camera` over `extent`: each
/// live slot's triangle in 32-bit floats as the vertex stage computes it, then each pixel's
/// centre against its edges in `f64`. Clip space is `y` up and the framebuffer's origin its
/// top left.
///
/// # Errors
///
/// `InvalidData` where the scene is not one the reference can hold a device to: a vertex
/// outside the depth range or outside [`MARGIN`]'s premises, a triangle of a pixel or less,
/// or a pixel's centre inside two triangles.
pub fn reference(
    records: &[InstanceRecord],
    camera: &CameraData,
    fraction: f32,
    extent: Extent,
) -> Result<Vec<Expected>> {
    let (width, height) = (f64::from(extent.width), f64::from(extent.height));
    let mut triangles = Vec::new();
    for (slot, record) in records.iter().enumerate() {
        let Some(drawn) = record.drawn(fraction, camera.cell) else { continue };
        let mut window = [[0.0; 2]; 3];
        for (corner, vertex) in window.iter_mut().zip(TRIANGLE) {
            let point = drawn.apply(vertex);
            premises(slot, point, camera)?;
            let [x, y, z, w] = camera.clip(point).map(f64::from);
            if !(w >= NEAREST && z > 0.0 && z < w) {
                return Err(invalid(format!(
                    "slot {slot} lies outside the depth range or nearer than {NEAREST}"
                )));
            }
            *corner = [(x / w + 1.0) / 2.0 * width, (1.0 - y / w) / 2.0 * height];
        }
        let [a, b, c] = window;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() <= 1.0 {
            return Err(invalid(format!("slot {slot}'s triangle covers a pixel or less")));
        }
        triangles.push((window, colour(slot)));
    }
    let mut out = Vec::with_capacity((extent.width * extent.height) as usize);
    for py in 0..extent.height {
        for px in 0..extent.width {
            let centre = [f64::from(px) + 0.5, f64::from(py) + 0.5];
            let mut inside = Vec::new();
            let mut near = Vec::new();
            for &(window, colour) in &triangles {
                match side(&window, centre) {
                    Side::Inside => inside.push(colour),
                    Side::Margin => near.push(colour),
                    Side::Outside => {}
                }
            }
            if inside.len() > 1 {
                return Err(invalid(format!("pixel ({px}, {py}) lies inside two triangles")));
            }
            let inside = inside.first().copied();
            out.push(match (inside, near.is_empty()) {
                (Some(colour), true) => Expected::Drawn(colour),
                (None, true) => Expected::Clear,
                (inside, false) => Expected::Either { inside, near },
            });
        }
    }
    Ok(out)
}

/// Holds one vertex at `point`, relative to the camera's cell, to [`REACH`] and
/// [`PRODUCT`].
fn premises(slot: usize, point: [f32; 3], camera: &CameraData) -> Result<()> {
    if !point.iter().all(|coordinate| coordinate.abs() < REACH) {
        return Err(invalid(format!("slot {slot}'s vertex {point:?} lies {REACH} m out or more")));
    }
    let m = &camera.clip_from_cell;
    for row in 0..4 {
        let products =
            [m[row] * point[0], m[4 + row] * point[1], m[8 + row] * point[2], m[12 + row]];
        if !products.iter().all(|product| product.abs() < PRODUCT) {
            return Err(invalid(format!(
                "slot {slot}'s clip row {row} sums {products:?}, one {PRODUCT} or more"
            )));
        }
    }
    Ok(())
}

/// Holds `records`, as the device's scatter wrote them, to `expected`, the records the
/// placements extract handed the renderer give: slot by slot, each live in one exactly where
/// it is live in the other, and a live record's previous and current placements bit for bit
/// equal, since the scatter copies them. Returns how many are live.
///
/// # Errors
///
/// `InvalidData` naming the first slot that differs.
pub fn held_to(records: &[InstanceRecord], expected: &[InstanceRecord]) -> Result<usize> {
    if records.len() != expected.len() {
        return Err(invalid(format!("{} records against {}", records.len(), expected.len())));
    }
    let mut live = 0;
    for (slot, (record, expected)) in records.iter().zip(expected).enumerate() {
        let is_live = |record: &InstanceRecord| record.flags & InstanceRecord::LIVE != 0;
        let same = match (is_live(record), is_live(expected)) {
            (true, true) => {
                live += 1;
                record.previous == expected.previous && record.current == expected.current
            }
            (false, false) => true,
            _ => false,
        };
        if !same {
            return Err(invalid(format!(
                "slot {slot}: the scatter wrote {record:?}, extract handed {expected:?}"
            )));
        }
    }
    Ok(live)
}

/// Where `point` lies against the triangle `window`, by its distance in pixels from each
/// edge.
fn side(window: &[[f64; 2]; 3], point: [f64; 2]) -> Side {
    let [a, b, c] = *window;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let mut inside = true;
    for (from, to) in [(a, b), (b, c), (c, a)] {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let cross = dx * (point[1] - from[1]) - dy * (point[0] - from[0]);
        let distance = cross * area.signum() / dx.hypot(dy);
        if distance <= -MARGIN {
            return Side::Outside;
        }
        inside &= distance >= MARGIN;
    }
    if inside { Side::Inside } else { Side::Margin }
}

/// Holds `pixels`, `extent.width` to a row, to `reference`.
///
/// # Errors
///
/// `InvalidData` naming the first wrong pixel, what it is and what it should be.
pub fn compare(pixels: &[[u8; 4]], reference: &[Expected], extent: Extent) -> Result<Counts> {
    if pixels.len() != reference.len() {
        return Err(invalid(format!("{} pixels against {}", pixels.len(), reference.len())));
    }
    let mut counts = Counts { drawn: 0, clear: 0, margin: 0 };
    for (at, (pixel, expected)) in pixels.iter().zip(reference).enumerate() {
        let (x, y) = (at % extent.width as usize, at / extent.width as usize);
        let right = match expected {
            Expected::Drawn(colour) => {
                counts.drawn += 1;
                pixel == colour
            }
            Expected::Clear => {
                counts.clear += 1;
                *pixel == CLEAR
            }
            Expected::Either { inside, near } => {
                counts.margin += 1;
                *inside == Some(*pixel)
                    || (inside.is_none() && *pixel == CLEAR)
                    || near.contains(pixel)
            }
        };
        if !right {
            return Err(invalid(format!("pixel ({x}, {y}) is {pixel:?}, expected {expected:?}")));
        }
    }
    Ok(counts)
}

/// How many pixels of `reference` are surely drawn in each slot's colour, slot by slot over
/// the seven colours.
#[must_use]
pub fn surely_drawn(reference: &[Expected]) -> [usize; 7] {
    let mut out = [0; 7];
    for (slot, count) in out.iter_mut().enumerate() {
        *count = reference.iter().filter(|&pixel| *pixel == Expected::Drawn(colour(slot))).count();
    }
    out
}

/// Writes decision 5.1's dump of `frame` into `directory`: `last-frame.txt`, the fraction,
/// the camera, each live record extract's placements give and each live record the device
/// wrote, in text, and `last-frame.rgba`, the target's pixels as bytes, four a pixel, row by
/// row from the top left.
///
/// # Errors
///
/// The file system's.
pub fn dump(frame: &LastFrame, directory: &Path) -> Result<()> {
    let io = |error: std::io::Error| {
        Error::new(ErrorKind::Io, format!("{}: {error}", directory.display()))
    };
    std::fs::create_dir_all(directory).map_err(io)?;
    let mut text = String::new();
    let _ = writeln!(text, "extent {} {}", frame.extent.width, frame.extent.height);
    let _ = writeln!(text, "fraction {:?}", frame.fraction);
    let _ = writeln!(text, "camera cell {:?}", frame.camera.cell);
    let _ = writeln!(text, "camera clip_from_cell {:?}", frame.camera.clip_from_cell);
    for (name, records) in [("expected", &frame.expected), ("slot", &frame.records)] {
        for (slot, record) in records.iter().enumerate() {
            if record.flags & InstanceRecord::LIVE != 0 {
                let _ = writeln!(text, "{name} {slot} {record:?}");
            }
        }
    }
    std::fs::write(directory.join("last-frame.txt"), text).map_err(io)?;
    let bytes: Vec<u8> = frame.pixels.iter().flatten().copied().collect();
    std::fs::write(directory.join("last-frame.rgba"), bytes).map_err(io)
}

#[cfg(test)]
mod tests {
    use lactose_renderer::InstanceRecord;
    use lactose_rhi_api::Extent;

    use super::{CLEAR, Expected, compare, held_to};

    const ONE: Extent = Extent { width: 1, height: 1 };
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// A pixel inside one triangle by the margin and near another's edge is that triangle's
    /// colour or the other's, never the clear colour; one inside none and near an edge may
    /// be the clear colour.
    #[test]
    fn the_clear_colour_is_refused_inside_a_triangle_near_another_s_edge() {
        let inside = [Expected::Either { inside: Some(RED), near: vec![BLUE] }];
        assert!(compare(&[RED], &inside, ONE).is_ok());
        assert!(compare(&[BLUE], &inside, ONE).is_ok());
        assert!(compare(&[CLEAR], &inside, ONE).is_err());
        let outside = [Expected::Either { inside: None, near: vec![BLUE] }];
        assert!(compare(&[CLEAR], &outside, ONE).is_ok());
        assert!(compare(&[BLUE], &outside, ONE).is_ok());
        assert!(compare(&[RED], &outside, ONE).is_err());
    }

    fn record(live: bool, previous: u8, current: u8) -> InstanceRecord {
        let mut bytes = [0_u8; 128];
        bytes[0] = previous;
        bytes[40] = current;
        bytes[120] = u8::from(live);
        InstanceRecord::from_bytes(&bytes)
    }

    /// The records read back are held to extract's slot for slot: liveness, and a live
    /// record's previous and current placements; the base and a free record's bytes are
    /// not compared.
    #[test]
    fn the_records_are_held_to_the_placements_slot_for_slot() {
        let expected = [record(true, 1, 2), record(false, 0, 0), record(true, 3, 4)];
        assert_eq!(held_to(&expected, &expected).expect("the same"), 2);
        let free = [record(true, 1, 2), record(false, 9, 9), record(true, 3, 4)];
        assert_eq!(held_to(&free, &expected).expect("a free record's bytes"), 2);
        let moved = [record(true, 1, 2), record(false, 0, 0), record(true, 3, 5)];
        assert!(held_to(&moved, &expected).is_err(), "a current placement differs");
        let swapped = [record(true, 3, 4), record(false, 0, 0), record(true, 1, 2)];
        assert!(held_to(&swapped, &expected).is_err(), "two slots swapped");
        let dead = [record(true, 1, 2), record(false, 0, 0), record(false, 3, 4)];
        assert!(held_to(&dead, &expected).is_err(), "a live slot not written");
        let extra = [record(true, 1, 2), record(true, 0, 0), record(true, 3, 4)];
        assert!(held_to(&extra, &expected).is_err(), "a slot live that extract left free");
        assert!(held_to(&expected[..2], &expected).is_err(), "a record short");
    }
}
