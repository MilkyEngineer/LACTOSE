//! The glTF reader: the JSON form of glTF 2.0 read with serde_json, and of it only what a sim
//! skeleton needs (`README.md`, "The glTF reader"): the nodes' names, children, transforms
//! and `extras`, and the skins' joints. Buffers, accessors, meshes, animations and every
//! other property are not read, so a source whose buffers are missing still reads.

use std::borrow::Cow;

use lactose_core::{Error, ErrorKind, Result};
use lactose_physics_api::UNIT_TOLERANCE;
use serde::Deserialize;

use crate::json::{self, Extras};
use crate::space::Pose;

/// How far from one a squared length may be and still count as one: the tolerance of a unit
/// rotation, `UNIT_TOLERANCE`, applied to rotations, to scale factors and to a matrix's
/// columns alike.
const TOLERANCE: f64 = UNIT_TOLERANCE.to_f64();

/// Why a scale is refused, and the remedy: body values and clip samples are authored in the
/// scaled unit too, so the cooker does not bake a scale into the rests alone.
const UNSCALED: &str = "a sim bone's space is not scaled; apply the scale to the armature \
                        before export";

/// A parsed `.gltf` file. Every object the reader reads must be written as an object
/// (`json::object`).
#[derive(Deserialize)]
pub(crate) struct Document {
    #[serde(deserialize_with = "json::object")]
    asset: AssetInfo,
    #[serde(default, deserialize_with = "json::objects")]
    pub(crate) nodes: Vec<Node>,
    #[serde(default, deserialize_with = "json::objects")]
    skins: Vec<Skin>,
}

#[derive(Deserialize)]
struct AssetInfo {
    version: String,
}

/// One node of the document.
#[derive(Deserialize)]
pub(crate) struct Node {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    children: Vec<usize>,
    translation: Option<[f64; 3]>,
    rotation: Option<[f64; 4]>,
    scale: Option<[f64; 3]>,
    matrix: Option<[f64; 16]>,
    /// Any JSON value, as glTF allows; the cooker reads one key of an object.
    #[serde(default)]
    pub(crate) extras: Extras,
}

#[derive(Deserialize)]
struct Skin {
    joints: Vec<usize>,
}

/// The node hierarchy of a document, checked to be a forest.
pub(crate) struct Hierarchy {
    /// Each node's parent.
    parents: Vec<Option<usize>>,
    /// Every node, each before its children, the roots in index order and children in the
    /// order their parent lists them.
    pub(crate) preorder: Vec<usize>,
}

impl Document {
    /// Parses a `.gltf` file: JSON of glTF major version 2. A `.glb` never reaches here: the
    /// processor refuses it by its magic.
    pub(crate) fn parse(source: &[u8]) -> Result<Self> {
        let mut deserializer = serde_json::Deserializer::from_slice(source);
        let document: Self = json::object(&mut deserializer)
            .and_then(|document| deserializer.end().map(|()| document))
            .map_err(|error| invalid(format!("not the JSON of a glTF document: {error}")))?;
        if !document.asset.version.starts_with("2.") {
            return Err(invalid(format!(
                "glTF version {}, where the reader reads version 2",
                document.asset.version
            )));
        }
        Ok(document)
    }

    /// `node` as an error names it: its index, then its name if it has one.
    pub(crate) fn label(&self, node: usize) -> String {
        match self.nodes.get(node).and_then(|node| node.name.as_deref()) {
            Some(name) => format!("node {node} `{name}`"),
            None => format!("node {node}"),
        }
    }

    /// The nodes' hierarchy, refused when it is not a forest: a child index past the nodes,
    /// a node listed as a child twice (two parents, or one parent twice), or a node whose
    /// parents never reach a root (the first such node by index is named).
    pub(crate) fn hierarchy(&self) -> Result<Hierarchy> {
        let count = self.nodes.len();
        let mut parents = vec![None; count];
        for (parent, node) in self.nodes.iter().enumerate() {
            for &child in &node.children {
                if child >= count {
                    return Err(self.refuse(parent, format!("child {child} is past the nodes")));
                }
                if let Some(first) = parents[child].replace(parent) {
                    return Err(self.refuse(
                        child,
                        format!("a child of both {} and {}", self.label(first), self.label(parent)),
                    ));
                }
            }
        }
        // With one parent each, the walk from the roots ends, and it reaches every node whose
        // parents lead to a root: a node it misses is in a cycle of parents, or below one.
        let mut preorder = Vec::with_capacity(count);
        let mut reached = vec![false; count];
        let mut stack: Vec<usize> =
            (0..count).filter(|&node| parents[node].is_none()).rev().collect();
        while let Some(node) = stack.pop() {
            preorder.push(node);
            reached[node] = true;
            stack.extend(self.nodes[node].children.iter().rev());
        }
        if let Some(node) = reached.iter().position(|&reached| !reached) {
            return Err(self.refuse(node, "its parents form a cycle or lead into one"));
        }
        Ok(Hierarchy { parents, preorder })
    }

    /// Whether `node` is a joint of a skin, or the document has no skin. A document's skins
    /// are its rigs, so where it has any, a bone is one of their joints.
    pub(crate) fn in_rig(&self, node: usize) -> Result<bool> {
        for (index, skin) in self.skins.iter().enumerate() {
            if let Some(&joint) = skin.joints.iter().find(|&&joint| joint >= self.nodes.len()) {
                return Err(invalid(format!("skin {index}: joint {joint} is past the nodes")));
            }
        }
        Ok(self.skins.is_empty() || self.skins.iter().any(|skin| skin.joints.contains(&node)))
    }

    /// The transform `node` gives its children, refused when a sim skeleton cannot hold it:
    /// a matrix beside a translation, rotation or scale; a rotation that is not a unit
    /// quaternion; a scale that is not one in every axis, uniform or not, a negative factor
    /// included; a matrix that is projective, scales, shears or mirrors. Each is within
    /// [`TOLERANCE`] of the exact rule.
    pub(crate) fn local(&self, node: usize) -> Result<Pose> {
        let source = &self.nodes[node];
        let Some(m) = source.matrix else {
            let translation = source.translation.unwrap_or([0.0; 3]);
            let rotation = source.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
            if !is_unit(rotation.iter().map(|c| c * c).sum()) {
                return Err(self
                    .refuse(node, format!("the rotation {rotation:?} is not a unit quaternion")));
            }
            if let Some(scale) = source.scale
                && !scale.iter().all(|&s| s > 0.0 && is_unit(s * s))
            {
                let what = if scale.iter().all(|&s| s == scale[0]) {
                    format!("the scale {} on every axis, not one", scale[0])
                } else {
                    format!("the non-uniform scale {scale:?}")
                };
                return Err(self.refuse(node, format!("{what}: {UNSCALED}")));
            }
            return Ok(Pose { translation, rotation });
        };
        if source.translation.is_some() || source.rotation.is_some() || source.scale.is_some() {
            return Err(self.refuse(node, "both a matrix and a translation, rotation or scale"));
        }
        // Column-major: the columns are the images of the axes, then the translation.
        let column = |c: usize| [m[4 * c], m[4 * c + 1], m[4 * c + 2]];
        let [x, y, z] = [column(0), column(1), column(2)];
        if [m[3], m[7], m[11], m[15]] != [0.0, 0.0, 0.0, 1.0] {
            return Err(self.refuse(node, "a projective matrix"));
        }
        if ![x, y, z].iter().all(|axis| is_unit(dot(*axis, *axis))) {
            return Err(self.refuse(node, format!("a matrix that scales: {UNSCALED}")));
        }
        if [dot(x, y), dot(y, z), dot(z, x)].iter().any(|d| d.abs() >= TOLERANCE) {
            return Err(self.refuse(node, "a matrix that shears"));
        }
        if dot(x, cross(y, z)) <= 0.0 {
            return Err(self.refuse(node, "a matrix that mirrors"));
        }
        Ok(Pose { translation: column(3), rotation: quaternion([x, y, z]) })
    }

    /// An `InvalidData` error that names `node`.
    pub(crate) fn refuse(&self, node: usize, what: impl AsRef<str>) -> Error {
        invalid(format!("{}: {}", self.label(node), what.as_ref()))
    }
}

impl Hierarchy {
    /// `node`'s parent, or `None` for a root.
    pub(crate) fn parent(&self, node: usize) -> Option<usize> {
        self.parents[node]
    }
}

fn is_unit(length_squared: f64) -> bool {
    (length_squared - 1.0).abs() < TOLERANCE
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The rotation of the orthonormal, right-handed `columns`, by the branch on the largest of
/// the trace and the diagonal, so the square root is of a value at least one.
fn quaternion(columns: [[f64; 3]; 3]) -> [f64; 4] {
    // `r[i][j]` is row `i` of column `j`.
    let r = |i: usize, j: usize| columns[j][i];
    let trace = r(0, 0) + r(1, 1) + r(2, 2);
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(r(2, 1) - r(1, 2)) / s, (r(0, 2) - r(2, 0)) / s, (r(1, 0) - r(0, 1)) / s, 0.25 * s]
    } else if r(0, 0) > r(1, 1) && r(0, 0) > r(2, 2) {
        let s = (1.0 + r(0, 0) - r(1, 1) - r(2, 2)).sqrt() * 2.0;
        [0.25 * s, (r(0, 1) + r(1, 0)) / s, (r(0, 2) + r(2, 0)) / s, (r(2, 1) - r(1, 2)) / s]
    } else if r(1, 1) > r(2, 2) {
        let s = (1.0 + r(1, 1) - r(0, 0) - r(2, 2)).sqrt() * 2.0;
        [(r(0, 1) + r(1, 0)) / s, 0.25 * s, (r(1, 2) + r(2, 1)) / s, (r(0, 2) - r(2, 0)) / s]
    } else {
        let s = (1.0 + r(2, 2) - r(0, 0) - r(1, 1)).sqrt() * 2.0;
        [(r(0, 2) + r(2, 0)) / s, (r(1, 2) + r(2, 1)) / s, 0.25 * s, (r(1, 0) - r(0, 1)) / s]
    }
}

pub(crate) fn invalid(message: impl Into<Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter turn about each axis, and the half turns, from their matrices.
    #[test]
    fn a_rotation_matrix_gives_its_quaternion() {
        let h = core::f64::consts::FRAC_1_SQRT_2;
        let cases: [([[f64; 3]; 3], [f64; 4]); 4] = [
            ([[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]], [h, 0.0, 0.0, h]),
            ([[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]], [0.0, h, 0.0, h]),
            ([[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]], [0.0, 1.0, 0.0, 0.0]),
            ([[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]], [1.0, 0.0, 0.0, 0.0]),
        ];
        for (columns, expected) in cases {
            let q = quaternion(columns);
            let error: f64 = q.iter().zip(expected).map(|(a, b)| (a - b).abs()).sum();
            assert!(error < 1e-15, "{columns:?} gave {q:?}, not {expected:?}");
        }
    }
}
