//! Authored placement GUIDs and the network ID of a cooked entity.

use lactose_asset_api::AssetId;
use lactose_core::{NetId, xxh64};

/// The GUID an authoring tool gives a placement when it is created. It never changes
/// afterwards, whatever is edited around it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlacementGuid(pub [u8; 16]);

impl core::fmt::Debug for PlacementGuid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PlacementGuid({:032x})", u128::from_be_bytes(self.0))
    }
}

/// The [`NetId`] of a cooked entity (architecture section 7).
///
/// - `scene` is the authoring scene the placement was made in.
/// - `placement` is the GUID of that placement.
/// - `child_path` locates the entity inside what was placed: the GUIDs of the nested
///   placements leading to it, outermost first, and empty for the placed entity itself.
///
/// The cooker and every client call this one function, so a client derives the ID without
/// a message and a re-cook preserves it. The ID is the xxh64, seed zero, of the scene ID as
/// eight little-endian bytes, then the placement GUID, then each path GUID, with the top
/// bit replaced by the cooked tag. The cooker checks the IDs of a whole world for
/// collisions; [`Baseline::from_rows`](crate::Baseline::from_rows) checks one region.
#[must_use]
pub fn cooked_net_id(
    scene: AssetId,
    placement: PlacementGuid,
    child_path: &[PlacementGuid],
) -> NetId {
    let mut bytes = Vec::with_capacity(8 + 16 * (1 + child_path.len()));
    bytes.extend_from_slice(&scene.to_bits().to_le_bytes());
    bytes.extend_from_slice(&placement.0);
    for guid in child_path {
        bytes.extend_from_slice(&guid.0);
    }
    NetId::cooked(xxh64(&bytes, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: AssetId = AssetId::from_name("scenes/village.scene");
    const PLACEMENT: PlacementGuid = PlacementGuid(*b"0123456789abcdef");
    const CHILD: PlacementGuid = PlacementGuid(*b"fedcba9876543210");

    /// Golden values. If one changes, every cooked entity's identity has changed, and with
    /// it every save and every client's view of the cooked world.
    #[test]
    fn cooked_ids_are_pinned() {
        assert_eq!(SCENE.to_bits(), 0x04bf_0d4d_d210_b8a6);
        assert_eq!(cooked_net_id(SCENE, PLACEMENT, &[]).to_bits(), 0xc27c_67b0_63c5_866d);
        assert_eq!(cooked_net_id(SCENE, PLACEMENT, &[CHILD]).to_bits(), 0xb0dc_1bcf_3ad0_4d8d);
        assert_eq!(
            cooked_net_id(SCENE, PLACEMENT, &[CHILD, CHILD]).to_bits(),
            0x8a59_3975_b75e_65a1
        );
    }

    #[test]
    fn cooked_id_is_the_tagged_hash_of_the_documented_bytes() {
        let mut bytes = SCENE.to_bits().to_le_bytes().to_vec();
        bytes.extend_from_slice(b"0123456789abcdef");
        bytes.extend_from_slice(b"fedcba9876543210");
        let id = cooked_net_id(SCENE, PLACEMENT, &[CHILD]);
        assert_eq!(id.to_bits(), xxh64(&bytes, 0) | 1 << 63);
        assert!(id.is_cooked());
    }

    #[test]
    fn every_input_matters() {
        let base = cooked_net_id(SCENE, PLACEMENT, &[CHILD]);
        assert_ne!(
            base,
            cooked_net_id(AssetId::from_name("scenes/other.scene"), PLACEMENT, &[CHILD])
        );
        assert_ne!(base, cooked_net_id(SCENE, CHILD, &[CHILD]));
        assert_ne!(base, cooked_net_id(SCENE, PLACEMENT, &[PLACEMENT]));
        assert_ne!(base, cooked_net_id(SCENE, PLACEMENT, &[]));
        assert_ne!(base, cooked_net_id(SCENE, PLACEMENT, &[CHILD, CHILD]));
        assert_ne!(
            cooked_net_id(SCENE, PLACEMENT, &[CHILD]),
            cooked_net_id(SCENE, CHILD, &[PLACEMENT])
        );
    }

    #[test]
    fn guid_debug_is_hex() {
        let guid = PlacementGuid([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0xFF]);
        assert_eq!(format!("{guid:?}"), "PlacementGuid(000102030405060708090a0b0c0d0eff)");
    }
}
