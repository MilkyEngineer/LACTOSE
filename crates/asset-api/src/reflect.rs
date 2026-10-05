//! Makes [`AssetRef`] a typed reference field the reflection registry knows about
//! (architecture section 7), so the load path can remap it and the world hash can see it.

use lactose_core::FieldId;
use lactose_reflect::{AssetReference, Field, FieldKind, FieldVisitor, ReferenceVisitor};

use crate::id::{AssetId, AssetRef};

impl<T: 'static> Field for AssetRef<T> {
    const KIND: FieldKind = FieldKind::asset_ref::<Self>();
    // The asset type is a compile-time marker only; every `AssetRef` has one encoding.
    const NAME: &'static str = "AssetRef";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.asset_ref(id, self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, id: FieldId, visitor: &mut V) {
        visitor.asset_ref(id, self);
    }
}

impl<T: 'static> AssetReference for AssetRef<T> {
    fn asset_id(&self) -> u64 {
        self.id().to_bits()
    }

    fn set_asset_id(&mut self, id: u64) {
        *self = Self::new(AssetId::from_bits(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mesh;

    #[test]
    fn asset_ref_reports_and_rewrites_its_identity() {
        let mut field = AssetRef::<Mesh>::new(AssetId::from_bits(7));
        assert_eq!(field.asset_id(), 7);
        field.set_asset_id(0);
        assert!(field.is_none());
        assert!(matches!(<AssetRef<Mesh> as Field>::KIND, FieldKind::AssetRef { .. }));
    }
}
