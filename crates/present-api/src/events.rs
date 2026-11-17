//! The request events (architecture sections 6.1 and 13): what a tick asks of
//! presentation. Each is a `lactose_ecs::PresentEvent`, emitted through the
//! `PresentEvents` parameter about an origin entity and keyed by (origin key, tick, kind,
//! sequence), so a re-simulated tick's re-emission is not delivered twice and a
//! mispredicted tick's is cancelled.
//!
//! An event carries where it happens, in simulation space, so presentation needs no row of
//! the origin entity to place it: the entity may be despawned by the time the event is
//! delivered, and a row that is only an instance has no transform in the presentation
//! snapshot.

use lactose_asset_api::AssetRef;
use lactose_ecs::{PresentEvent, PresentKind};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_reflect::{Padding, value};

use crate::components::pad;

/// The marker a reference to a cooked sound is typed by. The runtime type and its loader
/// are `lactose-audio`'s; see [`MeshAsset`](crate::MeshAsset) for how a reader re-types a
/// reference.
pub enum SoundAsset {}

/// The marker a reference to a cooked visual effect is typed by. The runtime type and its
/// loader are the renderer's; see [`MeshAsset`](crate::MeshAsset).
pub enum EffectAsset {}

/// A request to play a sound once, at a place.
///
/// The kind is `"present::audio"`. Two requests about one origin in one tick are two
/// events, told apart by their sequence. A cancelled request stops its sound if it still
/// plays. The listener is not an event: it is presentation's, per frame.
#[value]
#[derive(PartialEq, Debug)]
pub struct AudioRequest {
    /// Where the sound plays, in simulation space, at the tick it was emitted.
    #[field(id = 1)]
    pub position: Vec3,
    /// The sound. An empty reference plays nothing.
    #[field(id = 2)]
    pub sound: AssetRef<SoundAsset>,
}

impl AudioRequest {
    /// A request to play `sound` at `position`.
    #[must_use]
    pub const fn at(sound: AssetRef<SoundAsset>, position: Vec3) -> Self {
        Self { position, sound }
    }
}

impl PresentEvent for AudioRequest {
    const KIND: PresentKind = PresentKind::from_name("present::audio");
}

/// A request to spawn a visual effect once, at a placement. The effect does not follow the
/// origin entity.
///
/// The kind is `"present::vfx"`. A cancelled request removes its effect if it still runs.
#[value]
#[derive(PartialEq, Debug)]
pub struct VfxRequest {
    /// Where the effect's origin is, in simulation space, at the tick it was emitted.
    #[field(id = 1)]
    pub position: Vec3,
    /// How the effect is turned: a unit quaternion.
    #[field(id = 2)]
    pub rotation: Quat,
    /// The effect. An empty reference spawns nothing.
    #[field(id = 3)]
    pub effect: AssetRef<EffectAsset>,
}

impl VfxRequest {
    /// A request to spawn `effect` at `position`, turned by `rotation`.
    #[must_use]
    pub const fn at(effect: AssetRef<EffectAsset>, position: Vec3, rotation: Quat) -> Self {
        Self { position, rotation, effect }
    }
}

impl PresentEvent for VfxRequest {
    const KIND: PresentKind = PresentKind::from_name("present::vfx");
}

/// Which shape a [`DebugShape`] is, and so what its two points mean.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum DebugShapeKind {
    /// The segment from `a` to `b`.
    Line = 0,
    /// The edges of the axis-aligned box whose opposite corners are `a` and `b`.
    Box = 1,
    /// A sphere centred on `a` of radius `b.x`; `b.y` and `b.z` are zero.
    Sphere = 2,
}

/// A debug shape a tick asks to be drawn: how a system inside a world reaches the
/// [`DebugDraw`](crate::DebugDraw) buffer, which is the render side's and which no system
/// can name.
///
/// The kind is `"present::debug"`. A shape is drawn until presentation acquires a
/// snapshot of a later tick, so a system that wants a shape to stay emits it every tick; a
/// cancelled shape is dropped if it is still drawn. Presentation expands it into the
/// buffer with [`DebugDraw::shape`](crate::DebugDraw::shape).
#[value]
#[derive(PartialEq, Debug)]
pub struct DebugShape {
    /// The first point, in simulation space; see [`DebugShapeKind`].
    #[field(id = 1)]
    pub a: Vec3,
    /// The second point, in simulation space; see [`DebugShapeKind`].
    #[field(id = 2)]
    pub b: Vec3,
    /// The colour as `0xRRGGBBAA`: sRGB red, green and blue, then straight alpha.
    #[field(id = 3)]
    pub color: u32,
    /// Which shape.
    #[field(id = 4)]
    pub kind: DebugShapeKind,
    #[field(padding)]
    _pad: Padding<3>,
}

impl DebugShape {
    /// The segment from `from` to `to`.
    #[must_use]
    pub const fn line(from: Vec3, to: Vec3, color: u32) -> Self {
        Self { a: from, b: to, color, kind: DebugShapeKind::Line, _pad: pad() }
    }

    /// The edges of the axis-aligned box whose opposite corners are `min` and `max`.
    #[must_use]
    pub const fn aabb(min: Vec3, max: Vec3, color: u32) -> Self {
        Self { a: min, b: max, color, kind: DebugShapeKind::Box, _pad: pad() }
    }

    /// A sphere of `radius` metres centred on `centre`.
    #[must_use]
    pub const fn sphere(centre: Vec3, radius: Scalar, color: u32) -> Self {
        Self {
            a: centre,
            b: Vec3::new(radius, Scalar::ZERO, Scalar::ZERO),
            color,
            kind: DebugShapeKind::Sphere,
            _pad: pad(),
        }
    }
}

impl PresentEvent for DebugShape {
    const KIND: PresentKind = PresentKind::from_name("present::debug");
}
