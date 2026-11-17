//! The presentation components (architecture section 13): what an entity looks like, as the
//! cooker or a server spawn authors it. Each is `persistent`, `replicated` and `extracted`:
//! a region's baseline and a save hold it, a client receives it for an entity it did not
//! spawn, and extract copies it to the render side. So each is hashed state, and holds
//! integers, `lactose_math` quantities and asset references alone, never a raw float
//! (`README.md`, "The components").
//!
//! Every field is aligned to eight bytes or closed with explicit padding at `Scalar`'s 64
//! bits (D146); `lib.rs` asserts the sizes.

use lactose_asset_api::AssetRef;
use lactose_core::PlainArray;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_reflect::{Padding, ZeroByte, component, value};

/// `N` zero bytes, in a constant.
pub(crate) const fn pad<const N: usize>() -> Padding<N> {
    PlainArray([ZeroByte::Zero; N])
}

/// The marker a reference to a cooked mesh is typed by. The runtime mesh type and its
/// loader are the renderer's, which this crate cannot name; a reader re-types a reference
/// with `AssetRef::new(reference.id())` for the type that owns the kind, and a lookup
/// through a reference of the wrong kind fails there with `ErrorKind::InvalidData`.
pub enum MeshAsset {}

/// The marker a reference to a cooked material is typed by; see [`MeshAsset`].
pub enum MaterialAsset {}

/// The mesh an entity is drawn with. A row that has this and a
/// [`Transform`](lactose_physics_api::Transform) is an *instance*: extract writes it to the
/// [`InstanceSink`](crate::InstanceSink), and it is the only kind of row the sink is
/// given.
///
/// The mesh must be in the region's asset set (architecture section 14). An empty
/// reference draws nothing and is still an instance. There is no scale and no material
/// here: a cooked mesh names its materials, and neither a per-instance scale nor a
/// material override has a reader before milestone 3 (`README.md`, "Not in this crate
/// yet").
#[component(key = "present::mesh", persistent, replicated, extracted)]
#[derive(PartialEq, Eq, Debug)]
pub struct MeshInstance {
    /// The mesh.
    #[field(id = 1)]
    pub mesh: AssetRef<MeshAsset>,
}

impl MeshInstance {
    /// An instance of `mesh`.
    #[must_use]
    pub const fn new(mesh: AssetRef<MeshAsset>) -> Self {
        Self { mesh }
    }
}

/// How a [`Light`] spreads.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum LightKind {
    /// The same in every direction from the entity's origin.
    Point = 0,
    /// A cone about the entity's local -Z axis, the direction a yaw of zero faces.
    Spot = 1,
}

/// A light at the entity's [`Transform`](lactose_physics_api::Transform). A row that has
/// this and no transform lights nothing.
///
/// The quantities are the authored ones, in the simulation's units; nothing here validates
/// a write. The ranges below are what the cooker emits, and what a reader does with a
/// value outside them is the reader's to say (the milestone 3 renderer): the sun, shadows
/// and light profiles are not here yet.
#[component(key = "present::light", persistent, replicated, extracted)]
#[derive(PartialEq, Debug)]
pub struct Light {
    /// The colour, linear RGB in `x`, `y` and `z`, each at least zero; not clamped to one.
    #[field(id = 1)]
    pub color: Vec3,
    /// The luminous intensity along the light's brightest direction, in candela, at least
    /// zero.
    #[field(id = 2)]
    pub intensity: Scalar,
    /// The distance in metres beyond which the light contributes nothing, at least zero.
    #[field(id = 3)]
    pub range: Scalar,
    /// For a spot, the half-angle in radians within which the light is at full intensity;
    /// zero for a point light.
    #[field(id = 4)]
    pub inner_angle: Scalar,
    /// For a spot, the half-angle in radians beyond which the light contributes nothing,
    /// at least `inner_angle` and below a quarter turn; zero for a point light.
    #[field(id = 5)]
    pub outer_angle: Scalar,
    /// How the light spreads.
    #[field(id = 6)]
    pub kind: LightKind,
    #[field(padding)]
    _pad: Padding<7>,
}

impl Light {
    /// A point light.
    #[must_use]
    pub const fn point(color: Vec3, intensity: Scalar, range: Scalar) -> Self {
        Self {
            color,
            intensity,
            range,
            inner_angle: Scalar::ZERO,
            outer_angle: Scalar::ZERO,
            kind: LightKind::Point,
            _pad: pad(),
        }
    }

    /// A spot light about the entity's local -Z axis.
    #[must_use]
    pub const fn spot(
        color: Vec3,
        intensity: Scalar,
        range: Scalar,
        inner_angle: Scalar,
        outer_angle: Scalar,
    ) -> Self {
        Self {
            color,
            intensity,
            range,
            inner_angle,
            outer_angle,
            kind: LightKind::Spot,
            _pad: pad(),
        }
    }
}

/// A decal: a material projected onto the surfaces inside a box. The box is centred on the
/// entity's origin and turned by its rotation, and the material is projected along the
/// entity's local -Z axis. A row that has this and no
/// [`Transform`](lactose_physics_api::Transform) projects nothing.
#[component(key = "present::decal", persistent, replicated, extracted)]
#[derive(PartialEq, Debug)]
pub struct Decal {
    /// Half the box's extent along each local axis, in metres, each at least zero.
    #[field(id = 1)]
    pub half_extents: Vec3,
    /// The material projected. It must be in the region's asset set; an empty reference
    /// projects nothing.
    #[field(id = 2)]
    pub material: AssetRef<MaterialAsset>,
}

impl Decal {
    /// A decal of `material` in a box of `half_extents`.
    #[must_use]
    pub const fn new(material: AssetRef<MaterialAsset>, half_extents: Vec3) -> Self {
        Self { half_extents, material }
    }
}

/// What an orbit camera follows on this entity: the point it turns about and how far back
/// it rests. This is the authored half of the camera rig and the only half a world holds.
/// The orbit's yaw and pitch are `lactose-input-host`'s latest sample, and the boom's
/// length this frame, with its collision, is `lactose-presentation`'s own state, evaluated
/// per frame and never authoritative (architecture section 13); neither is a component, so
/// presentation declares nothing a world registers.
///
/// Presentation reads it from the presentation snapshot beside the row's transforms
/// ([`PresentFrame::transform`](crate::PresentFrame::transform)): the camera pivot the
/// snapshot holds is [`pivot_of`](Self::pivot_of) the interpolated transform.
#[component(key = "present::camera_rig", persistent, replicated, extracted)]
#[derive(PartialEq, Debug)]
pub struct CameraRig {
    /// The pivot, as an offset from the entity's origin in the entity's local frame, in
    /// metres.
    #[field(id = 1)]
    pub pivot: Vec3,
    /// The boom's rest length in metres: how far behind the pivot the camera sits when
    /// nothing is in the way, at least zero.
    #[field(id = 2)]
    pub boom: Scalar,
}

impl CameraRig {
    /// A rig that turns about `pivot` and rests `boom` metres behind it.
    #[must_use]
    pub const fn new(pivot: Vec3, boom: Scalar) -> Self {
        Self { pivot, boom }
    }

    /// The pivot in simulation space for an entity placed at `transform`:
    /// `transform.translation + transform.rotation * self.pivot`, computed through
    /// `lactose_math`. The one statement of the frame the offset is in, so presentation
    /// and a tool agree on it.
    #[must_use]
    pub fn pivot_of(&self, transform: &Transform) -> Vec3 {
        transform.translation + transform.rotation * self.pivot
    }
}
