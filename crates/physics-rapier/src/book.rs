//! The bookkeeping: what the rows say each body and joint is, what the solver holds of
//! each, and the work waiting for the next step. It is part of the snapshot bytes, so every
//! type here encodes as a fixed function of its value (`README.md`, "The snapshot bytes").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::StableKey;
use lactose_physics_api::{BodyType, Collider, Joint, RigidBody, Transform, Velocity};
use rapier3d_f64::dynamics::{ImpulseJointHandle, RigidBodyHandle, RigidBodyType};
use rapier3d_f64::geometry::ColliderHandle;
use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::convert::{key_code, key_from_code, same};

/// A key, ordered as `StableKey` and encoded as its [`key_code`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct Key(pub(crate) StableKey);

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u128(key_code(self.0))
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = u128::deserialize(deserializer)?;
        key_from_code(code).map(Key).ok_or_else(|| de::Error::custom("not a key code"))
    }
}

/// A row value kept as its plain-data bytes. Every value stored here passed validation, so
/// it holds no NaN and its memory bytes are its canonical bytes on the little-endian
/// targets the engine supports; decoding checks the bit pattern.
#[derive(Clone, Copy)]
pub(crate) struct Plain<T>(pub(crate) T);

impl<T: NoUninit> PartialEq for Plain<T> {
    fn eq(&self, other: &Self) -> bool {
        same(&self.0, &other.0)
    }
}

impl<T: NoUninit> Serialize for Plain<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytemuck::bytes_of(&self.0))
    }
}

impl<'de, T: CheckedBitPattern + NoUninit> Deserialize<'de> for Plain<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bytes<T>(PhantomData<T>);
        impl<T: CheckedBitPattern + NoUninit> Visitor<'_> for Bytes<T> {
            type Value = Plain<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{} bytes of a row value", size_of::<T>())
            }
            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Plain<T>, E> {
                bytemuck::checked::try_pod_read_unaligned(bytes)
                    .map(Plain)
                    .map_err(|_| E::custom("not the bytes of a row value"))
            }
        }
        deserializer.deserialize_bytes(Bytes(PhantomData))
    }
}

/// What a body is: the effective `RigidBody` (`None` for a fixed collider), the collider,
/// and whether its region freezes it.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Desc {
    pub(crate) body: Option<Plain<RigidBody>>,
    pub(crate) collider: Option<Plain<Collider>>,
    pub(crate) frozen: bool,
}

impl Desc {
    /// What the solver does with the body: a frozen body or a collider alone is fixed, a
    /// kinematic body follows the poses it is given.
    pub(crate) fn body_type(&self) -> RigidBodyType {
        match self.body {
            Some(Plain(body)) if !self.frozen => match body.body_type {
                BodyType::Dynamic => RigidBodyType::Dynamic,
                BodyType::Kinematic => RigidBodyType::KinematicPositionBased,
                BodyType::Fixed => RigidBodyType::Fixed,
            },
            _ => RigidBodyType::Fixed,
        }
    }

    /// The mass the body is given, as [`MassPlan`].
    pub(crate) fn mass_plan(&self) -> MassPlan {
        let mass = self.body.map(|Plain(body)| body.mass);
        match (self.collider, mass) {
            (Some(_), Some(mass)) if mass > lactose_math::Scalar::ZERO => {
                MassPlan::ColliderMass(mass.to_f64())
            }
            (Some(Plain(collider)), _) => MassPlan::ColliderDensity(collider.density.to_f64()),
            (None, Some(mass)) if mass > lactose_math::Scalar::ZERO => {
                MassPlan::Additional(mass.to_f64())
            }
            (None, _) => MassPlan::Additional(1.0),
        }
    }
}

/// Where a body's mass comes from. A body with a collider takes `RigidBody::mass` as the
/// collider's mass when it is positive, the collider's density otherwise; a body without one
/// takes the mass, or one when it is zero, as a solid unit ball's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum MassPlan {
    ColliderMass(f64),
    ColliderDensity(f64),
    Additional(f64),
}

/// What the solver holds for a body.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Live {
    /// What the solver's body and collider were last made, while a call has changed the
    /// entry's `wanted` since; `None` when they are `wanted`, as every entry is after a step.
    pub(crate) held: Option<Desc>,
    pub(crate) body: RigidBodyHandle,
    pub(crate) collider: Option<ColliderHandle>,
}

/// One body or fixed collider.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct BodyEntry {
    /// What the rows say it is.
    pub(crate) wanted: Desc,
    /// What the solver holds, once the next step has put it there.
    pub(crate) live: Option<Live>,
    /// A transform written since the last step; the created transform until the body is
    /// live.
    pub(crate) transform: Option<Plain<Transform>>,
    /// A velocity written since the last step.
    pub(crate) velocity: Option<Plain<Velocity>>,
    /// The velocity a frozen body had, given back when it thaws.
    pub(crate) parked: Option<Plain<Velocity>>,
}

impl BodyEntry {
    /// Keeps what the solver holds before a call changes `wanted`.
    pub(crate) fn hold(&mut self) {
        if let Some(live) = &mut self.live
            && live.held.is_none()
        {
            live.held = Some(self.wanted);
        }
    }
}

/// One joint.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct JointEntry {
    pub(crate) joint: Plain<Joint>,
    pub(crate) body_a: Key,
    pub(crate) body_b: Key,
    /// The solver's joint, once the next step has created it.
    pub(crate) handle: Option<ImpulseJointHandle>,
}

/// The whole bookkeeping. Every call the sync makes lands here; the next step applies it to
/// the solver in ascending key order (removals, then bodies, then joints), so the solver's
/// state after a step does not depend on the order the calls came in.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Book {
    pub(crate) bodies: BTreeMap<Key, BodyEntry>,
    pub(crate) joints: BTreeMap<Key, JointEntry>,
    /// Bodies with work for the next step.
    pub(crate) dirty: BTreeSet<Key>,
    /// Solver bodies to remove at the next step, with their colliders and joints.
    pub(crate) doomed_bodies: BTreeMap<Key, Live>,
    /// Solver joints to remove at the next step.
    pub(crate) doomed_joints: BTreeMap<Key, ImpulseJointHandle>,
}
