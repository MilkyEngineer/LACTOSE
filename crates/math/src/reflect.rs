//! The `lactose-reflect` field impls of the math types, so components hold them: the
//! scalar as a [`ScalarField`] that states its canonical bits, and every composite type as a
//! struct of scalars with explicit field IDs, as `#[value]` would declare it.

use core::mem::offset_of;

use lactose_core::FieldId;
use lactose_reflect::{
    Field, FieldInfo, FieldKind, FieldVisitor, ReferenceVisitor, ScalarField, StructInfo,
};

use crate::{Mat3, Mat4, Quat, Scalar, Transform, Vec2, Vec3, Vec4};

impl Field for Scalar {
    const KIND: FieldKind = FieldKind::scalar::<Self>();
    // The tag names the width: a scalar of another width is another encoding, so switching
    // the scalar (architecture section 16) is a schema change the dump shows.
    const NAME: &'static str = if Self::BYTES == 8 { "Scalar64" } else { "Scalar32" };

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.scalar(id, self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl ScalarField for Scalar {
    #[allow(
        clippy::useless_conversion,
        reason = "the bits are 32 wide when the scalar is, and zero-extend then"
    )]
    fn canonical_bits(&self) -> u64 {
        self.to_canonical_bits().into()
    }
}

/// Implements `Field` for a struct of fields that are themselves `Field`s, with the layout
/// `$layout` naming its description: each field reports its leaves under the ID of the
/// component field that holds the struct, as a `#[value]` struct does.
macro_rules! struct_fields {
    ($($ty:ident as $layout:ident { $($field:ident: $field_ty:ty = $id:literal),+ })+) => {$(
        const $layout: StructInfo = StructInfo::new::<$ty>(
            stringify!($ty),
            &[$(FieldInfo::new::<$field_ty>(FieldId($id), stringify!($field), offset_of!($ty, $field))),+],
            &[],
        );

        impl Field for $ty {
            const KIND: FieldKind = FieldKind::Struct(&$layout);
            const NAME: &'static str = stringify!($ty);

            fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
                $(self.$field.visit(id, visitor);)+
            }

            fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
        }
    )+};
}

struct_fields! {
    Vec2 as VEC2 { x: Scalar = 1, y: Scalar = 2 }
    Vec3 as VEC3 { x: Scalar = 1, y: Scalar = 2, z: Scalar = 3 }
    Vec4 as VEC4 { x: Scalar = 1, y: Scalar = 2, z: Scalar = 3, w: Scalar = 4 }
    Quat as QUAT { x: Scalar = 1, y: Scalar = 2, z: Scalar = 3, w: Scalar = 4 }
    Mat3 as MAT3 { x_axis: Vec3 = 1, y_axis: Vec3 = 2, z_axis: Vec3 = 3 }
    Mat4 as MAT4 { x_axis: Vec4 = 1, y_axis: Vec4 = 2, z_axis: Vec4 = 3, w_axis: Vec4 = 4 }
    Transform as TRANSFORM { translation: Vec3 = 1, rotation: Quat = 2, scale: Vec3 = 3 }
}

#[cfg(test)]
mod tests {
    use lactose_core::EntityRef;
    use lactose_reflect::{AssetReference, FieldKind};

    use super::*;

    /// Collects what a visitor sees.
    #[derive(Default)]
    struct Leaves(Vec<(u16, u64)>);

    impl FieldVisitor for Leaves {
        fn bytes(&mut self, _: FieldId, _: &[u8]) {
            unreachable!("math types hold scalars only")
        }

        fn scalar<S: ScalarField>(&mut self, id: FieldId, value: &S) {
            self.0.push((id.0, value.canonical_bits()));
        }

        fn f32(&mut self, _: FieldId, _: f32) {
            unreachable!("math types hold scalars only")
        }

        fn f64(&mut self, _: FieldId, _: f64) {
            unreachable!("math types hold scalars only")
        }

        fn entity_ref(&mut self, _: FieldId, _: EntityRef) {
            unreachable!("math types hold scalars only")
        }

        fn asset_ref<A: AssetReference>(&mut self, _: FieldId, _: &A) {
            unreachable!("math types hold scalars only")
        }
    }

    #[test]
    fn layouts_tile_their_types() {
        for (layout, size) in [
            (&VEC2, size_of::<Vec2>()),
            (&VEC3, size_of::<Vec3>()),
            (&VEC4, size_of::<Vec4>()),
            (&QUAT, size_of::<Quat>()),
            (&MAT3, size_of::<Mat3>()),
            (&MAT4, size_of::<Mat4>()),
            (&TRANSFORM, size_of::<Transform>()),
        ] {
            assert_eq!(layout.size(), size, "{}", layout.name());
            let mut end = 0;
            for field in layout.fields() {
                assert_eq!(field.offset(), end, "{}", layout.name());
                end += field.size();
            }
            assert_eq!(end, size, "{}", layout.name());
        }
    }

    #[test]
    fn a_transform_reports_ten_canonical_scalars_under_its_own_id() {
        let nan = Scalar::ZERO / Scalar::ZERO;
        let transform =
            Transform::new(Vec3::new(nan, Scalar::ONE, Scalar::TWO), Quat::IDENTITY, Vec3::ONE);
        let mut leaves = Leaves::default();
        transform.visit(FieldId(7), &mut leaves);
        assert_eq!(leaves.0.len(), 10);
        assert!(leaves.0.iter().all(|&(id, _)| id == 7));
        // Every NaN reports the one canonical pattern.
        let other_nan = -nan;
        assert_eq!(leaves.0[0].1, other_nan.canonical_bits());
        assert_eq!(leaves.0[1].1, Scalar::ONE.canonical_bits());
    }

    #[test]
    fn kinds_and_tags() {
        assert!(matches!(Scalar::KIND, FieldKind::Scalar { .. }));
        assert_eq!(Scalar::NAME, "Scalar64");
        assert!(matches!(Mat4::KIND, FieldKind::Struct(info) if info.fields().len() == 4));
        assert_eq!(Transform::NAME, "Transform");
    }
}
