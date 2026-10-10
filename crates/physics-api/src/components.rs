//! The components that describe a body, its collider and its joints, the two the write-back
//! writes, and the marks this crate keeps on a body's row (architecture sections 4, 15.1
//! and 16).
//!
//! Every component is laid out as its scalars first, then a tail whose size is a multiple
//! of eight bytes, so the layout holds whichever width the milestone 1 precision test picks
//! for [`Scalar`] without a padding change; `lib.rs` asserts the sizes for both widths.

use lactose_core::{EntityRef, PlainArray, StableKey};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_reflect::{Padding, ZeroByte, component, value};

/// `N` zero bytes, in a constant.
const fn pad<const N: usize>() -> Padding<N> {
    PlainArray([ZeroByte::Zero; N])
}

/// The placement of an entity in the simulation: where it is and how it is turned. No
/// scale: a scale is authoring or presentation data, and a collider's size is in its
/// [`Shape`]. The simulation's transform component for every entity, body or not; a crate
/// that needs it and no physics depends on this crate for the type, which pulls no backend.
///
/// The write-back writes it at the sync point that begins `Stage::Post` for every body the
/// step moved. A write made to it elsewhere (through `Mut` in a kernel, or `Commands::set`
/// from any system, in `Stage::Input`, `Stage::Simulation` or `Stage::Post`) is honoured by
/// the sync at the next physics sync point: a dynamic or fixed body is placed there without
/// a sweep, a kinematic body sweeps to it during the step. In `Stage::Physics` nothing but
/// the write-back may write a body's transform: the write-back overwrites it. Extracted, so
/// presentation interpolates between two of them, and replicated.
#[component(key = "physics::transform", persistent, replicated, extracted)]
#[derive(PartialEq, Debug)]
pub struct Transform {
    /// Where the entity's origin is.
    #[field(id = 1)]
    pub translation: Vec3,
    /// How the entity is turned: a unit quaternion.
    #[field(id = 2)]
    pub rotation: Quat,
}

impl Transform {
    /// At the origin, unrotated.
    pub const IDENTITY: Self = Self { translation: Vec3::ZERO, rotation: Quat::IDENTITY };

    /// A transform from its parts.
    #[must_use]
    pub const fn new(translation: Vec3, rotation: Quat) -> Self {
        Self { translation, rotation }
    }

    /// Unrotated, at `translation`.
    #[must_use]
    pub const fn at(translation: Vec3) -> Self {
        Self { translation, rotation: Quat::IDENTITY }
    }

    /// The same placement as a math transform with unit scale.
    #[must_use]
    pub const fn to_math(self) -> lactose_math::Transform {
        lactose_math::Transform::new(self.translation, self.rotation, Vec3::ONE)
    }

    /// The image of a local point: `rotation * point + translation`.
    #[must_use]
    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.rotation * point + self.translation
    }
}

/// How a body moves: its linear velocity in units per second and its angular velocity as
/// an axis scaled by radians per second, both in world space.
///
/// Written by the write-back for every body the step moved, beside [`Transform`]; a write
/// made to it elsewhere is honoured at the next physics sync point, so the section 6.1
/// `apply_gravity` kernel works on a body (set the body's `gravity_scale` to zero then, or
/// the solver adds its own). A write to a kinematic body's velocity is ignored by the
/// solver, which derives a kinematic body's velocity from where it sweeps. A dynamic or
/// kinematic body without this component gains one at the first write-back.
#[component(key = "physics::velocity", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Velocity {
    /// Units per second.
    #[field(id = 1)]
    pub linear: Vec3,
    /// Axis times radians per second.
    #[field(id = 2)]
    pub angular: Vec3,
}

impl Velocity {
    /// At rest.
    pub const ZERO: Self = Self { linear: Vec3::ZERO, angular: Vec3::ZERO };
}

/// Set on a body's row by the write-back in a tick whose step placed the body without a
/// sweep (a `Teleport` command, or a [`Transform`] row write on a dynamic or fixed body),
/// and zeroed by the ECS at the next input sync point: the no-interpolate flag extract and
/// replication read (architecture section 5.3). The sync adds it to a row when the row
/// becomes a body and removes it when the row stops being one. Transient, tracked, so a
/// `Changed<Teleported>` reader or extract sees it set and sees it cleared.
#[component(key = "physics::teleported", transient, replicated, extracted)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Teleported(#[field(id = 1)] pub bool);

/// What the solver does with a body.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum BodyType {
    /// Moved by forces, impulses and contacts.
    Dynamic = 0,
    /// Moved by the game: a [`Transform`] write sweeps it there during the step, pushing
    /// dynamic bodies out of its way; nothing pushes it.
    Kinematic = 1,
    /// Never moves. A row with a [`Collider`] and no [`RigidBody`] is this.
    Fixed = 2,
}

/// The bits of [`RigidBody::locked_axes`]: a locked axis keeps its coordinate whatever the
/// solver computes. A character capsule locks [`ROTATION`](locked_axes::ROTATION).
pub mod locked_axes {
    /// Nothing locked.
    pub const NONE: u8 = 0;
    /// Translation along x.
    pub const TRANSLATION_X: u8 = 1;
    /// Translation along y.
    pub const TRANSLATION_Y: u8 = 1 << 1;
    /// Translation along z.
    pub const TRANSLATION_Z: u8 = 1 << 2;
    /// Rotation about x.
    pub const ROTATION_X: u8 = 1 << 3;
    /// Rotation about y.
    pub const ROTATION_Y: u8 = 1 << 4;
    /// Rotation about z.
    pub const ROTATION_Z: u8 = 1 << 5;
    /// Every translation.
    pub const TRANSLATION: u8 = TRANSLATION_X | TRANSLATION_Y | TRANSLATION_Z;
    /// Every rotation.
    pub const ROTATION: u8 = ROTATION_X | ROTATION_Y | ROTATION_Z;
}

/// A body: an entity the solver moves, or that moves through the solver. The entity also
/// has a [`Transform`], and usually a [`Collider`]; a [`Velocity`] is added at the first
/// write-back if it lacks one.
///
/// Read when the body is created and whenever the row changes afterwards (the sync compares
/// against its mark at every physics sync point), so a kernel may switch a ragdoll's bones
/// from `Kinematic` to `Dynamic` by writing `body_type` through `Mut<RigidBody>`. In a
/// `Prediction` world a row with this component and no `ClaimedBy` is a proxy: a kinematic
/// body whatever `body_type` says, moved along its per-tick history by `lactose-predict`'s
/// writes to its [`Transform`] row.
#[component(key = "physics::rigid_body", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct RigidBody {
    /// The mass in mass units, or zero to take the mass the colliders' densities give.
    #[field(id = 1)]
    pub mass: Scalar,
    /// The fraction of linear velocity lost per second, zero for none.
    #[field(id = 2)]
    pub linear_damping: Scalar,
    /// The fraction of angular velocity lost per second, zero for none.
    #[field(id = 3)]
    pub angular_damping: Scalar,
    /// How much of the world's gravity acts on the body: one for all of it, zero for none.
    #[field(id = 4)]
    pub gravity_scale: Scalar,
    /// What the solver does with the body.
    #[field(id = 5)]
    pub body_type: BodyType,
    /// The [`locked_axes`] bits.
    #[field(id = 6)]
    pub locked_axes: u8,
    /// Whether the solver may put the body to sleep when it comes to rest. A sleeping body
    /// is not written back until it wakes; every row write and command on it wakes it.
    #[field(id = 7)]
    pub can_sleep: bool,
    #[field(padding)]
    _pad: Padding<5>,
}

impl RigidBody {
    const fn of_type(body_type: BodyType) -> Self {
        Self {
            mass: Scalar::ZERO,
            linear_damping: Scalar::ZERO,
            angular_damping: Scalar::ZERO,
            gravity_scale: Scalar::ONE,
            body_type,
            locked_axes: locked_axes::NONE,
            can_sleep: true,
            _pad: pad(),
        }
    }

    /// A dynamic body with the mass its colliders give, no damping, full gravity, nothing
    /// locked, allowed to sleep.
    #[must_use]
    pub const fn dynamic() -> Self {
        Self::of_type(BodyType::Dynamic)
    }

    /// A kinematic body with the same defaults.
    #[must_use]
    pub const fn kinematic() -> Self {
        Self::of_type(BodyType::Kinematic)
    }

    /// A fixed body with the same defaults. A row with a [`Collider`] alone is fixed too.
    #[must_use]
    pub const fn fixed() -> Self {
        Self::of_type(BodyType::Fixed)
    }
}

/// Which primitive a [`Shape`] is.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum ShapeKind {
    /// A ball of [`Shape::radius`].
    Sphere = 0,
    /// A capsule along the local y axis: a segment of half-length [`Shape::half_extents`]`.y`
    /// swept by [`Shape::radius`].
    Capsule = 1,
    /// A box of [`Shape::half_extents`].
    Cuboid = 2,
}

/// A collision primitive, in the local space of the collider that holds it or of the query
/// that casts it. The cooked collision meshes of static scenery are not here yet
/// (`README.md`, "Not in this crate yet").
#[value]
#[derive(PartialEq, Debug)]
pub struct Shape {
    /// Half the extent along each local axis; what each kind reads of it is in
    /// [`ShapeKind`].
    #[field(id = 1)]
    pub half_extents: Vec3,
    /// The radius of a sphere or capsule; unused by a cuboid.
    #[field(id = 2)]
    pub radius: Scalar,
    /// Which primitive.
    #[field(id = 3)]
    pub kind: ShapeKind,
    #[field(padding)]
    _pad: Padding<7>,
}

impl Shape {
    /// A ball.
    #[must_use]
    pub const fn sphere(radius: Scalar) -> Self {
        Self { half_extents: Vec3::ZERO, radius, kind: ShapeKind::Sphere, _pad: pad() }
    }

    /// A capsule along the local y axis: the segment from `-half_height` to `half_height`
    /// swept by `radius`.
    #[must_use]
    pub const fn capsule(half_height: Scalar, radius: Scalar) -> Self {
        Self {
            half_extents: Vec3::new(Scalar::ZERO, half_height, Scalar::ZERO),
            radius,
            kind: ShapeKind::Capsule,
            _pad: pad(),
        }
    }

    /// A box.
    #[must_use]
    pub const fn cuboid(half_extents: Vec3) -> Self {
        Self { half_extents, radius: Scalar::ZERO, kind: ShapeKind::Cuboid, _pad: pad() }
    }
}

/// Which colliders interact: a collider is in the groups of its `membership` bits and
/// touches a collider whose `membership` meets its `filter`, and vice versa. The same type
/// filters a query.
#[value]
#[derive(PartialEq, Eq, Debug)]
pub struct CollisionGroups {
    /// The groups this collider belongs to, one bit each.
    #[field(id = 1)]
    pub membership: u32,
    /// The groups this collider interacts with, one bit each.
    #[field(id = 2)]
    pub filter: u32,
}

impl CollisionGroups {
    /// In every group, touching every group.
    pub const ALL: Self = Self { membership: u32::MAX, filter: u32::MAX };

    /// In `membership`, touching `filter`.
    #[must_use]
    pub const fn new(membership: u32, filter: u32) -> Self {
        Self { membership, filter }
    }
}

/// The one collision shape of an entity, placed relative to its [`Transform`]. With a
/// [`RigidBody`] it is the body's collider; alone it is a fixed collider, which is what the
/// cooked static scenery of a region is. One collider per entity: a body that needs several
/// is not in this crate yet (`README.md`).
///
/// Read when the collider is created and whenever the row changes afterwards, like
/// [`RigidBody`]. A collider's shape is not in a physics snapshot: the backend rebuilds it
/// from this row at a restore.
#[component(key = "physics::collider", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Collider {
    /// The primitive.
    #[field(id = 1)]
    pub shape: Shape,
    /// Where the shape's centre is, in the entity's space.
    #[field(id = 2)]
    pub offset: Vec3,
    /// How the shape is turned, in the entity's space: a unit quaternion.
    #[field(id = 3)]
    pub rotation: Quat,
    /// The friction coefficient; two colliders' coefficients combine as the backend's rule
    /// says, the same on every machine.
    #[field(id = 4)]
    pub friction: Scalar,
    /// The restitution coefficient, zero for no bounce.
    #[field(id = 5)]
    pub restitution: Scalar,
    /// Mass per unit volume, which gives the body's mass when [`RigidBody::mass`] is zero.
    #[field(id = 6)]
    pub density: Scalar,
    /// The normal impulse per step above which a contact of this collider is reported as
    /// [`ContactPhase::Impact`](crate::ContactPhase::Impact) to this entity, when
    /// [`events`](Self::events) is set; Rapier's per-collider contact force threshold.
    #[field(id = 7)]
    pub impact_threshold: Scalar,
    /// Who it touches.
    #[field(id = 8)]
    pub groups: CollisionGroups,
    /// A sensor detects what enters it and reports [`TriggerEvent`](crate::TriggerEvent)s,
    /// but pushes nothing and is pushed by nothing.
    #[field(id = 9)]
    pub sensor: bool,
    /// Whether the pairs this collider is in report events: contact events and impacts
    /// for a solid collider, trigger events for a sensor. A pair reports if either of its
    /// colliders has it (Rapier's `ActiveEvents::COLLISION_EVENTS`, and
    /// `CONTACT_FORCE_EVENTS` for impacts). [`new`](Self::new) sets it false;
    /// [`sensor`](Self::sensor) sets it true.
    #[field(id = 10)]
    pub events: bool,
    /// An identifier game and presentation code give a surface (footstep sounds, decals,
    /// damage rules); the backend ignores it.
    #[field(id = 11)]
    pub material: u16,
    #[field(padding)]
    _pad: Padding<4>,
}

impl Collider {
    /// `shape` at the entity's origin, unrotated, with friction one half, no restitution,
    /// unit density, no impact threshold, in every group, not a sensor, reporting no
    /// events, material zero.
    #[must_use]
    pub const fn new(shape: Shape) -> Self {
        Self {
            shape,
            offset: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            friction: Scalar::HALF,
            restitution: Scalar::ZERO,
            density: Scalar::ONE,
            impact_threshold: Scalar::ZERO,
            groups: CollisionGroups::ALL,
            sensor: false,
            events: false,
            material: 0,
            _pad: pad(),
        }
    }

    /// The same collider placed at `offset` in the entity's space.
    #[must_use]
    pub const fn with_offset(mut self, offset: Vec3) -> Self {
        self.offset = offset;
        self
    }

    /// The same collider as a sensor that reports trigger events.
    #[must_use]
    pub const fn sensor(mut self) -> Self {
        self.sensor = true;
        self.events = true;
        self
    }

    /// The same collider reporting contact events, and impacts above `impact_threshold`.
    #[must_use]
    pub const fn with_events(mut self, impact_threshold: Scalar) -> Self {
        self.events = true;
        self.impact_threshold = impact_threshold;
        self
    }

    /// The same collider in `groups`.
    #[must_use]
    pub const fn with_groups(mut self, groups: CollisionGroups) -> Self {
        self.groups = groups;
        self
    }

    /// The same collider with `material`.
    #[must_use]
    pub const fn with_material(mut self, material: u16) -> Self {
        self.material = material;
        self
    }
}

/// What a [`Joint`] constrains.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum JointKind {
    /// The two anchors coincide and the bodies keep their relative orientation.
    Fixed = 0,
    /// The two anchors coincide; the bodies turn freely about it. A ragdoll's joints.
    Spherical = 1,
    /// The anchors coincide and the bodies turn about [`Joint::axis`] only, within the
    /// limits if limited.
    Revolute = 2,
    /// The bodies slide along [`Joint::axis`] only, within the limits if limited.
    Prismatic = 3,
}

/// A joint between two bodies: a row of its own, so it is spawned and despawned through
/// `Commands`, saved and replicated with the rest of the row, snapshotted and restored with
/// the world, and created in the solver by the sync when both bodies exist. Spawn it owned
/// by one of its bodies (`OwnedBy`) so it migrates and despawns with that body.
///
/// A joint whose bodies are not both live bodies at a physics sync point waits, and is
/// retried at every sync point until they are; one whose reference is empty (a body
/// despawned, so its [`EntityRef`] was emptied at apply) is removed from the solver and not
/// retried until the `Joint` row changes. A joint between a frozen body and a simulated
/// one is a joint to a fixed body. The reference fields are written through `Commands::set`
/// or the direct methods only, like every relationship. A limited joint evaluates
/// transcendentals in the backend: `README.md`, "The backend's obligations", says what the
/// backend lane must show before `limited` is relied on across architectures.
#[component(key = "physics::joint", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Joint {
    /// The anchor in the first body's space.
    #[field(id = 1)]
    pub anchor_a: Vec3,
    /// The anchor in the second body's space.
    #[field(id = 2)]
    pub anchor_b: Vec3,
    /// The axis of a revolute or prismatic joint, in the first body's space; unused
    /// otherwise.
    #[field(id = 3)]
    pub axis: Vec3,
    /// The lower limit, in radians for a revolute joint and units for a prismatic one, when
    /// `limited`.
    #[field(id = 4)]
    pub limit_min: Scalar,
    /// The upper limit, likewise.
    #[field(id = 5)]
    pub limit_max: Scalar,
    /// The first body.
    #[field(id = 6)]
    pub body_a: EntityRef,
    /// The second body.
    #[field(id = 7)]
    pub body_b: EntityRef,
    /// What the joint constrains.
    #[field(id = 8)]
    pub kind: JointKind,
    /// Whether the limits apply.
    #[field(id = 9)]
    pub limited: bool,
    #[field(padding)]
    _pad: Padding<6>,
}

impl Joint {
    /// A joint of `kind` between `body_a` and `body_b` with the anchors at the bodies'
    /// origins, the axis along local y and no limits.
    #[must_use]
    pub const fn new(kind: JointKind, body_a: StableKey, body_b: StableKey) -> Self {
        Self {
            anchor_a: Vec3::ZERO,
            anchor_b: Vec3::ZERO,
            axis: Vec3::Y,
            limit_min: Scalar::ZERO,
            limit_max: Scalar::ZERO,
            body_a: EntityRef::new(body_a),
            body_b: EntityRef::new(body_b),
            kind,
            limited: false,
            _pad: pad(),
        }
    }

    /// The same joint with its anchors.
    #[must_use]
    pub const fn with_anchors(mut self, anchor_a: Vec3, anchor_b: Vec3) -> Self {
        self.anchor_a = anchor_a;
        self.anchor_b = anchor_b;
        self
    }

    /// The same joint about `axis`, in the first body's space.
    #[must_use]
    pub const fn with_axis(mut self, axis: Vec3) -> Self {
        self.axis = axis;
        self
    }

    /// The same joint limited to `[min, max]`.
    #[must_use]
    pub const fn with_limits(mut self, min: Scalar, max: Scalar) -> Self {
        self.limit_min = min;
        self.limit_max = max;
        self.limited = true;
        self
    }
}

/// The backend's handle of the body the sync created for this row: present exactly while
/// the row is a body in the solver (`README.md`, "Bodies"). Rebuilt on load and never
/// saved, hashed or replicated (architecture section 15.1); in a prediction-world snapshot
/// like every column, but nothing trusts the column: the sync and `restore` work from the
/// bookkeeping [`Physics`](crate::Physics) carries, and a forged, removed or edited handle
/// row changes nothing but what `Option<&BodyHandle>` tells a kernel. Opaque to game code,
/// which keys everything by `StableKey`; the constructor and accessors are the backend's.
#[component(key = "physics::body_handle", derived)]
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct BodyHandle {
    #[field(id = 1)]
    index: u32,
    #[field(id = 2)]
    generation: u32,
}

impl BodyHandle {
    /// A handle from the backend's index and generation. The backend's; not part of the
    /// interface game code writes against.
    #[doc(hidden)]
    #[must_use]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The backend's index. The backend's.
    #[doc(hidden)]
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The backend's generation of the index. The backend's.
    #[doc(hidden)]
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// The backend's handle of the joint the sync created for this [`Joint`] row: present
/// exactly while the joint is in the solver, which is while both its bodies are. Derived
/// and untrusted, like [`BodyHandle`].
#[component(key = "physics::joint_handle", derived)]
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct JointHandle {
    #[field(id = 1)]
    index: u32,
    #[field(id = 2)]
    generation: u32,
}

impl JointHandle {
    /// A handle from the backend's index and generation. The backend's.
    #[doc(hidden)]
    #[must_use]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The backend's index. The backend's.
    #[doc(hidden)]
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The backend's generation of the index. The backend's.
    #[doc(hidden)]
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}
