//! A directory of sources cooked into one pack (`README.md`, "The command line").

use std::collections::BTreeMap;
use std::path::Path;

use lactose_asset::{PackBuilder, Storage};
use lactose_asset_api::{AssetId, AssetVersion, Processor};
use lactose_core::{Error, ErrorKind, Result};
use lactose_io::{EntryKind, Io};

use crate::skeleton::SkeletonProcessor;

/// The file name ending of a source the cooker cooks to a skeleton.
const SOURCE: &str = ".gltf";
/// The file name ending of a binary glTF, which the cooker refuses.
const BINARY: &str = ".glb";
/// The ending of the asset name of a cooked skeleton, in place of [`SOURCE`].
const SKELETON: &str = ".skeleton";
/// The file name ending of a pack: the one that places a file in the cooked pack set
/// (`lactose_asset::pack_set`).
const PACK: &str = ".pack";
/// How every asset is stored: zstd at a fixed level, so the bytes are a function of the
/// sources and of the build alone.
const STORAGE: Storage = Storage::Zstd { level: 3 };

/// One asset a cook wrote: its name, the ID that name hashes to (`AssetId::from_name`) and
/// the version the pack holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Published {
    /// The asset name: the source's path below the sources directory, `/`-separated, its
    /// `.gltf` ending replaced by `.skeleton`.
    pub name: String,
    /// `AssetId::from_name(&name)`.
    pub id: AssetId,
    /// The content hash of the cooked asset (`CookedHeader::version_of`).
    pub version: AssetVersion,
}

/// Cooks every `.gltf` file under `sources`, at any depth, to a skeleton and writes them
/// all as one pack at `pack`, replacing the file by rename and creating its directory.
/// Returns what it wrote, in byte order of the names.
///
/// Every request goes through `io`, so the future is driven by `Executor::block_on` or
/// awaited on an executor. Nothing is written unless every source cooks. Fails with:
/// `InvalidData` if `pack`'s file name does not end in `.pack`, for a file or directory
/// name below `sources` that is not UTF-8, for a source the processor refuses (the message
/// starts with the source's path below `sources`, then names the node) and for two names
/// that hash to one ID or a name that hashes to `AssetId::NONE`; `Unsupported` for a
/// `.glb` file or an entry that is neither a file nor a directory (a symbolic link among
/// them); `NotFound` if `sources` holds no `.gltf` file; and whatever `io` reports.
pub async fn cook(io: &Io, sources: &Path, pack: &Path) -> Result<Vec<Published>> {
    if !pack.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(PACK)) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("the pack's file name ends in `{PACK}`, which {} does not", pack.display()),
        ));
    }
    let names = list(io, sources).await?;
    if names.is_empty() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("no `{SOURCE}` file under {}", sources.display()),
        ));
    }
    let mut claimed = BTreeMap::new();
    let mut builder = PackBuilder::new();
    let mut published = Vec::with_capacity(names.len());
    for path in names {
        let bytes = io.read_file(sources.join(&path)).await.map_err(at(&path))?;
        let cooked = SkeletonProcessor.process(&bytes).map_err(at(&path))?;
        let name = format!("{}{SKELETON}", path.strip_suffix(SOURCE).expect("a source's name"));
        let id = claim(&mut claimed, &name)?;
        let version = builder.add(id, &cooked, STORAGE)?;
        published.push(Published { name, id, version });
    }
    let bytes = builder.finish()?;
    if let Some(directory) = pack.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        io.create_dir_all(directory).await.map_err(at(directory.display()))?;
    }
    io.replace(pack, bytes).await.map_err(|failed| at(pack.display())(failed.error))?;
    Ok(published)
}

/// The paths below `sources` of every source to cook, `/`-separated, in byte order: the
/// order the cook reads them in and adds them to the pack, whatever order the platform
/// lists a directory in.
async fn list(io: &Io, sources: &Path) -> Result<Vec<String>> {
    let mut sources_found = Vec::new();
    let mut directories = vec![String::new()];
    while let Some(directory) = directories.pop() {
        let listed = sources.join(&directory);
        for entry in io.read_dir(&listed).await.map_err(at(listed.display()))? {
            let Some(name) = entry.name.to_str() else {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("{directory}{}: a name that is not UTF-8", entry.name.display()),
                ));
            };
            let path = format!("{directory}{name}");
            match entry.kind {
                EntryKind::Directory => directories.push(format!("{path}/")),
                EntryKind::File if let Some(renamed) = lowercase_ending(name) => {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "{path}: an ending the cooker reads in lowercase alone, so that an \
                             asset's name is the same on every file system: rename it `{renamed}`"
                        ),
                    ));
                }
                EntryKind::File if name.ends_with(SOURCE) => sources_found.push(path),
                EntryKind::File if name.ends_with(BINARY) => {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!("{path}: a binary glTF, where the cooker reads `{SOURCE}`"),
                    ));
                }
                EntryKind::File => {}
                EntryKind::Other => {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!("{path}: neither a file nor a directory"),
                    ));
                }
            }
        }
    }
    sources_found.sort_unstable();
    Ok(sources_found)
}

/// `name` with its ending in lowercase, if it ends in `.gltf` or `.glb` written in another
/// case (`.GLTF`, `.Glb`).
fn lowercase_ending(name: &str) -> Option<String> {
    let (stem, ending) = name.rsplit_once('.')?;
    let lower = ending.to_ascii_lowercase();
    (lower != ending && [SOURCE, BINARY].contains(&format!(".{lower}").as_str()))
        .then(|| format!("{stem}.{lower}"))
}

/// The error of a request or a processor on `what`, a path, prefixed by it: the pool's
/// errors name no path.
fn at(what: impl core::fmt::Display) -> impl FnOnce(Error) -> Error {
    move |error| Error::new(error.kind(), format!("{what}: {}", error.message()))
}

/// The ID of `name`, claimed for it in `claimed`: refused if another name of the cook has
/// that ID, or if it is `AssetId::NONE`, which names no asset.
fn claim(claimed: &mut BTreeMap<AssetId, String>, name: &str) -> Result<AssetId> {
    let id = AssetId::from_name(name);
    if id.is_none() {
        return Err(Error::new(ErrorKind::InvalidData, format!("{name}: its ID is the empty ID")));
    }
    if let Some(other) = claimed.insert(id, name.to_owned()) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{name}: its ID {:#018x} is also {other}'s", id.to_bits()),
        ));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_claimed_once() {
        let mut claimed = BTreeMap::new();
        let id = claim(&mut claimed, "a.skeleton").unwrap();
        assert_eq!(id, AssetId::from_name("a.skeleton"));
        assert_eq!(claim(&mut claimed, "b.skeleton").unwrap(), AssetId::from_name("b.skeleton"));
        let error = claim(&mut claimed, "a.skeleton").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.message().contains("also a.skeleton's"), "{error}");
    }
}
