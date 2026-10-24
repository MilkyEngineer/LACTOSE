//! The snapshot bytes (`README.md`, "The snapshot bytes"): a four-byte tag, then the
//! postcard encoding of the bookkeeping and every serialised part of Rapier's world, with
//! the collider set's shapes carried as `Shape` values.
//!
//! Rapier's `ColliderSet` keeps its fields private, so the shapes are swapped through an
//! image of its serde layout: the set is encoded with Rapier's own derive, decoded as
//! [`SetImage`] (whose fields mirror the set's, in order, with the shape type a parameter),
//! and each `SharedShape` exchanged for the `Shape` it was built from. Reading runs the
//! same exchange backwards and decodes the result with Rapier's derive. Postcard is not
//! self-describing, so the image is correct exactly when its field order and types match
//! the pinned Rapier's; the round-trip test of the collider set's bytes checks it.

use std::sync::Arc;

use lactose_core::{Error, ErrorKind, Result};
use lactose_physics_api::Shape;
use rapier3d_f64::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet,
    RigidBodySet, SoftBodySet, SoftMeshRef,
};
use rapier3d_f64::geometry::{
    BroadPhaseBvh, ColliderChanges, ColliderFlags, ColliderHandle, ColliderMassProps,
    ColliderMaterial, ColliderParent, ColliderPosition, ColliderSet, ColliderType,
    ModifiedColliders, NarrowPhase, SharedShape,
};
use rapier3d_f64::math::Vector;
use rapier3d_f64::pipeline::{CollisionPipeline, PhysicsPipeline, PhysicsWorld};
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};

use crate::backend::Rapier;
use crate::book::{Book, Plain};
use crate::convert::{key_code, shape_of, shared_shape};

/// The first bytes of every snapshot: the format and its version.
const TAG: &[u8; 4] = b"LPR1";

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Rapier's `Collider`, field for field, with the shape type a parameter.
#[derive(Serialize, Deserialize)]
struct ColliderImage<S> {
    coll_type: ColliderType,
    shape: S,
    mprops: ColliderMassProps,
    changes: ColliderChanges,
    parent: Option<ColliderParent>,
    pos: ColliderPosition,
    material: ColliderMaterial,
    flags: ColliderFlags,
    contact_skin: f64,
    contact_force_event_threshold: f64,
    deformable_mesh_ref: Option<SoftMeshRef>,
    user_data: u128,
}

/// Rapier's arena entry.
#[derive(Serialize, Deserialize)]
enum EntryImage<T> {
    Free { next_free: Option<u32> },
    Occupied { generation: u32, value: T },
}

/// Rapier's arena.
#[derive(Serialize, Deserialize)]
struct ArenaImage<T> {
    items: Vec<EntryImage<T>>,
    generation: u32,
    free_list_head: Option<u32>,
    len: usize,
}

/// Rapier's `ColliderSet`.
#[derive(Serialize, Deserialize)]
struct SetImage<S> {
    colliders: ArenaImage<ColliderImage<S>>,
    modified_colliders: ModifiedColliders,
    removed_colliders: Vec<ColliderHandle>,
}

impl<S> SetImage<S> {
    /// The image with each shape exchanged by `swap`, or `None` when one cannot be.
    fn map<T>(self, swap: impl Fn(S) -> Option<T>) -> Option<SetImage<T>> {
        let mut items = Vec::with_capacity(self.colliders.items.len());
        for entry in self.colliders.items {
            items.push(match entry {
                EntryImage::Free { next_free } => EntryImage::Free { next_free },
                EntryImage::Occupied { generation, value: c } => EntryImage::Occupied {
                    generation,
                    value: ColliderImage {
                        coll_type: c.coll_type,
                        shape: swap(c.shape)?,
                        mprops: c.mprops,
                        changes: c.changes,
                        parent: c.parent,
                        pos: c.pos,
                        material: c.material,
                        flags: c.flags,
                        contact_skin: c.contact_skin,
                        contact_force_event_threshold: c.contact_force_event_threshold,
                        deformable_mesh_ref: c.deformable_mesh_ref,
                        user_data: c.user_data,
                    },
                },
            });
        }
        Some(SetImage {
            colliders: ArenaImage {
                items,
                generation: self.colliders.generation,
                free_list_head: self.colliders.free_list_head,
                len: self.colliders.len,
            },
            modified_colliders: self.modified_colliders,
            removed_colliders: self.removed_colliders,
        })
    }
}

/// What is written, by reference, in the order it is read.
#[derive(Serialize)]
struct WorldRef<'a> {
    book: &'a Book,
    gravity: Vector,
    integration_parameters: &'a IntegrationParameters,
    islands: &'a IslandManager,
    broad_phase: &'a BroadPhaseBvh,
    narrow_phase: &'a NarrowPhase,
    bodies: &'a RigidBodySet,
    colliders: SetImage<Plain<Shape>>,
    impulse_joints: &'a ImpulseJointSet,
    multibody_joints: &'a MultibodyJointSet,
    soft_bodies: &'a SoftBodySet,
}

/// What is read.
#[derive(Deserialize)]
struct WorldOwned {
    book: Book,
    gravity: Vector,
    integration_parameters: IntegrationParameters,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: SetImage<Plain<Shape>>,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
}

/// The collider set with its shapes as values. Fails on a shape this crate did not build,
/// which no call can put in the world.
fn shapes_out(colliders: &ColliderSet) -> Result<SetImage<Plain<Shape>>> {
    let bytes =
        postcard::to_allocvec(colliders).map_err(|_| invalid("the collider set did not encode"))?;
    let image: SetImage<SharedShape> = postcard::from_bytes(&bytes)
        .map_err(|_| invalid("the collider set's image does not match the solver's layout"))?;
    image
        .map(|shape| shape_of(&shape).map(Plain))
        .ok_or_else(|| invalid("a collider's shape is not one this backend builds"))
}

/// The collider set from its image, each shape built from its value by the constructor
/// `set_collider` uses.
fn shapes_in(image: SetImage<Plain<Shape>>) -> Result<ColliderSet> {
    let image = image
        .map(|Plain(shape)| shape.validate().is_ok().then(|| shared_shape(&shape)))
        .ok_or_else(|| invalid("a carried shape is not a valid Shape"))?;
    let bytes =
        postcard::to_allocvec(&image).map_err(|_| invalid("the collider set did not encode"))?;
    postcard::from_bytes(&bytes).map_err(|_| invalid("not the bytes of a collider set"))
}

pub(crate) fn write(backend: &Rapier, out: &mut Vec<u8>) -> Result<()> {
    let world = &backend.world;
    let image = WorldRef {
        book: &backend.book,
        gravity: world.gravity,
        integration_parameters: &world.integration_parameters,
        islands: &world.islands,
        broad_phase: &world.broad_phase,
        narrow_phase: &world.narrow_phase,
        bodies: &world.bodies,
        colliders: shapes_out(&world.colliders)?,
        impulse_joints: &world.impulse_joints,
        multibody_joints: &world.multibody_joints,
        soft_bodies: &world.soft_bodies,
    };
    let start = out.len();
    out.extend_from_slice(TAG);
    if postcard::serialize_with_flavor(&image, Append(out)).is_err() {
        out.truncate(start);
        return Err(invalid("the physics world did not encode"));
    }
    Ok(())
}

/// A postcard output that appends to the caller's vector, so the bytes are written once.
struct Append<'a>(&'a mut Vec<u8>);

impl postcard::ser_flavors::Flavor for Append<'_> {
    type Output = ();

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.0.extend_from_slice(data);
        Ok(())
    }

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.0.push(data);
        Ok(())
    }

    fn finalize(self) -> postcard::Result<()> {
        Ok(())
    }
}

/// The world and bookkeeping `bytes` hold, checked against each other, with a fresh
/// pipeline on `pool`.
pub(crate) fn read(bytes: &[u8], pool: &Arc<ThreadPool>) -> Result<(PhysicsWorld, Book)> {
    let body = bytes.strip_prefix(TAG).ok_or_else(|| invalid("not a physics snapshot"))?;
    let (image, rest): (WorldOwned, _) =
        postcard::take_from_bytes(body).map_err(|_| invalid("not the bytes of a physics world"))?;
    if !rest.is_empty() {
        return Err(invalid("bytes after the physics world"));
    }
    let mut world = PhysicsWorld {
        gravity: image.gravity,
        integration_parameters: image.integration_parameters,
        physics_pipeline: PhysicsPipeline::new(),
        collision_pipeline: CollisionPipeline::new(),
        islands: image.islands,
        broad_phase: image.broad_phase,
        narrow_phase: image.narrow_phase,
        bodies: image.bodies,
        colliders: shapes_in(image.colliders)?,
        impulse_joints: image.impulse_joints,
        multibody_joints: image.multibody_joints,
        soft_bodies: image.soft_bodies,
        ccd_solver: CCDSolver::new(),
    };
    world.set_thread_pool(Some(Arc::clone(pool)));
    check(&world, &image.book)?;
    Ok((world, image.book))
}

/// Whether the bookkeeping and the world name each other: every handle the bookkeeping
/// holds is in the world under its key, and the world holds nothing else.
fn check(world: &PhysicsWorld, book: &Book) -> Result<()> {
    let mismatch = || invalid("the bookkeeping and the physics world disagree");
    let (mut bodies, mut colliders, mut joints) = (0, 0, 0);
    for (key, entry) in &book.bodies {
        let code = key_code(key.0);
        let Some(live) = entry.live else {
            if entry.transform.is_none() {
                return Err(mismatch());
            }
            continue;
        };
        let body = world.bodies.get(live.body).ok_or_else(mismatch)?;
        if body.user_data != code {
            return Err(mismatch());
        }
        bodies += 1;
        if let Some(handle) = live.collider {
            let collider = world.colliders.get(handle).ok_or_else(mismatch)?;
            if collider.user_data != code || collider.parent() != Some(live.body) {
                return Err(mismatch());
            }
            colliders += 1;
        }
    }
    for (key, live) in &book.doomed_bodies {
        let body = world.bodies.get(live.body).ok_or_else(mismatch)?;
        if body.user_data != key_code(key.0) {
            return Err(mismatch());
        }
        bodies += 1;
        colliders += usize::from(live.collider.is_some());
    }
    for (key, entry) in &book.joints {
        if !book.bodies.contains_key(&entry.body_a) || !book.bodies.contains_key(&entry.body_b) {
            return Err(mismatch());
        }
        if let Some(handle) = entry.handle {
            let joint = world.impulse_joints.get(handle).ok_or_else(mismatch)?;
            if joint.data.user_data != key_code(key.0) {
                return Err(mismatch());
            }
            joints += 1;
        }
    }
    for (key, &handle) in &book.doomed_joints {
        let joint = world.impulse_joints.get(handle).ok_or_else(mismatch)?;
        if joint.data.user_data != key_code(key.0) {
            return Err(mismatch());
        }
        joints += 1;
    }
    let dirty_known = book.dirty.iter().all(|key| book.bodies.contains_key(key));
    let counts = world.bodies.len() == bodies
        && world.colliders.len() == colliders
        && world.impulse_joints.len() == joints
        && world.multibody_joints.iter().next().is_none()
        && world.soft_bodies.is_empty();
    if dirty_known && counts { Ok(()) } else { Err(mismatch()) }
}

#[cfg(test)]
mod tests {
    use lactose_core::{NetId, StableKey, Tick};
    use lactose_math::{Scalar, Vec3};
    use lactose_physics_api::{
        BodyDesc, Collider, PhysicsBackend, RigidBody, StepInput, StepOutput, Transform,
    };

    use super::*;

    fn key(n: u64) -> StableKey {
        StableKey::Net(NetId::cooked(n))
    }

    /// A backend holding one body of each shape and a fixed cuboid, stepped once, and a
    /// second body waiting for the next step.
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds the solver pool that lactose-app builds in a shipped binary"
    )]
    fn stepped() -> Rapier {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let mut backend = Rapier::new(Arc::new(pool));
        let shapes = [
            Shape::sphere(Scalar::HALF),
            Shape::capsule(Scalar::from_f64(0.3), Scalar::from_f64(0.2)),
            Shape::cuboid(Vec3::new(Scalar::ONE, Scalar::HALF, Scalar::from_f64(0.25))),
        ];
        for (n, shape) in (1..).zip(shapes) {
            let at = Transform::at(Vec3::new(Scalar::from_u64(n * 3), Scalar::ONE, Scalar::ZERO));
            let collider = Collider::new(shape);
            let desc = BodyDesc {
                body: Some(RigidBody::dynamic()),
                collider: Some(&collider),
                transform: &at,
                velocity: None,
                frozen: false,
            };
            backend.create_body(key(n), &desc).unwrap();
        }
        let ground = Collider::new(Shape::cuboid(Vec3::new(
            Scalar::from_i32(20),
            Scalar::ONE,
            Scalar::from_i32(20),
        )));
        let at = Transform::at(Vec3::new(Scalar::ZERO, -Scalar::ONE, Scalar::ZERO));
        let desc = BodyDesc {
            body: None,
            collider: Some(&ground),
            transform: &at,
            velocity: None,
            frozen: false,
        };
        backend.create_body(key(10), &desc).unwrap();
        let input = StepInput {
            tick: Tick(1),
            dt: Scalar::from_f64(1.0 / 60.0),
            gravity: Vec3::new(Scalar::ZERO, Scalar::from_i32(-10), Scalar::ZERO),
            commands: &[],
        };
        backend.step(&input, &mut StepOutput::default());
        backend.remove_body(key(2));
        backend
    }

    #[test]
    fn the_collider_set_reads_back_byte_for_byte_with_its_shapes_from_their_values() {
        let backend = stepped();
        let mut bytes = Vec::new();
        backend.snapshot(&mut bytes).unwrap();
        let (world, book) = read(&bytes, &backend.pool).unwrap();
        // Rapier's own encoding of the set (change flags, mass properties and shapes
        // included) is what it was: the image's layout is the set's, and each rebuilt shape
        // is the one the value was built into, with no flag, wake or mass update.
        let encode = |colliders: &ColliderSet| postcard::to_allocvec(colliders).unwrap();
        assert_eq!(encode(&world.colliders), encode(&backend.world.colliders));
        assert_eq!(
            postcard::to_allocvec(&book).unwrap(),
            postcard::to_allocvec(&backend.book).unwrap()
        );
    }

    #[test]
    fn bookkeeping_that_disagrees_with_the_world_is_refused() {
        let backend = stepped();
        let mut wrong = backend.book.clone();
        let entry = wrong.bodies.values_mut().next().unwrap();
        entry.live = None;
        entry.transform = None;
        let written = |book: &Book| {
            let world = &backend.world;
            let image = WorldRef {
                book,
                gravity: world.gravity,
                integration_parameters: &world.integration_parameters,
                islands: &world.islands,
                broad_phase: &world.broad_phase,
                narrow_phase: &world.narrow_phase,
                bodies: &world.bodies,
                colliders: shapes_out(&world.colliders).unwrap(),
                impulse_joints: &world.impulse_joints,
                multibody_joints: &world.multibody_joints,
                soft_bodies: &world.soft_bodies,
            };
            let mut bytes = TAG.to_vec();
            bytes.extend(postcard::to_allocvec(&image).unwrap());
            bytes
        };
        assert!(read(&written(&backend.book), &backend.pool).is_ok());
        let error = read(&written(&wrong), &backend.pool).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        // A body the book does not name is refused too.
        let mut short = backend.book.clone();
        short.doomed_bodies.clear();
        assert!(read(&written(&short), &backend.pool).is_err());
    }
}
