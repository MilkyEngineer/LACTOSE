//! The store trait `app` inserts as a `dyn` resource, and the values that cross it.

use lactose_core::{Error, ErrorKind};

use crate::budget::LoadPriority;
use crate::handle::{AssetHandle, UntypedHandle};
use crate::id::{Asset, AssetId, AssetRef};
use crate::version::{AssetSet, AssetVersion, VersionTable};

/// A missing or failed asset, as a value.
///
/// It is plain, comparable data so that `sim` can stamp it with a tick and hand it to both
/// worlds as an event (architecture sections 3 and 14). The message of the underlying
/// [`Error`] is logged by the store and does not travel with the failure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AssetFailure {
    /// The asset that failed.
    pub id: AssetId,
    /// The version that failed, or `None` when no version could be determined (the asset
    /// is unknown, or the newest version was asked for and none exists).
    pub version: Option<AssetVersion>,
    /// What went wrong, at the granularity policy needs.
    pub kind: ErrorKind,
}

impl core::fmt::Display for AssetFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "asset {:#018x}", self.id.to_bits())?;
        if let Some(version) = self.version {
            write!(f, " version {}", version.0)?;
        }
        write!(f, " failed: {:?}", self.kind)
    }
}

impl From<AssetFailure> for Error {
    fn from(failure: AssetFailure) -> Self {
        Error::new(failure.kind, failure.to_string())
    }
}

/// The answer to a lookup: the value, not yet, or never.
#[derive(Clone, PartialEq, Eq, Debug)]
#[must_use]
pub enum Readiness<T> {
    /// Published and available.
    Ready(T),
    /// Not published and not failed. A lookup never starts a load, so this is also the
    /// answer for something nobody has requested.
    Pending,
    /// The load failed. The failure stays the answer until a later request retries.
    Failed(AssetFailure),
}

impl<T> Readiness<T> {
    /// The value if ready.
    pub fn ready(self) -> Option<T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Pending | Self::Failed(_) => None,
        }
    }

    /// Applies `map` to a ready value and leaves the other answers alone.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Readiness<U> {
        match self {
            Self::Ready(value) => Readiness::Ready(map(value)),
            Self::Pending => Readiness::Pending,
            Self::Failed(failure) => Readiness::Failed(failure),
        }
    }
}

/// A request to load one asset.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LoadRequest {
    /// The asset to load.
    pub id: AssetId,
    /// The exact version to load, or `None` for the newest version the store's sources
    /// hold. Readers inside the determinism boundary always name the version.
    pub version: Option<AssetVersion>,
    /// Which requests the store serves first when its budgets bind.
    pub priority: LoadPriority,
}

impl LoadRequest {
    /// A request for the newest version of `id` at the default priority.
    #[must_use]
    pub fn newest(id: AssetId) -> Self {
        Self { id, version: None, priority: LoadPriority::default() }
    }

    /// A request for exactly `version` of `id` at the default priority.
    #[must_use]
    pub fn exact(id: AssetId, version: AssetVersion) -> Self {
        Self { id, version: Some(version), priority: LoadPriority::default() }
    }
}

/// The asset store: versioned publication and lookup of loaded assets.
///
/// `app` inserts one instance as a `dyn AssetStore` resource (architecture section 4). The
/// crate documentation states who may call what. Every method may be called from any thread
/// and none waits for IO or for a load; loading happens on background workers and a caller
/// asks again later.
///
/// What an implementation must uphold, beyond the signatures:
///
/// - A published `(AssetId, AssetVersion)` never changes and is never replaced.
/// - Nothing is loaded unasked. A store loads a requested asset, a dependency of one, or a
///   newer version of an asset it has already loaded (hot reload), so the streaming system's
///   requests and budgets decide what is resident.
/// - An asset is published only once each of its declared dependencies has a published
///   version. A dependency that cannot be published fails the dependant with the
///   dependency's [`ErrorKind`].
/// - A failed load is reported as an [`AssetFailure`] naming the asset and the version
///   that failed: `NotFound` for an asset or version the store's sources do not hold,
///   `Unsupported` for a kind with no registered loader, `VersionMismatch` for another
///   cooked format, and otherwise the kind the loader returned.
/// - Publication is atomic with respect to every method here: [`AssetStore::resolve`]
///   never observes half of a group the store publishes together.
/// - A published version stays available through [`AssetStore::get`] at least as long as
///   any handle to it is alive. The newest published version of an asset stays available
///   until a newer one is published or the streaming system unloads the asset.
///
/// `lactose_asset_api::conformance` checks these.
pub trait AssetStore: Send + Sync {
    /// Asks the store to load and publish an asset and, transitively, the newest version
    /// of each declared dependency that has no published version yet. Requesting something
    /// already published or already loading does nothing; requesting something that failed
    /// retries it.
    fn request(&self, request: LoadRequest);

    /// The published asset `(id, version)`. This is the lookup for readers inside the
    /// determinism boundary, which take `version` from their region's [`VersionTable`].
    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle>;

    /// The newest published version of `id`. For readers outside the determinism boundary
    /// only: the answer depends on load timing.
    ///
    /// A failed load of a newer version does not hide an older published one. The answer
    /// is `Failed` only when no version is published and a load of the asset has failed.
    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle>;

    /// The newest published version of every asset in `set`, as one consistent table.
    ///
    /// This is how a region's table is chosen where versions are not dictated from
    /// elsewhere (the server; a client and a replay are given the table and pin it with
    /// [`PinnedTable::pin`](crate::PinnedTable::pin)). The caller is `sim`, at the input
    /// sync point, and the result is journaled as an input (architecture sections 8.2 and
    /// 9.3). `Ready` means the set is published, which is the condition for a region to
    /// enter `Simulated`.
    ///
    /// The answer is `Failed`, with the failure of the lowest such [`AssetId`], if
    /// [`AssetStore::newest`] is `Failed` for any asset in the set; otherwise `Pending` if
    /// any has no published version; otherwise `Ready`. An empty set is ready.
    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable>;
}

impl dyn AssetStore + '_ {
    /// [`AssetStore::get`] through a typed reference.
    ///
    /// Fails with `ErrorKind::NotFound` for the empty reference and with
    /// `ErrorKind::InvalidData` if the asset is not of kind `T`.
    pub fn get_typed<T: Asset>(
        &self,
        reference: AssetRef<T>,
        version: AssetVersion,
    ) -> Readiness<AssetHandle<T>> {
        if reference.is_none() {
            return Readiness::Failed(not_found(reference.id()));
        }
        typed(self.get(reference.id(), version))
    }

    /// [`AssetStore::newest`] through a typed reference, with the failures of
    /// `get_typed`. Outside the determinism boundary only.
    pub fn newest_typed<T: Asset>(&self, reference: AssetRef<T>) -> Readiness<AssetHandle<T>> {
        if reference.is_none() {
            return Readiness::Failed(not_found(reference.id()));
        }
        typed(self.newest(reference.id()))
    }
}

fn not_found(id: AssetId) -> AssetFailure {
    AssetFailure { id, version: None, kind: ErrorKind::NotFound }
}

fn typed<T: Asset>(untyped: Readiness<UntypedHandle>) -> Readiness<AssetHandle<T>> {
    match untyped {
        Readiness::Ready(handle) => match handle.downcast() {
            Ok(handle) => Readiness::Ready(handle),
            Err(failure) => Readiness::Failed(failure),
        },
        Readiness::Pending => Readiness::Pending,
        Readiness::Failed(failure) => Readiness::Failed(failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_converts_to_an_error_of_the_same_kind() {
        let failure = AssetFailure {
            id: AssetId::from_bits(0x2A),
            version: Some(AssetVersion(3)),
            kind: ErrorKind::InvalidData,
        };
        let error = Error::from(failure);
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(error.message(), "asset 0x000000000000002a version 3 failed: InvalidData");
        let unversioned = AssetFailure { version: None, ..failure };
        assert_eq!(unversioned.to_string(), "asset 0x000000000000002a failed: InvalidData");
    }

    #[test]
    fn readiness_maps_only_ready() {
        let failure = not_found(AssetId::from_bits(1));
        assert_eq!(Readiness::Ready(2).map(|n| n * 2), Readiness::Ready(4));
        assert_eq!(Readiness::<u32>::Pending.map(|n| n * 2), Readiness::Pending);
        assert_eq!(Readiness::<u32>::Failed(failure).map(|n| n * 2), Readiness::Failed(failure));
        assert_eq!(Readiness::Ready(2).ready(), Some(2));
        assert_eq!(Readiness::<u32>::Pending.ready(), None);
    }
}
