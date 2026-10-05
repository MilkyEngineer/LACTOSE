//! Handles to published assets and the pinned table a region reads through.

use core::any::Any;
use core::ops::Deref;
use std::sync::Arc;

use lactose_core::ErrorKind;

use crate::id::{Asset, AssetId, AssetKind, AssetRef, SimulationAsset};
use crate::store::{AssetFailure, PublishedAssets, Readiness};
use crate::version::{AssetVersion, VersionTable};

/// A loaded asset: shared ownership of one published `(AssetId, AssetVersion)`.
///
/// The handle dereferences to the asset. That version stays in memory for as long as any
/// handle to it exists, and it never changes, so a holder never observes a reload; it
/// observes one by asking the store again. Cloning is an atomic increment, which is why
/// per-entity code inside the boundary borrows from a [`PinnedTable`] instead.
///
/// It holds a pointer, so it is not plain data and cannot be a component field. A component
/// stores an [`AssetRef`].
pub struct AssetHandle<T> {
    id: AssetId,
    version: AssetVersion,
    asset: Arc<T>,
}

impl<T> AssetHandle<T> {
    /// The asset this handle refers to.
    #[must_use]
    pub fn id(&self) -> AssetId {
        self.id
    }

    /// The published version this handle keeps alive.
    #[must_use]
    pub fn version(&self) -> AssetVersion {
        self.version
    }
}

impl<T> Clone for AssetHandle<T> {
    fn clone(&self) -> Self {
        Self { id: self.id, version: self.version, asset: Arc::clone(&self.asset) }
    }
}

impl<T> Deref for AssetHandle<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.asset
    }
}

impl<T> core::fmt::Debug for AssetHandle<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AssetHandle({:#018x} at {:#018x})", self.id.to_bits(), self.version.to_bits())
    }
}

/// An [`AssetHandle`] whose asset type is known only at run time. This is what crosses the
/// `dyn` store traits; it keeps its version alive exactly as the typed handle does.
#[derive(Clone)]
pub struct UntypedHandle {
    id: AssetId,
    version: AssetVersion,
    kind: AssetKind,
    asset: Arc<dyn Any + Send + Sync>,
}

impl UntypedHandle {
    /// Wraps a loaded asset as version `version` of `id`. Stores and loaders call this;
    /// the result is visible to readers only once the store publishes it.
    #[must_use]
    pub fn new<T: Asset>(id: AssetId, version: AssetVersion, asset: T) -> Self {
        Self { id, version, kind: T::KIND, asset: Arc::new(asset) }
    }

    /// The asset this handle refers to.
    #[must_use]
    pub fn id(&self) -> AssetId {
        self.id
    }

    /// The published version this handle keeps alive.
    #[must_use]
    pub fn version(&self) -> AssetVersion {
        self.version
    }

    /// The kind of the asset.
    #[must_use]
    pub fn kind(&self) -> AssetKind {
        self.kind
    }

    /// The typed handle, or an `ErrorKind::InvalidData` failure if the asset is not a `T`.
    pub fn downcast<T: Asset>(self) -> Result<AssetHandle<T>, AssetFailure> {
        let Self { id, version, asset, .. } = self;
        match asset.downcast() {
            Ok(asset) => Ok(AssetHandle { id, version, asset }),
            Err(_) => Err(wrong_kind(id, version)),
        }
    }

    fn downcast_ref<T: Asset>(&self) -> Result<&T, AssetFailure> {
        self.asset.downcast_ref().ok_or(wrong_kind(self.id, self.version))
    }
}

impl core::fmt::Debug for UntypedHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UntypedHandle({:#018x} at {:#018x})", self.id.to_bits(), self.version.to_bits())
    }
}

fn wrong_kind(id: AssetId, version: AssetVersion) -> AssetFailure {
    AssetFailure { id, version: Some(version), kind: ErrorKind::InvalidData }
}

/// A region's [`VersionTable`] together with a handle to every version in it.
///
/// This is the only route by which code inside the determinism boundary reads an asset: a
/// lookup borrows from the table, names no version, touches no store and no atomic, and
/// keeps giving the same answer whatever is published meanwhile. A region owns one as a
/// region resource and replaces it only at the input sync point.
///
/// The table holds every asset the region declares, which keeps them all alive, but only
/// a [`SimulationAsset`] can be read out of it.
#[derive(Clone, Debug)]
pub struct PinnedTable {
    table: VersionTable,
    // In the order of `table.iter()`, which is ascending `AssetId`.
    assets: Vec<UntypedHandle>,
}

impl PinnedTable {
    /// Takes a handle to every `(AssetId, AssetVersion)` of `table` from `store`.
    ///
    /// The answer is `Failed` with the failure of the lowest failed [`AssetId`], otherwise
    /// `Pending` if any entry is not published yet, otherwise `Ready`.
    pub fn pin(store: &dyn PublishedAssets, table: &VersionTable) -> Readiness<Self> {
        let mut assets = Vec::with_capacity(table.len());
        let mut pending = false;
        for (id, version) in table.iter() {
            match store.get(id, version) {
                Readiness::Ready(handle) => assets.push(handle),
                Readiness::Pending => pending = true,
                Readiness::Failed(failure) => return Readiness::Failed(failure),
            }
        }
        if pending {
            Readiness::Pending
        } else {
            Readiness::Ready(Self { table: table.clone(), assets })
        }
    }

    /// The table this was pinned from.
    #[must_use]
    pub fn table(&self) -> &VersionTable {
        &self.table
    }

    /// The asset `reference` names, at this table's version.
    ///
    /// Fails with `ErrorKind::NotFound` if the reference is empty or names an asset outside
    /// the table (the region's asset set does not declare it), and with
    /// `ErrorKind::InvalidData` if the asset is not of kind `T`.
    pub fn get<T: SimulationAsset>(&self, reference: AssetRef<T>) -> Result<&T, AssetFailure> {
        let id = reference.id();
        match self.assets.binary_search_by_key(&id, UntypedHandle::id) {
            Ok(index) => self.assets[index].downcast_ref(),
            Err(_) => Err(AssetFailure { id, version: None, kind: ErrorKind::NotFound }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Text(String);

    impl Asset for Text {
        const KIND: AssetKind = AssetKind::from_name("test.text");
        const FORMAT_VERSION: u32 = 1;
    }

    struct Other;

    impl Asset for Other {
        const KIND: AssetKind = AssetKind::from_name("test.other");
        const FORMAT_VERSION: u32 = 1;
    }

    #[test]
    fn untyped_handle_downcasts_only_to_its_kind() {
        let id = AssetId::from_bits(5);
        let version = AssetVersion::from_bits(2).unwrap();
        let untyped = UntypedHandle::new(id, version, Text("hello".into()));
        assert_eq!((untyped.id(), untyped.version(), untyped.kind()), (id, version, Text::KIND));
        let typed = untyped.clone().downcast::<Text>().unwrap();
        assert_eq!(typed.0, "hello");
        assert_eq!((typed.id(), typed.version()), (id, version));
        assert_eq!(typed.clone().0, "hello");
        let failure = untyped.downcast::<Other>().unwrap_err();
        assert_eq!(
            failure,
            AssetFailure { id, version: Some(version), kind: ErrorKind::InvalidData }
        );
    }

    #[test]
    fn handles_cross_threads_and_are_not_plain_data() {
        fn assert_shared<T: Send + Sync + Clone + 'static>() {}
        assert_shared::<AssetHandle<Text>>();
        assert_shared::<UntypedHandle>();
        assert_shared::<PinnedTable>();
    }
}
