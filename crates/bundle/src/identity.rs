//! Authored GUIDs and the network ID of a cooked entity.

use lactose_core::{NetId, xxh64};

/// A GUID an authoring tool assigns once, when a scene, a placement or an entity inside a
/// prefab is created, and stores in the authored file. It never changes afterwards,
/// whatever is renamed, moved or edited around it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AuthoredGuid(pub [u8; 16]);

impl core::fmt::Debug for AuthoredGuid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AuthoredGuid({:032x})", u128::from_be_bytes(self.0))
    }
}

/// The [`NetId`] of a cooked entity (architecture section 7).
///
/// - `scene` is the GUID of the authoring scene the placement was made in. It is the
///   scene's own GUID, stored in the scene file, and not its name or path: renaming or
///   moving a scene file must not change the identity of anything in it.
/// - `placement` is the GUID of the placement within that scene.
/// - `child_path` locates the entity inside what was placed: the GUID of each nested
///   placement leading to it, outermost first, ending with the entity's own GUID inside
///   its prefab. It is empty only for a placement that is a single entity. Every entity
///   inside a multi-entity prefab therefore needs an authored GUID; an index or a name
///   will not do, because reordering or renaming siblings must not change an ID.
///
/// The cooker and every client call this one function, so a client derives the ID without
/// a message and a re-cook preserves it. The ID is the xxh64, seed zero, of the scene GUID,
/// then the placement GUID, then each path GUID, sixteen bytes each, with the top bit
/// replaced by the cooked tag. The cooker checks the IDs of a whole world for collisions;
/// [`Baseline::from_rows`](crate::Baseline::from_rows) checks one region.
#[must_use]
pub fn cooked_net_id(
    scene: AuthoredGuid,
    placement: AuthoredGuid,
    child_path: &[AuthoredGuid],
) -> NetId {
    let mut bytes = Vec::with_capacity(16 * (2 + child_path.len()));
    bytes.extend_from_slice(&scene.0);
    bytes.extend_from_slice(&placement.0);
    for guid in child_path {
        bytes.extend_from_slice(&guid.0);
    }
    NetId::cooked(xxh64(&bytes, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: AuthoredGuid = AuthoredGuid(*b"scene-guid-00001");
    const PLACEMENT: AuthoredGuid = AuthoredGuid(*b"0123456789abcdef");
    const CHILD: AuthoredGuid = AuthoredGuid(*b"fedcba9876543210");

    /// Golden values. If one changes, every cooked entity's identity has changed, and with
    /// it every save and every client's view of the cooked world.
    #[test]
    fn cooked_ids_are_pinned() {
        let id = |path: &[AuthoredGuid]| cooked_net_id(SCENE, PLACEMENT, path).to_bits();
        assert_eq!(id(&[]), 0xf8fb_b482_dd8e_8bed);
        assert_eq!(id(&[CHILD]), 0xb740_9840_5620_619d);
        assert_eq!(id(&[CHILD, CHILD]), 0xab1d_b4f1_a8da_91a7);
    }

    #[test]
    fn cooked_id_is_the_tagged_hash_of_the_documented_bytes() {
        let bytes = b"scene-guid-000010123456789abcdeffedcba9876543210";
        let id = cooked_net_id(SCENE, PLACEMENT, &[CHILD]);
        assert_eq!(id.to_bits(), xxh64(bytes, 0) | 1 << 63);
        assert!(id.is_cooked());
    }

    #[test]
    fn every_input_matters() {
        let base = cooked_net_id(SCENE, PLACEMENT, &[CHILD]);
        assert_ne!(base, cooked_net_id(CHILD, PLACEMENT, &[CHILD]));
        assert_ne!(base, cooked_net_id(SCENE, CHILD, &[CHILD]));
        assert_ne!(base, cooked_net_id(SCENE, PLACEMENT, &[PLACEMENT]));
        assert_ne!(base, cooked_net_id(SCENE, PLACEMENT, &[]));
        assert_ne!(base, cooked_net_id(SCENE, PLACEMENT, &[CHILD, CHILD]));
        assert_ne!(base, cooked_net_id(SCENE, CHILD, &[PLACEMENT]));
    }

    #[test]
    fn guid_debug_is_hex() {
        let guid = AuthoredGuid([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0xFF]);
        assert_eq!(format!("{guid:?}"), "AuthoredGuid(000102030405060708090a0b0c0d0eff)");
    }
}
