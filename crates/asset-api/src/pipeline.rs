//! The seam between the cooker, the store and every crate that defines an asset kind:
//! [`Processor`] on the cook side, [`Loader`] on the runtime side, and the cooked record
//! that travels between them.

use lactose_core::{Error, ErrorKind, Result};

use crate::handle::UntypedHandle;
use crate::id::{Asset, AssetId, AssetKind};
use crate::version::AssetVersion;

/// What the cooker records about one published asset version beside its bytes.
///
/// A store reads headers from its pack index, so it knows an asset's kind and dependencies
/// before it reads a byte of the asset itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CookedHeader {
    kind: AssetKind,
    format_version: u32,
    dependencies: Vec<AssetId>,
}

impl CookedHeader {
    /// A header for bytes of kind `kind` in cooked format `format_version` that depend on
    /// `dependencies`. The list is stored sorted, without duplicates and without
    /// [`AssetId::NONE`].
    #[must_use]
    pub fn new(
        kind: AssetKind,
        format_version: u32,
        dependencies: impl IntoIterator<Item = AssetId>,
    ) -> Self {
        let mut dependencies: Vec<_> =
            dependencies.into_iter().filter(|id| !id.is_none()).collect();
        dependencies.sort_unstable();
        dependencies.dedup();
        Self { kind, format_version, dependencies }
    }

    /// The asset kind the bytes belong to.
    #[must_use]
    pub fn kind(&self) -> AssetKind {
        self.kind
    }

    /// The cooked format version the bytes were written in.
    #[must_use]
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// The assets this one refers to, in ascending order. The cooker rejects cycles.
    #[must_use]
    pub fn dependencies(&self) -> &[AssetId] {
        &self.dependencies
    }
}

/// The output of a [`Processor`]: cooked bytes and their header.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CookedAsset {
    /// What the pack index records.
    pub header: CookedHeader,
    /// The cooked bytes, exactly as a [`Loader`] of the same kind and format reads them.
    pub bytes: Vec<u8>,
}

impl CookedAsset {
    /// Cooked bytes of kind `T` in `T`'s current format.
    #[must_use]
    pub fn new<T: Asset>(bytes: Vec<u8>, dependencies: impl IntoIterator<Item = AssetId>) -> Self {
        Self { header: CookedHeader::new(T::KIND, T::FORMAT_VERSION, dependencies), bytes }
    }
}

/// Turns source data into cooked bytes. The cook-side counterpart of [`Loader`].
///
/// A processor is a pure function of its input: the same source yields the same
/// [`CookedAsset`] on every host, which is what lets the cooker assign an
/// [`AssetVersion`] by comparing outputs.
pub trait Processor: Send + Sync + 'static {
    /// The asset kind this processor cooks.
    type Asset: Asset;

    /// Cooks `source`. The result is built with [`CookedAsset::new`] for
    /// [`Self::Asset`](Processor::Asset) and declares every asset the cooked bytes refer
    /// to; a reference is derived from the referenced asset's name with
    /// [`AssetId::from_name`].
    fn process(&self, source: &[u8]) -> Result<CookedAsset>;
}

/// Turns cooked bytes into a runtime asset.
///
/// Loading is a plain function over bytes. Dependencies are declared at cook time and
/// travel in the [`CookedHeader`], so a store learns them from its index and schedules the
/// whole closure at once; a loader never asks for another asset, never performs IO and
/// never waits, and so needs no executor. It runs on a background worker.
///
/// A loader does not embed its dependencies either. The asset it returns keeps
/// [`AssetRef`](crate::AssetRef)s to them, and a reader resolves those through its own
/// table, so one published version means the same thing whichever versions of its
/// dependencies a region pins.
///
/// `bytes` are always decompressed: the store asks `io` for a read-with-decompress, so no
/// loader names a compression format (`docs/verification.md` section 3).
pub trait Loader: Send + Sync + 'static {
    /// The asset kind this loader produces.
    type Asset: Asset;

    /// Decodes `bytes`. `dependencies` is the list the processor declared, in ascending
    /// order. Malformed bytes are `ErrorKind::InvalidData`, never a panic.
    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Self::Asset>;
}

type LoadFn =
    dyn Fn(AssetId, AssetVersion, &[AssetId], &[u8]) -> Result<UntypedHandle> + Send + Sync;

/// A [`Loader`] with its asset type erased, which is the form a store keeps one in.
pub struct ErasedLoader {
    kind: AssetKind,
    format_version: u32,
    load: Box<LoadFn>,
}

impl ErasedLoader {
    /// Erases `loader`.
    #[must_use]
    pub fn new<L: Loader>(loader: L) -> Self {
        Self {
            kind: L::Asset::KIND,
            format_version: L::Asset::FORMAT_VERSION,
            load: Box::new(move |id, version, dependencies, bytes| {
                let asset = loader.load(bytes, dependencies)?;
                Ok(UntypedHandle::new(id, version, asset))
            }),
        }
    }

    /// The asset kind this loader produces. A store keeps one loader per kind.
    #[must_use]
    pub fn kind(&self) -> AssetKind {
        self.kind
    }

    /// Loads the cooked bytes of `(id, version)`.
    ///
    /// Fails with `ErrorKind::InvalidData` if `header` is of another kind, with
    /// `ErrorKind::VersionMismatch` if it is of another cooked format, and otherwise with
    /// whatever the loader returns. The handle is not visible to readers until the store
    /// publishes it.
    pub fn load(
        &self,
        id: AssetId,
        version: AssetVersion,
        header: &CookedHeader,
        bytes: &[u8],
    ) -> Result<UntypedHandle> {
        if header.kind != self.kind {
            return Err(Error::new(ErrorKind::InvalidData, "cooked asset is of another kind"));
        }
        if header.format_version != self.format_version {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "cooked format {} does not match loader format {}",
                    header.format_version, self.format_version
                ),
            ));
        }
        (self.load)(id, version, &header.dependencies, bytes)
    }
}

impl core::fmt::Debug for ErasedLoader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ErasedLoader")
            .field("kind", &self.kind)
            .field("format_version", &self.format_version)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Text(String, usize);

    impl Asset for Text {
        const KIND: AssetKind = AssetKind::from_name("test.text");
        const FORMAT_VERSION: u32 = 3;
    }

    struct TextLoader;

    impl Loader for TextLoader {
        type Asset = Text;

        fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Text> {
            let text = core::str::from_utf8(bytes)
                .map_err(|_| Error::new(ErrorKind::InvalidData, "text is not UTF-8"))?;
            Ok(Text(text.to_owned(), dependencies.len()))
        }
    }

    struct UpperCase;

    impl Processor for UpperCase {
        type Asset = Text;

        fn process(&self, source: &[u8]) -> Result<CookedAsset> {
            Ok(CookedAsset::new::<Text>(source.to_ascii_uppercase(), [AssetId::from_name("font")]))
        }
    }

    const ID: AssetId = AssetId::from_bits(1);
    const VERSION: AssetVersion = AssetVersion(1);

    #[test]
    fn header_normalises_dependencies() {
        let id = AssetId::from_bits;
        let header = CookedHeader::new(Text::KIND, 3, [id(9), AssetId::NONE, id(2), id(9)]);
        assert_eq!(header.dependencies(), [id(2), id(9)]);
        assert_eq!((header.kind(), header.format_version()), (Text::KIND, 3));
    }

    #[test]
    fn processor_output_loads_through_the_erased_loader() {
        let cooked = UpperCase.process(b"hello").unwrap();
        let loader = ErasedLoader::new(TextLoader);
        assert_eq!(loader.kind(), Text::KIND);
        let handle = loader.load(ID, VERSION, &cooked.header, &cooked.bytes).unwrap();
        assert_eq!((handle.id(), handle.version(), handle.kind()), (ID, VERSION, Text::KIND));
        assert_eq!(*handle.downcast::<Text>().unwrap(), Text("HELLO".into(), 1));
    }

    #[test]
    fn erased_loader_rejects_other_kinds_formats_and_bad_bytes() {
        let loader = ErasedLoader::new(TextLoader);
        let kind_of =
            |header, bytes: &[u8]| loader.load(ID, VERSION, &header, bytes).unwrap_err().kind();
        let other_kind = CookedHeader::new(AssetKind::from_name("test.other"), 3, []);
        assert_eq!(kind_of(other_kind, b"x"), ErrorKind::InvalidData);
        let other_format = CookedHeader::new(Text::KIND, 2, []);
        assert_eq!(kind_of(other_format, b"x"), ErrorKind::VersionMismatch);
        let good = CookedHeader::new(Text::KIND, 3, []);
        assert_eq!(kind_of(good, &[0xFF]), ErrorKind::InvalidData);
    }
}
