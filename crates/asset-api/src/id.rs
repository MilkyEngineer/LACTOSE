//! Asset identity: [`AssetId`], asset kinds and the typed reference field [`AssetRef`].

use core::cmp::Ordering;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use bytemuck::{Pod, Zeroable};
use lactose_core::xxh64;

/// The stable identity of an asset on disk, on the wire and in component fields: xxh64 of
/// its declared name.
///
/// The all-zero value is [`AssetId::NONE`], which names no asset. The cooker checks every
/// name it publishes for collisions and treats zero as taken, so a published asset never
/// has the ID zero and two published assets never share one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct AssetId(u64);

impl AssetId {
    /// The ID that names no asset.
    pub const NONE: Self = Self(0);

    /// The ID for a declared asset name such as `"props/barrel.mesh"`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }

    /// Rebuilds an ID from its wire or disk form.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The wire and disk form.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0
    }

    /// Whether this is [`AssetId::NONE`].
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Debug for AssetId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AssetId({:#018x})", self.0)
    }
}

/// Build, wire and disk key of an asset kind: xxh64 of its declared name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Pod, Zeroable)]
#[repr(transparent)]
pub struct AssetKind(pub u64);

impl AssetKind {
    /// The key for a declared kind name such as `"mesh"`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }
}

/// A runtime asset type: what a [`Loader`](crate::Loader) produces and a handle refers to.
///
/// A published asset is shared between threads and never mutated, hence the bounds.
pub trait Asset: Send + Sync + Sized + 'static {
    /// The stable key of this kind. Kind names are collision-checked where loaders are
    /// registered.
    const KIND: AssetKind;

    /// The version of this kind's cooked byte format. The cooker records it beside the
    /// cooked bytes, and a store refuses to hand bytes of another version to the loader.
    const FORMAT_VERSION: u32;
}

lactose_core::typed_plain_data! {
    /// A component field that refers to an asset of kind `T`, or to nothing.
    ///
    /// It is an [`AssetId`] and nothing more: plain data of the same size, typed at compile
    /// time only, and it keeps no asset alive. The all-zero value is the empty reference.
    /// Which version it resolves to is decided by the reader (see the crate documentation).
    pub struct AssetRef<T>(AssetId);
}

impl<T> AssetRef<T> {
    /// The empty reference.
    pub const NONE: Self = Self::new(AssetId::NONE);

    /// A reference to the asset `id`. Nothing checks here that `id` names an asset of kind
    /// `T`; a lookup through a mistyped reference fails with `ErrorKind::InvalidData`.
    #[must_use]
    pub const fn new(id: AssetId) -> Self {
        Self { value: id, marker: PhantomData }
    }

    /// The referenced ID, which is [`AssetId::NONE`] for the empty reference.
    #[must_use]
    pub const fn id(self) -> AssetId {
        self.value
    }

    /// Whether this is the empty reference.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.value.is_none()
    }
}

impl<T> Default for AssetRef<T> {
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> PartialEq for AssetRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T> Eq for AssetRef<T> {}

impl<T> PartialOrd for AssetRef<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for AssetRef<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl<T> Hash for AssetRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T> core::fmt::Debug for AssetRef<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AssetRef({:#018x})", self.value.to_bits())
    }
}

#[cfg(test)]
mod tests {
    use bytemuck::{CheckedBitPattern, NoUninit};

    use super::*;

    // Not plain data, like every real asset type.
    struct Mesh(#[expect(dead_code, reason = "only the type matters")] Vec<u8>);

    #[test]
    fn asset_id_is_the_hash_of_the_name() {
        const BARREL: AssetId = AssetId::from_name("props/barrel.mesh");
        assert_eq!(BARREL.to_bits(), xxh64(b"props/barrel.mesh", 0));
        assert_eq!(AssetId::from_bits(BARREL.to_bits()), BARREL);
        assert_ne!(BARREL, AssetId::from_name("props/crate.mesh"));
        assert!(!BARREL.is_none());
    }

    #[test]
    fn none_is_all_zero() {
        assert!(AssetId::NONE.is_none());
        assert_eq!(AssetId::default(), AssetId::NONE);
        assert_eq!(bytemuck::bytes_of(&AssetId::NONE), &[0u8; 8]);
        assert!(AssetRef::<Mesh>::NONE.is_none());
        assert_eq!(bytemuck::bytes_of(&AssetRef::<Mesh>::NONE), &[0u8; 8]);
        assert_eq!(AssetRef::<Mesh>::default(), AssetRef::NONE);
    }

    #[test]
    fn asset_ref_is_an_asset_id_in_memory() {
        let id = AssetId::from_name("props/barrel.mesh");
        let reference = AssetRef::<Mesh>::new(id);
        assert_eq!(reference.id(), id);
        assert_eq!(size_of::<AssetRef<Mesh>>(), size_of::<AssetId>());
        assert_eq!(align_of::<AssetRef<Mesh>>(), align_of::<AssetId>());
        assert_eq!(bytemuck::cast::<AssetRef<Mesh>, AssetId>(reference), id);
    }

    #[test]
    fn asset_ref_is_usable_as_a_component_field() {
        #[derive(Clone, Copy, NoUninit, CheckedBitPattern)]
        #[repr(C)]
        struct Renderable {
            mesh: AssetRef<Mesh>,
            fallback: AssetRef<Mesh>,
        }
        let mesh = AssetRef::new(AssetId::from_bits(7));
        let bytes = bytemuck::bytes_of(&Renderable { mesh, fallback: AssetRef::NONE }).to_vec();
        // A byte buffer has no alignment, as on the load and receive paths.
        let back: Renderable = bytemuck::checked::pod_read_unaligned(&bytes);
        assert_eq!((back.mesh, back.fallback), (mesh, AssetRef::NONE));
    }
}
