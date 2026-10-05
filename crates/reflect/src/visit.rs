//! Field visitors: the statically dispatched route from a typed value to code that treats
//! each kind of field differently.
//!
//! A field type chooses the method that matches its kind (double dispatch), so a visitor
//! never inspects a type at run time. A composite field (an array or a `#[value]` struct)
//! reports its leaves in declaration order under the field ID of the component field that
//! holds it.

use lactose_core::{EntityRef, FieldId};

use crate::Field;

/// Reads every non-padding field of a value. The world hasher is one.
pub trait FieldVisitor {
    /// An integer, `bool`, fieldless enum or identity newtype, as its bytes in memory. Every
    /// supported target is little-endian.
    fn bytes(&mut self, id: FieldId, bytes: &[u8]);

    /// A 32-bit float. Its NaN sign and payload are not canonical.
    fn f32(&mut self, id: FieldId, value: f32);

    /// A 64-bit float. Its NaN sign and payload are not canonical.
    fn f64(&mut self, id: FieldId, value: f64);

    /// A reference to an entity.
    fn entity_ref(&mut self, id: FieldId, value: EntityRef);

    /// A reference to an asset. `A` is the one type whose kind is
    /// [`FieldKind::AssetRef`](crate::FieldKind::AssetRef), the asset reference of
    /// `lactose-asset-api`.
    fn asset_ref<A: Field>(&mut self, id: FieldId, value: &A);
}

/// Rewrites the reference fields of a value in place. The load path's remap pass is one.
pub trait ReferenceVisitor {
    /// A reference to an entity.
    fn entity_ref(&mut self, id: FieldId, value: &mut EntityRef);

    /// A reference to an asset; see [`FieldVisitor::asset_ref`]. A visitor that knows the
    /// concrete type converts with `bytemuck::checked::cast`, which is the identity here.
    fn asset_ref<A: Field>(&mut self, id: FieldId, value: &mut A);
}
