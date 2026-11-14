//! The two store traits `app` inserts as `dyn` resources, and the values that cross them.
//!
//! The split is the determinism rule made structural. [`PublishedAssets`] answers only
//! questions whose answer is fixed by `(AssetId, AssetVersion)`, and is all a world inside
//! the boundary receives. [`AssetCatalog`] adds the questions whose answer depends on what
//! has been published so far, and is given only to code outside the boundary.

use lactose_core::{Error, ErrorKind};

use crate::budget::LoadPriority;
use crate::handle::{AssetHandle, UntypedHandle};
use crate::id::{Asset, AssetId, AssetRef};
use crate::version::{AssetSet, AssetVersion, VersionTable};

/// A missing or failed asset, as a value.
///
/// It is plain, comparable data so that it can be stamped with a tick, journaled and handed
/// to both worlds as an event (architecture sections 3 and 14). The message of the
/// underlying [`Error`] is logged by the store and does not travel with the failure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AssetFailure {
    /// The asset that failed.
    pub id: AssetId,
    /// The version that failed, or `None` when no version could be determined (the asset
    /// is unknown to the store's index).
    pub version: Option<AssetVersion>,
    /// What went wrong, at the granularity policy needs.
    pub kind: ErrorKind,
}

impl AssetFailure {
    /// The length of [`AssetFailure::to_bytes`].
    pub const ENCODED_LEN: usize = 17;

    /// The fixed encoding for journals and the wire: the asset ID as a little-endian
    /// `u64`, the version as a little-endian `u64` with zero for none, and the stable code
    /// of the kind ([`ErrorKind::to_u8`]).
    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut bytes = [0; Self::ENCODED_LEN];
        bytes[..8].copy_from_slice(&self.id.to_bits().to_le_bytes());
        let version = self.version.map_or(0, AssetVersion::to_bits);
        bytes[8..16].copy_from_slice(&version.to_le_bytes());
        bytes[16] = self.kind.to_u8();
        bytes
    }

    /// Decodes [`AssetFailure::to_bytes`]. `None` if the kind code is unknown to this
    /// build.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; Self::ENCODED_LEN]) -> Option<Self> {
        let word = |at: usize| {
            u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes of seventeen"))
        };
        Some(Self {
            id: AssetId::from_bits(word(0)),
            version: AssetVersion::from_bits(word(8)),
            kind: ErrorKind::from_u8(bytes[16])?,
        })
    }
}

impl core::fmt::Display for AssetFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "asset {:#018x}", self.id.to_bits())?;
        if let Some(version) = self.version {
            write!(f, " version {:#018x}", version.to_bits())?;
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
    /// The load failed. The failure stays the answer until the outcome of a later load
    /// replaces it: a request that retries does not change the answer, the retry's end does.
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

/// Access to published assets by exact version: the part of a store whose answers are
/// fixed by `(AssetId, AssetVersion)`.
///
/// This is the trait object a world inside the determinism boundary receives. Nothing here
/// can tell which version of an asset is current, so nothing here can leak load timing
/// into the simulation beyond "not ready yet", which the input sync point turns into an
/// input.
///
/// Every method may be called from any thread and none waits for IO or for a load; loading
/// happens on background workers and a caller asks again later.
///
/// What an implementation must uphold, beyond the signatures:
///
/// - A published `(AssetId, AssetVersion)` never changes and is never replaced, and its
///   content hashes to its version ([`CookedHeader::version_of`]). A store verifies that
///   on load, which [`ErasedLoader::load`] does for it; a mismatch is a failure value.
/// - Nothing is loaded unasked. A store loads a requested asset, a dependency of one, or
///   the new current version of an asset it has already loaded (hot reload). Which of
///   those loads it starts beyond the ones a request names is the store's own: whether and
///   when it reloads, and whether it goes on to an asset's other dependencies once one of
///   them has failed.
/// - An asset is published only once each of its declared dependencies has a current
///   published version. A dependency that cannot be published fails the dependant, before
///   any failure of the dependant's own, with the [`ErrorKind`] the dependency failed
///   with: of several, the one with the lowest [`AssetId`]. A dependency cycle among
///   assets that are not yet published fails every asset in it, and where following each
///   asset's first failing dependency leads round the cycle the kind is `InvalidData`.
///   For an index that does not change while the load is in flight, which failure a load
///   reports depends on neither the order of requests nor the order in which loads end.
///   A change of the index while a load is in flight can change what it waits for, and so
///   whether it fails at all.
/// - A failed load is reported as an [`AssetFailure`] naming the asset and the version
///   that failed: `NotFound` for an asset or version the store's sources do not hold,
///   `Unsupported` for a kind with no registered loader, `VersionMismatch` for another
///   cooked format, `InvalidData` for content that does not hash to its version, and
///   otherwise the kind the loader returned.
/// - A published version stays available through [`PublishedAssets::get`] at least as
///   long as any handle to it is alive.
/// - [`AssetId::NONE`] names no asset: a request for it does nothing, and a lookup of it
///   is `Failed` with `{ id: NONE, version: None, kind: NotFound }`, as a typed lookup
///   through an empty reference is.
///
/// `lactose_asset_api::conformance` checks these. Residency (unloading, budgets) is not
/// expressed yet; milestone 3 amends these traits with it.
///
/// [`CookedHeader::version_of`]: crate::CookedHeader::version_of
/// [`ErasedLoader::load`]: crate::ErasedLoader::load
pub trait PublishedAssets: Send + Sync {
    /// Asks the store to load and publish exactly `version` of `id` and, transitively, the
    /// current version of each declared dependency that has no published version yet.
    /// Requesting something already published or already loading does nothing, also when
    /// the index names that version: an already published version becomes current only
    /// through [`AssetCatalog::request_newest`] or a reload of the store's own. Requesting
    /// something that failed retries it.
    fn request(&self, id: AssetId, version: AssetVersion, priority: LoadPriority);

    /// The published asset `(id, version)`. Readers inside the determinism boundary take
    /// `version` from their region's [`VersionTable`]. `Failed` carries the failure of the
    /// last load of it to end.
    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle>;
}

/// The catalogue of a store: which version of each asset is the current one.
///
/// "Current" and "newest" mean the version the store's pack index publishes at the moment,
/// once it has loaded; versions have no order of their own. Every answer here depends on
/// what has been loaded and published so far, so this trait object is given only to code
/// outside the determinism boundary. What a region is to see is decided out there, with
/// [`AssetCatalog::resolve`], and enters the boundary as a [`VersionTable`] applied at the
/// input sync point and journaled (architecture sections 8.2 and 9.3).
///
/// A catalogue is also a [`PublishedAssets`], and a `&dyn AssetCatalog` coerces to a
/// `&dyn PublishedAssets` for handing into a boundary world.
///
/// What an implementation must uphold, beyond [`PublishedAssets`]:
///
/// - The current version of an asset changes only when a version of it is published while
///   the index names that version, whoever requested the load, or when a request for the
///   newest version, or a reload the store starts itself, finds the version the index
///   names already published. A failed load of the index's new version leaves the previous
///   current version in place.
/// - A change of the index changes no answer by itself. What follows from it (a request, a
///   reload) changes answers when a load ends, or when it makes an already published
///   version current.
/// - Publication is atomic with respect to every method here, and so is every other change
///   of an answer: a lookup never sees a change that [`AssetCatalog::generation`] does not
///   show yet, nor the reverse.
pub trait AssetCatalog: PublishedAssets {
    /// Asks the store to load and publish the version of `id` its pack index currently
    /// names, with the dependency, repeat and retry behaviour of
    /// [`PublishedAssets::request`], and to make that version current if it is published
    /// already. If the index names no version of `id`, the request is a failed load,
    /// `NotFound` with no version, recorded each time it is made.
    fn request_newest(&self, id: AssetId, priority: LoadPriority);

    /// The current published version of `id`.
    ///
    /// The answer is `Failed` only when the asset has no current published version. It
    /// then carries the failure of the last load to end of a version the index named when
    /// that load ended, whoever requested it, or of the last request for the newest
    /// version that found the index naming none, whichever came later. It stays the answer
    /// until another such load ends: a change of the index and a load in flight do not
    /// change it. An asset with neither a current version nor such a failure is `Pending`.
    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle>;

    /// The current version of every asset in `set` and of every asset those versions
    /// declare as a dependency, transitively, as one consistent table.
    ///
    /// `Ready` means the set is published, which is the condition for a region to enter
    /// `Simulated`; the table then holds everything the region can reach. The answer is
    /// `Failed`, with the failure of the lowest such [`AssetId`], if [`AssetCatalog::newest`]
    /// is `Failed` for any asset in the set; otherwise `Pending` if any has no published
    /// version; otherwise `Ready`. An empty set is ready.
    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable>;

    /// A counter that increases with every publication, every change of an asset's current
    /// version and every recorded failure, a retry that fails again included, and never
    /// otherwise. While it is unchanged, [`PublishedAssets::get`], [`AssetCatalog::newest`]
    /// and [`AssetCatalog::resolve`] answer as they did, so a caller resolves its regions
    /// again only after it moves; and since every retry ends in a move, a caller that
    /// retried a failure looks again when it moves and never waits for ever.
    fn generation(&self) -> u64;
}

impl dyn PublishedAssets + '_ {
    /// [`PublishedAssets::get`] through a typed reference.
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
}

impl dyn AssetCatalog + '_ {
    /// [`PublishedAssets::get`] through a typed reference, as on `dyn PublishedAssets`.
    pub fn get_typed<T: Asset>(
        &self,
        reference: AssetRef<T>,
        version: AssetVersion,
    ) -> Readiness<AssetHandle<T>> {
        let published: &dyn PublishedAssets = self;
        published.get_typed(reference, version)
    }

    /// [`AssetCatalog::newest`] through a typed reference.
    ///
    /// Fails with `ErrorKind::NotFound` for the empty reference and with
    /// `ErrorKind::InvalidData` if the asset is not of kind `T`.
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

    fn failure() -> AssetFailure {
        AssetFailure {
            id: AssetId::from_bits(0x2A),
            version: AssetVersion::from_bits(3),
            kind: ErrorKind::InvalidData,
        }
    }

    #[test]
    fn failure_converts_to_an_error_of_the_same_kind() {
        let error = Error::from(failure());
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(
            error.message(),
            "asset 0x000000000000002a version 0x0000000000000003 failed: InvalidData"
        );
        let unversioned = AssetFailure { version: None, ..failure() };
        assert_eq!(unversioned.to_string(), "asset 0x000000000000002a failed: InvalidData");
    }

    /// Golden bytes: a journal written by one build is replayed by another.
    #[test]
    fn failure_encoding_is_pinned() {
        let bytes = failure().to_bytes();
        assert_eq!(bytes, [0x2A, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 2]);
        assert_eq!(AssetFailure::from_bytes(&bytes), Some(failure()));

        let unversioned = AssetFailure { version: None, kind: ErrorKind::NotFound, ..failure() };
        let bytes = unversioned.to_bytes();
        assert_eq!(bytes, [0x2A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(AssetFailure::from_bytes(&bytes), Some(unversioned));

        let mut unknown_kind = bytes;
        unknown_kind[16] = 0;
        assert_eq!(AssetFailure::from_bytes(&unknown_kind), None);
        unknown_kind[16] = 200;
        assert_eq!(AssetFailure::from_bytes(&unknown_kind), None);
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
