//! Decision 5.1's CPU reference: the last fixed frame rasterised from its dump, the
//! renderer's instance records, the latched camera and the fraction, and the target's
//! pixels held to it at a quarter of a pixel's margin. `lactose-renderer`'s device test,
//! "The CPU reference", is the same method on its own scenes.

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
/// 32-bit floats, and differs only in the order and the fusion of its operations. The scene
/// lies within six metres of the origin, so every coordinate is under ten and every clip
/// coordinate a sum of four products each under about a hundred: the two differ by a few
/// times 1e-5 at most, which the division by a `w` of ten or more and the few hundred pixels
/// of half the extent leave under 1e-3 of a pixel. A quarter of a pixel holds both five
/// times over, and is still far less than the pixel that decision 5.1's mutation moves a
/// vertex by.
pub const MARGIN: f64 = 0.25;

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
    /// Within the margin of an edge: the clear colour or the colour of a triangle whose edge
    /// it is near or that it is inside.
    Either(Vec<[u8; 4]>),
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
/// outside the depth range, a triangle of a pixel or less, or a pixel's centre inside two
/// triangles.
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
            let [x, y, z, w] = camera.clip(drawn.apply(vertex)).map(f64::from);
            if !(w > 0.0 && z > 0.0 && z < w) {
                return Err(invalid(format!("slot {slot} lies outside the depth range")));
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
            out.push(match (inside.as_slice(), near.is_empty()) {
                (&[colour], true) => Expected::Drawn(colour),
                ([], true) => Expected::Clear,
                _ => Expected::Either([inside, near].concat()),
            });
        }
    }
    Ok(out)
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
            Expected::Either(near) => {
                counts.margin += 1;
                *pixel == CLEAR || near.contains(pixel)
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
/// the camera and each live record in text, and `last-frame.rgba`, the target's pixels as
/// bytes, four a pixel, row by row from the top left.
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
    for (slot, record) in frame.records.iter().enumerate() {
        if record.flags & InstanceRecord::LIVE != 0 {
            let _ = writeln!(text, "slot {slot} {record:?}");
        }
    }
    std::fs::write(directory.join("last-frame.txt"), text).map_err(io)?;
    let bytes: Vec<u8> = frame.pixels.iter().flatten().copied().collect();
    std::fs::write(directory.join("last-frame.rgba"), bytes).map_err(io)
}
