//! Field visitors: the statically dispatched route from a typed value to code that treats
//! each kind of field differently.
//!
//! A field type chooses the method that matches its kind (double dispatch), so a visitor
//! never inspects a type at run time. A composite field (an array or a `#[value]` struct)
//! reports its leaves in declaration order under the field ID of the component field that
//! holds it.

use lactose_core::{EntityRef, FieldId};

use crate::field::{AssetReference, ScalarField};

/// Reads every non-padding field of a value. The world hasher is one.
pub trait FieldVisitor {
    /// An integer, `bool`, fieldless enum or identity newtype, as its bytes in memory. Every
    /// supported target is little-endian.
    fn bytes(&mut self, id: FieldId, bytes: &[u8]);

    /// A scalar. [`ScalarField::canonical_bits`] is what a hasher mixes, but `S` is whatever
    /// type some crate implemented [`ScalarField`] for: a hasher that needs canonical floats
    /// restricts `S` itself, as that trait's documentation says.
    fn scalar<S: ScalarField>(&mut self, id: FieldId, value: &S);

    /// A raw 32-bit float, from a presentation component. Its NaN sign and payload are not
    /// canonical, so the world hasher in `lactose-sim` refuses a type that reports one.
    fn f32(&mut self, id: FieldId, value: f32);

    /// A raw 64-bit float; see [`FieldVisitor::f32`].
    fn f64(&mut self, id: FieldId, value: f64);

    /// A reference to an entity.
    fn entity_ref(&mut self, id: FieldId, value: EntityRef);

    /// A reference to an asset.
    fn asset_ref<A: AssetReference>(&mut self, id: FieldId, value: &A);
}

/// Rewrites the reference fields of a value in place. The load path's remap pass is one.
pub trait ReferenceVisitor {
    /// A reference to an entity.
    fn entity_ref(&mut self, id: FieldId, value: &mut EntityRef);

    /// A reference to an asset, rewritten through [`AssetReference::set_asset_id`].
    fn asset_ref<A: AssetReference>(&mut self, id: FieldId, value: &mut A);
}
